use bollard::exec::{CreateExecOptions, StartExecOptions};
use bollard::Docker;
use futures_util::StreamExt;

use super::types::{GithubClientConfig, GithubCloneResult};

/// GitHub repository operations functionality
#[derive(Debug)]
pub struct GitHubOperations {
    docker: Docker,
    container_id: String,
    config: GithubClientConfig,
}

impl GitHubOperations {
    pub fn new(docker: Docker, container_id: String, config: GithubClientConfig) -> Self {
        Self {
            docker,
            container_id,
            config,
        }
    }

    /// Get the container ID
    #[allow(dead_code)]
    pub fn container_id(&self) -> &str {
        &self.container_id
    }

    /// Clone a repository using gh client
    pub async fn repo_clone(
        &self,
        repository: &str,
        target_dir: Option<&str>,
    ) -> Result<GithubCloneResult, Box<dyn std::error::Error + Send + Sync>> {
        log::info!("Cloning repository '{}' via gh client...", repository);

        let mut clone_command = vec![
            "gh".to_string(),
            "repo".to_string(),
            "clone".to_string(),
            repository.to_string(),
        ];

        let target_directory = if let Some(dir) = target_dir {
            clone_command.push(dir.to_string());
            dir.to_string()
        } else {
            // Extract repo name from full repository path (e.g., "owner/repo" -> "repo")
            repository
                .split('/')
                .last()
                .unwrap_or(repository)
                .to_string()
        };

        match self.exec_command_allow_failure(clone_command).await {
            Ok((output, success)) => {
                log::debug!("Clone command output: {}", output);

                if success {
                    // Double-check success by looking for common success indicators
                    // This provides additional validation beyond exit code
                    if output.contains("Cloning into") || output.is_empty() {
                        log::info!("Repository cloned successfully");
                        Ok(GithubCloneResult {
                            success: true,
                            repository: repository.to_string(),
                            target_directory,
                            message: format!("Successfully cloned {}", repository),
                        })
                    } else {
                        // Exit code was 0 but output doesn't look like success
                        log::warn!(
                            "Clone command succeeded but output is unexpected: {}",
                            output
                        );
                        Ok(GithubCloneResult {
                            success: true,
                            repository: repository.to_string(),
                            target_directory,
                            message: format!("Clone completed with warnings: {}", output),
                        })
                    }
                } else {
                    // Analyze the failure to provide better error messages
                    let error_message = self.analyze_clone_failure(&output);
                    log::error!("Repository clone failed: {}", error_message);
                    Ok(GithubCloneResult {
                        success: false,
                        repository: repository.to_string(),
                        target_directory,
                        message: error_message,
                    })
                }
            }
            Err(e) => {
                log::error!("Repository clone command execution failed: {}", e);
                Ok(GithubCloneResult {
                    success: false,
                    repository: repository.to_string(),
                    target_directory,
                    message: format!("Command execution failed: {}", e),
                })
            }
        }
    }

    /// Check GitHub availability (check if gh command is available)
    #[allow(dead_code)]
    pub async fn check_availability(
        &self,
    ) -> Result<String, Box<dyn std::error::Error + Send + Sync>> {
        let command = vec!["gh".to_string(), "--version".to_string()];
        self.exec_command(command).await
    }

    /// List repositories for the authenticated user
    pub async fn repo_list(&self) -> Result<String, Box<dyn std::error::Error + Send + Sync>> {
        log::info!("Listing repositories for authenticated user...");

        let list_command = vec![
            "gh".to_string(),
            "repo".to_string(),
            "list".to_string(),
            "--limit".to_string(),
            "50".to_string(), // Reasonable limit
        ];

        match self.exec_command(list_command).await {
            Ok(output) => {
                log::debug!("Repository list output: {}", output);
                Ok(output)
            }
            Err(e) => {
                log::error!("Failed to list repositories: {}", e);
                Err(e)
            }
        }
    }

    /// Analyze clone failure output to provide better error messages
    fn analyze_clone_failure(&self, output: &str) -> String {
        let output_lower = output.to_lowercase();

        if output_lower.contains("repository not found") || output_lower.contains("not found") {
            "Repository not found. Please check the repository name and your access permissions."
                .to_string()
        } else if output_lower.contains("permission denied")
            || output_lower.contains("authentication")
        {
            "Permission denied. Please ensure you're authenticated with GitHub and have access to this repository.".to_string()
        } else if output_lower.contains("already exists") {
            "Directory already exists. Please choose a different target directory or remove the existing one.".to_string()
        } else if output_lower.contains("network") || output_lower.contains("connection") {
            "Network error. Please check your internet connection and try again.".to_string()
        } else if output.trim().is_empty() {
            "Clone failed with no error message. The repository may not exist or you may not have access.".to_string()
        } else {
            format!("Clone failed: {}", output.trim())
        }
    }

    /// Execute a command and return both output and success status
    pub async fn exec_command_allow_failure(
        &self,
        command: Vec<String>,
    ) -> Result<(String, bool), Box<dyn std::error::Error + Send + Sync>> {
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
                "PATH=/usr/local/bin:/usr/bin:/bin:/usr/local/sbin:/usr/sbin:/sbin".to_string(),
                "HOME=/root".to_string(),
            ]),
            ..Default::default()
        };

        let exec = self
            .docker
            .create_exec(&self.container_id, exec_config)
            .await?;

        let start_config = StartExecOptions {
            detach: false,
            ..Default::default()
        };

        let mut output = String::new();

        match self.docker.start_exec(&exec.id, Some(start_config)).await? {
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

        // Check the exit code of the executed command
        let exec_inspect = self.docker.inspect_exec(&exec.id).await?;
        let exit_code = exec_inspect.exit_code.unwrap_or(-1);
        let success = exit_code == 0;

        log::debug!(
            "Command completed with exit code: {}, success: {}",
            exit_code,
            success
        );

        Ok((output.trim().to_string(), success))
    }

    /// Execute a command in the container and return output (with error on failure)
    async fn exec_command(
        &self,
        command: Vec<String>,
    ) -> Result<String, Box<dyn std::error::Error + Send + Sync>> {
        let (output, success) = self.exec_command_allow_failure(command).await?;

        if success {
            Ok(output)
        } else {
            Err(format!("Command failed: {}", output).into())
        }
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
