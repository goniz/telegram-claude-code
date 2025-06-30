/// Telegram's maximum message length in characters
pub const TELEGRAM_MAX_MESSAGE_LENGTH: usize = 4096;


/// Escape reserved characters for Telegram MarkdownV2 formatting
/// According to Telegram's MarkdownV2 spec, these characters must be escaped:
/// '_', '*', '[', ']', '(', ')', '~', '`', '>', '#', '+', '-', '=', '|', '{', '}', '.', '!'
pub fn escape_markdown_v2(text: &str) -> String {
    text.chars()
        .map(|c| match c {
            '_' | '*' | '[' | ']' | '(' | ')' | '~' | '`' | '>' | '#' | '+' | '-' | '=' | '|'
            | '{' | '}' | '.' | '!' => {
                format!("\\{}", c)
            }
            _ => c.to_string(),
        })
        .collect()
}




/// Check if a message needs to be truncated and provide a truncated version with continuation notice
/// Returns (truncated_text, was_truncated)
pub fn truncate_if_needed(text: &str) -> (String, bool) {
    if text.len() <= TELEGRAM_MAX_MESSAGE_LENGTH {
        return (text.to_string(), false);
    }
    
    let truncation_notice = "\n\n\\.\\.\\.\\[message truncated\\]";
    let available_space = TELEGRAM_MAX_MESSAGE_LENGTH - truncation_notice.len();
    
    // Find a good breaking point (prefer line boundaries)
    let truncated = if let Some(last_newline) = text[..available_space].rfind('\n') {
        &text[..last_newline]
    } else if let Some(last_space) = text[..available_space].rfind(' ') {
        &text[..last_space]
    } else {
        &text[..available_space]
    };
    
    (format!("{}{}", truncated, truncation_notice), true)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_escape_markdown_v2_reserved_characters() {
        // Test each reserved character is properly escaped
        assert_eq!(escape_markdown_v2("_"), "\\_");
        assert_eq!(escape_markdown_v2("*"), "\\*");
        assert_eq!(escape_markdown_v2("["), "\\[");
        assert_eq!(escape_markdown_v2("]"), "\\]");
        assert_eq!(escape_markdown_v2("("), "\\(");
        assert_eq!(escape_markdown_v2(")"), "\\)");
        assert_eq!(escape_markdown_v2("~"), "\\~");
        assert_eq!(escape_markdown_v2("`"), "\\`");
        assert_eq!(escape_markdown_v2(">"), "\\>");
        assert_eq!(escape_markdown_v2("#"), "\\#");
        assert_eq!(escape_markdown_v2("+"), "\\+");
        assert_eq!(escape_markdown_v2("-"), "\\-");
        assert_eq!(escape_markdown_v2("="), "\\=");
        assert_eq!(escape_markdown_v2("|"), "\\|");
        assert_eq!(escape_markdown_v2("{"), "\\{");
        assert_eq!(escape_markdown_v2("}"), "\\}");
        assert_eq!(escape_markdown_v2("."), "\\.");
        assert_eq!(escape_markdown_v2("!"), "\\!");
    }

    #[test]
    fn test_escape_markdown_v2_non_reserved_characters() {
        // Test non-reserved characters are not escaped
        assert_eq!(escape_markdown_v2("a"), "a");
        assert_eq!(escape_markdown_v2("Z"), "Z");
        assert_eq!(escape_markdown_v2("0"), "0");
        assert_eq!(escape_markdown_v2("9"), "9");
        assert_eq!(escape_markdown_v2(" "), " ");
        assert_eq!(escape_markdown_v2("\n"), "\n");
        assert_eq!(escape_markdown_v2("🎯"), "🎯");
    }

    #[test]
    fn test_escape_markdown_v2_mixed_text() {
        // Test realistic examples with mixed content
        assert_eq!(
            escape_markdown_v2("user.name@example.com"),
            "user\\.name@example\\.com"
        );
        assert_eq!(
            escape_markdown_v2("https://github.com/user/repo"),
            "https://github\\.com/user/repo"
        );
        assert_eq!(
            escape_markdown_v2("Device code: ABC-123!"),
            "Device code: ABC\\-123\\!"
        );
        assert_eq!(
            escape_markdown_v2("Configuration [UPDATED]"),
            "Configuration \\[UPDATED\\]"
        );
    }

    #[test]
    fn test_escape_markdown_v2_empty_string() {
        assert_eq!(escape_markdown_v2(""), "");
    }

    #[test]
    fn test_markdownv2_url_format() {
        let url = "https://github.com/device";
        let display_text = "Click here";
        let formatted = format!("[{}]({})", escape_markdown_v2(display_text), url);
        assert_eq!(formatted, "[Click here](https://github.com/device)");
    }

    #[test]
    fn test_markdownv2_code_block_format() {
        let code = "ABC-123";
        let formatted = format!("```{}```", escape_markdown_v2(code));
        assert_eq!(formatted, "```ABC\\-123```");
    }


    #[test]
    fn test_truncate_if_needed_short_text() {
        let text = "This is a short message.";
        let (truncated, was_truncated) = truncate_if_needed(text);
        assert_eq!(truncated, text);
        assert!(!was_truncated);
    }

    #[test]
    fn test_truncate_if_needed_long_text() {
        // Create a message longer than the limit
        let text = "a".repeat(TELEGRAM_MAX_MESSAGE_LENGTH + 100);
        let (truncated, was_truncated) = truncate_if_needed(&text);
        
        assert!(was_truncated);
        assert!(truncated.len() <= TELEGRAM_MAX_MESSAGE_LENGTH);
        assert!(truncated.contains("\\[message truncated\\]"));
    }

    #[test]
    fn test_truncate_if_needed_with_newlines() {
        // Create a message with newlines that's too long (ensure it exceeds 4096 chars)
        let lines: Vec<String> = (0..500).map(|i| format!("This is a longer line number {} with more content to exceed the limit", i)).collect();
        let text = lines.join("\n");
        
        // Verify the text is actually longer than the limit
        assert!(text.len() > TELEGRAM_MAX_MESSAGE_LENGTH);
        
        let (truncated, was_truncated) = truncate_if_needed(&text);
        
        assert!(was_truncated);
        assert!(truncated.len() <= TELEGRAM_MAX_MESSAGE_LENGTH);
        assert!(truncated.contains("\\[message truncated\\]"));
        
        // The function should prefer breaking at newline boundaries when possible
        // Just verify that the truncation worked correctly
        assert!(truncated.len() < text.len());
    }
}
