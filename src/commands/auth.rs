use crate::github_client::{GithubClient, GithubClientConfig};
use crate::{escape_markdown_v2, handle_auth_state_updates, AuthSession, BotState};
use telegram_bot::claude_code_client::{AuthenticationHandle, ClaudeCodeClient};
use teloxide::{
    prelude::*,
    types::{InlineKeyboardButton, InlineKeyboardMarkup, ParseMode},
};
use url::Url;

/// Check if a text looks like an authentication code
pub fn is_authentication_code(text: &str) -> bool {
    let text = text.trim();

    // Check length (typical auth codes are 6-128 characters, Claude codes can be ~96 chars)
    if text.len() < 6 || text.len() > 128 {
        return false;
    }

    // Check if it contains only valid characters for auth codes
    let valid_chars = text
        .chars()
        .all(|c| c.is_alphanumeric() || c == '-' || c == '_' || c == '.' || c == '#');

    if !valid_chars {
        return false;
    }

    // Check if it looks like a code (has some structure)
    let alphanumeric_count = text.chars().filter(|c| c.is_alphanumeric()).count();

    if alphanumeric_count < 6 {
        return false;
    }

    // Additional check: if it contains a hash, it should look like a Claude code
    if text.contains('#') {
        let parts: Vec<&str> = text.split('#').collect();
        if parts.len() != 2 {
            return false;
        }
        if parts[0].len() < 20 || parts[1].len() < 20 {
            return false;
        }
    }

    true
}

/// Handle the /auth command
pub async fn handle_auth(
    bot: Bot,
    msg: Message,
    bot_state: BotState,
    chat_id: i64,
    args: Option<String>,
) -> ResponseResult<()> {
    let container_name = format!("coding-session-{}", chat_id);

    match ClaudeCodeClient::for_session(bot_state.docker.clone(), &container_name).await {
        Ok(client) => {
            if let Some(args) = args {
                let arg = args.trim().to_lowercase();
                match arg.as_str() {
                    "login" => {
                        handle_auth_login(bot, msg, bot_state, chat_id, client).await?;
                    }
                    "logout" => {
                        handle_auth_logout(bot, msg, bot_state, chat_id, client).await?;
                    }
                    _ => {
                        bot.send_message(
                            msg.chat.id,
                            "❓ *Invalid argument*\n\n*Usage:*\n• `/auth` \\- Show authentication status\n• `/auth login` \\- Start authentication process\n• `/auth logout` \\- Logout from both services",
                        )
                        .parse_mode(ParseMode::MarkdownV2)
                        .await?;
                    }
                }
            } else {
                handle_auth_status(bot, msg, bot_state, chat_id, client).await?;
            }
        }
        Err(e) => {
            bot.send_message(
                msg.chat.id,
                format!(
                    "❌ No active coding session found: {}\n\nPlease start a coding session first using /start",
                    escape_markdown_v2(&e.to_string())
                ),
            )
            .parse_mode(ParseMode::MarkdownV2)
            .await?;
        }
    }

    Ok(())
}

/// Handle authentication status display
async fn handle_auth_status(
    bot: Bot,
    msg: Message,
    bot_state: BotState,
    _chat_id: i64,
    client: ClaudeCodeClient,
) -> ResponseResult<()> {
    // Check GitHub authentication status
    let github_client = GithubClient::new(
        bot_state.docker.clone(),
        client.container_id().to_string(),
        GithubClientConfig::default(),
    );

    let (github_status, github_authenticated) = match github_client.check_auth_status().await {
        Ok(auth_result) => {
            let status = if auth_result.authenticated {
                if let Some(username) = &auth_result.username {
                    format!(
                        "GitHub Auth: Logged in as {} ✅",
                        escape_markdown_v2(username)
                    )
                } else {
                    "GitHub Auth: Logged in ✅".to_string()
                }
            } else {
                "GitHub Auth: Not logged in ❌".to_string()
            };
            (status, auth_result.authenticated)
        }
        Err(_) => ("GitHub Auth: Status unknown ❓".to_string(), false),
    };

    // Check Claude authentication status
    let (claude_status, claude_authenticated) = match client.check_auth_status().await {
        Ok(authenticated) => {
            let status = if authenticated {
                "Claude Auth: Logged in ✅".to_string()
            } else {
                "Claude Auth: Not logged in ❌".to_string()
            };
            (status, authenticated)
        }
        Err(_) => ("Claude Auth: Status unknown ❓".to_string(), false),
    };

    let message = format!(
        "🔐 *Authentication Status*\n\n{}\n{}",
        github_status, claude_status
    );

    // Create keyboard based on authentication status
    let mut keyboard_buttons = Vec::new();

    if !github_authenticated || !claude_authenticated {
        keyboard_buttons.push(vec![InlineKeyboardButton::callback(
            "🔐 Login to Services",
            "auth_login",
        )]);
    }

    if github_authenticated {
        keyboard_buttons.push(vec![InlineKeyboardButton::callback(
            "📂 List Repositories",
            "github_repo_list",
        )]);
    }

    let reply_markup = if !keyboard_buttons.is_empty() {
        Some(InlineKeyboardMarkup::new(keyboard_buttons))
    } else {
        None
    };

    let mut message_builder = bot
        .send_message(msg.chat.id, message)
        .parse_mode(ParseMode::MarkdownV2);

    if let Some(keyboard) = reply_markup {
        message_builder = message_builder.reply_markup(keyboard);
    }

    message_builder.await?;
    Ok(())
}

