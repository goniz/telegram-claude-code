use bollard::container::{Config, CreateContainerOptions, RemoveContainerOptions, ListContainersOptions};
use bollard::exec::{CreateExecOptions, StartExecOptions};
use bollard::image::CreateImageOptions;
use bollard::volume::CreateVolumeOptions;
use bollard::models::{Mount, MountTypeEnum, HostConfig};
use bollard::Docker;
use futures_util::StreamExt;
use std::collections::HashMap;

/// Configuration for coding container behavior
#[derive(Debug, Clone)]
pub struct CodingContainerConfig {
    pub persistent_volume_key: Option<String>,
}

impl Default for CodingContainerConfig {
    fn default() -> Self {
        Self {
            persistent_volume_key: None,
        }
    }
}

/// Container image used by the main application
/// This is the Claude Code runtime image that provides multi-language development environment with Claude Code pre-installed
pub const MAIN_CONTAINER_IMAGE: &str = "ghcr.io/goniz/telegram-claude-code-runtime:main";

/// Generate a volume name for a user's persistent authentication data
pub fn generate_volume_name(volume_key: &str) -> String {
    format!("dev-session-claude-{}", volume_key)
}

/// Prepare environment variables for container creation with dynamic GH_TOKEN support
/// Includes common development environment variables and optionally GH_TOKEN
fn prepare_container_env_vars_dynamic() -> Vec<String> {
    let mut env_vars = vec![
        "CODEX_ENV_PYTHON_VERSION=3.12".to_string(),
        "CODEX_ENV_NODE_VERSION=22".to_string(),
        "CODEX_ENV_RUST_VERSION=1.87.0".to_string(),
        "CODEX_ENV_GO_VERSION=1.23.8".to_string(),
    ];
    
    if let Ok(gh_token) = std::env::var("GH_TOKEN") {
        env_vars.push(format!("GH_TOKEN={}", gh_token));
    }
    
    env_vars
}

/// Create or get existing volume for user authentication persistence
/// This function is idempotent - it will not fail if the volume already exists
pub async fn ensure_user_volume(
    docker: &Docker,
    volume_key: &str,
) -> Result<String, Box<dyn std::error::Error + Send + Sync>> {
    let volume_name = generate_volume_name(volume_key);
    
    // Create the volume - Docker will return an error if it already exists
    let create_options = CreateVolumeOptions {
        name: volume_name.clone(),
        driver: "local".to_string(),
        driver_opts: HashMap::new(),
        labels: {
            let mut labels = HashMap::new();
            labels.insert("created_by".to_string(), "telegram-claude-code".to_string());
            labels.insert("volume_key".to_string(), volume_key.to_string());
            labels.insert("purpose".to_string(), "authentication_persistence".to_string());
            labels
        },
    };
    
    match docker.create_volume(create_options).await {
        Ok(_) => {
            log::info!("Created new volume '{}' for key {}", volume_name, volume_key);
            Ok(volume_name)
        }
        Err(e) => {
            // Check if the error is because the volume already exists
            if e.to_string().contains("already exists") || e.to_string().contains("Conflict") {
                log::info!("Volume '{}' already exists, reusing", volume_name);
                Ok(volume_name)
            } else {
                Err(format!("Failed to create volume '{}': {}", volume_name, e).into())
            }
        }
    }
}

/// Create volume mounts for authentication persistence
/// Maps volume paths to container paths for Claude and GitHub authentication
pub fn create_auth_mounts(volume_name: &str) -> Vec<Mount> {
    vec![
        // Mount volume to a base directory, then use symbolic links or init commands
        // to set up the proper structure
        Mount {
            target: Some("/volume_data".to_string()),
            source: Some(volume_name.to_string()),
            typ: Some(MountTypeEnum::VOLUME),
            read_only: Some(false),
            consistency: Some("default".to_string()),
            ..Default::default()
        },
    ]
}

/// Initialize Claude configuration in the container
/// This sets up the Claude configuration files regardless of volume usage
async fn init_claude_configuration(
    docker: &Docker,
    container_id: &str,
) -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
    log::info!("Initializing Claude configuration...");
    
    // Initialize .claude.json with required configuration
    exec_command_in_container(
        docker,
        container_id,
        vec!["sh".to_string(), "-c".to_string(), "echo '{ \"hasCompletedOnboarding\": true }' > /root/.claude.json".to_string()]
    ).await
    .map_err(|e| format!("Failed to initialize .claude.json: {}", e))?;
    
    // Set Claude configuration for trust dialog
    exec_command_in_container(
        docker,
        container_id,
        vec!["sh".to_string(), "-c".to_string(), "/opt/entrypoint.sh -c \"nvm use default && claude config set hasTrustDialogAccepted true\"".to_string()]
    ).await
    .map_err(|e| format!("Failed to set Claude trust dialog configuration: {}", e))?;
    
    log::info!("Claude configuration initialization completed");
    Ok(())
}

