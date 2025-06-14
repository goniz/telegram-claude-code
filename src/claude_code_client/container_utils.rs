use bollard::Docker;
use bollard::container::{CreateContainerOptions, Config, RemoveContainerOptions};
use bollard::exec::{CreateExecOptions, StartExecOptions};
use bollard::image::CreateImageOptions;
use futures_util::StreamExt;

/// Container image used by the main application
/// This is the Codex Universal image that provides multi-language development environment
pub const MAIN_CONTAINER_IMAGE: &str = "ghcr.io/openai/codex-universal:latest";

/// Retry a container operation with exponential backoff
async fn retry_container_operation<F, Fut, T>(
    operation: F, 
    max_retries: u32
) -> Result<T, Box<dyn std::error::Error + Send + Sync>>
where
    F: Fn() -> Fut,
    Fut: std::future::Future<Output = Result<T, bollard::errors::Error>>,
{
    let mut last_error_msg = String::new();
    
    for attempt in 0..max_retries {
        match operation().await {
            Ok(result) => return Ok(result),
            Err(e) => {
                last_error_msg = e.to_string();
                
                // Check if this is a retryable error
                let error_msg = last_error_msg.to_lowercase();
                let is_retryable = error_msg.contains("409") || // Conflict
                                   error_msg.contains("timeout") ||
                                   error_msg.contains("network") ||
                                   error_msg.contains("temporary");
                
                if !is_retryable || attempt == max_retries - 1 {
                    return Err(e.into());
                }
                
                // Exponential backoff: 1s, 2s, 4s
                let delay = std::time::Duration::from_secs(1 << attempt);
                log::warn!("Container operation failed (attempt {}), retrying in {:?}: {}", attempt + 1, delay, last_error_msg);
                tokio::time::sleep(delay).await;
            }
        }
    }
    
    Err(last_error_msg.into())
}

/// Helper function to execute a command in a container
pub async fn exec_command_in_container(docker: &Docker, container_id: &str, command: Vec<String>) -> Result<String, Box<dyn std::error::Error + Send + Sync>> {
    let exec_config = CreateExecOptions {
        cmd: Some(command),
        attach_stdout: Some(true),
        attach_stderr: Some(true),
        ..Default::default()
    };

    let exec = docker.create_exec(container_id, exec_config).await?;
    
    let start_config = StartExecOptions {
        detach: false,
        ..Default::default()
    };

    let mut output = String::new();
    
    match docker.start_exec(&exec.id, Some(start_config)).await? {
        bollard::exec::StartExecResults::Attached { output: mut output_stream, .. } => {
            while let Some(Ok(msg)) = output_stream.next().await {
                match msg {
                    bollard::container::LogOutput::StdOut { message } => {
                        output.push_str(&String::from_utf8_lossy(&message));
                    }
                    bollard::container::LogOutput::StdErr { message } => {
                        output.push_str(&String::from_utf8_lossy(&message));
                    }
                    _ => {}
                }
            }
        }
        bollard::exec::StartExecResults::Detached => {
            return Err("Unexpected detached execution".into());
        }
    }

    Ok(output.trim().to_string())
}

/// Helper function to wait for container readiness
pub async fn wait_for_container_ready(docker: &Docker, container_id: &str) -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
    log::info!("Waiting for container to be ready...");
    
    // First check if container is still running
    let inspect_result = docker.inspect_container(container_id, None).await?;
    if let Some(state) = inspect_result.state {
        if !state.running.unwrap_or(false) {
            let exit_code = state.exit_code.unwrap_or(-1);
            let error = state.error.unwrap_or_else(|| "Unknown error".to_string());
            return Err(format!("Container exited during startup (exit code: {}, error: {})", exit_code, error).into());
        }
    }
    
    // Try up to 45 times with progressive delays (up to ~90 seconds total)
    for attempt in 1..=45 {
        // Check container state first
        match docker.inspect_container(container_id, None).await {
            Ok(inspect) => {
                if let Some(state) = inspect.state {
                    if !state.running.unwrap_or(false) {
                        let exit_code = state.exit_code.unwrap_or(-1);
                        let error = state.error.unwrap_or_else(|| "Unknown error".to_string());
                        return Err(format!("Container stopped during readiness check (exit code: {}, error: {})", exit_code, error).into());
                    }
                }
            }
            Err(e) => {
                return Err(format!("Failed to inspect container during readiness check: {}", e).into());
            }
        }
        
        // Try a basic connectivity test
        match exec_command_in_container(docker, container_id, vec!["echo".to_string(), "ready".to_string()]).await {
            Ok(_) => {
                log::info!("Container is ready after {} attempts", attempt);
                return Ok(());
            }
            Err(e) => {
                log::debug!("Container readiness check failed (attempt {}): {}", attempt, e);
                
                // Progressive delay: start with 1s, increase to 3s for later attempts
                let delay = if attempt <= 10 { 1 } else if attempt <= 25 { 2 } else { 3 };
                tokio::time::sleep(tokio::time::Duration::from_secs(delay)).await;
            }
        }
    }
    
    Err("Container failed to become ready after 90+ seconds".into())
}

