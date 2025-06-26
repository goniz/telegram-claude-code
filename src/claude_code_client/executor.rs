use bollard::exec::{CreateExecOptions, StartExecOptions};
use bollard::Docker;
use futures_util::StreamExt;

use super::config::ClaudeCodeConfig;

/// Command execution functionality for Claude Code client
#[derive(Debug)]
pub struct CommandExecutor {
    docker: Docker,
    container_id: String,
    config: ClaudeCodeConfig,
}

impl CommandExecutor {
    pub fn new(docker: Docker, container_id: String, config: ClaudeCodeConfig) -> Self {
        Self {
            docker,
            container_id,
            config,
        }
    }

    /// Execute a command in the container and return output
    pub async fn exec_command(
        &self,
        command: Vec<String>,
    ) -> Result<String, Box<dyn std::error::Error + Send + Sync>> {
        log::debug!(
            "Executing command in container {}: {:?}",
            self.container_id,
            command
        );

        let exec_config = CreateExecOptions {
            cmd: Some(command.clone()),
            attach_stdout: Some(true),
            attach_stderr: Some(true),
            working_dir: self.config.working_directory.clone(),
            env: Some(vec![
                // Set up PATH to include NVM Node.js installation and standard paths
                "PATH=/root/.nvm/versions/node/v22.16.0/bin:/root/.nvm/versions/node/v20.19.2/bin:\
                 /root/.nvm/versions/node/v18.20.8/bin:/usr/local/sbin:/usr/local/bin:/usr/sbin:/\
                 usr/bin:/sbin:/bin"
                    .to_string(),
                // Ensure Node.js modules are available
                "NODE_PATH=/root/.nvm/versions/node/v22.16.0/lib/node_modules".to_string(),
            ]),
            ..Default::default()
        };

        log::debug!(
            "Creating exec for container {} with working_dir: {:?}",
            self.container_id,
            self.config.working_directory
        );

        let exec = match self
            .docker
            .create_exec(&self.container_id, exec_config)
            .await
        {
            Ok(exec) => {
                log::debug!("Successfully created exec with ID: {}", exec.id);
                exec
            }
            Err(e) => {
                log::error!(
                    "Failed to create exec for container {}: {}",
                    self.container_id,
                    e
                );
                return Err(e.into());
            }
        };

        let start_config = StartExecOptions {
            detach: false,
            ..Default::default()
        };

        let mut output = String::new();
        let mut stderr_output = String::new();

        match self.docker.start_exec(&exec.id, Some(start_config)).await? {
            bollard::exec::StartExecResults::Attached {
                output: mut output_stream,
                ..
            } => {
                log::debug!("Successfully attached to exec {}, reading output", exec.id);
                while let Some(Ok(msg)) = output_stream.next().await {
                    match msg {
                        bollard::container::LogOutput::StdOut { message } => {
                            let stdout_str = String::from_utf8_lossy(&message);
                            log::debug!("Command stdout: '{}'", stdout_str);
                            output.push_str(&stdout_str);
                        }
                        bollard::container::LogOutput::StdErr { message } => {
                            let stderr_str = String::from_utf8_lossy(&message);
                            log::debug!("Command stderr: '{}'", stderr_str);
                            stderr_output.push_str(&stderr_str);
                            output.push_str(&stderr_str);
                        }
                        _ => {}
                    }
                }
            }
            bollard::exec::StartExecResults::Detached => {
                log::error!("Unexpected detached execution for exec {}", exec.id);
                return Err("Unexpected detached execution".into());
            }
        }

        // Check the exit code of the executed command
        let exec_inspect = self.docker.inspect_exec(&exec.id).await?;
        let exit_code = exec_inspect.exit_code.unwrap_or(-1);

        log::debug!("Command completed with exit code: {}", exit_code);
        log::debug!("Command total output length: {} chars", output.len());
        log::debug!("Command stderr length: {} chars", stderr_output.len());

        if exit_code != 0 {
            log::warn!("Command failed with exit code {}", exit_code);
            log::debug!("Failed command output: '{}'", output.trim());
            // Command failed - return error with the output
            return Err(format!(
                "Command failed with exit code {}: {}",
                exit_code,
                output.trim()
            )
            .into());
        }

        log::debug!("Command succeeded, returning output");
        Ok(output.trim().to_string())
    }

    /// Helper method for basic command execution (used in tests)
    #[allow(dead_code)]
    pub async fn exec_basic_command(
        &self,
        command: Vec<String>,
    ) -> Result<String, Box<dyn std::error::Error + Send + Sync>> {
        self.exec_command(command).await
    }
}
