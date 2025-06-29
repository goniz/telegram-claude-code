use futures_util::{Stream, StreamExt};
use std::pin::Pin;
use tokio::sync::mpsc;
use tokio_stream::wrappers::UnboundedReceiverStream;

use super::executor::CommandExecutor;
use super::message_parser::{ClaudeMessageParser, ParseResult, ParsedClaudeMessage};
use super::streaming::ClaudeMessage;

/// Claude command execution functionality
#[derive(Debug)]
pub struct ClaudeCommandExecutor {
    executor: CommandExecutor,
}

impl ClaudeCommandExecutor {
    pub fn new(executor: CommandExecutor) -> Self {
        Self { executor }
    }

    /// Execute a Claude prompt with streaming output and fallback to batch processing
    pub async fn execute_claude_prompt(
        &self,
        prompt: &str,
        conversation_id: Option<&str>,
    ) -> Result<ClaudeExecutionResult, Box<dyn std::error::Error + Send + Sync>> {
        log::info!(
            "Executing Claude prompt: '{}' with conversation_id: {:?}",
            prompt,
            conversation_id
        );

        let cmd_args = self.build_command_args(prompt, conversation_id);

        // Try streaming execution first, fallback to batch processing
        match self.executor.exec_streaming_command(cmd_args.clone()).await {
            Ok(string_stream) => {
                log::info!("Using streaming execution for Claude command");
                let parsed_stream = self.create_parsed_stream(string_stream);
                Ok(ClaudeExecutionResult::Streaming(parsed_stream))
            }
            Err(e) => {
                log::warn!(
                    "Streaming execution failed, falling back to batch processing: {}",
                    e
                );
                // Fallback to non-streaming
                let output = self.executor.exec_command(cmd_args).await?;
                Ok(ClaudeExecutionResult::Batch(output))
            }
        }
    }

    /// Build Claude command arguments for execution
    pub fn build_command_args(&self, prompt: &str, conversation_id: Option<&str>) -> Vec<String> {
        let mut cmd_args = vec![
            "claude".to_string(),
            "--print".to_string(),
            "--verbose".to_string(),
            "--output-format".to_string(),
            "stream-json".to_string(),
        ];

        if let Some(conversation_id) = conversation_id {
            log::info!(
                "Building Claude command with conversation ID: {}",
                conversation_id
            );
            cmd_args.push("--resume".to_string());
            cmd_args.push(conversation_id.to_string());
        } else {
            log::info!("Building Claude command without conversation ID (new conversation)");
        }

        cmd_args.push(prompt.to_string());

        log::debug!("Built Claude command: {:?}", cmd_args);
        cmd_args
    }

    /// Process streaming JSON output and extract conversation information
    pub async fn process_streaming_json(
        &self,
        stream: &mut Pin<Box<dyn futures_util::Stream<Item = Result<String, String>> + Send>>,
    ) -> Result<Vec<ClaudeStreamEvent>, Box<dyn std::error::Error + Send + Sync>> {
        let mut events = Vec::new();

        while let Some(line_result) = stream.next().await {
            match line_result {
                Ok(line) => {
                    if let Some(event) = self.parse_streaming_line(&line)? {
                        events.push(event);
                    }
                }
                Err(e) => {
                    log::error!("Error in streaming: {}", e);
                    events.push(ClaudeStreamEvent::Error(e));
                    break;
                }
            }
        }

        Ok(events)
    }

    /// Parse a single JSON line from streaming output
    fn parse_streaming_line(
        &self,
        line: &str,
    ) -> Result<Option<ClaudeStreamEvent>, Box<dyn std::error::Error + Send + Sync>> {
        let line = line.trim();
        if line.is_empty() {
            return Ok(None);
        }

        match serde_json::from_str::<ClaudeMessage>(line) {
            Ok(message) => {
                let event = match message {
                    ClaudeMessage::System {
                        session_id,
                        subtype,
                        ..
                    } => ClaudeStreamEvent::System {
                        session_id,
                        subtype,
                    },
                    ClaudeMessage::Assistant {
                        message: assistant_msg,
                        session_id,
                    } => ClaudeStreamEvent::Assistant {
                        message: assistant_msg,
                        session_id,
                    },
                    ClaudeMessage::User {
                        message: user_msg,
                        session_id,
                    } => ClaudeStreamEvent::User {
                        message: user_msg,
                        session_id,
                    },
                    ClaudeMessage::Result {
                        result,
                        session_id,
                        is_error,
                        total_cost_usd,
                        duration_ms,
                        num_turns,
                        ..
                    } => ClaudeStreamEvent::Result {
                        result,
                        session_id,
                        is_error,
                        total_cost_usd,
                        duration_ms,
                        num_turns,
                    },
                };
                Ok(Some(event))
            }
            Err(_) => {
                // If JSON parsing fails, treat as plain text
                if !line.is_empty() {
                    Ok(Some(ClaudeStreamEvent::PlainText(line.to_string())))
                } else {
                    Ok(None)
                }
            }
        }
    }

    /// Process batch JSON output and extract events
    pub fn process_batch_output(
        &self,
        output: &str,
    ) -> Result<Vec<ClaudeStreamEvent>, Box<dyn std::error::Error + Send + Sync>> {
        let mut events = Vec::new();

        // Process each line of JSON output
        for line in output.lines() {
            if let Some(event) = self.parse_streaming_line(line)? {
                events.push(event);
            }
        }

        Ok(events)
    }

