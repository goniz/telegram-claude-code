use bollard::exec::{CreateExecOptions, StartExecOptions};
use bollard::Docker;
use futures_util::StreamExt;
use std::sync::Arc;
use tokio::time::{timeout, Duration};

use super::types::{GithubAuthResult, GithubClientConfig};

#[derive(Debug)]
pub struct OAuthProcess {
    exec_id: String,
    docker: Arc<Docker>,
}

impl OAuthProcess {
    /// Wait for the OAuth process to complete with a timeout
    pub async fn wait_for_completion(
        &self,
        timeout_secs: u64,
    ) -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
        log::debug!(
            "Waiting for OAuth process {} to complete (timeout: {}s)",
            self.exec_id,
            timeout_secs
        );

        let wait_result = timeout(Duration::from_secs(timeout_secs), async {
            // Poll the exec process until it completes
            loop {
                match self.docker.inspect_exec(&self.exec_id).await {
                    Ok(inspect) => {
                        if let Some(running) = inspect.running {
                            if !running {
                                log::debug!("OAuth process {} has completed", self.exec_id);
                                break;
                            }
                        }
                    }
                    Err(e) => {
                        log::warn!("Failed to inspect OAuth process {}: {}", self.exec_id, e);
                        break;
                    }
                }
                tokio::time::sleep(Duration::from_millis(500)).await;
            }
        })
        .await;

        match wait_result {
            Ok(_) => {
                log::info!("OAuth process completed successfully");
                Ok(())
            }
            Err(_) => {
                log::warn!("OAuth process timed out after {} seconds", timeout_secs);
                self.terminate().await?;
                Err("OAuth process timed out".into())
            }
        }
    }

    /// Terminate the OAuth process if it's still running
    pub async fn terminate(&self) -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
        log::debug!("Terminating OAuth process {}", self.exec_id);
        // Note: Docker doesn't provide direct exec termination, but the process should
        // auto-terminate when the authentication completes or times out
        Ok(())
    }
}

/// GitHub authentication functionality
#[derive(Debug)]
pub struct GitHubAuth {
    docker: Docker,
    container_id: String,
    config: GithubClientConfig,
}

impl GitHubAuth {
    pub fn new(docker: Docker, container_id: String, config: GithubClientConfig) -> Self {
        Self {
            docker,
            container_id,
            config,
        }
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

    /// Parse OAuth response from command output to extract URL and device code
    fn parse_oauth_response(&self, output: &str) -> (Option<String>, Option<String>) {
        let mut oauth_url = None;
        let mut device_code = None;

        // Look for patterns like:
        // "https://github.com/login/device"
        // "enter code: XXXX-XXXX"
        for line in output.lines() {
            let line = line.trim();

            // Look for GitHub device URL
            if line.contains("https://github.com/login/device") {
                if let Some(url_start) = line.find("https://github.com/login/device") {
                    let remaining = &line[url_start..];
                    if let Some(url_end) = remaining.find(char::is_whitespace) {
                        oauth_url = Some(remaining[..url_end].to_string());
                    } else {
                        oauth_url = Some(remaining.to_string());
                    }
                }
            }

            // Look for device code patterns
            if line.to_lowercase().contains("code") && line.contains("-") {
                // Extract codes that look like XXXX-XXXX or similar GitHub device code patterns
                let words: Vec<&str> = line.split_whitespace().collect();
                for word in words {
                    if word.contains("-") && word.len() >= 7 && word.len() <= 12 {
                        // Skip descriptive words like "one-time", "multi-factor", etc.
                        let word_lower = word.to_lowercase();
                        if word_lower == "one-time" || word_lower == "multi-factor" || 
                           word_lower == "two-factor" || word_lower.contains("time") {
                            continue;
                        }
                        
                        // GitHub device codes are typically alphanumeric with hyphens
                        // and should have at least 4 alphanumeric characters on each side of hyphen
                        if word.chars().all(|c| c.is_alphanumeric() || c == '-') {
                            let parts: Vec<&str> = word.split('-').collect();
                            if parts.len() == 2 && 
                               parts[0].len() >= 4 && parts[1].len() >= 4 &&
                               parts[0].chars().all(|c| c.is_alphanumeric()) &&
                               parts[1].chars().all(|c| c.is_alphanumeric()) {
                                device_code = Some(word.to_string());
                                break;
                            }
                        }
                    }
                }
            }
        }

        (oauth_url, device_code)
    }

    /// Wait for OAuth completion after user has visited the URL
    pub async fn wait_for_oauth_completion(
        &self,
        oauth_process: OAuthProcess,
    ) -> Result<GithubAuthResult, Box<dyn std::error::Error + Send + Sync>> {
        log::info!("Waiting for OAuth completion...");

        match oauth_process
            .wait_for_completion(self.config.exec_timeout_secs)
            .await
        {
            Ok(_) => {
                log::info!("OAuth process completed, checking final auth status");
                self.check_auth_status().await
            }
            Err(e) => {
                log::warn!("OAuth completion failed: {}", e);
                Ok(GithubAuthResult {
                    authenticated: false,
                    username: None,
                    message: format!("OAuth completion failed: {}", e),
                    oauth_url: None,
                    device_code: None,
                })
            }
        }
    }

    /// Check authentication status
    pub async fn check_auth_status(
        &self,
    ) -> Result<GithubAuthResult, Box<dyn std::error::Error + Send + Sync>> {
        let auth_command = vec!["gh".to_string(), "auth".to_string(), "status".to_string()];

        let exec_config = CreateExecOptions {
            cmd: Some(auth_command),
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

        // Check the exit code
        let exec_inspect = self.docker.inspect_exec(&exec.id).await?;
        let exit_code = exec_inspect.exit_code.unwrap_or(-1);

        let username = if exit_code == 0 {
            self.extract_username_from_status(&output)
        } else {
            None
        };

        Ok(GithubAuthResult {
            authenticated: exit_code == 0,
            username,
            message: if exit_code == 0 {
                "Authenticated with GitHub".to_string()
            } else {
                "Not authenticated with GitHub".to_string()
            },
            oauth_url: None,
            device_code: None,
        })
    }

    /// Extract username from gh auth status output
    fn extract_username_from_status(&self, output: &str) -> Option<String> {
        for line in output.lines() {
            if line.contains("Logged in to github.com as") {
                let parts: Vec<&str> = line.split_whitespace().collect();
                if let Some(username_part) = parts.last() {
                    // Remove any trailing punctuation
                    let username = username_part.trim_end_matches(&['(', ')', '.', ','][..]);
                    return Some(username.to_string());
                }
            }
        }
        None
    }
}
