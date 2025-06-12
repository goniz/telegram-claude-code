use bollard::Docker;
use bollard::exec::{CreateExecOptions, StartExecOptions};
use futures_util::StreamExt;
use serde::{Deserialize, Serialize};

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
    pub timeout_seconds: Option<u64>,
}

impl Default for ClaudeCodeConfig {
    fn default() -> Self {
        Self {
            model: "claude-opus-4".to_string(),
            max_tokens: Some(4096),
            temperature: Some(0.7),
            working_directory: Some("/workspace".to_string()),
            timeout_seconds: Some(300), // 5 minutes
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
                "claude -p '{}' --output-format json --model {} < {}",
                prompt.replace("'", "'\"'\"'"),
                self.config.model,
                temp_file
            ),
        ];

        let output = self.exec_command(command).await?;

        // Clean up temporary file
        let cleanup_command = vec!["rm".to_string(), temp_file];
        let _ = self.exec_command(cleanup_command).await;

        // Parse result
        match serde_json::from_str::<ClaudeCodeResult>(&output) {
            Ok(result) => Ok(result),
            Err(_) => Ok(ClaudeCodeResult {
                r#type: "result".to_string(),
                subtype: "success".to_string(),
                cost_usd: 0.0,
                is_error: false,
                duration_ms: 0,
                duration_api_ms: 0,
                num_turns: 1,
                result: output,
                session_id: "unknown".to_string(),
            }),
        }
    }

    /// Execute a code review on a file
    pub async fn review_code(&self, file_path: &str) -> Result<ClaudeCodeResult, Box<dyn std::error::Error + Send + Sync>> {
        let command = vec![
            "sh".to_string(),
            "-c".to_string(),
            format!(
                "claude -p 'Review this code for bugs, improvements, and best practices:' --output-format json --model {} < {}",
                self.config.model,
                file_path
            ),
        ];

        let output = self.exec_command(command).await?;
        self.parse_result(output)
    }

    /// Generate documentation for code
    pub async fn generate_docs(&self, file_path: &str) -> Result<ClaudeCodeResult, Box<dyn std::error::Error + Send + Sync>> {
        let command = vec![
            "sh".to_string(),
            "-c".to_string(),
            format!(
                "claude -p 'Generate comprehensive documentation for this code:' --output-format json --model {} < {}",
                self.config.model,
                file_path
            ),
        ];

        let output = self.exec_command(command).await?;
        self.parse_result(output)
    }

    /// Fix issues in code
    pub async fn fix_code(&self, file_path: &str, issue_description: &str) -> Result<ClaudeCodeResult, Box<dyn std::error::Error + Send + Sync>> {
        let prompt = format!("Fix the following issue in this code: {}", issue_description);
        
        let command = vec![
            "sh".to_string(),
            "-c".to_string(),
            format!(
                "claude -p '{}' --output-format json --model {} < {}",
                prompt.replace("'", "'\"'\"'"),
                self.config.model,
                file_path
            ),
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

    /// Authenticate Claude Code using Claude account (Pro/Max plan)
    /// This initiates the OAuth flow and returns the authentication URL
    pub async fn authenticate_claude_account(&self) -> Result<String, Box<dyn std::error::Error + Send + Sync>> {
        let command = vec![
            "claude".to_string(),
            "auth".to_string(),
            "login".to_string(),
            "--provider".to_string(),
            "claude".to_string(),
            "--no-browser".to_string(), // Prevent opening browser in container
        ];

        let output = self.exec_command(command).await?;
        
        // Extract the authentication URL from the output
        // Claude Code typically outputs something like "Visit this URL to authenticate: https://..."
        if let Some(url_line) = output.lines().find(|line| line.contains("http")) {
            if let Some(url_start) = url_line.find("http") {
                let url = url_line[url_start..].split_whitespace().next().unwrap_or("");
                Ok(url.to_string())
            } else {
                Err("Authentication URL not found in output".into())
            }
        } else {
            Err(format!("Could not parse authentication response: {}", output).into())
        }
    }

    /// Check authentication status
    pub async fn check_auth_status(&self) -> Result<bool, Box<dyn std::error::Error + Send + Sync>> {
        let command = vec![
            "claude".to_string(),
            "auth".to_string(),
            "status".to_string(),
        ];

        let output = self.exec_command(command).await?;
        
        // Check if the output indicates successful authentication
        Ok(output.to_lowercase().contains("authenticated") || 
           output.to_lowercase().contains("logged in") ||
           output.to_lowercase().contains("valid"))
    }

    /// Authenticate using device code flow for unattended authentication
    pub async fn authenticate_device_flow(&self) -> Result<(String, String), Box<dyn std::error::Error + Send + Sync>> {
        let command = vec![
            "claude".to_string(),
            "auth".to_string(),
            "device".to_string(),
            "--provider".to_string(),
            "claude".to_string(),
        ];

        let output = self.exec_command(command).await?;
        
        // Parse device code and verification URL
        let mut device_code = String::new();
        let mut verification_url = String::new();
        
        for line in output.lines() {
            if line.to_lowercase().contains("device code") || line.to_lowercase().contains("user code") {
                if let Some(code) = line.split(':').nth(1) {
                    device_code = code.trim().to_string();
                }
            }
            if line.contains("http") {
                if let Some(url_start) = line.find("http") {
                    verification_url = line[url_start..].split_whitespace().next().unwrap_or("").to_string();
                }
            }
        }
        
        if device_code.is_empty() || verification_url.is_empty() {
            return Err("Could not parse device authentication response".into());
        }
        
        Ok((device_code, verification_url))
    }

    /// Wait for device authentication to complete
    pub async fn wait_for_device_auth(&self, timeout_seconds: u64) -> Result<bool, Box<dyn std::error::Error + Send + Sync>> {
        let start_time = std::time::Instant::now();
        let timeout_duration = std::time::Duration::from_secs(timeout_seconds);
        
        while start_time.elapsed() < timeout_duration {
            if self.check_auth_status().await? {
                return Ok(true);
            }
            
            // Wait 5 seconds before checking again
            tokio::time::sleep(std::time::Duration::from_secs(5)).await;
        }
        
        Ok(false) // Timeout reached
    }

    /// Logout from Claude Code
    pub async fn logout(&self) -> Result<String, Box<dyn std::error::Error + Send + Sync>> {
        let command = vec![
            "claude".to_string(),
            "auth".to_string(),
            "logout".to_string(),
        ];

        self.exec_command(command).await
    }

    /// Check Claude Code version and availability
    pub async fn check_availability(&self) -> Result<String, Box<dyn std::error::Error + Send + Sync>> {
        let command = vec!["claude".to_string(), "--version".to_string()];
        self.exec_command(command).await
    }

    /// Get current authentication info
    pub async fn get_auth_info(&self) -> Result<String, Box<dyn std::error::Error + Send + Sync>> {
        let command = vec![
            "claude".to_string(),
            "auth".to_string(),
            "whoami".to_string(),
        ];

        self.exec_command(command).await
    }

    /// Execute a command in the container and return output
    async fn exec_command(&self, command: Vec<String>) -> Result<String, Box<dyn std::error::Error + Send + Sync>> {
        let exec_config = CreateExecOptions {
            cmd: Some(command),
            attach_stdout: Some(true),
            attach_stderr: Some(true),
            working_dir: self.config.working_directory.clone(),
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

        Ok(output.trim().to_string())
    }

    /// Parse Claude Code result from output
    fn parse_result(&self, output: String) -> Result<ClaudeCodeResult, Box<dyn std::error::Error + Send + Sync>> {
        match serde_json::from_str::<ClaudeCodeResult>(&output) {
            Ok(result) => Ok(result),
            Err(_) => Ok(ClaudeCodeResult {
                r#type: "result".to_string(),
                subtype: "success".to_string(),
                cost_usd: 0.0,
                is_error: false,
                duration_ms: 0,
                duration_api_ms: 0,
                num_turns: 1,
                result: output,
                session_id: "unknown".to_string(),
            }),
        }
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
        
        Ok(Self::new(docker, container_id, ClaudeCodeConfig::default()))
    }
}