/// Function to start a new coding session container
pub async fn start_coding_session(docker: &Docker, container_name: &str, claude_config: crate::ClaudeCodeConfig) -> Result<crate::ClaudeCodeClient, Box<dyn std::error::Error + Send + Sync>> {
    use crate::ClaudeCodeClient;
    
    // First, try to remove any existing container with the same name
    let _ = clear_coding_session(docker, container_name).await;
    
    // Pull the image if it doesn't exist
    let create_image_options = CreateImageOptions {
        from_image: MAIN_CONTAINER_IMAGE,
        ..Default::default()
    };
    
    let mut pull_stream = docker.create_image(Some(create_image_options), None, None);
    while let Some(result) = pull_stream.next().await {
        match result {
            Ok(_) => {}, // Image pull progress, continue
            Err(e) => {
                log::warn!("Image pull warning (might already exist): {}", e);
                break; // Continue even if pull fails (image might already exist)
            }
        }
    }
    
    let config = Config {
        image: Some(MAIN_CONTAINER_IMAGE),
        working_dir: Some("/workspace"),
        tty: Some(true),
        attach_stdin: Some(true),
        attach_stdout: Some(true),
        attach_stderr: Some(true),
        env: Some(vec![
            "CODEX_ENV_PYTHON_VERSION=3.12",
            "CODEX_ENV_NODE_VERSION=20", 
            "CODEX_ENV_RUST_VERSION=1.87.0",
            "CODEX_ENV_GO_VERSION=1.23.8",
            "CODEX_ENV_SWIFT_VERSION=6.1",
        ]),
        cmd: Some(vec!["/bin/bash"]),
        ..Default::default()
    };
    
    // Create container with retry logic to handle 409 conflicts
    let container = retry_container_operation(|| {
        let container_name = container_name.to_string();
        let config = config.clone();
        let docker = docker.clone();
        async move {
            let options = CreateContainerOptions {
                name: container_name.as_str(),
                ..Default::default()
            };
            docker.create_container(Some(options), config).await
        }
    }, 3).await?;
    
    // Start container with retry logic
    retry_container_operation(|| {
        let container_id = container.id.clone();
        let docker = docker.clone();
        async move {
            docker.start_container::<String>(&container_id, None).await
        }
    }, 3).await?;
    
    // Wait for container to be ready
    wait_for_container_ready(docker, &container.id).await?;
    
    // Create Claude Code client and install
    let claude_client = ClaudeCodeClient::new(docker.clone(), container.id.clone(), claude_config);
    claude_client.install().await?;
    
    Ok(claude_client)
}

/// Function to clear (stop and remove) a coding session container
pub async fn clear_coding_session(docker: &Docker, container_name: &str) -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
    // Try to stop the container first (ignore errors if it's not running)
    let _ = docker.stop_container(container_name, None).await;
    
    // Remove the container
    let remove_options = RemoveContainerOptions {
        force: true,
        ..Default::default()
    };
    
    match docker.remove_container(container_name, Some(remove_options)).await {
        Ok(()) => Ok(()),
        Err(e) => {
            // If container doesn't exist, that's fine
            if e.to_string().contains("No such container") {
                Ok(())
            } else {
                Err(e.into())
            }
        }
    }
}

/// Create a test container using the same configuration as the main application
/// This is a lightweight version for tests that need a container but not Claude Code installation
#[allow(dead_code)]
pub async fn create_test_container(docker: &Docker, container_name: &str) -> Result<String, Box<dyn std::error::Error + Send + Sync>> {
    // Remove any existing container with the same name
    let _ = clear_coding_session(docker, container_name).await;

    // Pull the image if it doesn't exist
    let create_image_options = CreateImageOptions {
        from_image: MAIN_CONTAINER_IMAGE,
        ..Default::default()
    };
    
    let mut pull_stream = docker.create_image(Some(create_image_options), None, None);
    while let Some(result) = pull_stream.next().await {
        match result {
            Ok(_) => {}, // Image pull progress, continue
            Err(e) => {
                log::warn!("Image pull warning (might already exist): {}", e);
                break; // Continue even if pull fails (image might already exist)
            }
        }
    }

    let options = CreateContainerOptions {
        name: container_name,
        ..Default::default()
    };

    let config = Config {
        image: Some(MAIN_CONTAINER_IMAGE),
        working_dir: Some("/workspace"),
        tty: Some(true),
        attach_stdin: Some(true),
        attach_stdout: Some(true),
        attach_stderr: Some(true),
        env: Some(vec![
            "CODEX_ENV_PYTHON_VERSION=3.12",
            "CODEX_ENV_NODE_VERSION=20", 
            "CODEX_ENV_RUST_VERSION=1.87.0",
            "CODEX_ENV_GO_VERSION=1.23.8",
            "CODEX_ENV_SWIFT_VERSION=6.1",
        ]),
        cmd: Some(vec!["/bin/bash"]),
        ..Default::default()
    };

    let container = docker.create_container(Some(options), config).await?;
    docker.start_container::<String>(&container.id, None).await?;

    // Wait for container to be ready
    wait_for_container_ready(docker, &container.id).await?;

    Ok(container.id)
}