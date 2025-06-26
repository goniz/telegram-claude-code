use crate::{AuthSession, BotState, escape_markdown_v2, handle_auth_state_updates};
use telegram_bot::claude_code_client::{AuthenticationHandle, ClaudeCodeClient};
use teloxide::{
    prelude::*,
    types::{ChatId, ParseMode},
};

/// Check if there's an existing authentication session
async fn check_existing_auth_session(
    bot: &Bot,
    chat_id: i64,
    msg_chat_id: ChatId,
    bot_state: &BotState,
) -> ResponseResult<bool> {
    let sessions = bot_state.auth_sessions.lock().await;
    if sessions.contains_key(&chat_id) {
        bot.send_message(
            msg_chat_id,
            "🔐 *Authentication Already in Progress*\n\nYou have an ongoing authentication \
             session\\.\n\nIf you need to provide a code, use `/authcode <your_code>`\n\nTo \
             restart authentication, please wait for the current session to complete or fail\\.",
        )
        .parse_mode(ParseMode::MarkdownV2)
        .await?;
        return Ok(true);
    }
    Ok(false)
}

/// Handle the /authenticateclaude command
pub async fn handle_claude_authentication(
    bot: Bot,
    msg: Message,
    bot_state: BotState,
    chat_id: i64,
) -> ResponseResult<()> {
    let container_name = format!("coding-session-{}", chat_id);

    match ClaudeCodeClient::for_session(bot_state.docker.clone(), &container_name).await {
        Ok(client) => {
            // Check if there's already an authentication session in progress
            if check_existing_auth_session(&bot, chat_id, msg.chat.id, &bot_state).await? {
                return Ok(());
            }

            // Send initial message
            bot.send_message(
                msg.chat.id,
                "🔐 Starting Claude account authentication process\\.\\.\\.\n\n⏳ Initiating \
                 OAuth flow\\.\\.\\.",
            )
            .parse_mode(ParseMode::MarkdownV2)
            .await?;

            match client.authenticate_claude_account().await {
                Ok(auth_handle) => {
                    // Extract channels from the handle
                    let AuthenticationHandle {
                        state_receiver,
                        code_sender,
                        cancel_sender, // No longer prefixed with _
                    } = auth_handle;

                    // Store authentication session
                    let session = AuthSession {
                        container_name: container_name.clone(),
                        code_sender: code_sender.clone(),
                        cancel_sender, // Store the cancel_sender
                    };

                    {
                        let mut sessions = bot_state.auth_sessions.lock().await;
                        sessions.insert(chat_id, session);
                    }

                    // Spawn a task to handle authentication state updates
                    tokio::spawn(handle_auth_state_updates(
                        state_receiver,
                        bot.clone(),
                        msg.chat.id,
                        bot_state.clone(),
                    ));
                }
                Err(e) => {
                    bot.send_message(
                        msg.chat.id,
                        format!(
                            "❌ Failed to initiate Claude account authentication: {}\n\nPlease \
                             ensure:\n• Your coding session is active\n• Claude Code is properly \
                             installed\n• Network connectivity is available",
                            escape_markdown_v2(&e.to_string())
                        ),
                    )
                    .parse_mode(ParseMode::MarkdownV2)
                    .await?;
                }
            }
        }
        Err(e) => {
            bot.send_message(
                msg.chat.id,
                format!(
                    "❌ No active coding session found: {}\n\nPlease start a coding session first \
                     using /start",
                    escape_markdown_v2(&e.to_string())
                ),
            )
            .parse_mode(ParseMode::MarkdownV2)
            .await?;
        }
    }

    Ok(())
}