/// Handle authentication login process
async fn handle_auth_login(
    bot: Bot,
    msg: Message,
    bot_state: BotState,
    chat_id: i64,
    client: ClaudeCodeClient,
) -> ResponseResult<()> {
    // Check GitHub authentication status first
    let github_client = GithubClient::new(
        bot_state.docker.clone(),
        client.container_id().to_string(),
        GithubClientConfig::default(),
    );

    let github_auth_result = github_client.check_auth_status().await;
    let github_already_authenticated = match &github_auth_result {
        Ok(result) => result.authenticated,
        Err(_) => false,
    };

    // Check Claude authentication status
    let claude_already_authenticated = client.check_auth_status().await.unwrap_or(false);

    let mut status_messages = Vec::new();

    // Handle GitHub authentication
    if github_already_authenticated {
        if let Ok(auth_result) = github_auth_result {
            if let Some(username) = &auth_result.username {
                status_messages.push(format!(
                    "GitHub Auth: Skipped \\- Already logged in as {} ✅",
                    escape_markdown_v2(username)
                ));
            } else {
                status_messages.push("GitHub Auth: Skipped \\- Already logged in ✅".to_string());
            }
        }
    } else {
        match github_client.login().await {
            Ok(auth_result) => {
                if auth_result.authenticated {
                    if let Some(username) = &auth_result.username {
                        status_messages.push(format!(
                            "GitHub Auth: Successful \\- Logged in as {} ✅",
                            escape_markdown_v2(username)
                        ));
                    } else {
                        status_messages.push("GitHub Auth: Successful ✅".to_string());
                    }
                } else if let (Some(oauth_url), Some(device_code)) =
                    (&auth_result.oauth_url, &auth_result.device_code)
                {
                    status_messages
                        .push("GitHub Auth: Waiting for OAuth completion 🔄".to_string());

                    let github_message = format!(
                        "🔗 *GitHub OAuth Authentication Required*\n\n*Please follow these steps:*\n\n1️⃣ *Click the button below to visit the authentication URL*\n\n2️⃣ *Enter this device code:*\n```{}```\n\n3️⃣ *Sign in to your GitHub account* and authorize the application\n\n4️⃣ *Return here* \\- authentication will be completed automatically\n\n⏱️ This code will expire in a few minutes\\.",
                        escape_markdown_v2(device_code)
                    );

                    let github_keyboard = InlineKeyboardMarkup::new(vec![
                        vec![InlineKeyboardButton::url(
                            "🔗 Open GitHub OAuth",
                            Url::parse(oauth_url)
                                .unwrap_or_else(|_| Url::parse("https://github.com").unwrap()),
                        )],
                        vec![InlineKeyboardButton::switch_inline_query_current_chat(
                            "📋 Copy Device Code",
                            device_code,
                        )],
                    ]);

                    bot.send_message(msg.chat.id, github_message)
                        .parse_mode(ParseMode::MarkdownV2)
                        .reply_markup(github_keyboard)
                        .await?;
                } else {
                    status_messages.push(format!(
                        "GitHub Auth: Failed \\- {}",
                        escape_markdown_v2(&auth_result.message)
                    ));
                }
            }
            Err(e) => {
                status_messages.push(format!(
                    "GitHub Auth: Failed \\- {}",
                    escape_markdown_v2(&e.to_string())
                ));
            }
        }
    }

    // Handle Claude authentication
    if claude_already_authenticated {
        status_messages.push("Claude Auth: Skipped \\- Already logged in ✅".to_string());
    } else {
        let sessions = bot_state.auth_sessions.lock().await;
        if sessions.contains_key(&chat_id) {
            status_messages.push("Claude Auth: Already in progress 🔄".to_string());
        } else {
            drop(sessions);

            match client.authenticate_claude_account().await {
                Ok(auth_handle) => {
                    status_messages
                        .push("Claude Auth: Waiting for OAuth completion 🔄".to_string());

                    let AuthenticationHandle {
                        state_receiver,
                        code_sender,
                        cancel_sender,
                    } = auth_handle;

                    let session = AuthSession {
                        container_name: format!("coding-session-{}", chat_id),
                        code_sender: code_sender.clone(),
                        cancel_sender,
                    };

                    {
                        let mut sessions = bot_state.auth_sessions.lock().await;
                        sessions.insert(chat_id, session);
                    }

                    tokio::spawn(handle_auth_state_updates(
                        state_receiver,
                        bot.clone(),
                        msg.chat.id,
                        bot_state.clone(),
                    ));
                }
                Err(e) => {
                    status_messages.push(format!(
                        "Claude Auth: Failed \\- {}",
                        escape_markdown_v2(&e.to_string())
                    ));
                }
            }
        }
    }

    let summary_message = format!(
        "🔐 *Authentication Login Results*\n\n{}",
        status_messages.join("\n")
    );

    bot.send_message(msg.chat.id, summary_message)
        .parse_mode(ParseMode::MarkdownV2)
        .await?;

    Ok(())
}

