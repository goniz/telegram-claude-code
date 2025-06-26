use bollard::image::CreateImageOptions;
use bollard::Docker;
use futures_util::StreamExt;
use std::collections::HashMap;
use std::path::Path;
use std::sync::Arc;
use teloxide::{
    dispatching::UpdateFilterExt,
    dptree,
    prelude::*,
    types::{CallbackQuery, InlineKeyboardButton, InlineKeyboardMarkup, ParseMode},
    utils::command::BotCommands,
};
use tokio::sync::Mutex;
use url::Url;

mod commands;

use telegram_bot::claude_code_client::{
    container_utils, AuthState, ClaudeCodeClient,
    GithubClient, GithubClientConfig,
};
use tokio::sync::mpsc;

/// Escape reserved characters for Telegram MarkdownV2 formatting
/// According to Telegram's MarkdownV2 spec, these characters must be escaped:
/// '_', '*', '[', ']', '(', ')', '~', '`', '>', '#', '+', '-', '=', '|', '{', '}', '.', '!'
fn escape_markdown_v2(text: &str) -> String {
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

/// Format GitHub repository list as MarkdownV2 list with hyperlinks
/// Parses the output from `gh repo list` and creates a formatted list with clickable links


// Define the commands that your bot will handle
#[derive(BotCommands, Clone)]
#[command(
    rename_rule = "lowercase",
    description = "These commands are supported:"
)]
enum Command {
    #[command(description = "Display this help message")]
    Help,
    #[command(description = "Start the bot and create a new coding session")]
    Start,
    #[command(description = "Clear the current session (stops and removes container)")]
    ClearSession,
    #[command(description = "Check Claude Code availability")]
    ClaudeStatus,
    #[command(
        description = "Authenticate Claude using your Claude account credentials (OAuth flow)"
    )]
    AuthenticateClaude,
    #[command(description = "Authenticate with GitHub using OAuth flow")]
    GitHubAuth,
    #[command(description = "Check GitHub authentication status")]
    GitHubStatus,
    #[command(description = "List GitHub repositories for the authenticated user")]
    GitHubRepoList,
    #[command(description = "Clone a GitHub repository")]
    GitHubClone(String),
    #[command(description = "Get Claude authentication debug log file")]
    DebugClaudeLogin,
    #[command(description = "Update Claude CLI to latest version")]
    UpdateClaude,
}

// Authentication session state
#[derive(Debug, Clone)]
#[allow(dead_code)]
struct AuthSession {
    container_name: String,
    code_sender: mpsc::UnboundedSender<String>,
}

// Global state for tracking authentication sessions
type AuthSessions = Arc<Mutex<HashMap<i64, AuthSession>>>;

#[derive(Clone)]
struct BotState {
    docker: Docker,
    auth_sessions: AuthSessions,
}

/// Find Claude authentication log file (fixed filename)
async fn find_claude_auth_log_file() -> Option<String> {
    let log_file_path = "/tmp/claude_auth_output.log";

    // Check if the fixed log file exists
    if Path::new(log_file_path).exists() {
        Some(log_file_path.to_string())
    } else {
        None
    }
}

/// Pull the runtime image asynchronously in the background
async fn pull_runtime_image_async(docker: Docker) {
    log::info!(
        "Pulling latest runtime image: {}",
        container_utils::MAIN_CONTAINER_IMAGE
    );
    let create_image_options = CreateImageOptions {
        from_image: container_utils::MAIN_CONTAINER_IMAGE,
        ..Default::default()
    };

    let mut pull_stream = docker.create_image(Some(create_image_options), None, None);
    while let Some(result) = pull_stream.next().await {
        match result {
            Ok(info) => {
                if let Some(status) = &info.status {
                    log::debug!("Image pull progress: {}", status);
                }
            }
            Err(e) => {
                log::warn!("Image pull warning (might already exist): {}", e);
                break; // Continue even if pull fails (image might already exist)
            }
        }
    }
    log::info!("Runtime image pull completed");
}

