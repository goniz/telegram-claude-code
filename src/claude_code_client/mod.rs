use bollard::exec::{CreateExecOptions, StartExecOptions};
use bollard::Docker;
use futures_util::StreamExt;
use serde::{Deserialize, Serialize};
use tokio::io::AsyncWriteExt;

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
pub struct InteractiveLoginSession {
    pub state: InteractiveLoginState,
    pub url: Option<String>,
    pub awaiting_user_code: bool,
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

    /// Execute a single prompt using Claude Code in print mode
    pub async fn execute_prompt(
        &self,
        prompt: &str,
    ) -> Result<ClaudeCodeResult, Box<dyn std::error::Error + Send + Sync>> {
        let mut command = vec![
            "claude".to_string(),
            "-p".to_string(),
            prompt.to_string(),
            "--output-format".to_string(),
            "json".to_string(),
            "--model".to_string(),
            self.config.model.clone(),
        ];

        // Add optional parameters
        if let Some(max_tokens) = self.config.max_tokens {
            command.extend_from_slice(&["--max-tokens".to_string(), max_tokens.to_string()]);
        }

        if let Some(temperature) = self.config.temperature {
            command.extend_from_slice(&["--temperature".to_string(), temperature.to_string()]);
        }

        let output = self.exec_command(command).await?;

        // Parse JSON response
        match serde_json::from_str::<ClaudeCodeResult>(&output) {
            Ok(result) => Ok(result),
            Err(_) => {
                // If JSON parsing fails, create a simple result with the raw output
                Ok(ClaudeCodeResult {
                    r#type: "result".to_string(),
                    subtype: "success".to_string(),
                    cost_usd: 0.0,
                    is_error: false,
                    duration_ms: 0,
                    duration_api_ms: 0,
                    num_turns: 1,
                    result: output,
                    session_id: "unknown".to_string(),
                })
            }
        }
    }

    /// Execute a prompt with stdin input (for processing files)
    pub async fn execute_with_stdin(
        &self,
        prompt: &str,
        stdin_content: &str,
    ) -> Result<ClaudeCodeResult, Box<dyn std::error::Error + Send + Sync>> {
        // Create a temporary file with the stdin content
        let temp_file = format!("/tmp/claude_input_{}", uuid::Uuid::new_v4());

        // Write content to temporary file
        let write_command = vec![
            "sh".to_string(),
            "-c".to_string(),
            format!(
                "echo '{}' > {}",
                stdin_content.replace("'", "'\"'\"'"),
                temp_file
            ),
        ];
        self.exec_command(write_command).await?;

        // Execute claude with input redirection
        let command = vec![
            "sh".to_string(),
            "-c".to_string(),
            format!(
                "claude -p '{}' --output-format json --model {} {} < {}",
                prompt.replace("'", "'\"'\"'"),
                self.config.model,
                self.config
                    .max_tokens
                    .map_or(String::new(), |t| format!("--max-tokens {}", t)),
                temp_file
            ),
        ];

        let output = self.exec_command(command).await?;

        // Clean up temporary file
        let cleanup_command = vec!["rm".to_string(), temp_file];
        let _ = self.exec_command(cleanup_command).await; // Ignore cleanup errors

        self.parse_result(output)
    }

    /// Execute a Claude Code chat command (interactive mode)
    pub async fn start_chat_session(
        &self,
        initial_message: Option<&str>,
    ) -> Result<ClaudeCodeResult, Box<dyn std::error::Error + Send + Sync>> {
        let mut command = vec![
            "claude".to_string(),
            "chat".to_string(),
            "--output-format".to_string(),
            "json".to_string(),
            "--model".to_string(),
            self.config.model.clone(),
        ];

        if let Some(message) = initial_message {
            command.extend_from_slice(&["--message".to_string(), message.to_string()]);
        }

        let output = self.exec_command(command).await?;
        self.parse_result(output)
    }

    /// Send a message to an existing chat session
    pub async fn send_chat_message(
        &self,
        session_id: &str,
        message: &str,
    ) -> Result<ClaudeCodeResult, Box<dyn std::error::Error + Send + Sync>> {
        let command = vec![
            "claude".to_string(),
            "chat".to_string(),
            "--session-id".to_string(),
            session_id.to_string(),
            "--message".to_string(),
            message.to_string(),
            "--output-format".to_string(),
            "json".to_string(),
        ];

        let output = self.exec_command(command).await?;
        self.parse_result(output)
    }

