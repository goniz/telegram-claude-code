use bollard::container::{Config, CreateContainerOptions, RemoveContainerOptions, ListContainersOptions};
use bollard::exec::{CreateExecOptions, StartExecOptions};
use bollard::image::CreateImageOptions;
use bollard::Docker;
use futures_util::StreamExt;
use std::collections::HashMap;

/// Container image used by the main application
/// This is the Claude Code runtime image that provides multi-language development environment with Claude Code pre-installed
pub const MAIN_CONTAINER_IMAGE: &str = "ghcr.io/goniz/telegram-claude-code-runtime:main";

/// Helper function to execute a command in a container
pub async fn exec_command_in_container(
    docker: &Docker,
    container_id: &str,
    command: Vec<String>,
) -> Result<String, Box<dyn std::error::Error + Send + Sync>> {
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
        bollard::exec::StartExecResults::Attached {
            output: mut output_stream,
            ..
        } => {
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
pub async fn wait_for_container_ready(
    docker: &Docker,
    container_id: &str,
) -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
    log::info!("Waiting for container to be ready...");

    // Try up to 30 times with 1 second delays (30 seconds total)
    for attempt in 1..=30 {
        match exec_command_in_container(
            docker,
            container_id,
            vec!["echo".to_string(), "ready".to_string()],
        )
        .await
        {
            Ok(_) => {
                log::info!("Container is ready after {} attempts", attempt);
                return Ok(());
            }
            Err(e) => {
                log::debug!(
                    "Container readiness check failed (attempt {}): {}",
                    attempt,
                    e
                );
                tokio::time::sleep(tokio::time::Duration::from_secs(1)).await;
            }
        }
    }

    Err("Container failed to become ready after 30 seconds".into())
}

/// Function to start a new coding session container
pub async fn start_coding_session(
    docker: &Docker,
    container_name: &str,
    claude_config: crate::ClaudeCodeConfig,
) -> Result<crate::ClaudeCodeClient, Box<dyn std::error::Error + Send + Sync>> {
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
            Ok(_) => {} // Image pull progress, continue
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
            "CODEX_ENV_NODE_VERSION=22",
            "CODEX_ENV_RUST_VERSION=1.87.0",
            "CODEX_ENV_GO_VERSION=1.23.8",
        ]),
        // Override the default command to prevent interactive shell hang
        // Run setup script then keep container alive with sleep
        cmd: Some(vec!["-c", "sleep infinity"]),
        // Set stop timeout to ensure graceful shutdown
        stop_timeout: Some(3),
        ..Default::default()
    };

    let container = docker.create_container(Some(options), config).await?;
    docker
        .start_container::<String>(&container.id, None)
        .await?;

    // Wait for container to be ready
    wait_for_container_ready(docker, &container.id).await?;

    // Create Claude Code client (Claude Code is pre-installed in the runtime image)
    let claude_client = ClaudeCodeClient::new(docker.clone(), container.id.clone(), claude_config);

    Ok(claude_client)
}

/// Function to clear (stop and remove) a coding session container
pub async fn clear_coding_session(
    docker: &Docker,
    container_name: &str,
) -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
    // Try to stop the container first (ignore errors if it's not running)
    let _ = docker.stop_container(container_name, None).await;

    // Remove the container
    let remove_options = RemoveContainerOptions {
        force: true,
        ..Default::default()
    };

    match docker
        .remove_container(container_name, Some(remove_options))
        .await
    {
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
pub async fn create_test_container(
    docker: &Docker,
    container_name: &str,
) -> Result<String, Box<dyn std::error::Error + Send + Sync>> {
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
            Ok(_) => {} // Image pull progress, continue
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
            "CODEX_ENV_NODE_VERSION=22",
            "CODEX_ENV_RUST_VERSION=1.87.0",
            "CODEX_ENV_GO_VERSION=1.23.8",
        ]),
        cmd: Some(vec!["/bin/bash"]),
        ..Default::default()
    };

    let container = docker.create_container(Some(options), config).await?;
    docker
        .start_container::<String>(&container.id, None)
        .await?;

    // Wait for container to be ready
    wait_for_container_ready(docker, &container.id).await?;

    Ok(container.id)
}

/// Clear all existing session containers on startup
/// This function finds and removes all containers with names matching the pattern "coding-session-*"
pub async fn clear_all_session_containers(
    docker: &Docker,
) -> Result<usize, Box<dyn std::error::Error + Send + Sync>> {
    log::info!("Clearing all existing session containers...");
    
    let mut filters = HashMap::new();
    filters.insert("name".to_string(), vec!["coding-session-".to_string()]);
    
    let list_options = ListContainersOptions {
        all: true,
        filters,
        ..Default::default()
    };
    
    let containers = docker.list_containers(Some(list_options)).await?;
    let mut cleared_count = 0;
    
    for container in containers {
        if let Some(names) = &container.names {
            for name in names {
                // Remove the leading "/" from container name
                let clean_name = name.strip_prefix('/').unwrap_or(name);
                if clean_name.starts_with("coding-session-") {
                    log::info!("Clearing existing session container: {}", clean_name);
                    match clear_coding_session(docker, clean_name).await {
                        Ok(()) => {
                            cleared_count += 1;
                            log::info!("Successfully cleared container: {}", clean_name);
                        }
                        Err(e) => {
                            log::warn!("Failed to clear container {}: {}", clean_name, e);
                        }
                    }
                    break; // Only process the first matching name
                }
            }
        }
    }
    
    log::info!("Cleared {} existing session containers", cleared_count);
    Ok(cleared_count)
}
