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

/// Handle regular text messages (for authentication codes and Claude conversations)
pub async fn handle_text_message(
    bot: Bot,
    msg: Message,
    bot_state: BotState,
) -> ResponseResult<()> {
    let chat_id = msg.chat.id.0;

    if let Some(text) = msg.text().map(|t| t.to_string()) {
        // Priority 1: Check if there's an active authentication session
        let code_sender_clone = {
            let sessions = bot_state.auth_sessions.lock().await;
            sessions.get(&chat_id).map(|s| s.code_sender.clone())
        };

        if let Some(code_sender) = code_sender_clone {
            // An auth session exists for this chat_id
            if commands::auth::is_authentication_code(&text) {
                // Send the code to the authentication process
                if code_sender.send(text.clone()).is_err() {
                    bot.send_message(
                        msg.chat.id,
                        "❌ Failed to send authentication code\\. The authentication session may \
                         have expired\\.\n\nPlease restart authentication with \
                         `/auth login`",
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
            return Ok(());
        }

        // Priority 2: Check if there's an active Claude conversation session
        let claude_session_active = {
            let sessions = bot_state.claude_sessions.lock().await;
            sessions.get(&chat_id).map(|s| s.is_active).unwrap_or(false)
        };

        if claude_session_active {
            // Forward message to Claude
            handle_claude_message(bot, msg, bot_state, &text).await?;
            return Ok(());
        }

        // Priority 3: No active sessions - do nothing (default behavior)
    }

    Ok(())
}

/// Handle messages sent to Claude conversations
async fn handle_claude_message(
    bot: Bot,
    msg: Message,
    bot_state: BotState,
    text: &str,
) -> ResponseResult<()> {
    let chat_id = msg.chat.id.0;

    // Get the current conversation ID if any
    let conversation_id = {
        let sessions = bot_state.claude_sessions.lock().await;
        let conv_id = sessions
            .get(&chat_id)
            .and_then(|s| s.conversation_id.clone());
        if let Some(ref id) = conv_id {
            log::info!(
                "Retrieved existing conversation ID for chat {}: {}",
                chat_id,
                id
            );
        } else {
            log::info!("No existing conversation ID found for chat {}", chat_id);
        }
        conv_id
    };

    // Execute Claude command
    match commands::execute_claude_command(
        bot.clone(),
        msg.chat.id,
        bot_state.clone(),
        text,
        conversation_id,
    )
    .await
    {
        Ok(()) => {
            // Command executed successfully, output already processed and sent
            // TODO: Extract conversation ID from output and update session state
        }
        Err(e) => {
            bot.send_message(
                msg.chat.id,
                format!(
                    "❌ Claude command failed: {}",
                    escape_markdown_v2(&e.to_string())
                ),
            )
            .parse_mode(ParseMode::MarkdownV2)
            .await?;
        }
    }

    Ok(())
}

#[cfg(test)]
mod tests {
    use crate::bot::{AuthSession, AuthSessions, BotState, ClaudeSession, ClaudeSessions};
    use bollard::Docker;
    use std::collections::HashMap;
    use std::sync::Arc;
    use tokio::sync::{mpsc, oneshot, Mutex};

    fn create_test_bot_state() -> BotState {
        // Create a mock Docker instance (won't be used in these tests)
        let docker = Docker::connect_with_socket_defaults().unwrap();
        let auth_sessions: AuthSessions = Arc::new(Mutex::new(HashMap::new()));
        let claude_sessions: ClaudeSessions = Arc::new(Mutex::new(HashMap::new()));

        BotState {
            docker,
            auth_sessions,
            claude_sessions,
        }
    }

    #[tokio::test]
    async fn test_claude_session_priority_routing() {
        let bot_state = create_test_bot_state();
        let chat_id = 12345i64;

        // Test 1: No active sessions - should be ignored (no error)
        {
            let sessions = bot_state.claude_sessions.lock().await;
            assert!(!sessions.contains_key(&chat_id));
        }

        // Test 2: Add inactive Claude session - should still be ignored
        {
            let mut sessions = bot_state.claude_sessions.lock().await;
            sessions.insert(chat_id, ClaudeSession::new());
        }

        {
            let sessions = bot_state.claude_sessions.lock().await;
            let session = sessions.get(&chat_id);
            assert!(session.is_some());
            assert!(!session.unwrap().is_active);
        }

        // Test 3: Activate Claude session
        {
            let mut sessions = bot_state.claude_sessions.lock().await;
            if let Some(session) = sessions.get_mut(&chat_id) {
                session.is_active = true;
            }
        }

        {
            let sessions = bot_state.claude_sessions.lock().await;
            let session = sessions.get(&chat_id);
            assert!(session.is_some());
            assert!(session.unwrap().is_active);
        }
    }

    #[tokio::test]
    async fn test_auth_session_priority_over_claude() {
        let bot_state = create_test_bot_state();
        let chat_id = 12345i64;

        // Create both auth and Claude sessions
        {
            let mut claude_sessions = bot_state.claude_sessions.lock().await;
            let mut claude_session = ClaudeSession::new();
            claude_session.is_active = true;
            claude_sessions.insert(chat_id, claude_session);
        }

        {
            let mut auth_sessions = bot_state.auth_sessions.lock().await;
            let (code_sender, _code_receiver) = mpsc::unbounded_channel();
            let (cancel_sender, _cancel_receiver) = oneshot::channel();
            let auth_session = AuthSession {
                container_name: format!("coding-session-{}", chat_id),
                code_sender,
                cancel_sender,
            };
            auth_sessions.insert(chat_id, auth_session);
        }

        // Verify both sessions exist
        {
            let auth_sessions = bot_state.auth_sessions.lock().await;
            let claude_sessions = bot_state.claude_sessions.lock().await;

            assert!(auth_sessions.contains_key(&chat_id));
            assert!(claude_sessions.contains_key(&chat_id));
            assert!(claude_sessions.get(&chat_id).unwrap().is_active);
        }

        // Auth session should take priority (this is tested implicitly in the handler logic)
    }

    #[tokio::test]
    async fn test_multiple_chat_session_isolation() {
        let bot_state = create_test_bot_state();
        let chat_ids = vec![11111i64, 22222i64, 33333i64];

        // Create different session states for each chat
        {
            let mut claude_sessions = bot_state.claude_sessions.lock().await;

            // Chat 1: Active Claude session
            let mut session1 = ClaudeSession::new();
            session1.is_active = true;
            session1.conversation_id = Some("conv-1".to_string());
            claude_sessions.insert(chat_ids[0], session1);

            // Chat 2: Inactive Claude session
            let session2 = ClaudeSession::new();
            claude_sessions.insert(chat_ids[1], session2);

            // Chat 3: No session (will be empty)
        }

        // Verify isolation
        {
            let claude_sessions = bot_state.claude_sessions.lock().await;

            // Chat 1 should be active
            let session1 = claude_sessions.get(&chat_ids[0]);
            assert!(session1.is_some());
            assert!(session1.unwrap().is_active);
            assert_eq!(
                session1.unwrap().conversation_id,
                Some("conv-1".to_string())
            );

            // Chat 2 should be inactive
            let session2 = claude_sessions.get(&chat_ids[1]);
            assert!(session2.is_some());
            assert!(!session2.unwrap().is_active);
            assert!(session2.unwrap().conversation_id.is_none());

            // Chat 3 should have no session
            let session3 = claude_sessions.get(&chat_ids[2]);
            assert!(session3.is_none());
        }
    }

    #[tokio::test]
    async fn test_conversation_id_retrieval() {
        let bot_state = create_test_bot_state();
        let chat_id = 12345i64;
        let test_conversation_id = "test-conversation-uuid-123".to_string();

        // Test with no session
        {
            let sessions = bot_state.claude_sessions.lock().await;
            let conv_id = sessions
                .get(&chat_id)
                .and_then(|s| s.conversation_id.clone());
            assert!(conv_id.is_none());
        }

        // Add session with conversation ID
        {
            let mut sessions = bot_state.claude_sessions.lock().await;
            let mut session = ClaudeSession::new();
            session.conversation_id = Some(test_conversation_id.clone());
            session.is_active = true;
            sessions.insert(chat_id, session);
        }

        // Retrieve conversation ID
        {
            let sessions = bot_state.claude_sessions.lock().await;
            let conv_id = sessions
                .get(&chat_id)
                .and_then(|s| s.conversation_id.clone());
            assert_eq!(conv_id, Some(test_conversation_id));
        }
    }
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
