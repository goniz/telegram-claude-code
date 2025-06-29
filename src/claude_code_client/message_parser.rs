use super::streaming::{ClaudeMessage, ContentBlock};

/// Results from parsing a Claude message line
#[derive(Debug)]
pub enum ParseResult {
    /// Successfully parsed a Claude message
    Message(ParsedClaudeMessage),
    /// Line contained plain text (not JSON)
    PlainText(String),
    /// Empty line or whitespace
    Empty,
}

/// A parsed Claude message with extracted information
#[derive(Debug)]
pub struct ParsedClaudeMessage {
    pub message: ClaudeMessage,
    pub conversation_id: Option<String>,
    pub message_type: MessageType,
}

/// Type of Claude message for easier handling
#[derive(Debug)]
pub enum MessageType {
    SystemInit {
        conversation_id: Option<String>,
    },
    AssistantText {
        text: String,
        conversation_id: Option<String>,
    },
    AssistantToolUse {
        name: String,
        input: Option<serde_json::Value>,
        conversation_id: Option<String>,
    },
    UserToolResult {
        content: String,
        conversation_id: Option<String>,
    },
    Result {
        result: String,
        conversation_id: String,
        is_error: bool,
        cost: Option<f64>,
        duration_ms: Option<u64>,
        num_turns: Option<u32>,
    },
    Other {
        conversation_id: Option<String>,
    },
}

/// Claude message parser
pub struct ClaudeMessageParser;

impl ClaudeMessageParser {
    /// Parse a single line from Claude output
    pub fn parse_line(line: &str) -> ParseResult {
        let line = line.trim();
        if line.is_empty() {
            return ParseResult::Empty;
        }

        match serde_json::from_str::<ClaudeMessage>(line) {
            Ok(message) => {
                let parsed = Self::process_message(message);
                ParseResult::Message(parsed)
            }
            Err(_) => ParseResult::PlainText(line.to_string()),
        }
    }

    /// Parse multiple lines from Claude output
    pub fn parse_lines(output: &str) -> Vec<ParseResult> {
        output
            .lines()
            .map(|line| Self::parse_line(line))
            .filter(|result| !matches!(result, ParseResult::Empty))
            .collect()
    }

    /// Process a parsed Claude message and extract relevant information
    fn process_message(message: ClaudeMessage) -> ParsedClaudeMessage {
        let conversation_id = Self::extract_conversation_id(&message);
        let message_type = Self::classify_message(&message);

        ParsedClaudeMessage {
            message,
            conversation_id,
            message_type,
        }
    }

    /// Extract conversation ID from any Claude message
    fn extract_conversation_id(message: &ClaudeMessage) -> Option<String> {
        match message {
            ClaudeMessage::System { session_id, .. } => session_id.clone(),
            ClaudeMessage::Assistant { session_id, .. } => session_id.clone(),
            ClaudeMessage::User { session_id, .. } => session_id.clone(),
            ClaudeMessage::Result { session_id, .. } => Some(session_id.clone()),
        }
    }

    /// Classify the message type for easier handling
    fn classify_message(message: &ClaudeMessage) -> MessageType {
        match message {
            ClaudeMessage::System {
                subtype,
                session_id,
                ..
            } => {
                if subtype == "init" {
                    MessageType::SystemInit {
                        conversation_id: session_id.clone(),
                    }
                } else {
                    MessageType::Other {
                        conversation_id: session_id.clone(),
                    }
                }
            }
            ClaudeMessage::Assistant {
                message,
                session_id,
            } => {
                if let Some(content_blocks) = &message.content {
                    for block in content_blocks {
                        match block {
                            ContentBlock::Text { text } => {
                                return MessageType::AssistantText {
                                    text: text.clone(),
                                    conversation_id: session_id.clone(),
                                };
                            }
                            ContentBlock::ToolUse { name, input, .. } => {
                                return MessageType::AssistantToolUse {
                                    name: name.clone(),
                                    input: input.clone(),
                                    conversation_id: session_id.clone(),
                                };
                            }
                        }
                    }
                }
                MessageType::Other {
                    conversation_id: session_id.clone(),
                }
            }
            ClaudeMessage::User {
                message,
                session_id,
            } => {
                if let Some(content) = &message.content {
                    for tool_result in content {
                        if let Some(result_content) = &tool_result.content {
                            return MessageType::UserToolResult {
                                content: result_content.clone(),
                                conversation_id: session_id.clone(),
                            };
                        }
                    }
                }
                MessageType::Other {
                    conversation_id: session_id.clone(),
                }
            }
            ClaudeMessage::Result {
                result,
                session_id,
                is_error,
                total_cost_usd,
                duration_ms,
                num_turns,
                ..
            } => MessageType::Result {
                result: result.clone(),
                conversation_id: session_id.clone(),
                is_error: *is_error,
                cost: *total_cost_usd,
                duration_ms: *duration_ms,
                num_turns: *num_turns,
            },
        }
    }
}

