use futures_util::{Stream, StreamExt};
use std::pin::Pin;
use tokio::sync::mpsc;
use tokio_stream::wrappers::UnboundedReceiverStream;

use super::executor::CommandExecutor;
use super::message_parser::{ClaudeMessageParser, ParseResult, ParsedClaudeMessage};

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
        Self::build_command_args_static(prompt, conversation_id)
    }

    /// Static version of `build_command_args` so it can be unit-tested without
    /// needing to construct a full `ClaudeCommandExecutor` (which normally
    /// requires an active Docker daemon).
    pub fn build_command_args_static(
        prompt: &str,
        conversation_id: Option<&str>,
    ) -> Vec<String> {
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_build_command_args_basic() {
        let prompt = "Write a hello world program";
        let args = ClaudeCommandExecutor::build_command_args_static(prompt, None);

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
        let prompt = "Continue the previous task";
        let conversation_id = "test-conversation-123";
        let args = ClaudeCommandExecutor::build_command_args_static(prompt, Some(conversation_id));

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
}
