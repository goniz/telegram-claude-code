use std::time::{Duration, Instant};

use super::message_parser::{MessageType, ParsedClaudeMessage};

/// Processes Claude responses and maintains conversation state
#[derive(Debug)]
pub struct ResponseProcessor {
    conversation_id: Option<String>,
}

impl ResponseProcessor {
    pub fn new() -> Self {
        Self {
            conversation_id: None,
        }
    }

    pub fn with_conversation_id(conversation_id: Option<String>) -> Self {
        Self { conversation_id }
    }

    /// Get the current conversation ID
    pub fn conversation_id(&self) -> Option<&String> {
        self.conversation_id.as_ref()
    }

    /// Update the conversation ID
    pub fn set_conversation_id(&mut self, conversation_id: Option<String>) {
        self.conversation_id = conversation_id;
    }

    /// Process a list of parsed Claude messages and extract structured responses
    pub fn process_messages(&mut self, messages: Vec<ParsedClaudeMessage>) -> ProcessedResponse {
        let mut responses = Vec::new();
        let mut tool_results = Vec::new();
        let mut session_info = None;
        let mut error_info = None;

        for parsed_message in messages {
            // Update conversation ID if found
            if let Some(conv_id) = &parsed_message.conversation_id {
                self.conversation_id = Some(conv_id.clone());
            }

            match &parsed_message.message_type {
                MessageType::SystemInit { .. } => {
                    responses.push(ResponseItem::SystemMessage(
                        "Claude session initialized".to_string(),
                    ));
                }
                MessageType::AssistantText { text, .. } => {
                    responses.push(ResponseItem::AssistantText(text.clone()));
                }
                MessageType::AssistantToolUse { name, input, .. } => {
                    let input_str = input
                        .as_ref()
                        .map(|v| serde_json::to_string_pretty(v).unwrap_or_default())
                        .unwrap_or_default();
                    responses.push(ResponseItem::ToolUse {
                        name: name.clone(),
                        input: input_str,
                        id: "".to_string(), // ParsedClaudeMessage doesn't store tool ID
                    });
                }
                MessageType::UserToolResult { content, .. } => {
                    tool_results.push(ToolResultItem {
                        tool_use_id: None,
                        content: content.clone(),
                    });
                }
                MessageType::Result {
                    result,
                    is_error,
                    cost,
                    duration_ms,
                    num_turns,
                    ..
                } => {
                    if *is_error {
                        error_info = Some(ErrorInfo {
                            message: result.clone(),
                            cost: *cost,
                            duration_ms: *duration_ms,
                            num_turns: *num_turns,
                        });
                    } else {
                        responses.push(ResponseItem::FinalResult(result.clone()));
                    }

                    // Create session info
                    session_info = Some(SessionInfo {
                        cost: *cost,
                        duration_ms: *duration_ms,
                        num_turns: *num_turns,
                    });
                }
                MessageType::Other { .. } => {
                    // Skip other message types
                }
            }
        }

        ProcessedResponse {
            responses,
            tool_results,
            session_info,
            error_info,
            conversation_id: self.conversation_id.clone(),
        }
    }


    /// Reset conversation state
    pub fn reset(&mut self) {
        self.conversation_id = None;
    }
}

impl Default for ResponseProcessor {
    fn default() -> Self {
        Self::new()
    }
}

/// Processed response from Claude
#[derive(Debug)]
pub struct ProcessedResponse {
    pub responses: Vec<ResponseItem>,
    pub tool_results: Vec<ToolResultItem>,
    pub session_info: Option<SessionInfo>,
    pub error_info: Option<ErrorInfo>,
    pub conversation_id: Option<String>,
}

impl ProcessedResponse {
    /// Check if the response contains any errors
    pub fn has_error(&self) -> bool {
        self.error_info.is_some()
    }

    /// Get error message if present
    pub fn error_message(&self) -> Option<&String> {
        self.error_info.as_ref().map(|e| &e.message)
    }

    /// Check if response is empty
    pub fn is_empty(&self) -> bool {
        self.responses.is_empty() && self.tool_results.is_empty()
    }
}

/// Individual response items from Claude
#[derive(Debug)]
pub enum ResponseItem {
    SystemMessage(String),
    AssistantText(String),
    ToolUse {
        name: String,
        input: String,
        id: String,
    },
    FinalResult(String),
    PlainText(String),
}

/// Tool result information
#[derive(Debug)]
pub struct ToolResultItem {
    pub tool_use_id: Option<String>,
    pub content: String,
}

impl ToolResultItem {
    /// Check if the tool result is large (should be sent as attachment)
    pub fn is_large(&self) -> bool {
        self.content.lines().count() > 20
    }

    /// Create a preview of the tool result content
    pub fn create_preview(&self, max_lines: usize) -> String {
        let lines: Vec<&str> = self.content.lines().collect();

        if lines.len() <= max_lines {
            // Content is short enough, show it all
            self.content.clone()
        } else {
            // Content is too long, show preview with truncation indicator
            let preview_lines = &lines[0..max_lines];
            let preview_content = preview_lines.join("\n");
            let remaining_lines = lines.len() - max_lines;

            format!(
                "{}...\n\n[{} more lines hidden]",
                preview_content, remaining_lines
            )
        }
    }
}

