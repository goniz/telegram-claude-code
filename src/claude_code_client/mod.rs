use bollard::exec::{CreateExecOptions, StartExecOptions};
use bollard::Docker;
use futures_util::StreamExt;
use serde::{Deserialize, Serialize};
use tokio::io::AsyncWriteExt;
use tokio::sync::{mpsc, oneshot};

pub mod container_utils;
pub mod github_client;

#[allow(unused_imports)]
pub use github_client::{GithubAuthResult, GithubClient, GithubClientConfig, GithubCloneResult};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ClaudeCodeResult {
    pub r#type: String,
    pub subtype: String,
    pub cost_usd: f64,
    pub is_error: bool,
    pub duration_ms: u64,
    pub duration_api_ms: u64,
    pub num_turns: u32,
    pub result: String,
    pub session_id: String,
}

#[derive(Debug, Clone)]
#[allow(dead_code)]
pub struct ClaudeCodeConfig {
    pub model: String,
    pub max_tokens: Option<u32>,
    pub temperature: Option<f32>,
    pub working_directory: Option<String>,
}

#[derive(Debug, Clone)]
pub enum InteractiveLoginState {
    DarkMode,
    SelectLoginMethod,
    ProvideUrl(String),
    WaitingForCode,
    LoginSuccessful,
    SecurityNotes,
    TrustFiles,
    Completed,
    Error(String),
}

#[derive(Debug, Clone)]
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

#[derive(Debug, Clone)]
pub struct InteractiveLoginSession {
    pub state: InteractiveLoginState,
    pub url: Option<String>,
    pub awaiting_user_code: bool,
}

#[derive(Debug)]
#[allow(dead_code)]
pub struct ClaudeAuthProcess {
    exec_id: String,
    docker: std::sync::Arc<Docker>,
}

#[allow(dead_code)]
impl ClaudeAuthProcess {
    /// Wait for the authentication process to complete with a timeout
    pub async fn wait_for_completion(
        &self,
        timeout_secs: u64,
    ) -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
        use std::time::Duration;
        use tokio::time::timeout;

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
            "Claude authentication process timed out".into()
        })?
    }

    /// Terminate the authentication process gracefully
    #[allow(dead_code)]
    pub async fn terminate(&self) -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
        // Note: Docker exec doesn't have a direct kill method, but the process should
        // terminate when the user completes authentication or when the container is stopped
        Ok(())
    }
}

impl Default for ClaudeCodeConfig {
    fn default() -> Self {
        Self {
            model: "claude-sonnet-4".to_string(),
            max_tokens: None,
            temperature: None,
            working_directory: Some("/workspace".to_string()),
        }
    }
}

#[derive(Debug)]
#[allow(dead_code)]
pub struct ClaudeCodeClient {
    docker: Docker,
    container_id: String,
    config: ClaudeCodeConfig,
}