/// Initialize volume structure by creating symbolic links to persistent storage
/// This sets up the authentication directories to point to volume storage
async fn init_volume_structure(
    docker: &Docker,
    container_id: &str,
) -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
    // Create the persistent directories in the volume if they don't exist
    let basic_commands = vec![
        // Create volume directories
        vec!["mkdir".to_string(), "-p".to_string(), "/volume_data/claude".to_string()],
        vec!["mkdir".to_string(), "-p".to_string(), "/volume_data/gh".to_string()],
        
        // Create parent directories for symlinks
        vec!["mkdir".to_string(), "-p".to_string(), "/root/.config".to_string()],
        
        // Remove existing directories/files if they exist (they might be empty from container creation)
        vec!["rm".to_string(), "-rf".to_string(), "/root/.claude".to_string()],
        vec!["rm".to_string(), "-rf".to_string(), "/root/.config/gh".to_string()],
    ];
    
    for command in basic_commands {
        exec_command_in_container(docker, container_id, command.clone()).await
            .map_err(|e| format!("Failed to initialize volume directory structure: {}", e))?;
    }
    
    // Handle .claude.json file for volume persistence
    // First check if it already exists in the volume (from previous sessions)
    let volume_claude_json_check = exec_command_in_container(
        docker, 
        container_id, 
        vec!["test".to_string(), "-f".to_string(), "/volume_data/claude.json".to_string()]
    ).await;
    
    if volume_claude_json_check.is_err() {
        // File doesn't exist in volume, copy the one we just created to volume
        exec_command_in_container(
            docker,
            container_id,
            vec!["cp".to_string(), "/root/.claude.json".to_string(), "/volume_data/claude.json".to_string()]
        ).await
        .map_err(|e| format!("Failed to copy .claude.json to volume: {}", e))?;
    }
    
    // Remove existing .claude.json if it exists
    let _ = exec_command_in_container(
        docker,
        container_id,
        vec!["rm".to_string(), "-f".to_string(), "/root/.claude.json".to_string()]
    ).await;
    
    // Create symbolic links to volume storage
    let symlink_commands = vec![
        vec!["ln".to_string(), "-sf".to_string(), "/volume_data/claude".to_string(), "/root/.claude".to_string()],
        vec!["ln".to_string(), "-sf".to_string(), "/volume_data/gh".to_string(), "/root/.config/gh".to_string()],
        vec!["ln".to_string(), "-sf".to_string(), "/volume_data/claude.json".to_string(), "/root/.claude.json".to_string()],
    ];
    
    for command in symlink_commands {
        exec_command_in_container(docker, container_id, command.clone()).await
            .map_err(|e| format!("Failed to create authentication symlink: {}", e))?;
    }
    
    log::info!("Volume structure initialization completed");
    Ok(())
}

/// Helper function to execute a command in a container
pub async fn exec_command_in_container(
    docker: &Docker,
    container_id: &str,
    command: Vec<String>,
) -> Result<String, Box<dyn std::error::Error + Send + Sync>> {
    println!("Exec Command: {:?}", &command);
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

    // Check the exit code of the command
    let inspect_exec = docker.inspect_exec(&exec.id).await?;
    if let Some(exit_code) = inspect_exec.exit_code {
        if exit_code != 0 {
            return Err(format!(
                "Command failed with exit code {}: {}",
                exit_code,
                output.trim()
            )
            .into());
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
    container_config: CodingContainerConfig,
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

    // Conditionally handle persistent volumes based on configuration
    let auth_mounts = if let Some(volume_key) = &container_config.persistent_volume_key {
        // Ensure user volume exists for authentication persistence
        let volume_name = ensure_user_volume(docker, volume_key).await?;
        
        // Create volume mounts for authentication persistence
        create_auth_mounts(&volume_name)
    } else {
        Vec::new()
    };

    let options = CreateContainerOptions {
        name: container_name,
        ..Default::default()
    };

    // Prepare environment variables for the container
    let env_vars_owned = prepare_container_env_vars_dynamic();
    let env_vars: Vec<&str> = env_vars_owned.iter().map(|s| s.as_str()).collect();

    let config = Config {
        image: Some(MAIN_CONTAINER_IMAGE),
        working_dir: Some("/workspace"),
        tty: Some(true),
        attach_stdin: Some(true),
        attach_stdout: Some(true),
        attach_stderr: Some(true),
        env: Some(env_vars),
        // Override the default command to prevent interactive shell hang
        // Run setup script then keep container alive with sleep
        cmd: Some(vec!["-c", "sleep infinity"]),
        host_config: Some(HostConfig {
            mounts: if auth_mounts.is_empty() { None } else { Some(auth_mounts) },
            ..Default::default()
        }),
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
    
    // Initialize Claude configuration (always needed regardless of volume usage)
    // This must come BEFORE init_volume_structure because init_volume_structure
    // tries to copy /root/.claude.json which is created by this function
    init_claude_configuration(docker, &container.id).await?;
    
    // Initialize volume structure for authentication persistence only if using persistent volumes
    if container_config.persistent_volume_key.is_some() {
        init_volume_structure(docker, &container.id).await?;
    }

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

    // Prepare environment variables for the container
    let env_vars_owned = prepare_container_env_vars_dynamic();
    let env_vars: Vec<&str> = env_vars_owned.iter().map(|s| s.as_str()).collect();

    let config = Config {
        image: Some(MAIN_CONTAINER_IMAGE),
        working_dir: Some("/workspace"),
        tty: Some(true),
        attach_stdin: Some(true),
        attach_stdout: Some(true),
        attach_stderr: Some(true),
        env: Some(env_vars),
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