    /// Create a stream of parsed Claude messages from a string stream
    fn create_parsed_stream(
        &self,
        string_stream: Pin<Box<dyn Stream<Item = Result<String, String>> + Send>>,
    ) -> Pin<Box<dyn Stream<Item = Result<ParsedClaudeMessage, String>> + Send>> {
        let (tx, rx) = mpsc::unbounded_channel();

        tokio::spawn(async move {
            let mut string_stream = string_stream;
            while let Some(line_result) = string_stream.next().await {
                match line_result {
                    Ok(line) => {
                        match ClaudeMessageParser::parse_line(&line) {
                            ParseResult::Message(parsed) => {
                                if tx.send(Ok(parsed)).is_err() {
                                    log::debug!("Receiver dropped, stopping parsed stream");
                                    break;
                                }
                            }
                            ParseResult::PlainText(text) => {
                                // For plain text, we could either skip it or create a special message type
                                // For now, we'll log it and skip
                                log::debug!("Skipping plain text in parsed stream: {}", text);
                            }
                            ParseResult::Empty => {
                                // Skip empty lines
                            }
                        }
                    }
                    Err(e) => {
                        if tx.send(Err(e)).is_err() {
                            log::debug!("Receiver dropped, stopping parsed stream");
                        }
                        break;
                    }
                }
            }
        });

        Box::pin(UnboundedReceiverStream::new(rx))
    }
}

/// Result of Claude command execution
pub enum ClaudeExecutionResult {
    Streaming(Pin<Box<dyn Stream<Item = Result<ParsedClaudeMessage, String>> + Send>>),
    Batch(String),
}

/// Events that can occur during Claude streaming
#[derive(Debug)]
pub enum ClaudeStreamEvent {
    System {
        session_id: Option<String>,
        subtype: String,
    },
    Assistant {
        message: super::streaming::AssistantMessage,
        session_id: Option<String>,
    },
    User {
        message: super::streaming::UserMessage,
        session_id: Option<String>,
    },
    Result {
        result: String,
        session_id: String,
        is_error: bool,
        total_cost_usd: Option<f64>,
        duration_ms: Option<u64>,
        num_turns: Option<u32>,
    },
    PlainText(String),
    Error(String),
}

impl ClaudeStreamEvent {
    /// Extract conversation ID from any event that contains one
    pub fn conversation_id(&self) -> Option<&String> {
        match self {
            ClaudeStreamEvent::System { session_id, .. } => session_id.as_ref(),
            ClaudeStreamEvent::Assistant { session_id, .. } => session_id.as_ref(),
            ClaudeStreamEvent::User { session_id, .. } => session_id.as_ref(),
            ClaudeStreamEvent::Result { session_id, .. } => Some(session_id),
            _ => None,
        }
    }

    /// Check if this is an initialization event
    pub fn is_init(&self) -> bool {
        matches!(
            self,
            ClaudeStreamEvent::System { subtype, .. } if subtype == "init"
        )
    }

    /// Check if this is an error event
    pub fn is_error(&self) -> bool {
        matches!(
            self,
            ClaudeStreamEvent::Result { is_error: true, .. } | ClaudeStreamEvent::Error(_)
        )
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_build_command_args_basic() {
        let executor = CommandExecutor::new(
            bollard::Docker::connect_with_local_defaults().unwrap(),
            "test".to_string(),
            super::super::config::ClaudeCodeConfig::default(),
        );
        let claude_executor = ClaudeCommandExecutor::new(executor);

        let prompt = "Write a hello world program";
        let args = claude_executor.build_command_args(prompt, None);

        let expected = vec![
            "claude",
            "--print",
            "--verbose",
            "--output-format",
            "stream-json",
            prompt,
        ];

        assert_eq!(args, expected);
    }

    #[test]
    fn test_build_command_args_with_resume() {
        let executor = CommandExecutor::new(
            bollard::Docker::connect_with_local_defaults().unwrap(),
            "test".to_string(),
            super::super::config::ClaudeCodeConfig::default(),
        );
        let claude_executor = ClaudeCommandExecutor::new(executor);

        let prompt = "Continue the previous task";
        let conversation_id = "test-conversation-123";
        let args = claude_executor.build_command_args(prompt, Some(conversation_id));

        let expected = vec![
            "claude",
            "--print",
            "--verbose",
            "--output-format",
            "stream-json",
            "--resume",
            conversation_id,
            prompt,
        ];

        assert_eq!(args, expected);
    }

    #[test]
    fn test_conversation_id_extraction() {
        let system_event = ClaudeStreamEvent::System {
            session_id: Some("test-id".to_string()),
            subtype: "init".to_string(),
        };
        assert_eq!(system_event.conversation_id(), Some(&"test-id".to_string()));
        assert!(system_event.is_init());

        let result_event = ClaudeStreamEvent::Result {
            result: "Success".to_string(),
            session_id: "test-result-id".to_string(),
            is_error: false,
            total_cost_usd: None,
            duration_ms: None,
            num_turns: None,
        };
        assert_eq!(
            result_event.conversation_id(),
            Some(&"test-result-id".to_string())
        );
        assert!(!result_event.is_error());

        let error_event = ClaudeStreamEvent::Error("Test error".to_string());
        assert_eq!(error_event.conversation_id(), None);
        assert!(error_event.is_error());
    }
}