// Main bot logic
#[tokio::main]
async fn main() {
    pretty_env_logger::init();
    log::info!("Starting Telegram bot...");

    let bot = Bot::from_env();

    // Initialize Docker client
    let docker =
        Docker::connect_with_socket_defaults().expect("Failed to connect to Docker daemon");

    log::info!("Connected to Docker daemon");

    // Clear any existing session containers from previous runs
    match container_utils::clear_all_session_containers(&docker).await {
        Ok(count) => {
            if count > 0 {
                log::info!("Cleared {} existing session containers on startup", count);
            }
        }
        Err(e) => {
            log::warn!("Failed to clear existing session containers: {}", e);
        }
    }

    // Start pulling the latest runtime image in the background
    tokio::spawn(pull_runtime_image_async(docker.clone()));

    // Initialize bot state
    let auth_sessions: AuthSessions = Arc::new(Mutex::new(HashMap::new()));
    let bot_state = BotState {
        docker: docker.clone(),
        auth_sessions: auth_sessions.clone(),
    };

    // Set up message handler that handles both commands and regular text
    let bot_state_clone1 = bot_state.clone();
    let bot_state_clone2 = bot_state.clone();
    let bot_state_clone3 = bot_state.clone();

    let handler = Update::filter_message()
        .branch(
            dptree::entry()
                .filter_command::<Command>()
                .endpoint(move |bot, msg, cmd| {
                    let bot_state = bot_state_clone1.clone();
                    answer(bot, msg, cmd, bot_state)
                }),
        )
        .branch(
            dptree::filter(|msg: Message| msg.text().is_some()).endpoint(move |bot, msg| {
                let bot_state = bot_state_clone2.clone();
                handle_text_message(bot, msg, bot_state)
            }),
        )
        .branch(
            Update::filter_callback_query().endpoint(move |bot, query| {
                let bot_state = bot_state_clone3.clone();
                handle_callback_query(bot, query, bot_state)
            }),
        );

    Dispatcher::builder(bot, handler)
        .enable_ctrlc_handler()
        .build()
        .dispatch()
        .await;
}

// Authentication state monitoring task
async fn handle_auth_state_updates(
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
                let message = format!(
                    "🔐 *Claude Account Authentication*\n\nTo complete authentication with your \
                     Claude account:\n\n*1\\. Click the button below to visit the authentication \
                     URL*\n\n*2\\. Sign in with your Claude account*\n\n*3\\. Complete the OAuth \
                     flow in your browser*\n\n*4\\. If prompted for a code, simply paste it here* \
                     \\(no command needed\\)\n\n✨ This will enable full access to your Claude \
                     subscription features\\!"
                );

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

// Check if authentication session is already in progress
// Handle regular text messages (for authentication codes)
async fn handle_text_message(bot: Bot, msg: Message, bot_state: BotState) -> ResponseResult<()> {
    let chat_id = msg.chat.id.0;

    if let Some(text) = msg.text() {
        // Check if there's an active authentication session
        let session = {
            let sessions = bot_state.auth_sessions.lock().await;
            sessions.get(&chat_id).cloned()
        };

        if let Some(auth_session) = session {
            // Check if the text looks like an authentication code
            if commands::authenticate_claude::is_authentication_code(text) {
                // Send the code to the authentication process
                if let Err(_) = auth_session.code_sender.send(text.to_string()) {
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
                // Not an auth code, inform user about the ongoing session
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
        // If no auth session is active, we don't respond to regular text messages
    }

    Ok(())
}

// Helper function to determine if a text looks like an authentication code


// Handler function for bot commands
async fn answer(bot: Bot, msg: Message, cmd: Command, bot_state: BotState) -> ResponseResult<()> {
    let chat_id = msg.chat.id.0;

    // Use the user ID for volume persistence (same user across different chats)
    let user_id = msg
        .from
        .as_ref()
        .map(|user| user.id.0 as i64)
        .unwrap_or(chat_id);

    match cmd {
        Command::Help => {
            commands::handle_help(bot, msg, bot_state).await?;
        }
        Command::ClearSession => {
            commands::handle_clear_session(bot, msg, bot_state, chat_id).await?;
        }
        Command::Start => {
            commands::handle_start(bot, msg, bot_state, chat_id, user_id).await?;
        }
        Command::ClaudeStatus => {
            commands::handle_claude_status(bot, msg, bot_state, chat_id).await?;
        }
        Command::AuthenticateClaude => {
            commands::handle_claude_authentication(bot, msg, bot_state, chat_id).await?;
        }
        Command::GitHubAuth => {
            commands::handle_github_authentication(bot, msg, bot_state, chat_id).await?;
        }
        Command::GitHubStatus => {
            commands::handle_github_status(bot, msg, bot_state, chat_id).await?;
        }
        Command::GitHubRepoList => {
            commands::handle_github_repo_list(bot, msg, bot_state, chat_id).await?;
        }
        Command::GitHubClone(repository) => {
            let repo_option = if repository.trim().is_empty() {
                None
            } else {
                Some(repository)
            };
            commands::handle_github_clone(bot, msg, bot_state, chat_id, repo_option).await?;
        }
        Command::DebugClaudeLogin => {
            commands::handle_debug_claude_login(bot, msg, chat_id).await?;
        }
        Command::UpdateClaude => {
            commands::handle_update_claude(bot, msg, bot_state, chat_id).await?;
        }
    }

    Ok(())
}

// Handle callback queries from inline keyboard buttons
async fn handle_callback_query(
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

                match ClaudeCodeClient::for_session(bot_state.docker.clone(), &container_name).await {
                    Ok(client) => {
                        let github_client = GithubClient::new(
                            bot_state.docker.clone(),
                            client.container_id().to_string(),
                            GithubClientConfig::default(),
                        );

                        // Perform the clone operation
                        commands::perform_github_clone(&bot, chat_id, &github_client, repository).await?;
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



#[cfg(test)]
mod markdown_v2_tests {
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






