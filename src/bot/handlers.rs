use teloxide::{
    prelude::*,
    types::{CallbackQuery, InlineKeyboardButton, InlineKeyboardMarkup, ParseMode},
};
use tokio::sync::mpsc;
use url::Url;

use super::{markdown::escape_markdown_v2, state::BotState};
use crate::commands;
use telegram_bot::claude_code_client::{
    AuthState, ClaudeCodeClient, GithubClient, GithubClientConfig,
};

/// Authentication state monitoring task
pub async fn handle_auth_state_updates(
    mut state_receiver: mpsc::UnboundedReceiver<AuthState>,
    bot: Bot,
    chat_id: ChatId,
    bot_state: BotState,
) {
    while let Some(state) = state_receiver.recv().await {
        log::debug!("Received authentication state update: {:?}", state);
        match state {
            AuthState::Starting => {
                let _ = bot
                    .send_message(chat_id, "🔄 Starting Claude authentication\\.\\.\\.")
                    .parse_mode(ParseMode::MarkdownV2)
                    .await;
            }
            AuthState::UrlReady(url) => {
                let message =
                    "🔐 *Claude Account Authentication*\n\nTo complete authentication with your \
                     Claude account:\n\n*1\\. Click the button below to visit the authentication \
                     URL*\n\n*2\\. Sign in with your Claude account*\n\n*3\\. Complete the OAuth \
                     flow in your browser*\n\n*4\\. If prompted for a code, simply paste it here* \
                     \\(no command needed\\)\n\n✨ This will enable full access to your Claude \
                     subscription features\\!"
                        .to_string();

                let keyboard = InlineKeyboardMarkup::new(vec![vec![InlineKeyboardButton::url(
                    "🔗 Open Claude OAuth",
                    Url::parse(&url).unwrap_or_else(|_| Url::parse("https://claude.ai").unwrap()),
                )]]);

                let _ = bot
                    .send_message(chat_id, message)
                    .parse_mode(ParseMode::MarkdownV2)
                    .reply_markup(keyboard)
                    .await;
            }
            AuthState::WaitingForCode => {
                let _ = bot
                    .send_message(
                        chat_id,
                        "🔑 *Authentication code required*\n\nPlease check your browser for an \
                         authentication code and paste it directly into this chat\\. No command \
                         needed\\!",
                    )
                    .parse_mode(ParseMode::MarkdownV2)
                    .await;
            }
            AuthState::Completed(message) => {
                let _ = bot
                    .send_message(chat_id, escape_markdown_v2(&message))
                    .parse_mode(ParseMode::MarkdownV2)
                    .await;
                // Clean up the session
                {
                    let mut sessions = bot_state.auth_sessions.lock().await;
                    sessions.remove(&chat_id.0);
                }
                break;
            }
            AuthState::Failed(error) => {
                let _ = bot
                    .send_message(
                        chat_id,
                        format!("❌ Authentication failed: {}", escape_markdown_v2(&error)),
                    )
                    .parse_mode(ParseMode::MarkdownV2)
                    .await;
                // Clean up the session
                {
                    let mut sessions = bot_state.auth_sessions.lock().await;
                    sessions.remove(&chat_id.0);
                }
                break;
            }
        }
    }

    // Log when the state_receiver is closed
    log::warn!(
        "Authentication state receiver closed for chat_id: {}",
        chat_id.0
    );

    // Clean up the session if it still exists
    {
        let mut sessions = bot_state.auth_sessions.lock().await;
        sessions.remove(&chat_id.0);
    }
}

/// Handle regular text messages (for authentication codes)
pub async fn handle_text_message(
    bot: Bot,
    msg: Message,
    bot_state: BotState,
) -> ResponseResult<()> {
    let chat_id = msg.chat.id.0;

    if let Some(text) = msg.text() {
        // Check if there's an active authentication session and get the code_sender if it exists
        let code_sender_clone = {
            let sessions = bot_state.auth_sessions.lock().await;
            sessions.get(&chat_id).map(|s| s.code_sender.clone())
        };

        if let Some(code_sender) = code_sender_clone {
            // An auth session exists for this chat_id
            if commands::authenticate_claude::is_authentication_code(text) {
                // Send the code to the authentication process
                if code_sender.send(text.to_string()).is_err() {
                    bot.send_message(
                        msg.chat.id,
                        "❌ Failed to send authentication code\\. The authentication session may \
                         have expired\\.\n\nPlease restart authentication with \
                         `/authenticateclaude`",
                    )
                    .parse_mode(ParseMode::MarkdownV2)
                    .await?;
                } else {
                    bot.send_message(
                        msg.chat.id,
                        "✅ Authentication code received\\! Please wait while we complete the \
                         authentication process\\.\\.\\.",
                    )
                    .parse_mode(ParseMode::MarkdownV2)
                    .await?;
                }
            } else {
                // Not an auth code, but an auth session is active. Inform user.
                bot.send_message(
                    msg.chat.id,
                    "🔐 *Authentication in Progress*\n\nI'm currently waiting for your \
                     authentication code\\. Please paste the code you received during the OAuth \
                     flow\\.\n\nIf you need to restart authentication, use `/authenticateclaude`",
                )
                .parse_mode(ParseMode::MarkdownV2)
                .await?;
            }
        }
        // If code_sender_clone was None, it means no auth session is active for this user.
        // In this case, we don't respond to regular text messages, so no 'else' block needed here.
    }

    Ok(())
}

/// Handle callback queries from inline keyboard buttons
pub async fn handle_callback_query(
    bot: Bot,
    query: CallbackQuery,
    bot_state: BotState,
) -> ResponseResult<()> {
    if let Some(data) = &query.data {
        if data.starts_with("clone:") {
            // Extract repository name from callback data
            let repository = data.strip_prefix("clone:").unwrap_or("");

            if let Some(message) = &query.message {
                // Handle both accessible and inaccessible messages
                let chat_id = message.chat().id;
                let container_name = format!("coding-session-{}", chat_id.0);

                // Answer the callback query to remove the loading state
                bot.answer_callback_query(&query.id).await?;

                match ClaudeCodeClient::for_session(bot_state.docker.clone(), &container_name).await
                {
                    Ok(client) => {
                        let github_client = GithubClient::new(
                            bot_state.docker.clone(),
                            client.container_id().to_string(),
                            GithubClientConfig::default(),
                        );

                        // Perform the clone operation
                        commands::perform_github_clone(&bot, chat_id, &github_client, repository)
                            .await?;
                    }
                    Err(e) => {
                        bot.send_message(
                            chat_id,
                            format!(
                                "❌ No active coding session found: {}\\n\\nPlease start a coding session \
                                 first using /start",
                                escape_markdown_v2(&e.to_string())
                            ),
                        )
                        .parse_mode(ParseMode::MarkdownV2)
                        .await?;
                    }
                }
            }
        } else {
            // Unknown callback data, just answer the query
            bot.answer_callback_query(&query.id).await?;
        }
    } else {
        // No callback data, just answer the query
        bot.answer_callback_query(&query.id).await?;
    }

    Ok(())
}
