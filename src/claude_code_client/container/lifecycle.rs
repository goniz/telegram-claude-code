//! Container lifecycle management functionality
//!
//! This module handles the creation, destruction, and lifecycle management of Docker containers
//! used by the telegram-claude-code application. It provides functions for starting coding
//! sessions, clearing containers, and managing container configurations.

use bollard::exec::{CreateExecOptions, StartExecOptions};
use bollard::models::{ContainerCreateBody, HostConfig};
use bollard::query_parameters::{
    CreateContainerOptions, CreateImageOptions, ListContainersOptions, RemoveContainerOptions,
};
use bollard::Docker;
use futures_util::StreamExt;
use std::collections::HashMap;

// Import volume management functions from the volume module
use super::volume::{create_auth_mounts, ensure_user_volume};
// Import file operations for container file management
use super::file_ops::container_put_file;

/// Configuration for coding container behavior
#[derive(Debug, Clone, Default)]
pub struct CodingContainerConfig {
    pub persistent_volume_key: Option<String>,
}

/// Container image used by the main application
/// This is the Claude Code runtime image that provides multi-language development environment with Claude Code pre-installed
pub const MAIN_CONTAINER_IMAGE: &str = "ghcr.io/goniz/telegram-claude-code-runtime:main";

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

/// Initialize Claude configuration in the container
/// This sets up the basic Claude configuration files (without settings.json)
async fn init_claude_configuration(
    docker: &Docker,
    container_id: &str,
) -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
    log::info!("Initializing Claude configuration...");

    // Initialize .claude.json with required configuration
    exec_command_in_container(
        docker,
        container_id,
        vec![
            "sh".to_string(),
            "-c".to_string(),
            "echo '{ \"hasCompletedOnboarding\": true }' > /root/.claude.json".to_string(),
        ],
    )
    .await
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

/// Initialize git configuration in the container
/// This sets up the git user email and name for commits
async fn init_git_configuration(
    docker: &Docker,
    container_id: &str,
) -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
    log::info!("Initializing git configuration...");

    // Set git user email
    exec_command_in_container(
        docker,
        container_id,
        vec![
            "git".to_string(),
            "config".to_string(),
            "--global".to_string(),
            "user.email".to_string(),
            "noreply@anthropic.com".to_string(),
        ],
    )
    .await
    .map_err(|e| format!("Failed to set git email: {}", e))?;

    // Set git user name
    exec_command_in_container(
        docker,
        container_id,
        vec![
            "git".to_string(),
            "config".to_string(),
            "--global".to_string(),
            "user.name".to_string(),
            "Claude".to_string(),
        ],
    )
    .await
    .map_err(|e| format!("Failed to set git name: {}", e))?;

    log::info!("Git configuration initialization completed");
    Ok(())
}