/// Check if a text looks like an authentication code
pub fn is_authentication_code(text: &str) -> bool {
    let text = text.trim();

    // Common patterns for authentication codes:
    // - Claude codes: long alphanumeric with _, -, # (e.g., 'yHNxk8SH0fw861QGEXP80UeTIzJUbSg6BDQWvtN80ecoOGAf#ybFaWRHX0Y5YdJaM9ET8_06if-w9Rwg0X-4lEMdyT7I')
    // - Other service codes: shorter alphanumeric with dashes or underscores
    // - Hexadecimal-looking codes
    // - Base64-looking codes

    // Check length (typical auth codes are 6-128 characters, Claude codes can be ~96 chars)
    if text.len() < 6 || text.len() > 128 {
        return false;
    }

    // Check if it contains only valid characters for auth codes
    // Allow alphanumeric, dashes, underscores, dots, and hash (for Claude codes)
    let valid_chars = text
        .chars()
        .all(|c| c.is_alphanumeric() || c == '-' || c == '_' || c == '.' || c == '#');

    if !valid_chars {
        return false;
    }

    // Check if it looks like a code (has some structure)
    // At least 6 alphanumeric characters
    let alphanumeric_count = text.chars().filter(|c| c.is_alphanumeric()).count();

    if alphanumeric_count < 6 {
        return false;
    }

    // Additional check: if it contains a hash, it should look like a Claude code
    // Claude codes have the pattern: base64-like#base64-like
    if text.contains('#') {
        let parts: Vec<&str> = text.split('#').collect();
        if parts.len() != 2 {
            return false; // Should have exactly one # dividing two parts
        }
        // Both parts should be substantial (at least 20 chars each for Claude codes)
        if parts[0].len() < 20 || parts[1].len() < 20 {
            return false;
        }
    }

    true
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_is_authentication_code_valid_codes() {
        // Test valid authentication codes
        assert!(is_authentication_code("abc123def456"));
        assert!(is_authentication_code("auth-code-123"));
        assert!(is_authentication_code("AUTH_CODE_456"));
        assert!(is_authentication_code("a1b2c3d4e5f6"));
        assert!(is_authentication_code("code123"));
        assert!(is_authentication_code("authentication.code.here"));
        assert!(is_authentication_code("ABCDEF123456"));
        assert!(is_authentication_code("auth_token_12345"));

        // Test Claude authentication code format
        assert!(is_authentication_code(
            "yHNxk8SH0fw861QGEXP80UeTIzJUbSg6BDQWvtN80ecoOGAf#\
             ybFaWRHX0Y5YdJaM9ET8_06if-w9Rwg0X-4lEMdyT7I"
        ));
        assert!(is_authentication_code(
            "abcd1234567890abcd1234567890#efgh5678901234efgh5678901234_code-part"
        ));
        assert!(is_authentication_code(
            "long_part_with_underscores_123#another_long_part_with_more_data_456"
        ));
    }

    #[test]
    fn test_is_authentication_code_invalid_codes() {
        // Test invalid authentication codes
        assert!(!is_authentication_code(""));
        assert!(!is_authentication_code("12345")); // Too short
        assert!(!is_authentication_code("a")); // Too short
        assert!(!is_authentication_code("hello world")); // Contains space
        assert!(!is_authentication_code("code@123")); // Contains @
        assert!(!is_authentication_code("code with spaces")); // Contains spaces
        assert!(!is_authentication_code("a".repeat(129).as_str())); // Too long
        assert!(!is_authentication_code("!@#$%^")); // Only special chars
        assert!(!is_authentication_code("ab123")); // Less than 6 alphanumeric

        // Test invalid Claude code formats
        assert!(!is_authentication_code("short#part")); // Parts too short for Claude code
        assert!(!is_authentication_code("abc#def#ghi")); // Multiple hash symbols
        assert!(!is_authentication_code("this_is_long_enough_but#short")); // Second part too short
        assert!(!is_authentication_code(
            "short#this_is_long_enough_but_first_was_short"
        )); // First part too short
    }

    #[test]
    fn test_is_authentication_code_edge_cases() {
        // Test edge cases
        assert!(is_authentication_code("      abc123def      ")); // With whitespace (trimmed)
        assert!(is_authentication_code("123456")); // All numeric, minimum length
        assert!(is_authentication_code("abcdef")); // All letters, minimum length
        assert!(is_authentication_code("a-b-c-1-2-3")); // With dashes
        assert!(is_authentication_code("a_b_c_1_2_3")); // With underscores
        assert!(is_authentication_code("a.b.c.1.2.3")); // With dots
    }
}