/// Handle authentication logout process
async fn handle_auth_logout(
    bot: Bot,
    msg: Message,
    bot_state: BotState,
    _chat_id: i64,
    client: ClaudeCodeClient,
) -> ResponseResult<()> {
    // Create GitHub client
    let github_client = GithubClient::new(
        bot_state.docker.clone(),
        client.container_id().to_string(),
        GithubClientConfig::default(),
    );

    let mut status_messages = Vec::new();

    // Handle Claude logout
    match client.logout_claude().await {
        Ok(message) => {
            status_messages.push(format!("Claude: {}", escape_markdown_v2(&message)));
        }
        Err(e) => {
            status_messages.push(format!("Claude: ❌ {}", escape_markdown_v2(&e.to_string())));
        }
    }

    // Handle GitHub logout
    match github_client.logout().await {
        Ok(auth_result) => {
            status_messages.push(format!(
                "GitHub: {}",
                escape_markdown_v2(&auth_result.message)
            ));
        }
        Err(e) => {
            status_messages.push(format!("GitHub: ❌ {}", escape_markdown_v2(&e.to_string())));
        }
    }

    let summary_message = format!("🚪 *Logout Results*\n\n{}", status_messages.join("\n"));

    bot.send_message(msg.chat.id, summary_message)
        .parse_mode(ParseMode::MarkdownV2)
        .await?;

    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_is_authentication_code_valid_codes() {
        assert!(is_authentication_code("abc123def456"));
        assert!(is_authentication_code("auth-code-123"));
        assert!(is_authentication_code("AUTH_CODE_456"));
        assert!(is_authentication_code("a1b2c3d4e5f6"));
        assert!(is_authentication_code("code123"));
        assert!(is_authentication_code("authentication.code.here"));
        assert!(is_authentication_code("ABCDEF123456"));
        assert!(is_authentication_code("auth_token_12345"));

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
        assert!(!is_authentication_code(""));
        assert!(!is_authentication_code("12345"));
        assert!(!is_authentication_code("a"));
        assert!(!is_authentication_code("hello world"));
        assert!(!is_authentication_code("code@123"));
        assert!(!is_authentication_code("code with spaces"));
        assert!(!is_authentication_code("a".repeat(129).as_str()));
        assert!(!is_authentication_code("!@#$%^"));
        assert!(!is_authentication_code("ab123"));

        assert!(!is_authentication_code("short#part"));
        assert!(!is_authentication_code("abc#def#ghi"));
        assert!(!is_authentication_code("this_is_long_enough_but#short"));
        assert!(!is_authentication_code(
            "short#this_is_long_enough_but_first_was_short"
        ));
    }

    #[test]
    fn test_is_authentication_code_edge_cases() {
        assert!(is_authentication_code("      abc123def      "));
        assert!(is_authentication_code("123456"));
        assert!(is_authentication_code("abcdef"));
        assert!(is_authentication_code("a-b-c-1-2-3"));
        assert!(is_authentication_code("a_b_c_1_2_3"));
        assert!(is_authentication_code("a.b.c.1.2.3"));
    }
}
