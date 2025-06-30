/// Telegram's maximum message length in characters
pub const TELEGRAM_MAX_MESSAGE_LENGTH: usize = 4096;

/// Maximum length to use for chunks, leaving some buffer for markdown formatting
const SAFE_CHUNK_SIZE: usize = 3800;

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

/// Split a message into chunks that fit within Telegram's message length limit
/// Attempts to split at natural boundaries (newlines, then word boundaries)
pub fn split_message(text: &str) -> Vec<String> {
    if text.len() <= TELEGRAM_MAX_MESSAGE_LENGTH {
        return vec![text.to_string()];
    }

    let mut chunks = Vec::new();
    let mut current_chunk = String::new();
    
    // Split by lines first to preserve formatting
    let lines: Vec<&str> = text.lines().collect();
    
    for line in lines {
        let line_with_newline = if current_chunk.is_empty() {
            line.to_string()
        } else {
            format!("\n{}", line)
        };

        // If adding this line would exceed the limit
        if current_chunk.len() + line_with_newline.len() > SAFE_CHUNK_SIZE {
            // If we have accumulated content, save it as a chunk
            if !current_chunk.is_empty() {
                chunks.push(current_chunk.clone());
                current_chunk.clear();
            }
            
            // If this single line is too long, split it by words
            if line.len() > SAFE_CHUNK_SIZE {
                let word_chunks = split_line_by_words(line);
                chunks.extend(word_chunks);
            } else {
                current_chunk = line.to_string();
            }
        } else {
            current_chunk.push_str(&line_with_newline);
        }
    }
    
    // Add any remaining content
    if !current_chunk.is_empty() {
        chunks.push(current_chunk);
    }
    
    // If we still have no chunks (empty input), return one empty chunk
    if chunks.is_empty() {
        chunks.push(String::new());
    }
    
    chunks
}

/// Split a single long line by word boundaries
fn split_line_by_words(line: &str) -> Vec<String> {
    let mut chunks = Vec::new();
    let mut current_chunk = String::new();
    
    let words: Vec<&str> = line.split_whitespace().collect();
    
    for word in words {
        let word_with_space = if current_chunk.is_empty() {
            word.to_string()
        } else {
            format!(" {}", word)
        };
        
        // If adding this word would exceed the limit
        if current_chunk.len() + word_with_space.len() > SAFE_CHUNK_SIZE {
            // Save current chunk if it has content
            if !current_chunk.is_empty() {
                chunks.push(current_chunk.clone());
                current_chunk.clear();
            }
            
            // If this single word is still too long, split it by characters
            if word.len() > SAFE_CHUNK_SIZE {
                let char_chunks = split_word_by_chars(word);
                chunks.extend(char_chunks);
            } else {
                current_chunk = word.to_string();
            }
        } else {
            current_chunk.push_str(&word_with_space);
        }
    }
    
    // Add any remaining content
    if !current_chunk.is_empty() {
        chunks.push(current_chunk);
    }
    
    chunks
}

/// Split a single long word by character boundaries (fallback for extremely long words)
fn split_word_by_chars(word: &str) -> Vec<String> {
    let mut chunks = Vec::new();
    let chars: Vec<char> = word.chars().collect();
    
    for chunk_start in (0..chars.len()).step_by(SAFE_CHUNK_SIZE) {
        let chunk_end = std::cmp::min(chunk_start + SAFE_CHUNK_SIZE, chars.len());
        let chunk: String = chars[chunk_start..chunk_end].iter().collect();
        chunks.push(chunk);
    }
    
    chunks
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
    fn test_split_message_short_text() {
        let text = "This is a short message.";
        let chunks = split_message(text);
        assert_eq!(chunks.len(), 1);
        assert_eq!(chunks[0], text);
    }

    #[test]
    fn test_split_message_empty_text() {
        let text = "";
        let chunks = split_message(text);
        assert_eq!(chunks.len(), 1);
        assert_eq!(chunks[0], "");
    }

    #[test]
    fn test_split_message_long_text_by_lines() {
        // Create a message with multiple lines that exceeds the safe chunk size
        let lines: Vec<String> = (0..100).map(|i| format!("This is line number {} with some content.", i)).collect();
        let text = lines.join("\n");
        
        let chunks = split_message(&text);
        
        // Should be split into multiple chunks
        assert!(chunks.len() > 1);
        
        // Each chunk should be within the safe size limit
        for chunk in &chunks {
            assert!(chunk.len() <= SAFE_CHUNK_SIZE);
        }
        
        // Joining chunks should reconstruct the original (with potential line break differences)
        let reconstructed = chunks.join("\n");
        let original_lines: Vec<&str> = text.lines().collect();
        let reconstructed_lines: Vec<&str> = reconstructed.lines().collect();
        assert_eq!(original_lines, reconstructed_lines);
    }

    #[test]
    fn test_split_message_very_long_single_line() {
        // Create a very long single line
        let text = "word ".repeat(1000); // This will be much longer than SAFE_CHUNK_SIZE
        
        let chunks = split_message(&text);
        
        // Should be split into multiple chunks
        assert!(chunks.len() > 1);
        
        // Each chunk should be within the safe size limit
        for chunk in &chunks {
            assert!(chunk.len() <= SAFE_CHUNK_SIZE);
        }
    }

    #[test]
    fn test_split_message_extremely_long_word() {
        // Create an extremely long single word (no spaces)
        let text = "a".repeat(SAFE_CHUNK_SIZE * 2);
        
        let chunks = split_message(&text);
        
        // Should be split into multiple chunks
        assert!(chunks.len() > 1);
        
        // Each chunk should be within the safe size limit
        for chunk in &chunks {
            assert!(chunk.len() <= SAFE_CHUNK_SIZE);
        }
        
        // Joining chunks should reconstruct the original
        let reconstructed = chunks.join("");
        assert_eq!(reconstructed, text);
    }

    #[test]
    fn test_split_message_preserves_formatting() {
        let text = "Line 1\n\nLine 3\n    Indented line\n```\nCode block\n```";
        let chunks = split_message(text);
        
        if chunks.len() == 1 {
            // If it fits in one chunk, it should be unchanged
            assert_eq!(chunks[0], text);
        } else {
            // If split, joining should preserve the structure
            let reconstructed = chunks.join("\n");
            let original_lines: Vec<&str> = text.lines().collect();
            let reconstructed_lines: Vec<&str> = reconstructed.lines().collect();
            assert_eq!(original_lines, reconstructed_lines);
        }
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