/// Convenience methods for handling parsed messages
impl ParsedClaudeMessage {
    /// Check if this is an initialization message
    pub fn is_init(&self) -> bool {
        matches!(self.message_type, MessageType::SystemInit { .. })
    }

    /// Check if this is an error result
    pub fn is_error(&self) -> bool {
        matches!(
            self.message_type,
            MessageType::Result { is_error: true, .. }
        )
    }

    /// Get assistant text content if this is a text message
    pub fn get_text(&self) -> Option<&String> {
        match &self.message_type {
            MessageType::AssistantText { text, .. } => Some(text),
            _ => None,
        }
    }

    /// Get tool use information if this is a tool use message
    pub fn get_tool_use(&self) -> Option<(&String, &Option<serde_json::Value>)> {
        match &self.message_type {
            MessageType::AssistantToolUse { name, input, .. } => Some((name, input)),
            _ => None,
        }
    }

    /// Get tool result content if this is a tool result message
    pub fn get_tool_result(&self) -> Option<&String> {
        match &self.message_type {
            MessageType::UserToolResult { content, .. } => Some(content),
            _ => None,
        }
    }

    /// Get result information if this is a result message
    pub fn get_result(&self) -> Option<(&String, bool, Option<f64>, Option<u64>, Option<u32>)> {
        match &self.message_type {
            MessageType::Result {
                result,
                is_error,
                cost,
                duration_ms,
                num_turns,
                ..
            } => Some((result, *is_error, *cost, *duration_ms, *num_turns)),
            _ => None,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_parse_line_empty() {
        let result = ClaudeMessageParser::parse_line("");
        assert!(matches!(result, ParseResult::Empty));

        let result = ClaudeMessageParser::parse_line("   \t  ");
        assert!(matches!(result, ParseResult::Empty));
    }

    #[test]
    fn test_parse_line_plain_text() {
        let result = ClaudeMessageParser::parse_line("This is not JSON");
        match result {
            ParseResult::PlainText(text) => assert_eq!(text, "This is not JSON"),
            _ => panic!("Expected PlainText result"),
        }
    }

    #[test]
    fn test_parse_line_valid_json() {
        let json_line = r#"{"type":"system","subtype":"init","session_id":"test-session","cwd":"/workspace","tools":[],"model":"claude-3"}"#;
        let result = ClaudeMessageParser::parse_line(json_line);

        match result {
            ParseResult::Message(parsed) => {
                assert_eq!(parsed.conversation_id, Some("test-session".to_string()));
                assert!(parsed.is_init());
            }
            _ => panic!("Expected Message result"),
        }
    }

    #[test]
    fn test_parse_lines_multiple() {
        let output = r#"{"type":"system","subtype":"init","session_id":"test-session","cwd":"/workspace","tools":[],"model":"claude-3"}
        
This is plain text
{"type":"result","subtype":"success","result":"Done","session_id":"test-session","is_error":false}"#;

        let results = ClaudeMessageParser::parse_lines(output);
        assert_eq!(results.len(), 3);

        match &results[0] {
            ParseResult::Message(parsed) => assert!(parsed.is_init()),
            _ => panic!("Expected Message result"),
        }

        match &results[1] {
            ParseResult::PlainText(text) => assert_eq!(text, "This is plain text"),
            _ => panic!("Expected PlainText result"),
        }

        match &results[2] {
            ParseResult::Message(parsed) => {
                if let Some((result, is_error, _, _, _)) = parsed.get_result() {
                    assert_eq!(result, "Done");
                    assert!(!is_error);
                } else {
                    panic!("Expected result message");
                }
            }
            _ => panic!("Expected Message result"),
        }
    }

    #[test]
    fn test_conversation_id_extraction() {
        let json_line = r#"{"type":"assistant","message":{"content":[{"type":"text","text":"Hello"}]},"session_id":"conv-123"}"#;
        let result = ClaudeMessageParser::parse_line(json_line);

        match result {
            ParseResult::Message(parsed) => {
                assert_eq!(parsed.conversation_id, Some("conv-123".to_string()));
                assert_eq!(parsed.get_text(), Some(&"Hello".to_string()));
            }
            _ => panic!("Expected Message result"),
        }
    }
}
