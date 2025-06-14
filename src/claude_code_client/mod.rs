use bollard::Docker;
use bollard::exec::{CreateExecOptions, StartExecOptions};
use futures_util::StreamExt;
use serde::{Deserialize, Serialize};
use tokio::io::AsyncWriteExt;

pub mod container_utils;

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
    pub async fn execute_prompt(&self, prompt: &str) -> Result<ClaudeCodeResult, Box<dyn std::error::Error + Send + Sync>> {
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
    pub async fn execute_with_stdin(&self, prompt: &str, stdin_content: &str) -> Result<ClaudeCodeResult, Box<dyn std::error::Error + Send + Sync>> {
        // Create a temporary file with the stdin content
        let temp_file = format!("/tmp/claude_input_{}", uuid::Uuid::new_v4());
        
        // Write content to temporary file
        let write_command = vec![
            "sh".to_string(),
            "-c".to_string(),
            format!("echo '{}' > {}", stdin_content.replace("'", "'\"'\"'"), temp_file),
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
                self.config.max_tokens.map_or(String::new(), |t| format!("--max-tokens {}", t)),
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
    pub async fn start_chat_session(&self, initial_message: Option<&str>) -> Result<ClaudeCodeResult, Box<dyn std::error::Error + Send + Sync>> {
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
    pub async fn send_chat_message(&self, session_id: &str, message: &str) -> Result<ClaudeCodeResult, Box<dyn std::error::Error + Send + Sync>> {
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
    pub async fn run_coding_task(&self, task: &str, files: Vec<&str>) -> Result<ClaudeCodeResult, Box<dyn std::error::Error + Send + Sync>> {
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
    fn parse_result(&self, output: String) -> Result<ClaudeCodeResult, Box<dyn std::error::Error + Send + Sync>> {
        match serde_json::from_str::<ClaudeCodeResult>(&output) {
            Ok(result) => Ok(result),
            Err(_) => {
                // If JSON parsing fails, create a simple result with the raw output
                Ok(ClaudeCodeResult {
                    r#type: "result".to_string(),
                    subtype: if output.to_lowercase().contains("error") { "error" } else { "success" }.to_string(),
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
    pub async fn get_session_status(&self) -> Result<ClaudeCodeResult, Box<dyn std::error::Error + Send + Sync>> {
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
    pub async fn create_commit(&self, message: Option<&str>) -> Result<ClaudeCodeResult, Box<dyn std::error::Error + Send + Sync>> {
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
    pub async fn authenticate_claude_account(&self) -> Result<String, Box<dyn std::error::Error + Send + Sync>> {
        // First validate container health before attempting authentication
        if let Err(e) = self.validate_container_health().await {
            return Err(format!("Container health check failed: {}. The container may have terminated unexpectedly. Please try restarting your coding session.", e).into());
        }

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
                // Check if this is a container-related error
                if self.is_container_error(&e) {
                    return Err(format!("Container issue detected during authentication check: {}. The container may have terminated. Please restart your coding session.", e).into());
                }
                return Err(format!("Unable to check authentication status: {}", e).into());
            }
        }
    }

    /// Interactive Claude login using TTY to get OAuth URL
    async fn interactive_claude_login(&self) -> Result<String, Box<dyn std::error::Error + Send + Sync>> {
        use bollard::exec::{CreateExecOptions, StartExecOptions};
        use futures_util::StreamExt;
        use std::time::Duration;
        use tokio::time::sleep;

        // Validate container health before attempting interactive session
        if let Err(e) = self.validate_container_health().await {
            return Err(format!("Container health check failed before authentication: {}. The container may have terminated. Please restart your coding session.", e).into());
        }

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

        let exec_result = self.docker.create_exec(&self.container_id, exec_config).await;
        let exec = match exec_result {
            Ok(exec) => exec,
            Err(e) => {
                let error_msg = e.to_string();
                if error_msg.to_lowercase().contains("container") && (
                    error_msg.contains("not found") || 
                    error_msg.contains("not running") || 
                    error_msg.contains("terminated")) {
                    return Err(format!("Failed to create exec session - container may have terminated: {}. Please restart your coding session.", error_msg).into());
                }
                return Err(format!("Failed to create exec session: {}", error_msg).into());
            }
        };
        
        let start_config = StartExecOptions {
            detach: false,
            tty: true,
            ..Default::default()
        };

        let mut oauth_url = String::new();
        let mut authentication_found = false;
        
        match self.docker.start_exec(&exec.id, Some(start_config)).await? {
            bollard::exec::StartExecResults::Attached { mut output, input } => {
                // Give some time for the interactive CLI to start
                sleep(Duration::from_millis(500)).await;
                
                // Send the /login command
                {
                    let mut stdin = input;
                    stdin.write_all(b"/login\n").await?;
                    stdin.flush().await?;
                    
                    // Wait a bit for the login options to appear
                    sleep(Duration::from_millis(1000)).await;
                    
                    // Send option 1 for account auth
                    stdin.write_all(b"1\n").await?;
                    stdin.flush().await?;
                }
                
                // Read output and look for OAuth URL
                let timeout = tokio::time::timeout(Duration::from_secs(10), async {
                    while let Some(Ok(msg)) = output.next().await {
                        match msg {
                            bollard::container::LogOutput::StdOut { message } => {
                                let text = String::from_utf8_lossy(&message);
                                log::debug!("Claude CLI output: {}", text);
                                
                                // Look for OAuth URL patterns
                                if text.contains("https://") && (text.contains("claude.ai") || text.contains("anthropic") || text.contains("oauth") || text.contains("auth")) {
                                    // Extract the URL
                                    for line in text.lines() {
                                        if line.trim().starts_with("https://") {
                                            oauth_url = line.trim().to_string();
                                            authentication_found = true;
                                            break;
                                        }
                                    }
                                }
                                
                                // Also look for any authentication-related instructions
                                if text.contains("Visit") || text.contains("Open") || text.contains("browser") {
                                    oauth_url.push_str(&text);
                                    authentication_found = true;
                                }
                                
                                if authentication_found {
                                    break;
                                }
                            }
                            bollard::container::LogOutput::StdErr { message } => {
                                let text = String::from_utf8_lossy(&message);
                                log::debug!("Claude CLI stderr: {}", text);
                                
                                // Sometimes OAuth URL might come through stderr
                                if text.contains("https://") && (text.contains("claude.ai") || text.contains("anthropic")) {
                                    for line in text.lines() {
                                        if line.trim().starts_with("https://") {
                                            oauth_url = line.trim().to_string();
                                            authentication_found = true;
                                            break;
                                        }
                                    }
                                }
                            }
                            _ => {}
                        }
                    }
                }).await;
                
                match timeout {
                    Ok(_) => {
                        if authentication_found && !oauth_url.is_empty() {
                            return Ok(format!(
                                "🔐 **Claude Account Authentication**\n\n\
                                To complete authentication with your Claude account:\n\n\
                                **1. Visit this authentication URL:**\n{}\n\n\
                                **2. Sign in with your Claude account:**\n\
                                - Use your existing Claude Pro/Team account credentials\n\
                                - Or create a new Claude account if you don't have one\n\n\
                                **3. Complete the OAuth flow:**\n\
                                - Grant permission to Claude Code\n\
                                - Follow any additional instructions\n\n\
                                **4. Return to continue using Claude Code**\n\n\
                                ✨ This will enable full access to your Claude subscription features!",
                                oauth_url
                            ));
                        } else {
                            // Fallback to manual instructions if we couldn't extract the URL
                            return Ok(self.get_fallback_auth_instructions().await);
                        }
                    }
                    Err(_) => {
                        log::warn!("Timeout waiting for OAuth URL from Claude CLI");
                        return Ok(self.get_fallback_auth_instructions().await);
                    }
                }
            }
            bollard::exec::StartExecResults::Detached => {
                return Err("Unexpected detached execution in interactive mode".into());
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

💡 **Note:** If you encounter issues, ensure you have a valid Claude account and subscription."#.to_string()
    }

    /// Check authentication status
    pub async fn check_auth_status(&self) -> Result<bool, Box<dyn std::error::Error + Send + Sync>> {
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
                if error_msg.contains("invalid api key") || 
                   error_msg.contains("authentication") ||
                   error_msg.contains("unauthorized") ||
                   error_msg.contains("api key") ||
                   error_msg.contains("token") ||
                   error_msg.contains("not authenticated") ||
                   error_msg.contains("login required") ||
                   error_msg.contains("please log in") ||
                   error_msg.contains("auth required") ||
                   error_msg.contains("permission denied") ||
                   error_msg.contains("access denied") ||
                   error_msg.contains("forbidden") {
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
            Ok(false) => Ok("❌ Claude Code is not authenticated. Please set up your Anthropic API key.".to_string()),
            Err(e) => Err(format!("Unable to check authentication status: {}", e).into()),
        }
    }

    /// Install Claude Code via npm
    pub async fn install(&self) -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
        log::info!("Starting Claude Code installation...");
        
        let install_command = vec![
            "npm".to_string(),
            "install".to_string(),
            "-g".to_string(),
            "@anthropic-ai/claude-code".to_string()
        ];
        
        match self.exec_command(install_command).await {
            Ok(output) => {
                log::info!("Claude Code installation completed successfully");
                log::debug!("Installation output: {}", output);
                Ok(())
            }
            Err(e) => {
                log::error!("Claude Code installation failed: {}", e);
                Err(format!("Failed to install Claude Code: {}", e).into())
            }
        }
    }

    /// Check Claude Code version and availability
    pub async fn check_availability(&self) -> Result<String, Box<dyn std::error::Error + Send + Sync>> {
        let command = vec!["claude".to_string(), "--version".to_string()];
        self.exec_command(command).await
    }

    /// Execute a command in the container and return output
    async fn exec_command(&self, command: Vec<String>) -> Result<String, Box<dyn std::error::Error + Send + Sync>> {
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

        let exec = self.docker.create_exec(&self.container_id, exec_config).await?;
        
        let start_config = StartExecOptions {
            detach: false,
            ..Default::default()
        };

        let mut output = String::new();
        
        match self.docker.start_exec(&exec.id, Some(start_config)).await? {
            bollard::exec::StartExecResults::Attached { output: mut output_stream, .. } => {
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
                return Err(format!("Command failed with exit code {}: {}", exit_code, output.trim()).into());
            }
        }

        Ok(output.trim().to_string())
    }

    /// Helper method for basic command execution (used in tests)
    #[allow(dead_code)]
    pub async fn exec_basic_command(&self, command: Vec<String>) -> Result<String, Box<dyn std::error::Error + Send + Sync>> {
        self.exec_command(command).await
    }
}

// Usage example for integration with the Telegram bot
#[allow(dead_code)]
impl ClaudeCodeClient {
    /// Helper method to create a client for a coding session
    pub async fn for_session(docker: Docker, container_name: &str) -> Result<Self, Box<dyn std::error::Error + Send + Sync>> {
        // Find the container by name
        let containers = docker.list_containers(None::<bollard::container::ListContainersOptions<String>>).await?;
        
        let container = containers
            .iter()
            .find(|c| {
                c.names.as_ref()
                    .map(|names| names.iter().any(|name| name.trim_start_matches('/') == container_name))
                    .unwrap_or(false)
            })
            .ok_or("Container not found")?;

        let container_id = container.id.as_ref().ok_or("Container ID not found")?.clone();
        
        // Validate container is running and healthy
        let client = Self::new(docker, container_id, ClaudeCodeConfig::default());
        client.validate_container_health().await?;
        
        Ok(client)
    }

    /// Validate that the container is running and healthy
    pub async fn validate_container_health(&self) -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
        // Check container state
        let inspect_result = self.docker.inspect_container(&self.container_id, None).await;
        match inspect_result {
            Ok(container_info) => {
                // Check if container is running
                if let Some(state) = container_info.state {
                    if !state.running.unwrap_or(false) {
                        let exit_code = state.exit_code.unwrap_or(-1);
                        let error = state.error.unwrap_or_else(|| "Unknown error".to_string());
                        return Err(format!("Container is not running (exit code: {}, error: {})", exit_code, error).into());
                    }
                    
                    // Check if container is healthy (not in restarting state)
                    if state.restarting.unwrap_or(false) {
                        return Err("Container is in restarting state".into());
                    }
                    
                    // Check if container has been running for a reasonable amount of time
                    if let Some(_started_at) = state.started_at {
                        // Container just started, give it a moment to stabilize
                        tokio::time::sleep(tokio::time::Duration::from_secs(1)).await;
                    }
                }
                
                // Perform a basic connectivity test
                self.test_container_connectivity().await?;
                
                Ok(())
            }
            Err(e) => {
                if e.to_string().contains("No such container") {
                    Err("Container not found or has been removed".into())
                } else {
                    Err(format!("Failed to inspect container: {}", e).into())
                }
            }
        }
    }

    /// Test basic connectivity to the container
    async fn test_container_connectivity(&self) -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
        // Try a simple echo command to test container responsiveness
        let test_command = vec!["echo".to_string(), "health_check".to_string()];
        
        match tokio::time::timeout(
            tokio::time::Duration::from_secs(5),
            self.exec_command(test_command)
        ).await {
            Ok(Ok(output)) => {
                if output.trim() == "health_check" {
                    Ok(())
                } else {
                    Err(format!("Container connectivity test failed: unexpected output '{}'", output).into())
                }
            }
            Ok(Err(e)) => {
                Err(format!("Container connectivity test failed: {}", e).into())
            }
            Err(_) => {
                Err("Container connectivity test timed out".into())
            }
        }
    }

    /// Check if an error is related to container issues (termination, not found, etc.)
    fn is_container_error(&self, error: &Box<dyn std::error::Error + Send + Sync>) -> bool {
        let error_msg = error.to_string().to_lowercase();
        error_msg.contains("container") && (
            error_msg.contains("not found") ||
            error_msg.contains("not running") ||
            error_msg.contains("terminated") ||
            error_msg.contains("stopped") ||
            error_msg.contains("exited") ||
            error_msg.contains("no such container") ||
            error_msg.contains("container is not running") ||
            error_msg.contains("409") || // Docker conflict
            error_msg.contains("timeout")
        )
    }
}