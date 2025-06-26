use crate::{escape_markdown_v2, BotState, Command};
use teloxide::utils::command::BotCommands;
use teloxide::{prelude::*, types::ParseMode};

/// Generate help text in BotFather format from command descriptions
pub fn generate_help_text() -> String {
    let descriptions = Command::descriptions().to_string();

    // Parse the default descriptions and convert to BotFather format
    descriptions
        .lines()
        .enumerate()
        .filter(|(i, line)| {
            // Skip the header and any lines that don't start with a command
            *i >= 2 && !line.trim().is_empty() && line.trim().starts_with('/')
        })
        .map(|(_, line)| {
            // Remove the leading slash and replace em dash with hyphen
            line.trim()
                .strip_prefix('/')
                .unwrap_or(line.trim())
                .replace(" — ", " - ")
        })
        .collect::<Vec<String>>()
        .join("\n")
}

/// Handle the /help command
pub async fn handle_help(bot: Bot, msg: Message, _bot_state: BotState) -> ResponseResult<()> {
    let help_text = generate_help_text();
    bot.send_message(msg.chat.id, escape_markdown_v2(&help_text))
        .parse_mode(ParseMode::MarkdownV2)
        .await?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use regex::Regex;

    #[test]
    fn test_help_format_matches_botfather_requirements() {
        // Get the dynamically generated help text
        let help_text = generate_help_text();

        // Regex to match the pattern "command - description"
        // Command: lowercase letters (no slash prefix)
        // Separator: " - " (space-hyphen-space)
        // Description: any non-empty text
        let line_pattern = Regex::new(r"^[a-z]+ - .+$").unwrap();

        // Verify each line follows the pattern "command - description"
        for line in help_text.lines() {
            assert!(!line.is_empty(), "Line should not be empty");
            assert!(
                line_pattern.is_match(line),
                "Line should match pattern 'command - description': {}",
                line
            );

            // Additional validation: ensure separator exists and splits correctly
            let parts: Vec<&str> = line.split(" - ").collect();
            assert_eq!(
                parts.len(),
                2,
                "Line should have exactly one ' - ' separator: {}",
                line
            );
            assert!(
                !parts[0].is_empty(),
                "Command part should not be empty: {}",
                line
            );
            assert!(
                !parts[1].is_empty(),
                "Description part should not be empty: {}",
                line
            );
        }

        // Verify we have a non-empty help text
        assert!(!help_text.is_empty(), "Help text should not be empty");
    }

    #[test]
    fn test_help_text_escaping_for_markdownv2() {
        // Get the raw help text
        let help_text = generate_help_text();

        // Apply escaping
        let escaped_help_text = escape_markdown_v2(&help_text);

        // Verify that special characters are properly escaped
        // The help text should not contain unescaped MarkdownV2 special characters
        // after escaping, except for intentional formatting

        // Check that if the original contains special chars, they are escaped
        if help_text.contains('-') {
            assert!(
                escaped_help_text.contains("\\-"),
                "Hyphen should be escaped in MarkdownV2 format"
            );
        }

        // Verify the escaped text is different from original if special chars exist
        let has_special_chars = help_text.chars().any(|c| "_*[]()~`>#+-=|{}.!".contains(c));

        if has_special_chars {
            assert_ne!(
                help_text, escaped_help_text,
                "Escaped text should differ from original when special characters exist"
            );
        }

        // Verify that the escaping function produces a valid result
        assert!(
            !escaped_help_text.is_empty(),
            "Escaped help text should not be empty"
        );
    }
}
