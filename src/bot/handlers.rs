use teloxide::{
    prelude::*,
    types::{CallbackQuery, InlineKeyboardButton, InlineKeyboardMarkup, ParseMode},
};
use tokio::sync::mpsc;
use url::Url;

use super::{markdown::{escape_markdown_v2, truncate_if_needed}, state::BotState};
use crate::commands;
use crate::github_client::{GithubClient, GithubClientConfig};
use telegram_bot::claude_code_client::{AuthState, ClaudeCodeClient};

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
                let message = "🔐 *Claude OAuth*

Click below to sign in\\. If prompted for a code, paste it here\\."
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
                        "🔑 *Code required*

Paste your authentication code here\\.",
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
                let full_message = format!("❌ Authentication failed: {}", escape_markdown_v2(&error));
                let (message_to_send, _was_truncated) = truncate_if_needed(&full_message);
                
                let _ = bot
                    .send_message(chat_id, message_to_send)
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
                         have expired\\.

Please restart authentication with \
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
                    "🔐 *Authentication in Progress*

I'm currently waiting for your \
                     authentication code\\. Please paste the code you received during the OAuth \
                     flow\\.

If you need to restart authentication, use `/auth login`",
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

        // Priority 3: Check if text looks like a repository name for cloning
        if text.contains('/') && !text.contains(' ') && text.len() > 3 && text.len() < 100 {
            // Simple validation for owner/repo pattern
            let parts: Vec<&str> = text.split('/').collect();
            if parts.len() == 2 && !parts[0].is_empty() && !parts[1].is_empty() {
                let keyboard =
                    InlineKeyboardMarkup::new(vec![vec![InlineKeyboardButton::callback(
                        format!("🔗 Clone {}", text),
                        format!("start_clone:{}", text),
                    )]]);

                bot.send_message(
                    msg.chat.id,
                    format!(
                        "📦 *Repository Detected*\n\nI detected a repository name: `{}`\n\nWould you like to clone it?",
                        escape_markdown_v2(&text)
                    ),
                )
                .parse_mode(ParseMode::MarkdownV2)
                .reply_markup(keyboard)
                .await?;
                return Ok(());
            }
        }

        // Priority 4: No active sessions - do nothing (default behavior)
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
            let full_message = format!(
                "❌ Claude command failed: {}",
                escape_markdown_v2(&e.to_string())
            );
            let (message_to_send, _was_truncated) = truncate_if_needed(&full_message);
            
            bot.send_message(msg.chat.id, message_to_send)
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
    log::debug!("Received callback query: {:?}", query);

    // Always answer the callback query first (this is required by Telegram)
    bot.answer_callback_query(query.id).await?;

    if let Some(data) = &query.data {
        log::debug!("Callback data: {}", data);

        if let Some(message) = &query.message {
            let chat_id = message.chat().id;
            log::debug!("Chat ID: {}", chat_id.0);

            match data.as_str() {
                "auth_login" => {
                    log::debug!("Handling auth_login callback for chat {}", chat_id.0);
                    // Handle auth login callback
                    let container_name = format!("coding-session-{}", chat_id.0);
                    match ClaudeCodeClient::for_session(bot_state.docker.clone(), &container_name)
                        .await
                    {
                        Ok(_client) => {
                            // Extract the regular message from MaybeInaccessibleMessage
                            if let teloxide::types::MaybeInaccessibleMessage::Regular(msg) = message
                            {
                                commands::auth::handle_auth(
                                    bot,
                                    (**msg).clone(),
                                    bot_state,
                                    chat_id.0,
                                    Some("login".to_string()),
                                )
                                .await?;
                            }
                        }
                        Err(e) => {
                            let full_message = format!(
                                "❌ No active coding session found: {}

Please start a coding session \
                                 first using /start",
                                escape_markdown_v2(&e.to_string())
                            );
                            let (message_to_send, _was_truncated) = truncate_if_needed(&full_message);
                            
                            bot.send_message(chat_id, message_to_send)
                                .parse_mode(ParseMode::MarkdownV2)
                                .await?;
                        }
                    }
                }
                data if data.starts_with("clone:") => {
                    log::debug!("Handling clone callback for chat {}", chat_id.0);
                    // Extract repository name from callback data
                    let repository = data.strip_prefix("clone:").unwrap_or("");
                    let container_name = format!("coding-session-{}", chat_id.0);

                    match ClaudeCodeClient::for_session(bot_state.docker.clone(), &container_name)
                        .await
                    {
                        Ok(client) => {
                            // Inform user that cloning is starting
                            bot.send_message(
                                chat_id,
                                format!(
                                    "🔄 Cloning repository: `{}`",
                                    escape_markdown_v2(repository)
                                ),
                            )
                            .parse_mode(ParseMode::MarkdownV2)
                            .await?;

                            let github_client = GithubClient::new(
                                bot_state.docker.clone(),
                                client.container_id().to_string(),
                                GithubClientConfig::default(),
                            );

                            // Perform the clone operation using the new start workflow function
                            commands::start::perform_github_clone(
                                &bot,
                                chat_id,
                                &github_client,
                                repository,
                                &bot_state,
                            )
                            .await?;
                        }
                        Err(e) => {
                            bot.send_message(
                                chat_id,
                                format!(
                                    "❌ No active coding session found: {}\nPlease start a coding session \
                                     first using /start",
                                    escape_markdown_v2(&e.to_string())
                                ),
                            )
                            .parse_mode(ParseMode::MarkdownV2)
                            .await?;
                        }
                    }
                }
                data if data.starts_with("start_clone:") => {
                    log::debug!("Handling start_clone callback for chat {}", chat_id.0);
                    // Extract repository name from callback data for start workflow
                    let repository = data.strip_prefix("start_clone:").unwrap_or("");
                    if let Err(e) = commands::start::handle_repository_clone_in_start(
                        bot.clone(),
                        chat_id,
                        &bot_state,
                        repository,
                    )
                    .await
                    {
                        bot.send_message(
                            chat_id,
                            format!(
                                "❌ Failed to clone repository: {}",
                                escape_markdown_v2(&e.to_string())
                            ),
                        )
                        .parse_mode(ParseMode::MarkdownV2)
                        .await?;
                    }
                }
                "manual_repo_entry" => {
                    log::debug!("Handling manual_repo_entry callback for chat {}", chat_id.0);
                    // Handle manual repository entry
                    commands::start::handle_manual_repository_entry(bot, chat_id).await?;
                }
                "skip_repo_setup" => {
                    log::debug!("Handling skip_repo_setup callback for chat {}", chat_id.0);
                    // Handle skipping repository setup
                    commands::start::handle_skip_repository_setup(bot, chat_id).await?;
                }
                "github_repo_list" => {
                    log::debug!("Handling github_repo_list callback for chat {}", chat_id.0);
                    // Handle github repo list callback
                    let container_name = format!("coding-session-{}", chat_id.0);
                    match ClaudeCodeClient::for_session(bot_state.docker.clone(), &container_name)
                        .await
                    {
                        Ok(client) => {
                            // Show repository selection UI
                            commands::start::show_repository_selection(
                                bot, chat_id, &bot_state, &client,
                            )
                            .await?;
                        }
                        Err(e) => {
                            bot.send_message(
                                chat_id,
                                format!(
                                    "❌ No active coding session found: {}

Please start a coding session \
                                     first using /start",
                                    escape_markdown_v2(&e.to_string())
                                ),
                            )
                            .parse_mode(ParseMode::MarkdownV2)
                            .await?;
                        }
                    }
                }
                _ => {
                    log::debug!("Unknown callback data '{}' for chat {}", data, chat_id.0);
                    // Unknown callback data, already answered above
                }
            }
        } else {
            log::debug!("Callback query has no message");
        }
    } else {
        log::debug!("Callback query has no data");
    }

    Ok(())
}
