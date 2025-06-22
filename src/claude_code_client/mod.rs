use bollard::exec::{CreateExecOptions, StartExecOptions};
use bollard::Docker;
use futures_util::StreamExt;
use serde::{Deserialize, Serialize};
use tokio::fs::OpenOptions;
use tokio::io::{AsyncWriteExt, BufWriter};
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

#[derive(Debug, Clone, PartialEq)]
pub enum InteractiveLoginState {
    Starting,
    SelectLoginMethod,
    ProvideUrl(String),
    WaitingForCode,
    LoginSuccessful,
    OAuthPortError,
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
    pub last_output: Option<String>,
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
    /// Strip ANSI escape sequences from text to clean CLI output
    fn strip_ansi_codes(text: &str) -> String {
        // Regular expression to match ANSI escape sequences
        // This matches: ESC[ followed by any number of digits, semicolons, and ends with a letter
        let mut result = String::new();
        let mut chars = text.chars().peekable();
        
        while let Some(ch) = chars.next() {
            if ch == '\x1b' {
                // Found escape character, check if it's followed by '['
                if chars.peek() == Some(&'[') {
                    chars.next(); // consume '['
                    // Skip until we find a letter (the command character)
                    while let Some(next_ch) = chars.next() {
                        if next_ch.is_ascii_alphabetic() {
                            break;
                        }
                    }
                } else {
                    // Not an ANSI sequence, keep the character
                    result.push(ch);
                }
            } else {
                result.push(ch);
            }
        }
        
        result
    }

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
        cancel_receiver: oneshot::Receiver<()>,
    ) -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
        use std::time::Duration;
        use tokio::time::sleep;

        // Entry point debug log
        log::debug!("Starting background_interactive_login function for container: {}", self.container_id);

        // Create log file for Claude CLI output
        let log_file_path = "/tmp/claude_auth_output.log";
        let log_file = match OpenOptions::new()
            .create(true)
            .write(true)
            .truncate(true)
            .open(log_file_path)
            .await
        {
            Ok(file) => {
                log::info!("Created Claude CLI output log file: {}", log_file_path);
                Some(BufWriter::new(file))
            }
            Err(e) => {
                log::warn!("Failed to create Claude CLI output log file: {}", e);
                None
            }
        };

        // Send starting state
        let _ = state_sender.send(AuthState::Starting);

        // Create exec with TTY enabled for interactive mode
        log::debug!("Creating exec configuration for Claude CLI interactive mode");
        let exec_config = CreateExecOptions {
            cmd: Some(vec!["claude".to_string(), "/login".to_string()]),
            attach_stdin: Some(true),
            attach_stdout: Some(true),
            attach_stderr: Some(true),
            tty: Some(true),
            privileged: Some(false),
            working_dir: self.config.working_directory.clone(),
            env: Some(vec![
                "PATH=/root/.nvm/versions/node/v22.16.0/bin:/root/.nvm/versions/node/v20.19.2/bin:\
                 /root/.nvm/versions/node/v18.20.8/bin:/usr/local/sbin:/usr/local/bin:/usr/sbin:/\
                 usr/bin:/sbin:/bin"
                    .to_string(),
                "NODE_PATH=/root/.nvm/versions/node/v22.16.0/lib/node_modules".to_string(),
                "TERM=xterm-256color".to_string(),
                "TERMINFO=/usr/share/terminfo".to_string(),
                "DEBIAN_FRONTEND=noninteractive".to_string(),
            ]),
            ..Default::default()
        };

        log::debug!("Attempting to create exec for container: {}", self.container_id);
        let exec = match self.docker.create_exec(&self.container_id, exec_config).await {
            Ok(exec) => {
                log::debug!("Successfully created exec with ID: {}", exec.id);
                exec
            },
            Err(e) => {
                log::error!("Failed to create exec for container {}: {}", self.container_id, e);
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
            state: InteractiveLoginState::Starting,
            url: None,
            awaiting_user_code: false,
            last_output: None,
        };

        log::debug!("Starting exec {} with interactive TTY", exec.id);
        match self.docker.start_exec(&exec.id, Some(start_config)).await? {
            bollard::exec::StartExecResults::Attached { mut output, input } => {
                log::debug!("Successfully attached to exec {}, waiting for CLI to initialize", exec.id);

                let mut stdin = input;
                let mut log_writer = log_file;

                // Wrap cancel_receiver in Option to handle one-time usage
                let mut cancel_receiver = Some(cancel_receiver);

                // Add buffering support for CLI output to prevent partial parsing
                let mut output_buffer = String::new();
                let buffer_timeout_ms = 200;

                // Process the interactive session with channel communication
                log::debug!("Starting interactive authentication loop with 2-minute timeout");
                let timeout_result = tokio::time::timeout(Duration::from_secs(120), async {
                    loop {
                        log::debug!("Waiting for events in authentication select loop");
                        tokio::select! {
                            // Handle cancellation - only if the sender explicitly sends cancellation
                            result = async {
                                if let Some(receiver) = cancel_receiver.take() {
                                    receiver.await
                                } else {
                                    // If receiver was already consumed, return a pending future
                                    std::future::pending::<Result<(), oneshot::error::RecvError>>().await
                                }
                            } => {
                                log::debug!("Received cancellation signal, result: {:?}", result);
                                if result.is_ok() {
                                    log::info!("Authentication cancelled by user");
                                    let _ = state_sender.send(AuthState::Failed("Authentication cancelled".to_string()));
                                    return Ok(());
                                }
                                log::debug!("Cancel receiver error (sender likely dropped), continuing authentication");
                                // If cancel_receiver errors (sender dropped), continue normally
                                // This prevents immediate cancellation when sender is dropped
                            }
                            
                            // Handle auth code input from user
                            code = code_receiver.recv() => {
                                log::debug!("Code receiver branch triggered");
                                if let Some(code) = code {
                                    log::info!("Received auth code from user, sending to CLI");
                                    log::debug!("Auth code: '{}' (length: {})", &code, code.len());
                                    log::debug!("Current session state: {:?}", session.state);

                                    if let Some(last_output) = session.last_output.as_ref() {
                                        if last_output.contains("Press Enter to retry") {
                                            stdin.write_all(b"\r").await?;
                                            stdin.flush().await?;
                                        }
                                    }

                                    // Send auth code with proper line ending
                                    let _ = stdin.write_all(code.as_bytes()).await;
                                    let _ = stdin.flush().await;
                                    
                                    // Add a small delay to ensure the TTY processes the input
                                    tokio::time::sleep(Duration::from_millis(100)).await;

                                    let _ = stdin.write_all(b"\r").await;
                                    let _ = stdin.flush().await;
                                    
                                    log::debug!("Successfully sent auth code to CLI");
                                } else {
                                    break;
                                }
                            }
                            
                            // Handle CLI output
                            msg = output.next() => {
                                log::debug!("CLI output branch triggered");
                                if let Some(Ok(msg)) = msg {
                                    let raw_text = match msg {
                                        bollard::container::LogOutput::StdOut { message } => {
                                            log::debug!("Received stdout from CLI");
                                            String::from_utf8_lossy(&message).to_string()
                                        }
                                        bollard::container::LogOutput::StdErr { message } => {
                                            log::debug!("Received stderr from CLI");
                                            String::from_utf8_lossy(&message).to_string()
                                        }
                                        bollard::container::LogOutput::Console { message } => {
                                            log::debug!("Received console output from CLI");
                                            String::from_utf8_lossy(&message).to_string()
                                        }
                                        bollard::container::LogOutput::StdIn { message: _ } => {
                                            log::debug!("Received stdin from CLI (ignoring)");
                                            continue; // Skip stdin as it's our own input
                                        }
                                    };

                                    // Strip ANSI escape sequences for clean processing
                                    let text = Self::strip_ansi_codes(&raw_text);
                                    session.last_output = Some(text.clone());

                                    log::debug!("Claude CLI output: {}", text);

                                    // Write all CLI output to log file (both raw and cleaned)
                                    if let Some(ref mut writer) = log_writer {
                                        use std::time::SystemTime;
                                        let timestamp = SystemTime::now()
                                            .duration_since(SystemTime::UNIX_EPOCH)
                                            .map(|d| d.as_secs())
                                            .unwrap_or(0);
                                        let log_entry = format!("[{}] TEXT: {}", timestamp, text);
                                        if let Err(e) = writer.write_all(log_entry.as_bytes()).await {
                                            log::warn!("Failed to write to Claude CLI log file: {}", e);
                                        } else if let Err(e) = writer.flush().await {
                                            log::warn!("Failed to flush Claude CLI log file: {}", e);
                                        }
                                    }

                                    // Add to buffer for accumulated processing
                                    output_buffer.push_str(&text);
                                    log::debug!("Added to output buffer, total length: {}", output_buffer.len());

                                    // Use timeout to wait for 200ms of no new output, then process buffer
                                    let buffer_result = tokio::time::timeout(
                                        Duration::from_millis(buffer_timeout_ms),
                                        async {
                                            // Wait for more output or timeout
                                            loop {
                                                tokio::select! {
                                                    // If we get more output within 200ms, add it to buffer
                                                    msg = output.next() => {
                                                        if let Some(Ok(msg)) = msg {
                                                            let raw_text = match msg {
                                                                bollard::container::LogOutput::StdOut { message } => {
                                                                    String::from_utf8_lossy(&message).to_string()
                                                                }
                                                                bollard::container::LogOutput::StdErr { message } => {
                                                                    String::from_utf8_lossy(&message).to_string()
                                                                }
                                                                bollard::container::LogOutput::Console { message } => {
                                                                    String::from_utf8_lossy(&message).to_string()
                                                                }
                                                                bollard::container::LogOutput::StdIn { message: _ } => {
                                                                    continue; // Skip stdin
                                                                }
                                                            };
                                                            let additional_text = Self::strip_ansi_codes(&raw_text);
                                                            output_buffer.push_str(&additional_text);
                                                            log::debug!("Added more to buffer, new length: {}", output_buffer.len());
                                                            
                                                            // Write to log file
                                                            if let Some(ref mut writer) = log_writer {
                                                                use std::time::SystemTime;
                                                                let timestamp = SystemTime::now()
                                                                    .duration_since(SystemTime::UNIX_EPOCH)
                                                                    .map(|d| d.as_secs())
                                                                    .unwrap_or(0);
                                                                let log_entry = format!("[{}] TEXT: {}", timestamp, additional_text);
                                                                if let Err(e) = writer.write_all(log_entry.as_bytes()).await {
                                                                    log::warn!("Failed to write to Claude CLI log file: {}", e);
                                                                } else if let Err(e) = writer.flush().await {
                                                                    log::warn!("Failed to flush Claude CLI log file: {}", e);
                                                                }
                                                            }
                                                            // Continue waiting for more or timeout
                                                        } else {
                                                            return "stream_ended";
                                                        }
                                                    }
                                                    // If no more output for 200ms, break and process
                                                    _ = tokio::time::sleep(Duration::from_millis(buffer_timeout_ms)) => {
                                                        return "timeout";
                                                    }
                                                }
                                            }
                                        }
                                    ).await;

                                    // Process the accumulated buffer
                                    log::debug!("Processing accumulated buffer (reason: {:?}, length: {})", 
                                        buffer_result.as_ref().map_or("timeout", |r| r), output_buffer.len());
                                    let buffered_text = std::mem::take(&mut output_buffer);
                                    
                                    // Update state based on accumulated output buffer
                                    log::debug!("Parsing accumulated CLI output to determine new state");
                                    let new_state = self.parse_cli_output_for_state(session.state.clone(), &buffered_text);
                                    log::debug!("State transition: {:?} -> {:?}", session.state, new_state);

                                    if new_state == session.state {
                                        log::debug!("No state change detected, continuing to wait for next output");
                                        continue; // No change, continue waiting
                                    }

                                    match &new_state {
                                        InteractiveLoginState::Starting => {
                                            log::info!("State: Starting - waiting output from CLI");

                                            session.state = new_state.clone();
                                            let _ = state_sender.send(AuthState::Starting);
                                            log::debug!("Successfully handled Starting state");
                                        }
                                        InteractiveLoginState::SelectLoginMethod => {
                                            log::debug!("State: SelectLoginMethod detected, choosing option 1 (account authentication)");
                                            if let Err(e) = stdin.write_all(b"\r").await {
                                                log::error!("Failed to send login method selection: {}", e);
                                                return Err(e.into());
                                            }
                                            if let Err(e) = stdin.flush().await {
                                                log::error!("Failed to flush stdin for login method: {}", e);
                                                return Err(e.into());
                                            }
                                            session.state = new_state.clone();
                                            log::debug!("Successfully selected login method");
                                        }
                                        InteractiveLoginState::ProvideUrl(url) => {
                                            log::info!("State: ProvideUrl - Authentication URL detected: {}", url);
                                            session.url = Some(url.clone());

                                            // Send URL to user via channel
                                            log::debug!("Sending UrlReady state to user");
                                            let _ = state_sender.send(AuthState::UrlReady(url.clone()));
                                            let _ = state_sender.send(AuthState::WaitingForCode);
                                            session.state = InteractiveLoginState::WaitingForCode;
                                            log::info!("Authentication URL sent to user, waiting for code input");
                                            log::debug!("Successfully handled ProvideUrl state");
                                        }
                                        InteractiveLoginState::WaitingForCode => {
                                            log::info!("State: WaitingForCode - Authentication code required - waiting for user input");
                                            session.awaiting_user_code = true;
                                            session.state = new_state.clone();
                                            
                                            // Notify that we're waiting for a code
                                            log::debug!("Sending WaitingForCode state to user");
                                            let _ = state_sender.send(AuthState::WaitingForCode);
                                            log::debug!("Successfully handled WaitingForCode state");
                                        }
                                        InteractiveLoginState::LoginSuccessful => {
                                            log::info!("State: LoginSuccessful - pressing enter to continue");
                                          
                                            let _ = stdin.write_all(b"/exit").await;
                                            let _ = stdin.flush().await;

                                            // Add a small delay to ensure the TTY processes the input
                                            tokio::time::sleep(Duration::from_millis(100)).await;

                                            let _ = stdin.write_all(b"\r").await;
                                            let _ = stdin.flush().await;
                                            
                                            session.state = new_state.clone();

                                            let success_msg = "✅ Claude Code authentication completed successfully!".to_string();
                                            log::debug!("Sending final completion state to user");
                                            let _ = state_sender.send(AuthState::Completed(success_msg));
                                            log::debug!("Successfully handled LoginSuccessful state");
                                            return Ok(());
                                        }
                                        InteractiveLoginState::OAuthPortError => {
                                            log::warn!("State: OAuthPortError - Port 54545 is already in use");
                                            let error_msg = "OAuth error: Port 54545 is already in use. Please ensure no other applications are using this port.\n\nPress Enter to retry.";
                                            log::debug!("Sending OAuth port error state to user");
                                            let _ = state_sender.send(AuthState::Failed(error_msg.to_string()));
                                            log::error!("Authentication failed due to OAuth port conflict");
                                            return Err(error_msg.into());
                                        }
                                        InteractiveLoginState::Error(err) => {
                                            log::warn!("State: Error encountered in interactive login: {}", err);
                                            log::debug!("Sending error state to user");
                                            let _ = state_sender.send(AuthState::Failed(err.clone()));
                                            log::error!("Authentication failed due to error state: {}", err);
                                            return Err(err.clone().into());
                                        }
                                    }

                                    // Small delay between state transitions
                                    log::debug!("Pausing 200ms between state transitions");
                                    sleep(Duration::from_millis(200)).await;
                                } else {
                                    // Stream ended
                                    log::warn!("CLI output stream ended unexpectedly (received None)");
                                    break;
                                }
                            }
                        }
                    }

                    log::debug!("Exiting authentication loop");
                    Ok::<(), Box<dyn std::error::Error + Send + Sync>>(())
                }).await;

                // Flush and close log file
                if let Some(mut writer) = log_writer {
                    if let Err(e) = writer.flush().await {
                        log::warn!("Failed to flush Claude CLI log file before closing: {}", e);
                    }
                    log::info!("Claude CLI output logged to: {}", log_file_path);
                }

                log::debug!("Authentication timeout result: {:?}", timeout_result.is_ok());
                match timeout_result {
                    Ok(Ok(())) => {
                        log::info!("Authentication completed successfully within timeout");
                    }
                    Ok(Err(e)) => {
                        log::error!("Error occurred during interactive login: {}", e);
                        log::debug!("Sending authentication error state to user");
                        let _ = state_sender.send(AuthState::Failed(format!("Authentication error: {}", e)));
                    }
                    Err(_) => {
                        log::warn!("Authentication process timed out after 1 minutes (60 seconds)");
                        log::debug!("Timeout context - CLI may be unresponsive or waiting for input");
                        log::debug!("Current session state: {:?}", session.state);
                        log::debug!("Awaiting user code: {}", session.awaiting_user_code);
                        log::debug!("Session URL: {:?}", session.url);
                        let _ = state_sender.send(AuthState::Failed("Authentication timed out after 60 seconds".to_string()));
                    }
                }
            }
            bollard::exec::StartExecResults::Detached => {
                log::error!("Unexpected detached execution in interactive mode - this should not happen");
                let _ = state_sender.send(AuthState::Failed("Unexpected detached execution in interactive mode".to_string()));
                return Err("Unexpected detached execution in interactive mode".into());
            }
        }

        log::debug!("background_interactive_login function completed successfully");
        Ok(())
    }

    /// Parse CLI output to determine the new state
    fn parse_cli_output_for_state(&self, current_state: InteractiveLoginState, output: &str) -> InteractiveLoginState {
        let output_lower = output.to_lowercase();
    
        log::debug!("Current state: {:?}", current_state);
        log::debug!("Output buffer length: {}, contains 'select login method': {}", output.len(), output_lower.contains("select login method"));

        // Check for OAuth port error first
        if output_lower.contains("oauth error: port 54545 is already in use") || 
           (output_lower.contains("port 54545") && output_lower.contains("already in use")) {
            log::debug!("Detected OAuth port 54545 conflict error");
            return InteractiveLoginState::OAuthPortError;
        }
        
        if output_lower.contains("select login method") || output_lower.contains("claude account with subscription") {
            log::debug!("Detected login method selection screen");
            InteractiveLoginState::SelectLoginMethod
        } else if output_lower.contains("use the url below to sign in") || output_lower.contains("visit:") || output_lower.contains("https://") {
            // Extract URL from output - look for any https:// URL
            for line in output.lines() {
                let trimmed = line.trim();
                if let Some(url_start) = trimmed.find("https://") {
                    let url_part = &trimmed[url_start..];
                    // Find URL end by looking for various delimiters
                    let url_end = url_part.find(|c: char| c.is_whitespace() || c == ')' || c == ']' || c == '>' || c == '"' || c == '\'' || c == ',')
                        .unwrap_or(url_part.len());
                    let url = &url_part[..url_end];
                    
                    // Validate URL format
                    if url.len() > 8 && url.contains("claude.ai") {
                        return InteractiveLoginState::ProvideUrl(url.to_string());
                    }
                }
            }

            // Fallback: extract first https:// URL from entire output
            if let Some(url_start) = output.find("https://") {
                let url_part = &output[url_start..];
                let url_end = url_part.find(|c: char| c.is_whitespace() || c == ')' || c == ']' || c == '>' || c == '"' || c == '\'')
                    .unwrap_or(url_part.len());
                let url = &url_part[..url_end].trim();
                
                if url.len() > 8 {
                    return InteractiveLoginState::ProvideUrl(url.to_string());
                }
            }

            InteractiveLoginState::Error("Valid authentication URL not found in CLI output".to_string())
        } else if output_lower.contains("paste code here if prompted") {
            InteractiveLoginState::WaitingForCode
        } else if output_lower.contains("login successful") {
            InteractiveLoginState::LoginSuccessful
        } else {
            // Don't treat everything as an error, just continue with current state
            log::debug!("Unrecognized CLI output state, continuing with current state: {}", output);
            log::warn!("Unrecognized CLI output state: {}", output);
            current_state // Default state to continue processing
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

    /// Update Claude CLI to latest version
    pub async fn update_claude(
        &self,
    ) -> Result<String, Box<dyn std::error::Error + Send + Sync>> {
        let command = vec!["claude".to_string(), "update".to_string()];
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
