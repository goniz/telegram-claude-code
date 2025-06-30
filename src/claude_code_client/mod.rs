use bollard::Docker;
use serde::{Deserialize, Serialize};

pub mod auth;
pub mod claude_command;
pub mod config;
pub mod container;
pub mod container_cred_storage;
pub mod container_utils;
pub mod executor;
pub mod message_parser;
pub mod response_processor;
pub mod streaming;

pub use auth::{AuthState, AuthenticationHandle};
pub use claude_command::{ClaudeCommandExecutor, ClaudeExecutionResult};
pub use config::ClaudeCodeConfig;
pub use container_cred_storage::ContainerCredStorage;
pub use executor::CommandExecutor;
pub use message_parser::{ClaudeMessageParser, MessageType, ParseResult, ParsedClaudeMessage};
pub use response_processor::{
    ErrorInfo, LiveMessage, ProcessedResponse, ResponseItem, ResponseProcessor, SessionInfo,
    ToolResultItem,
};
pub use streaming::{AssistantMessage, ClaudeMessage, ContentBlock, ToolResult, UserMessage};

// Re-export OAuth types from oauth module
pub use crate::oauth::{ClaudeAuth, Config as OAuthConfig, Credentials, OAuthError};

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

#[derive(Debug)]
#[allow(dead_code)]
pub struct ClaudeCodeClient {
    docker: Docker,
    container_id: String,
    config: ClaudeCodeConfig,
    auth_manager: auth::AuthenticationManager,
    executor: CommandExecutor,
    claude_executor: ClaudeCommandExecutor,
}

#[allow(dead_code)]
impl ClaudeCodeClient {
    /// Create a new Claude Code client for the specified container
    pub fn new(docker: Docker, container_id: String, config: ClaudeCodeConfig) -> Self {
        let auth_manager =
            auth::AuthenticationManager::new(docker.clone(), container_id.clone(), config.clone());
        let executor = CommandExecutor::new(docker.clone(), container_id.clone(), config.clone());
        let claude_executor = ClaudeCommandExecutor::new(executor.clone());

        Self {
            docker,
            container_id,
            config,
            auth_manager,
            executor,
            claude_executor,
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
        self.auth_manager.authenticate_claude_account().await
    }

    /// Check authentication status using OAuth credentials
    pub async fn check_auth_status(
        &self,
    ) -> Result<bool, Box<dyn std::error::Error + Send + Sync>> {
        // Delegate to executor for status commands
        let status_command = vec![
            "claude".to_string(),
            "--output-format=json".to_string(),
            "--print".to_string(),
            "say hi".to_string(),
        ];

        match self.executor.exec_command(status_command).await {
            Ok(output) => match self.parse_result(output) {
                Ok(result) => {
                    let is_authenticated = !result.is_error;
                    Ok(is_authenticated)
                }
                Err(_) => Ok(false),
            },
            Err(_) => Ok(false),
        }
    }

    /// Get current authentication info
    pub async fn get_auth_info(&self) -> Result<String, Box<dyn std::error::Error + Send + Sync>> {
        match self.check_auth_status().await {
            Ok(true) => Ok("✅ Claude Code is authenticated and ready to use".to_string()),
            Ok(false) => Ok(
                "❌ Claude Code is not authenticated. Please authenticate with your Claude \
                 account using OAuth."
                    .to_string(),
            ),
            Err(e) => Err(format!("Unable to check authentication status: {}", e).into()),
        }
    }

    /// Logout from Claude Code by removing stored credentials
    pub async fn logout_claude(&self) -> Result<String, Box<dyn std::error::Error + Send + Sync>> {
        // Remove the credentials file
        let remove_command = vec![
            "rm".to_string(),
            "-f".to_string(),
            "/root/.claude/.credentials.json".to_string(),
        ];

        match self.executor.exec_command(remove_command).await {
            Ok(_) => {
                // Verify the logout was successful
                match self.check_auth_status().await {
                    Ok(false) => Ok("✅ Successfully logged out from Claude Code".to_string()),
                    Ok(true) => Ok(
                        "⚠️ Logout may not have been successful - Claude Code still appears \
                         authenticated"
                            .to_string(),
                    ),
                    Err(_) => {
                        Ok("✅ Logged out from Claude Code (status check failed)".to_string())
                    }
                }
            }
            Err(e) => Err(format!("Failed to logout from Claude Code: {}", e).into()),
        }
    }

    /// Check Claude Code version and availability
    pub async fn check_availability(
        &self,
    ) -> Result<String, Box<dyn std::error::Error + Send + Sync>> {
        let command = vec!["claude".to_string(), "--version".to_string()];
        self.executor.exec_command(command).await
    }

    /// Update Claude CLI to latest version
    pub async fn update_claude(&self) -> Result<String, Box<dyn std::error::Error + Send + Sync>> {
        let command = vec![
            "/opt/entrypoint.sh".to_string(),
            "-c".to_string(),
            "nvm use default && npm install -g @anthropic-ai/claude-code".to_string(),
        ];
        self.executor.exec_command(command).await
    }

    /// Helper method for basic command execution (used in tests)
    #[allow(dead_code)]
    pub async fn exec_basic_command(
        &self,
        command: Vec<String>,
    ) -> Result<String, Box<dyn std::error::Error + Send + Sync>> {
        self.executor.exec_command(command).await
    }

    /// Execute a command and return a stream of output lines (for real-time processing)
    pub async fn exec_streaming_command(
        &self,
        command: Vec<String>,
    ) -> Result<
        std::pin::Pin<Box<dyn futures_util::Stream<Item = Result<String, String>> + Send>>,
        Box<dyn std::error::Error + Send + Sync>,
    > {
        self.executor.exec_streaming_command(command).await
    }

    /// Execute a Claude prompt with streaming or batch processing
    pub async fn execute_claude_prompt(
        &self,
        prompt: &str,
        conversation_id: Option<&str>,
    ) -> Result<ClaudeExecutionResult, Box<dyn std::error::Error + Send + Sync>> {
        self.claude_executor
            .execute_claude_prompt(prompt, conversation_id)
            .await
    }

    /// Build Claude command arguments
    pub fn build_claude_command_args(
        &self,
        prompt: &str,
        conversation_id: Option<&str>,
    ) -> Vec<String> {
        self.claude_executor
            .build_command_args(prompt, conversation_id)
    }

    /// Process parsed Claude messages using the response processor
    pub fn process_claude_messages(
        &self,
        messages: Vec<ParsedClaudeMessage>,
        processor: &mut ResponseProcessor,
    ) -> ProcessedResponse {
        processor.process_messages(messages)
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
        Self::for_session_with_working_dir(docker, container_name, None).await
    }

    /// Helper method to create a client for a coding session with custom working directory
    pub async fn for_session_with_working_dir(
        docker: Docker,
        container_name: &str,
        working_directory: Option<String>,
    ) -> Result<Self, Box<dyn std::error::Error + Send + Sync>> {
        // Find the container by name
        let containers = docker
            .list_containers(None::<bollard::query_parameters::ListContainersOptions>)
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

        let mut config = ClaudeCodeConfig::default();
        if let Some(dir) = working_directory {
            // Ensure working directory is absolute for Docker exec
            let absolute_dir = if std::path::Path::new(&dir).is_absolute() {
                dir
            } else {
                // Use the config's default working directory as the base instead of hardcoding
                let base_dir = config.working_directory.as_deref().unwrap_or("/workspace");
                format!("{}/{}", base_dir, dir)
            };
            config.working_directory = Some(absolute_dir);
        }

        Ok(Self::new(docker, container_id, config))
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
