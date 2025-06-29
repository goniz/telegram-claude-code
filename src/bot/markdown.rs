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
}
