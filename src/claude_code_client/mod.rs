use bollard::Docker;
use bollard::exec::{CreateExecOptions, StartExecOptions};
use futures_util::StreamExt;
use serde::{Deserialize, Serialize};
use tokio::sync::{mpsc, oneshot};

pub mod container_utils;
pub mod github_client;
pub mod container_cred_storage;

#[allow(unused_imports)]
pub use github_client::{GithubAuthResult, GithubClient, GithubClientConfig, GithubCloneResult};
pub use container_cred_storage::ContainerCredStorage;

// Re-export OAuth types from claude_oauth module
pub use crate::claude_oauth::{ClaudeAuth, Config as OAuthConfig, Credentials, OAuthError};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Usage {
    pub input_tokens: u64,
    #[serde(default)]
    pub cache_creation_input_tokens: Option<u64>,
    #[serde(default)]
    pub cache_read_input_tokens: Option<u64>,
    pub output_tokens: u64,
    #[serde(default)]
    pub server_tool_use: Option<ServerToolUse>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ServerToolUse {
    #[serde(default)]
    pub web_search_requests: u64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ClaudeCodeResult {
    pub r#type: String,
    pub subtype: String,
    #[serde(alias = "cost_usd")]
    pub total_cost_usd: f64,
    pub is_error: bool,
    pub duration_ms: u64,
    pub duration_api_ms: u64,
    pub num_turns: u32,
    pub result: String,
    pub session_id: String,
    #[serde(default)]
    pub usage: Option<Usage>,
}

#[derive(Debug, Clone)]
#[allow(dead_code)]
pub struct ClaudeCodeConfig {
    pub model: String,
    pub max_tokens: Option<u32>,
    pub temperature: Option<f32>,
    pub working_directory: Option<String>,
    /// OAuth configuration for Claude authentication
    pub oauth_config: OAuthConfig,
}

#[derive(Debug, Clone, PartialEq)]
pub enum AuthState {
    Starting,
    UrlReady(String),
    WaitingForCode,
    Completed(String),
    Failed(String),
}

#[derive(Debug)]
pub struct AuthenticationHandle {
    pub state_receiver: mpsc::UnboundedReceiver<AuthState>,
    pub code_sender: mpsc::UnboundedSender<String>,
    pub cancel_sender: oneshot::Sender<()>,
}

impl Default for ClaudeCodeConfig {
    fn default() -> Self {
        Self {
            model: "claude-sonnet-4".to_string(),
            max_tokens: None,
            temperature: None,
            working_directory: Some("/workspace".to_string()),
            oauth_config: OAuthConfig::default(),
        }
    }
}

#[derive(Debug)]
#[allow(dead_code)]
pub struct ClaudeCodeClient {
    docker: Docker,
    container_id: String,
    config: ClaudeCodeConfig,
    oauth_client: ClaudeAuth,
}

#[allow(dead_code)]
impl ClaudeCodeClient {
    /// Create a new Claude Code client for the specified container
    pub fn new(docker: Docker, container_id: String, config: ClaudeCodeConfig) -> Self {
        let storage_dir = std::env::current_dir().unwrap_or_else(|_| std::path::PathBuf::from("."));
        let oauth_client = ClaudeAuth::with_file_storage(config.oauth_config.clone(), storage_dir);

        Self {
            docker,
            container_id,
            config,
            oauth_client,
        }
    }

    /// Get the container ID
    pub fn container_id(&self) -> &str {
        &self.container_id
    }

    /// Parse the output from Claude Code and handle different response formats
    fn parse_result(
        &self,
        output: String,
    ) -> Result<ClaudeCodeResult, Box<dyn std::error::Error + Send + Sync>> {
        match serde_json::from_str::<ClaudeCodeResult>(&output) {
            Ok(result) => Ok(result),
            Err(_) => {
                // If JSON parsing fails, create a simple result with the raw output
                Ok(ClaudeCodeResult {
                    r#type: "result".to_string(),
                    subtype: if output.to_lowercase().contains("error") {
                        "error"
                    } else {
                        "success"
                    }
                    .to_string(),
                    total_cost_usd: 0.0,
                    is_error: output.to_lowercase().contains("error"),
                    duration_ms: 0,
                    duration_api_ms: 0,
                    num_turns: 1,
                    result: output,
                    session_id: "unknown".to_string(),
                    usage: None,
                })
            }
        }
    }

    /// Authenticate Claude Code using OAuth 2.0 flow
    /// Returns an AuthenticationHandle for channel-based communication
    pub async fn authenticate_claude_account(
        &self,
    ) -> Result<AuthenticationHandle, Box<dyn std::error::Error + Send + Sync>> {
        // Check if valid credentials already exist
        match self.oauth_client.load_credentials().await {
            Ok(Some(credentials)) if !credentials.is_expired() => {
                // Check if credentials work with Claude Code
                if let Ok(true) = self.check_oauth_credentials(&credentials).await {
                    let (state_sender, state_receiver) = mpsc::unbounded_channel();
                    let (code_sender, _code_receiver) = mpsc::unbounded_channel();
                    let (cancel_sender, _cancel_receiver) = oneshot::channel();

                    // Send immediate completion state
                    let _ = state_sender.send(AuthState::Completed(
                        "✅ Claude Code is already authenticated and ready to use!".to_string(),
                    ));

                    return Ok(AuthenticationHandle {
                        state_receiver,
                        code_sender,
                        cancel_sender,
                    });
                }
            }
            _ => {
                // No valid credentials found, start OAuth flow
            }
        }

        // Start OAuth authentication flow
        self.start_oauth_flow().await
    }

    /// Start OAuth authentication flow with channel communication
    async fn start_oauth_flow(
        &self,
    ) -> Result<AuthenticationHandle, Box<dyn std::error::Error + Send + Sync>> {
        let (state_sender, state_receiver) = mpsc::unbounded_channel();
        let (code_sender, code_receiver) = mpsc::unbounded_channel();
        let (cancel_sender, cancel_receiver) = oneshot::channel();

        // Clone necessary data for the background task
        let storage_dir = std::env::current_dir().unwrap_or_else(|_| std::path::PathBuf::from("."));
        let oauth_client =
            ClaudeAuth::with_file_storage(self.config.oauth_config.clone(), storage_dir);
        let docker = self.docker.clone();
        let container_id = self.container_id.clone();

        // Spawn the background authentication task
        tokio::spawn(async move {
            let _ = Self::background_oauth_flow(
                oauth_client,
                docker,
                container_id,
                state_sender,
                code_receiver,
                cancel_receiver,
            )
            .await;
        });

        Ok(AuthenticationHandle {
            state_receiver,
            code_sender,
            cancel_sender,
        })
    }

    /// Background task for OAuth authentication flow
    async fn background_oauth_flow(
        oauth_client: ClaudeAuth,
        docker: Docker,
        container_id: String,
        state_sender: mpsc::UnboundedSender<AuthState>,
        mut code_receiver: mpsc::UnboundedReceiver<String>,
        cancel_receiver: oneshot::Receiver<()>,
    ) -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
        use std::time::Duration;

        log::debug!(
            "Starting OAuth authentication flow for container: {}",
            container_id
        );

        // Send starting state
        let _ = state_sender.send(AuthState::Starting);

        // Generate OAuth login URL
        let login_url = match oauth_client.generate_login_url().await {
            Ok(url) => {
                log::info!("Generated OAuth login URL: {}", url);
                url
            }
            Err(e) => {
                log::error!("Failed to generate OAuth login URL: {}", e);
                let _ = state_sender.send(AuthState::Failed(format!(
                    "Failed to generate login URL: {}",
                    e
                )));
                return Err(e.into());
            }
        };

        // Send URL to user
        let _ = state_sender.send(AuthState::UrlReady(login_url));
        let _ = state_sender.send(AuthState::WaitingForCode);

        // Wait for authorization code with timeout
        let timeout_result = tokio::time::timeout(Duration::from_secs(300), async {
            tokio::select! {
                // Handle cancellation
                _ = cancel_receiver => {
                    log::info!("OAuth authentication cancelled by user");
                    let _ = state_sender.send(AuthState::Failed("Authentication cancelled".to_string()));
                    return Ok(());
                }
                
                // Handle auth code input
                code = code_receiver.recv() => {
                    if let Some(auth_code) = code {
                        log::info!("Received authorization code from user");
                        
                        // Exchange authorization code for tokens
                        match oauth_client.exchange_code(&auth_code).await {
                            Ok(credentials) => {
                                log::info!("Successfully obtained OAuth credentials");
                                
                                // Save credentials to file
                                if let Err(e) = oauth_client.save_credentials(&credentials).await {
                                    log::warn!("Failed to save credentials: {}", e);
                                }
                                
                                // Test credentials with Claude Code
                                let client = ClaudeCodeClient::new(
                                    docker.clone(),
                                    container_id.clone(),
                                    ClaudeCodeConfig::default(),
                                );
                                
                                match client.setup_oauth_credentials(&credentials).await {
                                    Ok(_) => {
                                        log::info!("Successfully configured Claude Code with OAuth credentials");
                                        let _ = oauth_client.cleanup_state().await;
                                        let _ = state_sender.send(AuthState::Completed(
                                            "✅ Claude Code authentication completed successfully!".to_string()
                                        ));
                                    }
                                    Err(e) => {
                                        log::error!("Failed to configure Claude Code with credentials: {}", e);
                                        let _ = state_sender.send(AuthState::Failed(
                                            format!("Failed to configure credentials: {}", e)
                                        ));
                                    }
                                }
                            }
                            Err(e) => {
                                log::error!("Failed to exchange authorization code: {}", e);
                                let _ = state_sender.send(AuthState::Failed(
                                    format!("Failed to exchange authorization code: {}", e)
                                ));
                            }
                        }
                    } else {
                        let _ = state_sender.send(AuthState::Failed("No authorization code received".to_string()));
                    }
                }
            }
            Ok::<(), Box<dyn std::error::Error + Send + Sync>>(())
        }).await;

        match timeout_result {
            Ok(Ok(())) => {
                log::info!("OAuth authentication completed successfully");
            }
            Ok(Err(e)) => {
                log::error!("Error during OAuth authentication: {}", e);
                let _ =
                    state_sender.send(AuthState::Failed(format!("Authentication error: {}", e)));
            }
            Err(_) => {
                log::warn!("OAuth authentication timed out after 5 minutes");
                let _ = state_sender.send(AuthState::Failed(
                    "Authentication timed out after 5 minutes".to_string(),
                ));
            }
        }

        Ok(())
    }

    /// Check if OAuth credentials are valid by testing them with Claude Code
    async fn check_oauth_credentials(
        &self,
        credentials: &Credentials,
    ) -> Result<bool, Box<dyn std::error::Error + Send + Sync>> {
        log::debug!("Checking OAuth credentials validity");

        // Try to use the credentials with Claude Code
        match self.setup_oauth_credentials(credentials).await {
            Ok(_) => {
                // Test with a simple status command
                match self.check_auth_status().await {
                    Ok(true) => {
                        log::info!("OAuth credentials are valid and working");
                        Ok(true)
                    }
                    Ok(false) => {
                        log::warn!(
                            "OAuth credentials exist but Claude Code reports not authenticated"
                        );
                        Ok(false)
                    }
                    Err(e) => {
                        log::warn!("Failed to check auth status with OAuth credentials: {}", e);
                        Ok(false)
                    }
                }
            }
            Err(e) => {
                log::warn!("Failed to setup OAuth credentials: {}", e);
                Ok(false)
            }
        }
    }

    /// Setup OAuth credentials in the container for Claude Code to use
    async fn setup_oauth_credentials(
        &self,
        credentials: &Credentials,
    ) -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
        log::debug!("Setting up OAuth credentials in container");

        // Write credentials to file in the container
        let credentials_json = serde_json::to_string_pretty(&serde_json::json!({
            "claudeAiOauth": credentials
        }))?;

        // Use a temporary file approach to write credentials
        let temp_path = format!("/tmp/claude_credentials_{}.json", self.container_id);
        std::fs::write(&temp_path, &credentials_json)?;

        // Copy credentials file into container
        let copy_command = format!(
            "docker cp {} {}:/root/.claude/credentials.json",
            temp_path, self.container_id
        );

        let copy_result = tokio::process::Command::new("sh")
            .arg("-c")
            .arg(&copy_command)
            .output()
            .await?;

        // Clean up temporary file
        let _ = std::fs::remove_file(&temp_path);

        if !copy_result.status.success() {
            let error = String::from_utf8_lossy(&copy_result.stderr);
            return Err(format!("Failed to copy credentials to container: {}", error).into());
        }

        // Set proper permissions
        let chmod_command = vec![
            "chmod".to_string(),
            "600".to_string(),
            "/root/.claude/credentials.json".to_string(),
        ];

        self.exec_command(chmod_command).await?;

        log::info!("Successfully setup OAuth credentials in container");
        Ok(())
    }

    /// Check authentication status using OAuth credentials
    pub async fn check_auth_status(
        &self,
    ) -> Result<bool, Box<dyn std::error::Error + Send + Sync>> {
        log::debug!(
            "Checking Claude authentication status for container: {}",
            self.container_id
        );

        // First, check if OAuth credentials exist and are valid
        match self.oauth_client.load_credentials().await {
            Ok(Some(credentials)) if !credentials.is_expired() => {
                log::debug!("Found valid OAuth credentials");

                // Test with Claude Code status command
                let status_command = vec![
                    "claude".to_string(),
                    "status".to_string(),
                    "--output-format".to_string(),
                    "json".to_string(),
                ];

                match self.exec_command(status_command).await {
                    Ok(output) => {
                        log::debug!("Claude status command output: '{}'", output);

                        match self.parse_result(output) {
                            Ok(result) => {
                                let is_authenticated = !result.is_error;
                                log::info!("Authentication status: {}", is_authenticated);
                                Ok(is_authenticated)
                            }
                            Err(e) => {
                                log::warn!("Failed to parse status output: {}", e);
                                Ok(false)
                            }
                        }
                    }
                    Err(e) => {
                        log::warn!("Claude status command failed: {}", e);
                        Ok(false)
                    }
                }
            }
            Ok(Some(_credentials)) => {
                log::warn!("OAuth credentials are expired");
                Ok(false)
            }
            Ok(None) => {
                log::debug!("No OAuth credentials found");
                Ok(false)
            }
            Err(e) => {
                log::warn!("Failed to load OAuth credentials: {}", e);
                Ok(false)
            }
        }
    }

    /// Get current authentication info
    pub async fn get_auth_info(&self) -> Result<String, Box<dyn std::error::Error + Send + Sync>> {
        match self.check_auth_status().await {
            Ok(true) => {
                // Get additional credential info if available
                match self.oauth_client.load_credentials().await {
                    Ok(Some(credentials)) => {
                        let expires_info = if let Some(seconds) = credentials.expires_in_seconds() {
                            format!(" (expires in {} seconds)", seconds)
                        } else {
                            " (expired)".to_string()
                        };
                        
                        Ok(format!(
                            "✅ Claude Code is authenticated and ready to use{}\nScopes: {}",
                            expires_info,
                            credentials.scopes.join(", ")
                        ))
                    }
                    _ => Ok("✅ Claude Code is authenticated and ready to use".to_string()),
                }
            }
            Ok(false) => Ok(
                "❌ Claude Code is not authenticated. Please authenticate with your Claude account using OAuth."
                    .to_string(),
            ),
            Err(e) => Err(format!("Unable to check authentication status: {}", e).into()),
        }
    }

    /// Refresh OAuth credentials if they are expired but refresh token is valid
    pub async fn refresh_credentials(
        &self,
    ) -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
        log::debug!("Attempting to refresh OAuth credentials");

        match self.oauth_client.load_credentials().await {
            Ok(Some(credentials)) if credentials.is_expired() => {
                log::info!("Credentials are expired, but refresh is not implemented yet");
                // TODO: Implement refresh token flow in claude_oauth module
                Err("Token refresh not yet implemented".into())
            }
            Ok(Some(_)) => {
                log::debug!("Credentials are still valid, no refresh needed");
                Ok(())
            }
            Ok(None) => {
                log::debug!("No credentials found to refresh");
                Err("No credentials found".into())
            }
            Err(e) => {
                log::error!("Failed to load credentials for refresh: {}", e);
                Err(e.into())
            }
        }
    }

    /// Check Claude Code version and availability
    pub async fn check_availability(
        &self,
    ) -> Result<String, Box<dyn std::error::Error + Send + Sync>> {
        let command = vec!["claude".to_string(), "--version".to_string()];
        self.exec_command(command).await
    }

    /// Update Claude CLI to latest version
    pub async fn update_claude(&self) -> Result<String, Box<dyn std::error::Error + Send + Sync>> {
        let command = vec![
            "sh".to_string(),
            "-c".to_string(),
            "/opt/entrypoint.sh -c \"nvm use default && claude update\"".to_string(),
        ];
        self.exec_command(command).await
    }

    /// Execute a command in the container and return output
    async fn exec_command(
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

// Usage example for integration with the Telegram bot
#[allow(dead_code)]
impl ClaudeCodeClient {
    /// Helper method to create a client for a coding session
    pub async fn for_session(
        docker: Docker,
        container_name: &str,
    ) -> Result<Self, Box<dyn std::error::Error + Send + Sync>> {
        // Find the container by name
        let containers = docker
            .list_containers(None::<bollard::container::ListContainersOptions<String>>)
            .await?;

        let container = containers
            .iter()
            .find(|c| {
                c.names
                    .as_ref()
                    .map(|names| {
                        names
                            .iter()
                            .any(|name| name.trim_start_matches('/') == container_name)
                    })
                    .unwrap_or(false)
            })
            .ok_or("Container not found")?;

        let container_id = container
            .id
            .as_ref()
            .ok_or("Container ID not found")?
            .clone();

        Ok(Self::new(docker, container_id, ClaudeCodeConfig::default()))
    }

    /// Create a client with custom OAuth configuration
    pub fn with_oauth_config(
        docker: Docker,
        container_id: String,
        oauth_config: OAuthConfig,
    ) -> Self {
        let config = ClaudeCodeConfig {
            oauth_config,
            ..Default::default()
        };
        Self::new(docker, container_id, config)
    }
}
