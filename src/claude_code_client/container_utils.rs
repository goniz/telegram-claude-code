use bollard::container::{Config, CreateContainerOptions, RemoveContainerOptions};
use bollard::exec::{CreateExecOptions, StartExecOptions};
use bollard::image::CreateImageOptions;
use bollard::volume::{CreateVolumeOptions, ListVolumesOptions};
use bollard::models::{Mount, MountTypeEnum, HostConfig};
use bollard::Docker;
use futures_util::StreamExt;
use std::collections::HashMap;

/// Container image used by the main application
/// This is the Claude Code runtime image that provides multi-language development environment with Claude Code pre-installed
pub const MAIN_CONTAINER_IMAGE: &str = "ghcr.io/goniz/telegram-claude-code-runtime:main";

/// Generate a volume name for a user's persistent authentication data
pub fn generate_volume_name(telegram_user_id: i64) -> String {
    format!("dev-session-claude-{}", telegram_user_id)
}

/// Create or get existing volume for user authentication persistence
/// This function is idempotent - it will not fail if the volume already exists
pub async fn ensure_user_volume(
    docker: &Docker,
    telegram_user_id: i64,
) -> Result<String, Box<dyn std::error::Error + Send + Sync>> {
    let volume_name = generate_volume_name(telegram_user_id);
    
    // Check if volume already exists
    let list_options = ListVolumesOptions {
        filters: {
            let mut filters = HashMap::new();
            filters.insert("name".to_string(), vec![volume_name.clone()]);
            filters
        },
    };
    
    match docker.list_volumes(Some(list_options)).await {
        Ok(volumes) => {
            // Check if our volume exists in the list
            let volume_exists = volumes.volumes
                .as_ref()
                .map(|vol_list| {
                    vol_list.iter().any(|vol| {
                        Some(volume_name.clone()) == Some(vol.name.clone())
                    })
                })
                .unwrap_or(false);
                
            if volume_exists {
                log::info!("Volume '{}' already exists, reusing", volume_name);
                return Ok(volume_name);
            }
        }
        Err(e) => {
            log::warn!("Failed to list volumes (will attempt creation anyway): {}", e);
        }
    }
    
    // Create the volume if it doesn't exist
    let create_options = CreateVolumeOptions {
        name: volume_name.clone(),
        driver: "local".to_string(),
        driver_opts: HashMap::new(),
        labels: {
            let mut labels = HashMap::new();
            labels.insert("created_by".to_string(), "telegram-claude-code".to_string());
            labels.insert("telegram_user_id".to_string(), telegram_user_id.to_string());
            labels.insert("purpose".to_string(), "authentication_persistence".to_string());
            labels
        },
    };
    
    match docker.create_volume(create_options).await {
        Ok(_) => {
            log::info!("Created new volume '{}' for user {}", volume_name, telegram_user_id);
            Ok(volume_name)
        }
        Err(e) => {
            // Check if the error is because the volume already exists
            if e.to_string().contains("already exists") || e.to_string().contains("Conflict") {
                log::info!("Volume '{}' already exists (race condition), continuing", volume_name);
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

/// Initialize volume structure by creating symbolic links to persistent storage
/// This sets up the authentication directories to point to volume storage
async fn init_volume_structure(
    docker: &Docker,
    container_id: &str,
) -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
    // Create the persistent directories in the volume if they don't exist
    let init_commands = vec![
        // Create volume directories
        vec!["mkdir".to_string(), "-p".to_string(), "/volume_data/claude".to_string()],
        vec!["mkdir".to_string(), "-p".to_string(), "/volume_data/gh".to_string()],
        
        // Create parent directories for symlinks
        vec!["mkdir".to_string(), "-p".to_string(), "/root/.config".to_string()],
        
        // Remove existing directories/files if they exist (they might be empty from container creation)
        vec!["rm".to_string(), "-rf".to_string(), "/root/.claude".to_string()],
        vec!["rm".to_string(), "-rf".to_string(), "/root/.config/gh".to_string()],
        
        // Create symbolic links to volume storage
        vec!["ln".to_string(), "-sf".to_string(), "/volume_data/claude".to_string(), "/root/.claude".to_string()],
        vec!["ln".to_string(), "-sf".to_string(), "/volume_data/gh".to_string(), "/root/.config/gh".to_string()],
    ];
    
    for command in init_commands {
        match exec_command_in_container(docker, container_id, command.clone()).await {
            Ok(_) => {
                log::debug!("Successfully executed volume init command: {:?}", command);
            }
            Err(e) => {
                log::warn!("Volume init command failed (continuing anyway): {:?} - {}", command, e);
            }
        }
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
    telegram_user_id: i64,
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

    // Ensure user volume exists for authentication persistence
    let volume_name = ensure_user_volume(docker, telegram_user_id).await?;
    
    // Create volume mounts for authentication persistence
    let auth_mounts = create_auth_mounts(&volume_name);

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
        host_config: Some(HostConfig {
            mounts: Some(auth_mounts),
            ..Default::default()
        }),
        ..Default::default()
    };

    let container = docker.create_container(Some(options), config).await?;
    docker
        .start_container::<String>(&container.id, None)
        .await?;

    // Wait for container to be ready
    wait_for_container_ready(docker, &container.id).await?;
    
    // Initialize volume structure for authentication persistence
    init_volume_structure(docker, &container.id).await?;

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
