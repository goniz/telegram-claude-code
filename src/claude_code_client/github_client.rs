use bollard::exec::{CreateExecOptions, StartExecOptions};
use bollard::Docker;
use futures_util::StreamExt;
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct GithubAuthResult {
    pub authenticated: bool,
    pub username: Option<String>,
    pub message: String,
    pub oauth_url: Option<String>,
    pub device_code: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct GithubCloneResult {
    pub success: bool,
    pub repository: String,
    pub target_directory: String,
    pub message: String,
}

#[derive(Debug, Clone)]
pub struct GithubClientConfig {
    pub working_directory: Option<String>,
}

impl Default for GithubClientConfig {
    fn default() -> Self {
        Self {
            working_directory: Some("/workspace".to_string()),
        }
    }
}

#[derive(Debug)]
#[allow(dead_code)]
pub struct GithubClient {
    docker: Docker,
    container_id: String,
    config: GithubClientConfig,
}

#[allow(dead_code)]
impl GithubClient {
    /// Create a new GitHub client for the specified container
    pub fn new(docker: Docker, container_id: String, config: GithubClientConfig) -> Self {
        Self {
            docker,
            container_id,
            config,
        }
    }

    /// Get the container ID
    pub fn container_id(&self) -> &str {
        &self.container_id
    }

    /// Authenticate with GitHub using gh client
    pub async fn login(
        &self,
    ) -> Result<GithubAuthResult, Box<dyn std::error::Error + Send + Sync>> {
        log::info!("Starting GitHub authentication via gh client...");

        // First check if already authenticated
        match self.check_auth_status().await {
            Ok(auth_result) if auth_result.authenticated => {
                log::info!("Already authenticated with GitHub");
                return Ok(auth_result);
            }
            _ => {
                log::info!("Not authenticated, proceeding with login flow");
            }
        }

        // Start the OAuth device flow using gh auth login --web
        let login_command = vec![
            "gh".to_string(),
            "auth".to_string(),
            "login".to_string(),
            "--web".to_string(),
        ];

        match self.exec_command_interactive(login_command).await {
            Ok(output) => {
                log::info!("GitHub login command executed successfully");
                log::debug!("Login output: {}", output);

                // Parse the output to extract OAuth URL and device code
                let (oauth_url, device_code) = self.parse_oauth_response(&output);

                if let (Some(url), Some(code)) = (&oauth_url, &device_code) {
                    log::info!("OAuth flow initiated - URL: {}, Code: {}", url, code);
                    Ok(GithubAuthResult {
                        authenticated: false, // User needs to complete OAuth flow
                        username: None,
                        message: format!("Please visit {} and enter code: {}", url, code),
                        oauth_url: oauth_url,
                        device_code: device_code,
                    })
                } else {
                    // If we can't parse OAuth details, check if authentication was somehow completed
                    self.check_auth_status().await
                }
            }
            Err(e) => {
                log::error!("GitHub login failed: {}", e);
                Ok(GithubAuthResult {
                    authenticated: false,
                    username: None,
                    message: format!("Login failed: {}", e),
                    oauth_url: None,
                    device_code: None,
                })
            }
        }
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

        match self.exec_command(clone_command).await {
            Ok(output) => {
                log::debug!("Clone command output: {}", output);

                // Check if the output contains error patterns
                let is_error = output.contains("exec failed")
                    || output.contains("executable file not found")
                    || output.contains("not found")
                    || output.contains("permission denied")
                    || output.contains("authentication required")
                    || output.contains("repository not found")
                    || output.contains("404");

                if is_error {
                    log::error!("Repository clone failed with error in output: {}", output);
                    Ok(GithubCloneResult {
                        success: false,
                        repository: repository.to_string(),
                        target_directory,
                        message: format!("Clone failed: {}", output),
                    })
                } else {
                    log::info!("Repository cloned successfully");
                    Ok(GithubCloneResult {
                        success: true,
                        repository: repository.to_string(),
                        target_directory,
                        message: format!("Successfully cloned {}", repository),
                    })
                }
            }
            Err(e) => {
                log::error!("Repository clone failed: {}", e);
                Ok(GithubCloneResult {
                    success: false,
                    repository: repository.to_string(),
                    target_directory,
                    message: format!("Clone failed: {}", e),
                })
            }
        }
    }

    /// Check GitHub authentication status
    pub async fn check_auth_status(
        &self,
    ) -> Result<GithubAuthResult, Box<dyn std::error::Error + Send + Sync>> {
        let auth_command = vec!["gh".to_string(), "auth".to_string(), "status".to_string()];

        match self.exec_command(auth_command).await {
            Ok(output) => {
                log::debug!("Auth status output: {}", output);

                // Parse the output to determine authentication status
                if output.contains("Logged in to github.com") {
                    // Try to extract username from output
                    let username = self.extract_username_from_auth_status(&output);
                    Ok(GithubAuthResult {
                        authenticated: true,
                        username,
                        message: "Authenticated with GitHub".to_string(),
                        oauth_url: None,
                        device_code: None,
                    })
                } else {
                    Ok(GithubAuthResult {
                        authenticated: false,
                        username: None,
                        message: "Not authenticated with GitHub".to_string(),
                        oauth_url: None,
                        device_code: None,
                    })
                }
            }
            Err(e) => {
                log::warn!("Failed to check auth status: {}", e);
                Ok(GithubAuthResult {
                    authenticated: false,
                    username: None,
                    message: format!("Auth status check failed: {}", e),
                    oauth_url: None,
                    device_code: None,
                })
            }
        }
    }

    /// Check if gh client is available
    pub async fn check_availability(
        &self,
    ) -> Result<String, Box<dyn std::error::Error + Send + Sync>> {
        let version_command = vec!["gh".to_string(), "--version".to_string()];

        self.exec_command(version_command).await
    }

    /// Helper method for basic command execution (used in tests)
    #[allow(dead_code)]
    pub async fn exec_basic_command(
        &self,
        command: Vec<String>,
    ) -> Result<String, Box<dyn std::error::Error + Send + Sync>> {
        self.exec_command(command).await
    }

    /// Execute a command in the container and return output (for interactive OAuth flow)
    async fn exec_command_interactive(
        &self,
        command: Vec<String>,
    ) -> Result<String, Box<dyn std::error::Error + Send + Sync>> {
        // For the OAuth flow, we need to simulate providing default inputs to gh
        // We'll create a command that pipes default responses to gh auth login
        let wrapped_command = vec![
            "bash".to_string(),
            "-c".to_string(),
            format!("echo '' | {} 2>&1", command.join(" ")),
        ];

        let exec_config = CreateExecOptions {
            cmd: Some(wrapped_command),
            attach_stdout: Some(true),
            attach_stderr: Some(true),
            working_dir: self.config.working_directory.clone(),
            env: Some(vec![
                // Set up PATH to include standard paths and potential gh installation locations
                "PATH=/usr/local/bin:/usr/bin:/bin:/usr/local/sbin:/usr/sbin:/sbin".to_string(),
                // Set HOME directory for gh configuration
                "HOME=/root".to_string(),
                // Ensure we get the device flow output
                "TERM=xterm".to_string(),
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

        Ok(output.trim().to_string())
    }

    /// Parse OAuth response to extract URL and device code
    fn parse_oauth_response(&self, output: &str) -> (Option<String>, Option<String>) {
        let mut oauth_url = None;
        let mut device_code = None;

        for line in output.lines() {
            // Look for lines like "! First copy your one-time code: D023-3C2D"
            if line.contains("First copy your one-time code:") {
                if let Some(code_part) = line.split("code:").nth(1) {
                    device_code = Some(code_part.trim().to_string());
                }
            }

            // Look for lines like "Open this URL to continue in your web browser: https://github.com/login/device"
            if line.contains("Open this URL to continue")
                || line.contains("https://github.com/login/device")
            {
                if let Some(url_part) = line.split("browser:").nth(1) {
                    oauth_url = Some(url_part.trim().to_string());
                } else if line.contains("https://github.com/login/device") {
                    // Extract URL directly if it's just the URL
                    oauth_url = Some("https://github.com/login/device".to_string());
                }
            }
        }

        (oauth_url, device_code)
    }

    /// Execute a command in the container and return output
    async fn exec_command(
        &self,
        command: Vec<String>,
    ) -> Result<String, Box<dyn std::error::Error + Send + Sync>> {
        let exec_config = CreateExecOptions {
            cmd: Some(command),
            attach_stdout: Some(true),
            attach_stderr: Some(true),
            working_dir: self.config.working_directory.clone(),
            env: Some(vec![
                // Set up PATH to include standard paths and potential gh installation locations
                "PATH=/usr/local/bin:/usr/bin:/bin:/usr/local/sbin:/usr/sbin:/sbin".to_string(),
                // Set HOME directory for gh configuration
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

        // Check the exit code of the command
        let inspect_exec = self.docker.inspect_exec(&exec.id).await?;
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

    /// Extract username from auth status output
    fn extract_username_from_auth_status(&self, output: &str) -> Option<String> {
        // Look for patterns like "Logged in to github.com as username"
        for line in output.lines() {
            if line.contains("Logged in to github.com as") {
                if let Some(username_part) = line.split(" as ").nth(1) {
                    // Extract just the username part, removing any additional text
                    let username = username_part
                        .split_whitespace()
                        .next()
                        .unwrap_or("")
                        .trim_matches('(')
                        .trim_matches(')')
                        .to_string();
                    if !username.is_empty() {
                        return Some(username);
                    }
                }
            }
        }
        None
    }
}
