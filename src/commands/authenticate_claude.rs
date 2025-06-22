use teloxide::{prelude::*, types::{ChatId, ParseMode}};
use crate::{
    escape_markdown_v2, BotState, AuthSession,
    claude_code_client::{ClaudeCodeClient, AuthenticationHandle},
    handle_auth_state_updates
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
                        cancel_sender: _cancel_sender,
                    } = auth_handle;

                    // Store authentication session
                    let session = AuthSession {
                        container_name: container_name.clone(),
                        code_sender: code_sender.clone(),
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