/// Initialize Claude settings.json file
/// This creates the settings.json file with proper tool permissions
async fn init_claude_settings(
    docker: &Docker,
    container_id: &str,
    claude_dir_path: &str,
) -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
    log::info!("Initializing Claude settings.json...");

    // Create the .claude directory if it doesn't exist
    exec_command_in_container(
        docker,
        container_id,
        vec![
            "mkdir".to_string(),
            "-p".to_string(),
            claude_dir_path.to_string(),
        ],
    )
    .await
    .map_err(|e| format!("Failed to create Claude directory: {}", e))?;

    let settings_json = r#"{
  "permissions": {
    "defaultMode": "acceptEdits",
    "allow": [
      "Edit",
      "Read", 
      "Write",
      "Bash",
      "Glob",
      "Grep",
      "LS",
      "MultiEdit",
      "Task"
    ]
  }
}"#;

    let settings_path = format!("{}/settings.json", claude_dir_path);

    // Use container_put_file to write settings.json
    container_put_file(
        docker,
        container_id,
        &settings_path,
        settings_json.as_bytes(),
        Some(0o644),
    )
    .await
    .map_err(|e| format!("Failed to write settings.json: {}", e))?;

    log::info!("Claude settings.json initialization completed");
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
        vec![
            "mkdir".to_string(),
            "-p".to_string(),
            "/volume_data/claude".to_string(),
        ],
        vec![
            "mkdir".to_string(),
            "-p".to_string(),
            "/volume_data/gh".to_string(),
        ],
        // Create parent directories for symlinks
        vec![
            "mkdir".to_string(),
            "-p".to_string(),
            "/root/.config".to_string(),
        ],
        // Remove existing directories/files if they exist (they might be empty from container creation)
        vec![
            "rm".to_string(),
            "-rf".to_string(),
            "/root/.claude".to_string(),
        ],
        vec![
            "rm".to_string(),
            "-rf".to_string(),
            "/root/.config/gh".to_string(),
        ],
    ];

    for command in basic_commands {
        exec_command_in_container(docker, container_id, command.clone())
            .await
            .map_err(|e| format!("Failed to initialize volume directory structure: {}", e))?;
    }

    // Handle .claude.json file for volume persistence
    // First check if it already exists in the volume (from previous sessions)
    let volume_claude_json_check = exec_command_in_container(
        docker,
        container_id,
        vec![
            "test".to_string(),
            "-f".to_string(),
            "/volume_data/claude.json".to_string(),
        ],
    )
    .await;

    if volume_claude_json_check.is_err() {
        // File doesn't exist in volume, copy the one we just created to volume
        exec_command_in_container(
            docker,
            container_id,
            vec![
                "cp".to_string(),
                "/root/.claude.json".to_string(),
                "/volume_data/claude.json".to_string(),
            ],
        )
        .await
        .map_err(|e| format!("Failed to copy .claude.json to volume: {}", e))?;
    }

    // Remove existing .claude.json if it exists
    let _ = exec_command_in_container(
        docker,
        container_id,
        vec![
            "rm".to_string(),
            "-f".to_string(),
            "/root/.claude.json".to_string(),
        ],
    )
    .await;

    // Create symbolic links to volume storage
    let symlink_commands = vec![
        vec![
            "ln".to_string(),
            "-sf".to_string(),
            "/volume_data/claude".to_string(),
            "/root/.claude".to_string(),
        ],
        vec![
            "ln".to_string(),
            "-sf".to_string(),
            "/volume_data/gh".to_string(),
            "/root/.config/gh".to_string(),
        ],
        vec![
            "ln".to_string(),
            "-sf".to_string(),
            "/volume_data/claude.json".to_string(),
            "/root/.claude.json".to_string(),
        ],
    ];

    for command in symlink_commands {
        exec_command_in_container(docker, container_id, command.clone())
            .await
            .map_err(|e| format!("Failed to create authentication symlink: {}", e))?;
    }

    // Initialize Claude settings.json in the volume directory
    init_claude_settings(docker, container_id, "/volume_data/claude").await?;

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
        from_image: Some(MAIN_CONTAINER_IMAGE.to_string()),
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
        name: Some(container_name.to_string()),
        ..Default::default()
    };

    // Prepare environment variables for the container
    let env_vars = prepare_container_env_vars_dynamic();

    let config = ContainerCreateBody {
        image: Some(MAIN_CONTAINER_IMAGE.to_string()),
        working_dir: Some("/workspace".to_string()),
        tty: Some(true),
        attach_stdin: Some(true),
        attach_stdout: Some(true),
        attach_stderr: Some(true),
        env: Some(env_vars),
        // Override the default command to prevent interactive shell hang
        // Run setup script then keep container alive with sleep
        cmd: Some(vec!["-c".to_string(), "sleep infinity".to_string()]),
        host_config: Some(HostConfig {
            mounts: if auth_mounts.is_empty() {
                None
            } else {
                Some(auth_mounts)
            },
            ..Default::default()
        }),
        // Set stop timeout to ensure graceful shutdown
        stop_timeout: Some(3),
        ..Default::default()
    };

    let container = docker.create_container(Some(options), config).await?;
    docker
        .start_container(
            &container.id,
            None::<bollard::query_parameters::StartContainerOptions>,
        )
        .await?;

    // Wait for container to be ready
    wait_for_container_ready(docker, &container.id).await?;

    // Initialize Claude configuration (always needed regardless of volume usage)
    // This must come BEFORE init_volume_structure because init_volume_structure
    // tries to copy /root/.claude.json which is created by this function
    init_claude_configuration(docker, &container.id).await?;

    // Initialize git configuration for commits (always needed for git operations)
    init_git_configuration(docker, &container.id).await?;

    // Initialize volume structure for authentication persistence only if using persistent volumes
    if container_config.persistent_volume_key.is_some() {
        init_volume_structure(docker, &container.id).await?;
    } else {
        // For non-volume scenarios, create settings.json in the regular location
        init_claude_settings(docker, &container.id, "/root/.claude").await?;
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
    let _ = docker
        .stop_container(
            container_name,
            None::<bollard::query_parameters::StopContainerOptions>,
        )
        .await;

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
        from_image: Some(MAIN_CONTAINER_IMAGE.to_string()),
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
        name: Some(container_name.to_string()),
        ..Default::default()
    };

    // Prepare environment variables for the container
    let env_vars = prepare_container_env_vars_dynamic();

    let config = ContainerCreateBody {
        image: Some(MAIN_CONTAINER_IMAGE.to_string()),
        working_dir: Some("/workspace".to_string()),
        tty: Some(true),
        attach_stdin: Some(true),
        attach_stdout: Some(true),
        attach_stderr: Some(true),
        env: Some(env_vars),
        cmd: Some(vec!["/bin/bash".to_string()]),
        ..Default::default()
    };

    let container = docker.create_container(Some(options), config).await?;
    docker
        .start_container(
            &container.id,
            None::<bollard::query_parameters::StartContainerOptions>,
        )
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
        filters: Some(filters),
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