    /// Run Claude Code in coding mode with a specific task
    pub async fn run_coding_task(
        &self,
        task: &str,
        files: Vec<&str>,
    ) -> Result<ClaudeCodeResult, Box<dyn std::error::Error + Send + Sync>> {
        let mut command = vec![
            "claude".to_string(),
            "code".to_string(),
            "--task".to_string(),
            task.to_string(),
            "--output-format".to_string(),
            "json".to_string(),
            "--model".to_string(),
            self.config.model.clone(),
        ];

        for file in files {
            command.extend_from_slice(&["--file".to_string(), file.to_string()]);
        }

        let output = self.exec_command(command).await?;
        self.parse_result(output)
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

    /// Show current session status
    pub async fn get_session_status(
        &self,
    ) -> Result<ClaudeCodeResult, Box<dyn std::error::Error + Send + Sync>> {
        let command = vec![
            "claude".to_string(),
            "status".to_string(),
            "--output-format".to_string(),
            "json".to_string(),
        ];

        let output = self.exec_command(command).await?;
        self.parse_result(output)
    }

    /// Create a commit with Claude Code
    pub async fn create_commit(
        &self,
        message: Option<&str>,
    ) -> Result<ClaudeCodeResult, Box<dyn std::error::Error + Send + Sync>> {
        let mut command = vec![
            "claude".to_string(),
            "commit".to_string(),
            "--output-format".to_string(),
            "json".to_string(),
        ];

        if let Some(msg) = message {
            command.extend_from_slice(&["-m".to_string(), msg.to_string()]);
        }

        let output = self.exec_command(command).await?;
        self.parse_result(output)
    }

    /// Authenticate Claude Code using Claude account (OAuth flow)
    /// This initiates the account-based authentication process through interactive CLI
    pub async fn authenticate_claude_account(
        &self,
    ) -> Result<String, Box<dyn std::error::Error + Send + Sync>> {
        // Check if authentication is already set up
        match self.check_auth_status().await {
            Ok(true) => {
                return Ok("✅ Claude Code is already authenticated and ready to use!".to_string());
            }
            Ok(false) => {
                // Launch Claude CLI in interactive mode and perform account authentication
                return self.interactive_claude_login().await;
            }
            Err(e) => {
                return Err(format!("Unable to check authentication status: {}", e).into());
            }
        }
    }

    /// Interactive Claude login using TTY with comprehensive state handling
    async fn interactive_claude_login(
        &self,
    ) -> Result<String, Box<dyn std::error::Error + Send + Sync>> {
        use std::time::Duration;
        use tokio::time::sleep;

        // Create exec with TTY enabled for interactive mode
        let exec_config = CreateExecOptions {
            cmd: Some(vec!["claude".to_string()]),
            attach_stdin: Some(true),
            attach_stdout: Some(true),
            attach_stderr: Some(true),
            tty: Some(true),
            working_dir: self.config.working_directory.clone(),
            env: Some(vec![
                "PATH=/root/.nvm/versions/node/v22.16.0/bin:/root/.nvm/versions/node/v20.19.2/bin:/root/.nvm/versions/node/v18.20.8/bin:/usr/local/sbin:/usr/local/bin:/usr/sbin:/usr/bin:/sbin:/bin".to_string(),
                "NODE_PATH=/root/.nvm/versions/node/v22.16.0/lib/node_modules".to_string(),
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
                let mut output_buffer = String::new();

                // Claude CLI automatically prompts for login method on first run
                // No need to send /login command

                // Process the interactive session
                let timeout = tokio::time::timeout(Duration::from_secs(30), async {
                    while let Some(Ok(msg)) = output.next().await {
                        let text = match msg {
                            bollard::container::LogOutput::StdOut { message } => {
                                String::from_utf8_lossy(&message).to_string()
                            }
                            bollard::container::LogOutput::StdErr { message } => {
                                String::from_utf8_lossy(&message).to_string()
                            }
                            _ => continue,
                        };

                        output_buffer.push_str(&text);
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
                                log::debug!("URL detected: {}", url);
                                session.url = Some(url.clone());
                                session.state = new_state.clone();

                                // Return URL to user
                                return Ok(format!(
                                    "🔐 **Claude Account Authentication**\n\n\
                                    To complete authentication with your Claude account:\n\n\
                                    **1. Visit this authentication URL:**\n{}\n\n\
                                    **2. Sign in with your Claude account**\n\n\
                                    **3. Complete the OAuth flow**\n\n\
                                    **4. Once complete, return here and continue**\n\n\
                                    ✨ This will enable full access to your Claude subscription features!",
                                    url
                                ));
                            }
                            InteractiveLoginState::WaitingForCode => {
                                log::debug!("Waiting for code from user");
                                session.awaiting_user_code = true;
                                session.state = new_state.clone();

                                // This would need to be handled differently in a real bot scenario
                                // For now, return an instruction to the user
                                return Ok(format!(
                                    "🔐 **Claude Authentication - Code Required**\n\n\
                                    Please paste the authentication code you received and send it back.\n\n\
                                    The bot will continue the authentication process with your code."
                                ));
                            }
                            InteractiveLoginState::LoginSuccessful => {
                                log::debug!("Login successful, pressing enter to continue");
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
                                log::debug!("Trust files prompt detected, pressing enter and ending session");
                                stdin.write_all(b"\n").await?;
                                stdin.flush().await?;
                                session.state = InteractiveLoginState::Completed;

                                return Ok(format!(
                                    "✅ **Claude Authentication Completed!**\n\n\
                                    Your Claude account has been successfully authenticated.\n\n\
                                    You can now use Claude Code with your account privileges."
                                ));
                            }
                            InteractiveLoginState::Completed => {
                                break;
                            }
                            InteractiveLoginState::Error(err) => {
                                log::warn!("Error in interactive login: {}", err);
                                session.state = new_state.clone();
                            }
                        }

                        // Small delay between state transitions
                        sleep(Duration::from_millis(200)).await;
                    }

                    // If we get here without a clear result, return what we have
                    Ok::<String, Box<dyn std::error::Error + Send + Sync>>(output_buffer)
                }).await;

                match timeout {
                    Ok(Ok(result)) => {
                        if result.is_empty() {
                            return Ok(self.get_fallback_auth_instructions().await);
                        } else {
                            return Ok(result);
                        }
                    }
                    Ok(Err(e)) => {
                        log::warn!("Error in interactive login: {}", e);
                        return Ok(self.get_fallback_auth_instructions().await);
                    }
                    Err(_) => {
                        log::warn!("Timeout in interactive login");
                        return Ok(self.get_fallback_auth_instructions().await);
                    }
                }
            }
            bollard::exec::StartExecResults::Detached => {
                return Err("Unexpected detached execution in interactive mode".into());
            }
        }
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

    /// Continue interactive login with user-provided code
    pub async fn continue_login_with_code(
        &self,
        code: &str,
    ) -> Result<String, Box<dyn std::error::Error + Send + Sync>> {
        use std::time::Duration;
        use tokio::time::sleep;

        log::info!("Continuing login with user code: {}", code);

        // Create exec with TTY enabled for interactive mode to continue the session
        let exec_config = CreateExecOptions {
            cmd: Some(vec!["claude".to_string()]),
            attach_stdin: Some(true),
            attach_stdout: Some(true),
            attach_stderr: Some(true),
            tty: Some(true),
            working_dir: self.config.working_directory.clone(),
            env: Some(vec![
                "PATH=/root/.nvm/versions/node/v22.16.0/bin:/root/.nvm/versions/node/v20.19.2/bin:/root/.nvm/versions/node/v18.20.8/bin:/usr/local/sbin:/usr/local/bin:/usr/sbin:/usr/bin:/sbin:/bin".to_string(),
                "NODE_PATH=/root/.nvm/versions/node/v22.16.0/lib/node_modules".to_string(),
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
            tty: true,
            ..Default::default()
        };

        match self.docker.start_exec(&exec.id, Some(start_config)).await? {
            bollard::exec::StartExecResults::Attached { mut output, input } => {
                // Give some time for the interactive CLI to start
                sleep(Duration::from_millis(500)).await;

                let mut stdin = input;
                let mut output_buffer = String::new();

                // Send the authentication code
                stdin.write_all(format!("{}\n", code).as_bytes()).await?;
                stdin.flush().await?;

                // Process the response
                let timeout = tokio::time::timeout(Duration::from_secs(20), async {
                    while let Some(Ok(msg)) = output.next().await {
                        let text = match msg {
                            bollard::container::LogOutput::StdOut { message } => {
                                String::from_utf8_lossy(&message).to_string()
                            }
                            bollard::container::LogOutput::StdErr { message } => {
                                String::from_utf8_lossy(&message).to_string()
                            }
                            _ => continue,
                        };

                        output_buffer.push_str(&text);
                        log::debug!("Claude CLI code response: {}", text);

                        // Check for completion states
                        let new_state = self.parse_cli_output_for_state(&text);

                        match &new_state {
                            InteractiveLoginState::LoginSuccessful => {
                                log::debug!("Login successful after code input, pressing enter to continue");
                                stdin.write_all(b"\n").await?;
                                stdin.flush().await?;

                                return Ok(format!(
                                    "✅ **Authentication Code Accepted!**\n\n\
                                    Claude account authentication was successful.\n\n\
                                    You can now use Claude Code with your account privileges."
                                ));
                            }
                            InteractiveLoginState::SecurityNotes => {
                                log::debug!("Security notes detected after code, pressing enter to continue");
                                stdin.write_all(b"\n").await?;
                                stdin.flush().await?;
                            }
                            InteractiveLoginState::TrustFiles => {
                                log::debug!("Trust files prompt detected, pressing enter and ending session");
                                stdin.write_all(b"\n").await?;
                                stdin.flush().await?;

                                return Ok(format!(
                                    "✅ **Claude Authentication Completed!**\n\n\
                                    Your Claude account has been successfully authenticated.\n\n\
                                    You can now use Claude Code with your account privileges."
                                ));
                            }
                            InteractiveLoginState::Error(err) => {
                                log::warn!("Error after code input: {}", err);
                                return Ok(format!(
                                    "❌ **Authentication Error**\n\n\
                                    There was an issue with the authentication code: {}\n\n\
                                    Please try the authentication process again with `/authenticateclaude`",
                                    err
                                ));
                            }
                            _ => {
                                // Continue processing
                            }
                        }

                        // Small delay between processing
                        sleep(Duration::from_millis(200)).await;
                    }

                    Ok::<String, Box<dyn std::error::Error + Send + Sync>>(output_buffer)
                }).await;

                match timeout {
                    Ok(Ok(result)) => {
                        if result.is_empty() {
                            return Ok(format!(
                                "🔐 **Code Processed**\n\n\
                                Authentication code has been submitted.\n\n\
                                If authentication is not complete, please check your Claude CLI session or try again."
                            ));
                        } else {
                            return Ok(format!(
                                "✅ **Authentication Process Continued**\n\n\
                                Code has been processed. Authentication may be complete.\n\n\
                                Try using Claude Code commands to verify authentication status."
                            ));
                        }
                    }
                    Ok(Err(e)) => {
                        log::warn!("Error processing code: {}", e);
                        return Ok(format!(
                            "❌ **Error Processing Code**\n\n\
                            There was an error processing your authentication code: {}\n\n\
                            Please try the authentication process again with `/authenticateclaude`",
                            e
                        ));
                    }
                    Err(_) => {
                        log::warn!("Timeout processing authentication code");
                        return Ok(format!(
                            "⏰ **Timeout Processing Code**\n\n\
                            The authentication code processing timed out.\n\n\
                            Please check if authentication was successful or try again."
                        ));
                    }
                }
            }
            bollard::exec::StartExecResults::Detached => {
                return Err("Unexpected detached execution in code continuation mode".into());
            }
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
        // Try to run a simple claude command to check if authentication is working
        let command = vec![
            "claude".to_string(),
            "-p".to_string(),
            "test authentication".to_string(),
            "--model".to_string(),
            "claude-sonnet-4".to_string(),
        ];

        match self.exec_command(command).await {
            Ok(_) => {
                // If the command succeeds, authentication is working
                Ok(true)
            }
            Err(e) => {
                let error_msg = e.to_string().to_lowercase();
                if error_msg.contains("invalid api key")
                    || error_msg.contains("authentication")
                    || error_msg.contains("unauthorized")
                    || error_msg.contains("api key")
                    || error_msg.contains("token")
                    || error_msg.contains("not authenticated")
                    || error_msg.contains("login required")
                    || error_msg.contains("please log in")
                    || error_msg.contains("auth required")
                    || error_msg.contains("permission denied")
                    || error_msg.contains("access denied")
                    || error_msg.contains("forbidden")
                {
                    // These errors indicate authentication issues
                    Ok(false)
                } else {
                    // Other errors (network, container issues, etc.) should be bubbled up
                    Err(e)
                }
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
                "PATH=/root/.nvm/versions/node/v22.16.0/bin:/root/.nvm/versions/node/v20.19.2/bin:/root/.nvm/versions/node/v18.20.8/bin:/usr/local/sbin:/usr/local/bin:/usr/sbin:/usr/bin:/sbin:/bin".to_string(),
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