#[allow(dead_code)]
impl ClaudeCodeClient {
    /// Create a new Claude Code client for the specified container
    pub fn new(docker: Docker, container_id: String, config: ClaudeCodeConfig) -> Self {
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
                    cost_usd: 0.0,
                    is_error: output.to_lowercase().contains("error"),
                    duration_ms: 0,
                    duration_api_ms: 0,
                    num_turns: 1,
                    result: output,
                    session_id: "unknown".to_string(),
                })
            }
        }
    }

    /// Authenticate Claude Code using Claude account (OAuth flow)
    /// This initiates the account-based authentication process through interactive CLI
    /// Returns an AuthenticationHandle for channel-based communication
    pub async fn authenticate_claude_account(
        &self,
    ) -> Result<AuthenticationHandle, Box<dyn std::error::Error + Send + Sync>> {
        // Check if authentication is already set up
        match self.check_auth_status().await {
            Ok(true) => {
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
            Ok(false) => {
                // Launch Claude CLI in interactive mode and perform account authentication
                return self.spawn_interactive_claude_login().await;
            }
            Err(e) => {
                return Err(format!("Unable to check authentication status: {}", e).into());
            }
        }
    }

    /// Spawn interactive Claude login as background task with channel communication
    /// Returns an AuthenticationHandle for managing the authentication process
    async fn spawn_interactive_claude_login(
        &self,
    ) -> Result<AuthenticationHandle, Box<dyn std::error::Error + Send + Sync>> {
        let (state_sender, state_receiver) = mpsc::unbounded_channel();
        let (code_sender, code_receiver) = mpsc::unbounded_channel();
        let (cancel_sender, cancel_receiver) = oneshot::channel();

        // Clone necessary data for the background task
        let docker = self.docker.clone();
        let container_id = self.container_id.clone();
        let config = self.config.clone();

        // Spawn the background authentication task
        tokio::spawn(async move {
            let client = ClaudeCodeClient::new(docker, container_id, config);
            let _ = client.background_interactive_login(state_sender, code_receiver, cancel_receiver).await;
        });

        Ok(AuthenticationHandle {
            state_receiver,
            code_sender,
            cancel_sender,
        })
    }

    /// Background task for interactive Claude login with channel communication
    async fn background_interactive_login(
        &self,
        state_sender: mpsc::UnboundedSender<AuthState>,
        mut code_receiver: mpsc::UnboundedReceiver<String>,
        mut cancel_receiver: oneshot::Receiver<()>,
    ) -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
        use std::time::Duration;
        use tokio::time::sleep;

        // Send starting state
        let _ = state_sender.send(AuthState::Starting);

        // Create exec with TTY enabled for interactive mode
        let exec_config = CreateExecOptions {
            cmd: Some(vec!["claude".to_string()]),
            attach_stdin: Some(true),
            attach_stdout: Some(true),
            attach_stderr: Some(true),
            tty: Some(true),
            working_dir: self.config.working_directory.clone(),
            env: Some(vec![
                "PATH=/root/.nvm/versions/node/v22.16.0/bin:/root/.nvm/versions/node/v20.19.2/bin:\
                 /root/.nvm/versions/node/v18.20.8/bin:/usr/local/sbin:/usr/local/bin:/usr/sbin:/\
                 usr/bin:/sbin:/bin"
                    .to_string(),
                "NODE_PATH=/root/.nvm/versions/node/v22.16.0/lib/node_modules".to_string(),
                "TERM=xterm".to_string(),
            ]),
            ..Default::default()
        };

        let exec = match self.docker.create_exec(&self.container_id, exec_config).await {
            Ok(exec) => exec,
            Err(e) => {
                let _ = state_sender.send(AuthState::Failed(format!("Failed to create exec: {}", e)));
                return Err(e.into());
            }
        };

        let start_config = StartExecOptions {
            detach: false,
            tty: true,
            ..Default::default()
        };

        let mut session = InteractiveLoginSession {
            state: InteractiveLoginState::DarkMode,
            url: None,
            awaiting_user_code: false,
        };

        match self.docker.start_exec(&exec.id, Some(start_config)).await? {
            bollard::exec::StartExecResults::Attached { mut output, input } => {
                // Give some time for the interactive CLI to start
                sleep(Duration::from_millis(500)).await;

                let mut stdin = input;

                // Process the interactive session with channel communication
                let timeout_result = tokio::time::timeout(Duration::from_secs(300), async {
                    loop {
                        tokio::select! {
                            // Handle cancellation - only if the sender explicitly sends cancellation
                            result = &mut cancel_receiver => {
                                if result.is_ok() {
                                    log::info!("Authentication cancelled by user");
                                    let _ = state_sender.send(AuthState::Failed("Authentication cancelled".to_string()));
                                    return Ok(());
                                }
                                // If cancel_receiver errors (sender dropped), continue normally
                                // This prevents immediate cancellation when sender is dropped
                            }
                            
                            // Handle auth code input from user
                            code = code_receiver.recv() => {
                                if let Some(code) = code {
                                    log::info!("Received auth code from user, sending to CLI");
                                    if let Err(e) = stdin.write_all(format!("{}\n", code).as_bytes()).await {
                                        let _ = state_sender.send(AuthState::Failed(format!("Failed to send code: {}", e)));
                                        return Err(e.into());
                                    }
                                    if let Err(e) = stdin.flush().await {
                                        let _ = state_sender.send(AuthState::Failed(format!("Failed to flush stdin: {}", e)));
                                        return Err(e.into());
                                    }
                                }
                            }
                            
                            // Handle CLI output
                            msg = output.next() => {
                                if let Some(Ok(msg)) = msg {
                                    let text = match msg {
                                        bollard::container::LogOutput::StdOut { message } => {
                                            String::from_utf8_lossy(&message).to_string()
                                        }
                                        bollard::container::LogOutput::StdErr { message } => {
                                            String::from_utf8_lossy(&message).to_string()
                                        }
                                        _ => continue,
                                    };

                                    log::debug!("Claude CLI output: {}", text);

                                    // Update state based on output
                                    let new_state = self.parse_cli_output_for_state(&text);

                                    match &new_state {
                                        InteractiveLoginState::DarkMode => {
                                            log::debug!("Dark mode detected, pressing enter");
                                            stdin.write_all(b"\n").await?;
                                            stdin.flush().await?;
                                            session.state = new_state.clone();
                                        }
                                        InteractiveLoginState::SelectLoginMethod => {
                                            log::debug!("Select login method detected, choosing option 1");
                                            stdin.write_all(b"1\n").await?;
                                            stdin.flush().await?;
                                            session.state = new_state.clone();
                                        }
                                        InteractiveLoginState::ProvideUrl(url) => {
                                            log::info!("Authentication URL detected: {}", url);
                                            session.url = Some(url.clone());
                                            session.state = new_state.clone();

                                            // Send URL to user via channel
                                            let _ = state_sender.send(AuthState::UrlReady(url.clone()));
                                        }
                                        InteractiveLoginState::WaitingForCode => {
                                            log::info!("Authentication code required - waiting for user input");
                                            session.awaiting_user_code = true;
                                            session.state = new_state.clone();
                                            
                                            // Notify that we're waiting for a code
                                            let _ = state_sender.send(AuthState::WaitingForCode);
                                        }
                                        InteractiveLoginState::LoginSuccessful => {
                                            log::info!("Login successful, pressing enter to continue");
                                            stdin.write_all(b"\n").await?;
                                            stdin.flush().await?;
                                            session.state = new_state.clone();
                                        }
                                        InteractiveLoginState::SecurityNotes => {
                                            log::debug!("Security notes detected, pressing enter to continue");
                                            stdin.write_all(b"\n").await?;
                                            stdin.flush().await?;
                                            session.state = new_state.clone();
                                        }
                                        InteractiveLoginState::TrustFiles => {
                                            log::info!("Trust files prompt detected, completing authentication");
                                            stdin.write_all(b"\n").await?;
                                            stdin.flush().await?;
                                            session.state = InteractiveLoginState::Completed;

                                            let success_msg = "✅ **Claude Authentication Completed!**\n\nYour Claude account has been successfully authenticated.\n\nYou can now use Claude Code with your account privileges.".to_string();
                                            let _ = state_sender.send(AuthState::Completed(success_msg));
                                            return Ok(());
                                        }
                                        InteractiveLoginState::Completed => {
                                            log::info!("Authentication process completed");
                                            let success_msg = "✅ Claude Code authentication completed successfully!".to_string();
                                            let _ = state_sender.send(AuthState::Completed(success_msg));
                                            return Ok(());
                                        }
                                        InteractiveLoginState::Error(err) => {
                                            log::warn!("Error in interactive login: {}", err);
                                            let _ = state_sender.send(AuthState::Failed(err.clone()));
                                            return Err(err.clone().into());
                                        }
                                    }

                                    // Small delay between state transitions
                                    sleep(Duration::from_millis(200)).await;
                                } else {
                                    // Stream ended
                                    log::info!("CLI output stream ended");
                                    break;
                                }
                            }
                        }
                    }

                    Ok::<(), Box<dyn std::error::Error + Send + Sync>>(())
                }).await;

                match timeout_result {
                    Ok(Ok(())) => {
                        log::info!("Authentication completed successfully");
                    }
                    Ok(Err(e)) => {
                        log::error!("Error in interactive login: {}", e);
                        let _ = state_sender.send(AuthState::Failed(format!("Authentication error: {}", e)));
                    }
                    Err(_) => {
                        log::warn!("Timeout in interactive login after 5 minutes");
                        let _ = state_sender.send(AuthState::Failed("Authentication timed out after 5 minutes".to_string()));
                    }
                }
            }
            bollard::exec::StartExecResults::Detached => {
                let _ = state_sender.send(AuthState::Failed("Unexpected detached execution in interactive mode".to_string()));
                return Err("Unexpected detached execution in interactive mode".into());
            }
        }

        Ok(())
    }

    /// Parse CLI output to determine the current state
    fn parse_cli_output_for_state(&self, output: &str) -> InteractiveLoginState {
        let output_lower = output.to_lowercase();

        if output_lower.contains("dark mode") {
            InteractiveLoginState::DarkMode
        } else if output_lower.contains("select login method") {
            InteractiveLoginState::SelectLoginMethod
        } else if output_lower.contains("use the url below to sign in") {
            // Extract URL from output
            for line in output.lines() {
                let trimmed = line.trim();
                if trimmed.starts_with("https://") {
                    return InteractiveLoginState::ProvideUrl(trimmed.to_string());
                }
            }

            // If no URL found on separate line, look for URL pattern in the text
            if let Some(url_start) = output.find("https://") {
                let url_part = &output[url_start..];
                if let Some(url_end) = url_part.find(char::is_whitespace) {
                    let url = &url_part[..url_end];
                    return InteractiveLoginState::ProvideUrl(url.to_string());
                } else {
                    // URL goes to end of string
                    return InteractiveLoginState::ProvideUrl(url_part.trim().to_string());
                }
            }

            InteractiveLoginState::Error("URL not found in sign-in output".to_string())
        } else if output_lower.contains("paste code here if prompted") {
            InteractiveLoginState::WaitingForCode
        } else if output_lower.contains("login successful") {
            InteractiveLoginState::LoginSuccessful
        } else if output_lower.contains("security notes") {
            InteractiveLoginState::SecurityNotes
        } else if output_lower.contains("do you trust the files in this folder") {
            InteractiveLoginState::TrustFiles
        } else {
            // Don't treat everything as an error, just continue with current state
            InteractiveLoginState::DarkMode // Default state to continue processing
        }
    }

    /// Fallback authentication instructions if interactive mode fails
    async fn get_fallback_auth_instructions(&self) -> String {
        r#"🔐 **Claude Account Authentication**

To authenticate with your Claude account, please follow these steps:

**1. Start Claude CLI interactively:**
   Run `claude` in your terminal

**2. Use the login command:**
   Type `/login` and press Enter

**3. Select account authentication:**
   Choose option 1 for "Account authentication"

**4. Follow the OAuth flow:**
   - Visit the provided authentication URL
   - Sign in with your Claude Pro/Team account
   - Complete the authorization process

**5. Return to Claude Code:**
   Once authenticated, Claude Code will have access to your account

✨ **Benefits:**
- Full integration with your Claude subscription
- Access to all your Claude Pro/Team features
- No separate API key management required

💡 **Note:** If you encounter issues, ensure you have a valid Claude account and subscription."#
            .to_string()
    }

    /// Check authentication status
    pub async fn check_auth_status(
        &self,
    ) -> Result<bool, Box<dyn std::error::Error + Send + Sync>> {
        // Try to run a simple claude command with JSON output to check if authentication is working
        let command = vec![
            "claude".to_string(),
            "-p".to_string(),
            "  ".to_string(),
            "--output-format".to_string(),
            "json".to_string(),
        ];

        match self.exec_command(command).await {
            Ok(output) => {
                // Try to parse the JSON output
                match self.parse_result(output) {
                    Ok(result) => {
                        // Authentication is successful if there's no error in the result
                        Ok(!result.is_error)
                    }
                    Err(e) => {
                        // If JSON parsing fails, bubble up the error
                        Err(e)
                    }
                }
            }
            Err(_) => {
                // Other errors (network, container issues, etc.) should be bubbled up
                Ok(false)
            }
        }
    }

    /// Get current authentication info
    pub async fn get_auth_info(&self) -> Result<String, Box<dyn std::error::Error + Send + Sync>> {
        // Check if authenticated and return status
        match self.check_auth_status().await {
            Ok(true) => Ok("✅ Claude Code is authenticated and ready to use".to_string()),
            Ok(false) => Ok(
                "❌ Claude Code is not authenticated. Please set up your Anthropic API key."
                    .to_string(),
            ),
            Err(e) => Err(format!("Unable to check authentication status: {}", e).into()),
        }
    }

    /// Check Claude Code version and availability
    pub async fn check_availability(
        &self,
    ) -> Result<String, Box<dyn std::error::Error + Send + Sync>> {
        let command = vec!["claude".to_string(), "--version".to_string()];
        self.exec_command(command).await
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
        if let Some(exit_code) = exec_inspect.exit_code {
            if exit_code != 0 {
                // Command failed - return error with the output
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
}
