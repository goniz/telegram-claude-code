use bollard::exec::{CreateExecOptions, StartExecOptions};
use bollard::Docker;
use futures_util::StreamExt;
use serde::{Deserialize, Serialize};
use std::sync::Arc;
use tokio::time::{timeout, Duration};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct GithubAuthResult {
    pub authenticated: bool,
    pub username: Option<String>,
    pub message: String,
    pub oauth_url: Option<String>,
    pub device_code: Option<String>,
}

#[derive(Debug)]
pub struct OAuthProcess {
    exec_id: String,
    docker: Arc<Docker>,
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
    pub exec_timeout_secs: u64,
}

impl Default for GithubClientConfig {
    fn default() -> Self {
        Self {
            working_directory: Some("/workspace".to_string()),
            exec_timeout_secs: 60, // 60 seconds timeout for auth operations
        }
    }
}

impl OAuthProcess {
    /// Wait for the OAuth process to complete with a timeout
    pub async fn wait_for_completion(
        &self,
        timeout_secs: u64,
    ) -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
        let timeout_duration = Duration::from_secs(timeout_secs);

        timeout(timeout_duration, async {
            // Wait for the exec process to complete
            loop {
                let inspect_result = self.docker.inspect_exec(&self.exec_id).await?;
                if let Some(false) = inspect_result.running {
                    // Process has completed
                    break;
                }
                tokio::time::sleep(Duration::from_millis(500)).await;
            }
            Ok::<(), Box<dyn std::error::Error + Send + Sync>>(())
        })
        .await
        .map_err(|_| -> Box<dyn std::error::Error + Send + Sync> {
            "OAuth process timed out".into()
        })?
    }

    /// Terminate the OAuth process gracefully
    #[allow(dead_code)]
    pub async fn terminate(&self) -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
        // Note: Docker exec doesn't have a direct kill method, but the process should
        // terminate when the user completes OAuth or when the container is stopped
        Ok(())
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

    /// Authenticate with GitHub using gh client (refactored OAuth flow)
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

        // Start the OAuth flow using gh auth login (interactive)
        let login_command = vec![
            "gh".to_string(),
            "auth".to_string(),
            "login".to_string(),
            "--git-protocol".to_string(),
            "https".to_string(),
        ];

        match self.start_oauth_flow_with_early_return(login_command).await {
            Ok((auth_result, _oauth_process)) => {
                // We've successfully extracted OAuth URL and device code
                // The process is still running in the background
                log::info!("OAuth flow initiated successfully");
                Ok(auth_result)
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

    /// Start OAuth flow with early return of credentials and background process
    async fn start_oauth_flow_with_early_return(
        &self,
        command: Vec<String>,
    ) -> Result<(GithubAuthResult, OAuthProcess), Box<dyn std::error::Error + Send + Sync>> {
        let exec_config = CreateExecOptions {
            cmd: Some(command.clone()),
            attach_stdout: Some(true),
            attach_stderr: Some(true),
            working_dir: self.config.working_directory.clone(),
            env: Some(vec![
                "PATH=/usr/local/bin:/usr/bin:/bin:/usr/local/sbin:/usr/sbin:/sbin".to_string(),
                "HOME=/root".to_string(),
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

        let mut output_buffer = String::new();
        let mut oauth_url = None;
        let mut device_code = None;

        // Create OAuth process handle for later waiting/cleanup
        let oauth_process = OAuthProcess {
            exec_id: exec.id.clone(),
            docker: Arc::new(self.docker.clone()),
        };

        // Start the exec process
        match self.docker.start_exec(&exec.id, Some(start_config)).await? {
            bollard::exec::StartExecResults::Attached {
                output: mut output_stream,
                ..
            } => {
                // Stream output and look for OAuth credentials
                let timeout_duration = Duration::from_secs(30); // Short timeout for credential detection

                let stream_result = timeout(timeout_duration, async {
                    while let Some(Ok(msg)) = output_stream.next().await {
                        match msg {
                            bollard::container::LogOutput::StdOut { message } => {
                                let new_output = String::from_utf8_lossy(&message);
                                output_buffer.push_str(&new_output);
                                log::debug!("OAuth output: {}", new_output);
                            }
                            bollard::container::LogOutput::StdErr { message } => {
                                let new_output = String::from_utf8_lossy(&message);
                                output_buffer.push_str(&new_output);
                                log::debug!("OAuth stderr: {}", new_output);
                            }
                            _ => {}
                        }

                        // Parse current output for OAuth credentials
                        let (url, code) = self.parse_oauth_response(&output_buffer);
                        if url.is_some() && code.is_some() {
                            oauth_url = url;
                            device_code = code;
                            log::info!("Found OAuth credentials, returning early");
                            break;
                        }
                    }
                    Ok::<(), Box<dyn std::error::Error + Send + Sync>>(())
                })
                .await;

                match stream_result {
                    Ok(_) => {
                        if let (Some(url), Some(code)) = (&oauth_url, &device_code) {
                            log::info!("OAuth flow initiated - URL: {}, Code: {}", url, code);
                            let auth_result = GithubAuthResult {
                                authenticated: false, // User needs to complete OAuth flow
                                username: None,
                                message: format!("Please visit {} and enter code: {}", url, code),
                                oauth_url,
                                device_code,
                            };
                            Ok((auth_result, oauth_process))
                        } else {
                            // If we didn't find OAuth credentials in the initial output,
                            // check if authentication was somehow completed
                            let auth_result = self.check_auth_status().await.unwrap_or_else(|e| {
                                GithubAuthResult {
                                    authenticated: false,
                                    username: None,
                                    message: format!("Failed to parse OAuth response: {}", e),
                                    oauth_url: None,
                                    device_code: None,
                                }
                            });
                            Ok((auth_result, oauth_process))
                        }
                    }
                    Err(_) => Err("Timeout waiting for OAuth credentials in command output".into()),
                }
            }
            bollard::exec::StartExecResults::Detached => {
                Err("Unexpected detached execution".into())
            }
        }
    }

    /// Wait for OAuth process to complete (utility method for callers)
    #[allow(dead_code)]
    pub async fn wait_for_oauth_completion(
        &self,
        oauth_process: OAuthProcess,
    ) -> Result<GithubAuthResult, Box<dyn std::error::Error + Send + Sync>> {
        // Wait for the OAuth process to complete with 60-second timeout
        oauth_process.wait_for_completion(60).await?;

        // Check final authentication status
        self.check_auth_status().await
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

    /// List GitHub repositories for the authenticated user
    pub async fn repo_list(&self) -> Result<String, Box<dyn std::error::Error + Send + Sync>> {
        log::info!("Listing GitHub repositories for authenticated user...");

        let list_command = vec!["gh".to_string(), "repo".to_string(), "list".to_string()];

        match self.exec_command(list_command).await {
            Ok(output) => {
                log::debug!("Repo list command output: {}", output);
                Ok(output)
            }
            Err(e) => {
                log::error!("Failed to list repositories: {}", e);
                Err(e)
            }
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

    /// Execute a command in the container and return output (for interactive OAuth flow)
    async fn exec_command_interactive(
        &self,
        command: Vec<String>,
    ) -> Result<String, Box<dyn std::error::Error + Send + Sync>> {
        // Just launch the gh command without any wrappers or input provision

        let exec_config = CreateExecOptions {
            cmd: Some(command.clone()),
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

        // Wrap the exec operation in a timeout
        let timeout_duration = Duration::from_secs(self.config.exec_timeout_secs);

        let exec_result = timeout(timeout_duration, async {
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
            Ok::<String, Box<dyn std::error::Error + Send + Sync>>(output)
        })
        .await;

        match exec_result {
            Ok(result) => result,
            Err(_) => Err(format!(
                "Command timed out after {} seconds: {}",
                self.config.exec_timeout_secs,
                command.join(" ")
            )
            .into()),
        }
    }

    /// Parse OAuth response to extract URL and device code from interactive flow
    fn parse_oauth_response(&self, output: &str) -> (Option<String>, Option<String>) {
        let mut oauth_url = None;
        let mut device_code = None;

        for line in output.lines() {
            // Look for device code in various formats
            if line.contains("First copy your one-time code:") || line.contains("one-time code:") {
                if let Some(code_part) = line.split("code:").nth(1) {
                    device_code = Some(code_part.trim().to_string());
                }
            }

            // Look for URLs in various formats
            if line.contains("https://github.com/login/device") {
                if let Some(url_start) = line.find("https://github.com/login/device") {
                    let url_part = &line[url_start..];
                    let url = url_part.split_whitespace().next().unwrap_or(url_part);
                    oauth_url = Some(url.to_string());
                }
            } else if line.contains("https://github.com/login/oauth") {
                // Handle other OAuth URLs
                if let Some(url_start) = line.find("https://github.com/login/oauth") {
                    let url_part = &line[url_start..];
                    let url = url_part.split_whitespace().next().unwrap_or(url_part);
                    oauth_url = Some(url.to_string());
                }
            } else if line.contains("Open this URL to continue") || line.contains("browser:") {
                // Fallback for other URL formats
                if let Some(url_part) = line.split("browser:").nth(1) {
                    oauth_url = Some(url_part.trim().to_string());
                }
            }
        }

        (oauth_url, device_code)
    }

    /// Analyze clone failure output to provide better error messages
    fn analyze_clone_failure(&self, output: &str) -> String {
        let output_lower = output.to_lowercase();

        // Check for common error patterns and provide helpful messages
        if output_lower.contains("repository not found") || output_lower.contains("404") {
            "Repository not found. Please check the repository name and ensure it exists."
                .to_string()
        } else if output_lower.contains("permission denied") || output_lower.contains("403") {
            "Permission denied. The repository may be private or require authentication."
                .to_string()
        } else if output_lower.contains("authentication required") || output_lower.contains("auth")
        {
            "Authentication required. Please authenticate with GitHub first using 'gh auth login'."
                .to_string()
        } else if output_lower.contains("network") || output_lower.contains("connection") {
            "Network error. Please check your internet connection and try again.".to_string()
        } else if output_lower.contains("timeout") {
            "Operation timed out. The repository may be very large or network is slow.".to_string()
        } else if output_lower.contains("already exists") {
            "Target directory already exists. Please choose a different directory or remove the \
             existing one."
                .to_string()
        } else if output_lower.contains("not found") && output_lower.contains("gh") {
            "GitHub CLI (gh) not found. Please ensure GitHub CLI is installed and available in \
             PATH."
                .to_string()
        } else if output_lower.contains("fatal:") {
            // Extract the fatal error message
            if let Some(start) = output_lower.find("fatal:") {
                let fatal_msg = &output[start..];
                if let Some(end) = fatal_msg.find('\n') {
                    format!("Git error: {}", &fatal_msg[..end])
                } else {
                    format!("Git error: {}", fatal_msg)
                }
            } else {
                format!("Git fatal error occurred: {}", output.trim())
            }
        } else if output.trim().is_empty() {
            "Clone failed with no error message. This may indicate a configuration issue."
                .to_string()
        } else {
            // Generic error message with the actual output
            format!("Clone failed: {}", output.trim())
        }
    }

    /// Execute a command in the container and return output with success status  
    /// This method returns the output even if the command fails (non-zero exit code)
    /// Made public for testing purposes
    pub async fn exec_command_allow_failure(
        &self,
        command: Vec<String>,
    ) -> Result<(String, bool), Box<dyn std::error::Error + Send + Sync>> {
        let exec_config = CreateExecOptions {
            cmd: Some(command.clone()),
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

        // Wrap the exec operation in a timeout
        let timeout_duration = Duration::from_secs(self.config.exec_timeout_secs);

        let exec_result = timeout(timeout_duration, async {
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

            // Check the exit code of the command but return output regardless
            let inspect_exec = self.docker.inspect_exec(&exec.id).await?;
            let success = if let Some(exit_code) = inspect_exec.exit_code {
                exit_code == 0
            } else {
                false // If we can't determine exit code, assume failure
            };

            Ok::<(String, bool), Box<dyn std::error::Error + Send + Sync>>((
                output.trim().to_string(),
                success,
            ))
        })
        .await;

        match exec_result {
            Ok(result) => result,
            Err(_) => Err(format!(
                "Command timed out after {} seconds: {}",
                self.config.exec_timeout_secs,
                command.join(" ")
            )
            .into()),
        }
    }

    /// Execute a command in the container and return output
    async fn exec_command(
        &self,
        command: Vec<String>,
    ) -> Result<String, Box<dyn std::error::Error + Send + Sync>> {
        let exec_config = CreateExecOptions {
            cmd: Some(command.clone()),
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

        // Wrap the exec operation in a timeout
        let timeout_duration = Duration::from_secs(self.config.exec_timeout_secs);

        let exec_result = timeout(timeout_duration, async {
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

            Ok::<String, Box<dyn std::error::Error + Send + Sync>>(output.trim().to_string())
        })
        .await;

        match exec_result {
            Ok(result) => result,
            Err(_) => Err(format!(
                "Command timed out after {} seconds: {}",
                self.config.exec_timeout_secs,
                command.join(" ")
            )
            .into()),
        }
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