/// Session information
#[derive(Debug)]
pub struct SessionInfo {
    pub cost: Option<f64>,
    pub duration_ms: Option<u64>,
    pub num_turns: Option<u32>,
}

impl SessionInfo {
    /// Format session info as a summary string
    pub fn format_summary(&self) -> String {
        let mut summary_parts = Vec::new();

        if let Some(cost) = self.cost {
            if cost > 0.0 {
                summary_parts.push(format!("${:.4}", cost));
            }
        }
        if let Some(duration) = self.duration_ms {
            summary_parts.push(format!("{}ms", duration));
        }
        if let Some(turns) = self.num_turns {
            summary_parts.push(format!("{} turns", turns));
        }

        if summary_parts.is_empty() {
            "Session completed".to_string()
        } else {
            summary_parts.join(" • ")
        }
    }
}

/// Error information
#[derive(Debug)]
pub struct ErrorInfo {
    pub message: String,
    pub cost: Option<f64>,
    pub duration_ms: Option<u64>,
    pub num_turns: Option<u32>,
}

/// Live message state for real-time updates (used for streaming)
#[derive(Debug)]
pub struct LiveMessage {
    pub content: String,
    pub last_update: Instant,
    pub is_finalized: bool,
}

impl LiveMessage {
    pub fn new(content: String) -> Self {
        Self {
            content,
            last_update: Instant::now(),
            is_finalized: false,
        }
    }

    pub fn should_update(&self) -> bool {
        !self.is_finalized && self.last_update.elapsed() > Duration::from_millis(500)
    }

    pub fn update_content(&mut self, new_content: String) -> bool {
        if self.content != new_content {
            self.content = new_content;
            self.last_update = Instant::now();
            true
        } else {
            false
        }
    }

    pub fn finalize(&mut self) {
        self.is_finalized = true;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_response_processor_basic() {
        let mut processor = ResponseProcessor::new();
        assert!(processor.conversation_id().is_none());

        let messages = vec![
            ParsedClaudeMessage {
                message: crate::claude_code_client::ClaudeMessage::System {
                    subtype: "init".to_string(),
                    session_id: Some("test-session".to_string()),
                    cwd: Some("/workspace".to_string()),
                    tools: None,
                    model: None,
                },
                conversation_id: Some("test-session".to_string()),
                message_type: MessageType::SystemInit {
                    conversation_id: Some("test-session".to_string()),
                },
            },
            ParsedClaudeMessage {
                message: crate::claude_code_client::ClaudeMessage::Assistant {
                    message: crate::claude_code_client::AssistantMessage {
                        id: Some("msg1".to_string()),
                        content: Some(vec![crate::claude_code_client::ContentBlock::Text {
                            text: "Hello!".to_string(),
                        }]),
                    },
                    session_id: Some("test-session".to_string()),
                },
                conversation_id: Some("test-session".to_string()),
                message_type: MessageType::AssistantText {
                    text: "Hello!".to_string(),
                    conversation_id: Some("test-session".to_string()),
                },
            },
        ];

        let response = processor.process_messages(messages);

        assert_eq!(response.conversation_id, Some("test-session".to_string()));
        assert_eq!(
            processor.conversation_id(),
            Some(&"test-session".to_string())
        );
        assert_eq!(response.responses.len(), 2);
        assert!(!response.has_error());
    }

    #[test]
    fn test_tool_result_preview() {
        let short_content = "Line 1\nLine 2\nLine 3";
        let tool_result = ToolResultItem {
            tool_use_id: Some("tool1".to_string()),
            content: short_content.to_string(),
        };

        assert!(!tool_result.is_large());
        assert_eq!(tool_result.create_preview(20), short_content);

        // Create content with more than 20 lines
        let lines: Vec<String> = (1..=30).map(|i| format!("Line {}", i)).collect();
        let long_content = lines.join("\n");

        let large_tool_result = ToolResultItem {
            tool_use_id: Some("tool2".to_string()),
            content: long_content.clone(),
        };

        assert!(large_tool_result.is_large());
        let preview = large_tool_result.create_preview(10);
        assert!(preview.contains("Line 1"));
        assert!(preview.contains("Line 10"));
        assert!(!preview.contains("Line 11"));
        assert!(preview.contains("more lines hidden"));
    }

    #[test]
    fn test_session_info_formatting() {
        let session_info = SessionInfo {
            cost: Some(0.05),
            duration_ms: Some(1500),
            num_turns: Some(3),
        };

        let summary = session_info.format_summary();
        assert!(summary.contains("$0.05"));
        assert!(summary.contains("1500ms"));
        assert!(summary.contains("3 turns"));
        assert!(summary.contains("•"));

        let empty_session = SessionInfo {
            cost: None,
            duration_ms: None,
            num_turns: None,
        };

        assert_eq!(empty_session.format_summary(), "Session completed");
    }

    #[test]
    fn test_live_message() {
        let mut live_msg = LiveMessage::new("Initial content".to_string());

        assert!(live_msg.update_content("New content".to_string()));
        assert!(!live_msg.update_content("New content".to_string())); // Same content

        assert!(!live_msg.should_update()); // Too soon
        std::thread::sleep(Duration::from_millis(600));
        assert!(live_msg.should_update()); // Enough time passed

        live_msg.finalize();
        assert!(!live_msg.should_update()); // Finalized
    }
}
