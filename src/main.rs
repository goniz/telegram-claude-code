use bollard::image::CreateImageOptions;
use bollard::Docker;
use futures_util::StreamExt;
use std::collections::HashMap;
use std::sync::Arc;
use std::path::Path;
use teloxide::{prelude::*, utils::command::BotCommands, types::{ParseMode, InlineKeyboardMarkup, InlineKeyboardButton, InputFile}, dispatching::UpdateFilterExt, dptree};
use tokio::sync::Mutex;
use url::Url;

mod claude_code_client;
use claude_code_client::{container_utils, ClaudeCodeClient, ClaudeCodeConfig, GithubClient, GithubClientConfig, AuthenticationHandle, AuthState};
use tokio::sync::mpsc;

/// Escape reserved characters for Telegram MarkdownV2 formatting
/// According to Telegram's MarkdownV2 spec, these characters must be escaped:
/// '_', '*', '[', ']', '(', ')', '~', '`', '>', '#', '+', '-', '=', '|', '{', '}', '.', '!'
fn escape_markdown_v2(text: &str) -> String {
    text.chars()
        .map(|c| match c {
            '_' | '*' | '[' | ']' | '(' | ')' | '~' | '`' | '>' | '#' | '+' | '-' | '=' | '|' | '{' | '}' | '.' | '!' => {
                format!("\\{}", c)
            }
            _ => c.to_string(),
        })
        .collect()
}

/// Format GitHub repository list as MarkdownV2 list with hyperlinks
/// Parses the output from `gh repo list` and creates a formatted list with clickable links
fn format_repo_list_markdown_v2(repo_list: &str) -> String {
    let lines: Vec<&str> = repo_list.trim().lines().collect();
    if lines.is_empty() {
        return "💡 No repositories found or no repositories accessible with current authentication\\.".to_string();
    }

    let mut formatted_repos = Vec::new();
    
    for line in lines {
        let line = line.trim();
        if line.is_empty() {
            continue;
        }
        
        // gh repo list output format is typically: "repo_name    description"
        // Split by whitespace and take the first part as the repo name
        let parts: Vec<&str> = line.split_whitespace().collect();
        if let Some(repo_name) = parts.first() {
            // Create GitHub URL for the repository
            let github_url = format!("https://github.com/{}", repo_name);
            
            // Extract description (everything after the first whitespace-separated token)
            let description = if parts.len() > 1 {
                parts[1..].join(" ")
            } else {
                String::new()
            };
            
            // Create MarkdownV2 list item with hyperlink
            // Format: • [repo_name](https://github.com/repo_name) - description
            let escaped_repo_name = escape_markdown_v2(repo_name);
            let escaped_description = if !description.is_empty() {
                format!(" \\- {}", escape_markdown_v2(&description))
            } else {
                String::new()
            };
            
            let formatted_item = format!(
                "• [{}]({}){}", 
                escaped_repo_name, 
                github_url, 
                escaped_description
            );
            formatted_repos.push(formatted_item);
        }
    }
    
    if formatted_repos.is_empty() {
        "💡 No repositories found or no repositories accessible with current authentication\\.".to_string()
    } else {
        formatted_repos.join("\n")
    }
}

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
    #[command(description = "Get Claude authentication debug log file")]
    DebugClaudeLogin,
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

/// Find Claude authentication log files for a specific chat ID
async fn find_claude_auth_log_file(chat_id: i64) -> Option<String> {
    let log_pattern = format!("/tmp/claude_auth_output_{}.log", chat_id);
    
    // Check if the specific log file exists
    if Path::new(&log_pattern).exists() {
        return Some(log_pattern);
    }
    
    // If specific file doesn't exist, look for any recent Claude auth log files
    match tokio::fs::read_dir("/tmp").await {
        Ok(mut entries) => {
            let mut claude_files = Vec::new();
            
            while let Ok(Some(entry)) = entries.next_entry().await {
                if let Some(file_name) = entry.file_name().to_str() {
                    if file_name.starts_with("claude_auth_output_") && file_name.ends_with(".log") {
                        if let Ok(metadata) = entry.metadata().await {
                            claude_files.push((entry.path(), metadata.modified().unwrap_or(std::time::SystemTime::UNIX_EPOCH)));
                        }
                    }
                }
            }
            
            // Sort by modification time (newest first)
            claude_files.sort_by(|a, b| b.1.cmp(&a.1));
            
            // Return the most recent file if any exist
            claude_files.first().map(|(path, _)| path.to_string_lossy().to_string())
        }
        Err(_) => None
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
    
    let handler = Update::filter_message()
        .branch(
            dptree::entry()
                .filter_command::<Command>()
                .endpoint(move |bot, msg, cmd| {
                    let bot_state = bot_state_clone1.clone();
                    answer(bot, msg, cmd, bot_state)
                })
        )
        .branch(
            dptree::filter(|msg: Message| msg.text().is_some())
                .endpoint(move |bot, msg| {
                    let bot_state = bot_state_clone2.clone();
                    handle_text_message(bot, msg, bot_state)
                })
        );

    Dispatcher::builder(bot, handler)
        .enable_ctrlc_handler()
        .build()
        .dispatch()
        .await;
}

// Generate help text in BotFather format from command descriptions
fn generate_help_text() -> String {
    let descriptions = Command::descriptions().to_string();
    
    // Parse the default descriptions and convert to BotFather format
    descriptions
        .lines()
        .skip(2) // Skip the header "These commands are supported:" and empty line
        .map(|line| {
            // Remove the leading slash and replace em dash with hyphen
            line.trim()
                .strip_prefix('/')
                .unwrap_or(line.trim())
                .replace(" — ", " - ")
        })
        .collect::<Vec<String>>()
        .join("\n")
}

// Authentication state monitoring task
async fn handle_auth_state_updates(
    mut state_receiver: mpsc::UnboundedReceiver<AuthState>,
    bot: Bot,
    chat_id: ChatId,
    bot_state: BotState,
) {
    while let Some(state) = state_receiver.recv().await {
        match state {
            AuthState::Starting => {
                let _ = bot.send_message(
                    chat_id,
                    "🔄 Starting Claude authentication\\.\\.\\."
                )
                .parse_mode(ParseMode::MarkdownV2)
                .await;
            }
            AuthState::UrlReady(url) => {
                let message = format!(
                    "🔐 *Claude Account Authentication*\n\nTo complete authentication with your Claude account:\n\n*1\\. Click the button below to visit the authentication URL*\n\n*2\\. Sign in with your Claude account*\n\n*3\\. Complete the OAuth flow in your browser*\n\n*4\\. If prompted for a code, simply paste it here* \\(no command needed\\)\n\n✨ This will enable full access to your Claude subscription features\\!"
                );
                
                let keyboard = InlineKeyboardMarkup::new(vec![
                    vec![InlineKeyboardButton::url("🔗 Open Claude OAuth", Url::parse(&url).unwrap_or_else(|_| Url::parse("https://claude.ai").unwrap()))],
                ]);
                
                let _ = bot.send_message(chat_id, message)
                    .parse_mode(ParseMode::MarkdownV2)
                    .reply_markup(keyboard)
                    .await;
            }
            AuthState::WaitingForCode => {
                let _ = bot.send_message(
                    chat_id,
                    "🔑 *Authentication code required*\n\nPlease check your browser for an authentication code and paste it directly into this chat\\. No command needed\\!"
                )
                .parse_mode(ParseMode::MarkdownV2)
                .await;
            }
            AuthState::Completed(message) => {
                let _ = bot.send_message(chat_id, message)
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
                let _ = bot.send_message(
                    chat_id,
                    format!("❌ Authentication failed: {}", escape_markdown_v2(&error))
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
}

// Check if authentication session is already in progress
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
            "🔐 *Authentication Already in Progress*\n\n\
             You have an ongoing authentication session\\.\n\n\
             If you need to provide a code, use `/authcode <your_code>`\n\n\
             To restart authentication, please wait for the current session to complete or fail\\."
        )
        .parse_mode(ParseMode::MarkdownV2)
        .await?;
        return Ok(true);
    }
    Ok(false)
}

// Handle GitHub authentication process
async fn handle_github_authentication(
    bot: Bot,
    msg: Message,
    bot_state: BotState,
    chat_id: i64,
) -> ResponseResult<()> {
    let container_name = format!("coding-session-{}", chat_id);
    
    match ClaudeCodeClient::for_session(bot_state.docker.clone(), &container_name).await {
        Ok(client) => {
            // Send initial message
            bot.send_message(
                msg.chat.id,
                "🔐 Starting GitHub authentication process\\.\\.\\.\n\n⏳ Initiating OAuth flow\\.\\.\\."
            )
            .parse_mode(ParseMode::MarkdownV2)
            .await?;
            
            // Create GitHub client using same docker instance and container ID
            let github_client = GithubClient::new(
                bot_state.docker.clone(), 
                client.container_id().to_string(), 
                GithubClientConfig::default()
            );
            
            match github_client.login().await {
                Ok(auth_result) => {
                    let message = if auth_result.authenticated {
                        if let Some(username) = &auth_result.username {
                            format!("✅ GitHub authentication successful\\!\n\n👤 Logged in as: {}\n\n🎯 You can now use GitHub features in your coding session\\.", escape_markdown_v2(username))
                        } else {
                            "✅ GitHub authentication successful\\!\n\n🎯 You can now use GitHub features in your coding session\\.".to_string()
                        }
                    } else if let (Some(oauth_url), Some(device_code)) = (&auth_result.oauth_url, &auth_result.device_code) {
                        let message = format!("🔗 *GitHub OAuth Authentication Required*\n\n*Please follow these steps:*\n\n1️⃣ *Click the button below to visit the authentication URL*\n\n2️⃣ *Enter this device code:*\n```{}```\n\n3️⃣ *Sign in to your GitHub account* and authorize the application\n\n4️⃣ *Return here* \\- authentication will be completed automatically\n\n⏱️ This code will expire in a few minutes, so please complete the process promptly\\.\n\n💡 *Tip:* Use /githubstatus to check if authentication completed successfully\\.", escape_markdown_v2(device_code));
                        
                        let keyboard = InlineKeyboardMarkup::new(vec![
                            vec![InlineKeyboardButton::url("🔗 Open GitHub OAuth", Url::parse(oauth_url).unwrap_or_else(|_| Url::parse("https://github.com").unwrap()))],
                            vec![InlineKeyboardButton::switch_inline_query_current_chat("📋 Copy Device Code", device_code)],
                        ]);
                        
                        bot.send_message(msg.chat.id, message)
                            .parse_mode(ParseMode::MarkdownV2)
                            .reply_markup(keyboard)
                            .await?;
                        return Ok(());
                    } else {
                        format!("ℹ️ GitHub authentication status: {}", escape_markdown_v2(&auth_result.message))
                    };
                    
                    bot.send_message(msg.chat.id, message)
                        .parse_mode(ParseMode::MarkdownV2)
                        .await?;
                }
                Err(e) => {
                    let error_msg = e.to_string();
                    let user_message = if error_msg.contains("timed out after") {
                        format!("⏰ GitHub authentication timed out: {}\n\nThis usually means:\n• The authentication process is taking longer than expected\n• There may be network connectivity issues\n• The GitHub CLI might be unresponsive\n\nPlease try again in a few moments\\.", escape_markdown_v2(&error_msg))
                    } else {
                        format!("❌ Failed to initiate GitHub authentication: {}\n\nPlease ensure:\n• Your coding session is active\n• GitHub CLI \\(gh\\) is properly installed\n• Network connectivity is available", escape_markdown_v2(&error_msg))
                    };
                    
                    bot.send_message(msg.chat.id, user_message)
                        .parse_mode(ParseMode::MarkdownV2)
                        .await?;
                }
            }
        }
        Err(e) => {
            bot.send_message(
                msg.chat.id, 
                format!("❌ No active coding session found: {}\n\nPlease start a coding session first using /start", escape_markdown_v2(&e.to_string()))
            )
            .parse_mode(ParseMode::MarkdownV2)
            .await?;
        }
    }
    
    Ok(())
}

// Handle GitHub status check
async fn handle_github_status(
    bot: Bot,
    msg: Message,
    bot_state: BotState,
    chat_id: i64,
) -> ResponseResult<()> {
    let container_name = format!("coding-session-{}", chat_id);
    
    match ClaudeCodeClient::for_session(bot_state.docker.clone(), &container_name).await {
        Ok(client) => {
            let github_client = GithubClient::new(
                bot_state.docker.clone(), 
                client.container_id().to_string(), 
                GithubClientConfig::default()
            );
            
            match github_client.check_auth_status().await {
                Ok(auth_result) => {
                    let message = if auth_result.authenticated {
                        if let Some(username) = &auth_result.username {
                            format!("✅ *GitHub Authentication Status: Authenticated*\n\n👤 *Logged in as:* {}\n\n🎯 You can now use GitHub features like:\n• Repository cloning\n• Git operations\n• GitHub CLI commands", escape_markdown_v2(username))
                        } else {
                            "✅ *GitHub Authentication Status: Authenticated*\n\n🎯 You can now use GitHub features like:\n• Repository cloning\n• Git operations\n• GitHub CLI commands".to_string()
                        }
                    } else {
                        "❌ *GitHub Authentication Status: Not Authenticated*\n\n🔐 Use `/githubauth` to start the authentication process\\.\n\nYou'll receive an OAuth URL and device code to complete authentication in your browser\\.".to_string()
                    };
                    
                    let keyboard = if auth_result.authenticated {
                        InlineKeyboardMarkup::new(vec![
                            vec![InlineKeyboardButton::switch_inline_query_current_chat("📂 List Repositories", "/githubrepolist")],
                        ])
                    } else {
                        InlineKeyboardMarkup::new(vec![
                            vec![InlineKeyboardButton::switch_inline_query_current_chat("🔐 Start Authentication", "/githubauth")],
                        ])
                    };
                    
                    bot.send_message(msg.chat.id, message)
                        .parse_mode(ParseMode::MarkdownV2)
                        .reply_markup(keyboard)
                        .await?;
                }
                Err(e) => {
                    bot.send_message(
                        msg.chat.id,
                        format!("❌ Failed to check GitHub authentication status: {}\n\nThis could be due to:\n• GitHub CLI not being available\n• Network connectivity issues\n• Container problems", escape_markdown_v2(&e.to_string()))
                    )
                    .parse_mode(ParseMode::MarkdownV2)
                    .await?;
                }
            }
        }
        Err(e) => {
            bot.send_message(
                msg.chat.id, 
                format!("❌ No active coding session found: {}\n\nPlease start a coding session first using /start", escape_markdown_v2(&e.to_string()))
            )
            .parse_mode(ParseMode::MarkdownV2)
            .await?;
        }
    }
    
    Ok(())
}

// Handle debug Claude login command
async fn handle_debug_claude_login(
    bot: Bot,
    msg: Message,
    chat_id: i64,
) -> ResponseResult<()> {
    // Send initial message
    bot.send_message(
        msg.chat.id,
        "🔍 Searching for Claude authentication debug logs\\.\\.\\."
    )
    .parse_mode(ParseMode::MarkdownV2)
    .await?;

    match find_claude_auth_log_file(chat_id).await {
        Some(log_file_path) => {
            // Check if file exists and get its size
            match tokio::fs::metadata(&log_file_path).await {
                Ok(metadata) => {
                    let file_size = metadata.len();
                    let file_name = Path::new(&log_file_path)
                        .file_name()
                        .unwrap_or_default()
                        .to_string_lossy()
                        .to_string();
                    
                    // Telegram has a 50MB file size limit
                    if file_size > 50 * 1024 * 1024 {
                        bot.send_message(
                            msg.chat.id,
                            format!("📁 Found debug log file: `{}`\n\n⚠️ File is too large to send via Telegram \\({:.1} MB\\)\\. Log files are automatically cleaned up periodically\\.", 
                                escape_markdown_v2(&file_name), 
                                file_size as f64 / (1024.0 * 1024.0)
                            )
                        )
                        .parse_mode(ParseMode::MarkdownV2)
                        .await?;
                        return Ok(());
                    }
                    
                    // Send the file as an attachment
                    let input_file = InputFile::file(&log_file_path);
                    let caption = format!(
                        "🔍 *Claude Authentication Debug Log*\n\n📁 File: `{}`\n📊 Size: {:.1} KB\n🕒 Chat ID: `{}`\n\n💡 This log contains detailed information about the Claude authentication process\\.",
                        escape_markdown_v2(&file_name),
                        file_size as f64 / 1024.0,
                        chat_id
                    );
                    
                    bot.send_document(msg.chat.id, input_file)
                        .caption(caption)
                        .parse_mode(ParseMode::MarkdownV2)
                        .await?;
                }
                Err(e) => {
                    bot.send_message(
                        msg.chat.id,
                        format!("❌ Found log file but couldn't access it: {}", escape_markdown_v2(&e.to_string()))
                    )
                    .parse_mode(ParseMode::MarkdownV2)
                    .await?;
                }
            }
        }
        None => {
            bot.send_message(
                msg.chat.id,
                "📂 *No Claude authentication debug logs found*\n\n💡 Debug logs are created when:\n• Claude authentication is attempted\n• An authentication session fails or encounters errors\n\n🔄 Try running `/authenticateclaude` first to generate debug logs\\."
            )
            .parse_mode(ParseMode::MarkdownV2)
            .await?;
        }
    }
    
    Ok(())
}

async fn handle_github_repo_list(
    bot: Bot,
    msg: Message,
    bot_state: BotState,
    chat_id: i64,
) -> ResponseResult<()> {
    let container_name = format!("coding-session-{}", chat_id);
    
    match ClaudeCodeClient::for_session(bot_state.docker.clone(), &container_name).await {
        Ok(client) => {
            let github_client = GithubClient::new(
                bot_state.docker.clone(), 
                client.container_id().to_string(), 
                GithubClientConfig::default()
            );
            
            match github_client.repo_list().await {
                Ok(repo_list) => {
                    if repo_list.trim().is_empty() {
                        bot.send_message(
                            msg.chat.id, 
                            "📁 *GitHub Repository List*\n\n💡 No repositories found or no repositories accessible with current authentication\\.".to_string()
                        )
                        .parse_mode(ParseMode::MarkdownV2)
                        .await?;
                    } else {
                        let formatted_repo_list = format_repo_list_markdown_v2(&repo_list);
                        bot.send_message(
                            msg.chat.id, 
                            format!("📁 *GitHub Repository List*\n\n{}", formatted_repo_list)
                        )
                        .parse_mode(ParseMode::MarkdownV2)
                        .await?;
                    }
                }
                Err(e) => {
                    let error_message = if e.to_string().contains("authentication required") 
                        || e.to_string().contains("not authenticated") {
                        "❌ *GitHub Authentication Required*\n\n🔐 Please authenticate with GitHub first using /githubauth"
                    } else if e.to_string().contains("gh: command not found") 
                        || e.to_string().contains("executable file not found") {
                        "❌ *GitHub CLI Not Available*\n\n⚠️ The GitHub CLI \\(gh\\) is not installed in the coding session\\."
                    } else {
                        &format!("❌ *Failed to list repositories*\n\n🔍 Error: {}", escape_markdown_v2(&e.to_string()))
                    };
                    
                    bot.send_message(msg.chat.id, error_message)
                        .parse_mode(ParseMode::MarkdownV2)
                        .await?;
                }
            }
        }
        Err(e) => {
            bot.send_message(
                msg.chat.id, 
                format!("❌ No active coding session found: {}\\n\\nPlease start a coding session first using /start", escape_markdown_v2(&e.to_string()))
            )
            .parse_mode(ParseMode::MarkdownV2)
            .await?;
        }
    }
    
    Ok(())
}

// Handle clear session command
async fn handle_clear_session(
    bot: Bot,
    msg: Message,
    bot_state: BotState,
    chat_id: i64,
) -> ResponseResult<()> {
    let container_name = format!("coding-session-{}", chat_id);

    // Also clear any pending authentication session
    {
        let mut sessions = bot_state.auth_sessions.lock().await;
        sessions.remove(&chat_id);
    }

    match container_utils::clear_coding_session(&bot_state.docker, &container_name).await {
        Ok(()) => {
            bot.send_message(
                msg.chat.id,
                "🧹 Coding session cleared successfully\\!\n\nThe container has been stopped and removed\\."
            )
            .parse_mode(ParseMode::MarkdownV2)
            .await?;
        }
        Err(e) => {
            bot.send_message(msg.chat.id, format!("❌ Failed to clear session: {}", escape_markdown_v2(&e.to_string())))
                .parse_mode(ParseMode::MarkdownV2)
                .await?;
        }
    }
    
    Ok(())
}

// Handle Claude authentication process
async fn handle_claude_authentication(
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
                "🔐 Starting Claude account authentication process\\.\\.\\.\n\n⏳ Initiating OAuth flow\\.\\.\\."
            )
            .parse_mode(ParseMode::MarkdownV2)
            .await?;

            match client.authenticate_claude_account().await {
                Ok(auth_handle) => {
                    // Extract channels from the handle
                    let AuthenticationHandle { state_receiver, code_sender, cancel_sender: _cancel_sender } = auth_handle;

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
                        format!("❌ Failed to initiate Claude account authentication: {}\n\nPlease ensure:\n• Your coding session is active\n• Claude Code is properly installed\n• Network connectivity is available", escape_markdown_v2(&e.to_string()))
                    )
                    .parse_mode(ParseMode::MarkdownV2)
                    .await?;
                }
            }
        }
        Err(e) => {
            bot.send_message(
                msg.chat.id,
                format!("❌ No active coding session found: {}\n\nPlease start a coding session first using /start", escape_markdown_v2(&e.to_string()))
            )
            .parse_mode(ParseMode::MarkdownV2)
            .await?;
        }
    }
    
    Ok(())
}

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
            if is_authentication_code(text) {
                // Send the code to the authentication process
                if let Err(_) = auth_session.code_sender.send(text.to_string()) {
                    bot.send_message(
                        msg.chat.id,
                        "❌ Failed to send authentication code\\. The authentication session may have expired\\.\n\nPlease restart authentication with `/authenticateclaude`"
                    )
                    .parse_mode(ParseMode::MarkdownV2)
                    .await?;
                } else {
                    bot.send_message(
                        msg.chat.id,
                        "✅ Authentication code received\\! Please wait while we complete the authentication process\\.\\.\\."
                    )
                    .parse_mode(ParseMode::MarkdownV2)
                    .await?;
                }
            } else {
                // Not an auth code, inform user about the ongoing session
                bot.send_message(
                    msg.chat.id,
                    "🔐 *Authentication in Progress*\n\nI'm currently waiting for your authentication code\\. Please paste the code you received during the OAuth flow\\.\n\nIf you need to restart authentication, use `/authenticateclaude`"
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
fn is_authentication_code(text: &str) -> bool {
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
    let valid_chars = text.chars().all(|c| c.is_alphanumeric() || c == '-' || c == '_' || c == '.' || c == '#');
    
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

// Handler function for bot commands
async fn answer(bot: Bot, msg: Message, cmd: Command, bot_state: BotState) -> ResponseResult<()> {
    let chat_id = msg.chat.id.0;
    
    // Use the user ID for volume persistence (same user across different chats)
    let user_id = msg.from.as_ref().map(|user| user.id.0 as i64).unwrap_or(chat_id);

    match cmd {
        Command::Help => {
            let help_text = generate_help_text();
            bot.send_message(msg.chat.id, escape_markdown_v2(&help_text))
                .parse_mode(ParseMode::MarkdownV2)
                .await?;
        }
        Command::ClearSession => {
            handle_clear_session(bot.clone(), msg, bot_state.clone(), chat_id).await?;
        }
        Command::Start => {
            let container_name = format!("coding-session-{}", chat_id);

            // Send initial welcome message
            bot.send_message(
                msg.chat.id,
                "Hello\\! I'm your Claude Code Chat Bot 🤖🐳\n\n🚀 Starting new coding session\\.\\.\\.\n\n⏳ Creating container with Claude Code\\.\\.\\."
            )
            .parse_mode(ParseMode::MarkdownV2)
            .await?;

            match container_utils::start_coding_session(
                &bot_state.docker,
                &container_name,
                ClaudeCodeConfig::default(),
                container_utils::CodingContainerConfig { 
                    persistent_volume_key: Some(user_id.to_string()) 
                },
            )
            .await
            {
                Ok(claude_client) => {
                    let container_id_short = claude_client.container_id().chars().take(12).collect::<String>();
                    let message = format!("✅ Coding session started successfully\\!\n\n*Container ID:* `{}`\n*Container Name:* `{}`\n\n🎯 Claude Code is pre\\-installed and ready to use\\!\n\nYou can now run code and manage your development environment\\.",
                                escape_markdown_v2(&container_id_short), escape_markdown_v2(&container_name));
                    
                    let keyboard = InlineKeyboardMarkup::new(vec![
                        vec![
                            InlineKeyboardButton::switch_inline_query_current_chat("🔐 Auth Claude", "/authenticateclaude"),
                            InlineKeyboardButton::switch_inline_query_current_chat("🐙 Auth GitHub", "/githubauth"),
                        ],
                        vec![
                            InlineKeyboardButton::switch_inline_query_current_chat("📊 Claude Status", "/claudestatus"),
                            InlineKeyboardButton::switch_inline_query_current_chat("📋 GitHub Status", "/githubstatus"),
                        ],
                    ]);
                    
                    bot.send_message(msg.chat.id, message)
                        .parse_mode(ParseMode::MarkdownV2)
                        .reply_markup(keyboard)
                        .await?;
                }
                Err(e) => {
                    bot.send_message(
                        msg.chat.id,
                        format!("❌ Failed to start coding session: {}\n\nThis could be due to:\n• Container creation failure\n• Runtime image pull failure\n• Network connectivity issues", escape_markdown_v2(&e.to_string()))
                    )
                    .parse_mode(ParseMode::MarkdownV2)
                    .await?;
                }
            }
        }
        Command::ClaudeStatus => {
            let container_name = format!("coding-session-{}", chat_id);

            match ClaudeCodeClient::for_session(bot_state.docker.clone(), &container_name).await {
                Ok(client) => match client.check_availability().await {
                    Ok(version) => {
                        bot.send_message(
                            msg.chat.id,
                            format!("✅ Claude Code is available\\!\n\n*Version:* `{}`", escape_markdown_v2(&version)),
                        )
                        .parse_mode(ParseMode::MarkdownV2)
                        .await?;
                    }
                    Err(e) => {
                        bot.send_message(
                            msg.chat.id,
                            format!("❌ Claude Code check failed: {}", escape_markdown_v2(&e.to_string())),
                        )
                        .parse_mode(ParseMode::MarkdownV2)
                        .await?;
                    }
                },
                Err(e) => {
                    bot.send_message(
                        msg.chat.id,
                        format!("❌ No active coding session found: {}", escape_markdown_v2(&e.to_string())),
                    )
                    .parse_mode(ParseMode::MarkdownV2)
                    .await?;
                }
            }
        }
        Command::AuthenticateClaude => {
            handle_claude_authentication(bot.clone(), msg, bot_state.clone(), chat_id).await?;
        }

        Command::GitHubAuth => {
            handle_github_authentication(bot.clone(), msg, bot_state.clone(), chat_id).await?;
        }
        Command::GitHubStatus => {
            handle_github_status(bot.clone(), msg, bot_state.clone(), chat_id).await?;
        }
        Command::GitHubRepoList => {
            handle_github_repo_list(bot.clone(), msg, bot_state.clone(), chat_id).await?;
        }
        Command::DebugClaudeLogin => {
            handle_debug_claude_login(bot.clone(), msg, chat_id).await?;
        }
    }

    Ok(())
}

#[cfg(test)]
mod help_format_tests {
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
            assert!(line_pattern.is_match(line), 
                "Line should match pattern 'command - description': {}", line);
            
            // Additional validation: ensure separator exists and splits correctly
            let parts: Vec<&str> = line.split(" - ").collect();
            assert_eq!(parts.len(), 2, "Line should have exactly one ' - ' separator: {}", line);
            assert!(!parts[0].is_empty(), "Command part should not be empty: {}", line);
            assert!(!parts[1].is_empty(), "Description part should not be empty: {}", line);
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
            assert!(escaped_help_text.contains("\\-"), 
                "Hyphen should be escaped in MarkdownV2 format");
        }
        
        // Verify the escaped text is different from original if special chars exist
        let has_special_chars = help_text.chars().any(|c| 
            "_*[]()~`>#+-=|{}.!".contains(c)
        );
        
        if has_special_chars {
            assert_ne!(help_text, escaped_help_text, 
                "Escaped text should differ from original when special characters exist");
        }
        
        // Verify that the escaping function produces a valid result
        assert!(!escaped_help_text.is_empty(), "Escaped help text should not be empty");
    }
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

#[cfg(test)]
mod repo_format_tests {
    use super::*;

    #[test]
    fn test_format_repo_list_markdown_v2_empty_input() {
        assert_eq!(
            format_repo_list_markdown_v2(""),
            "💡 No repositories found or no repositories accessible with current authentication\\."
        );
        assert_eq!(
            format_repo_list_markdown_v2("   \n  \n  "),
            "💡 No repositories found or no repositories accessible with current authentication\\."
        );
    }

    #[test]
    fn test_format_repo_list_markdown_v2_single_repo() {
        let input = "user/repo1\tA sample repository";
        let expected = "• [user/repo1](https://github.com/user/repo1) \\- A sample repository";
        assert_eq!(format_repo_list_markdown_v2(input), expected);
    }

    #[test]
    fn test_format_repo_list_markdown_v2_single_repo_no_description() {
        let input = "user/repo1";
        let expected = "• [user/repo1](https://github.com/user/repo1)";
        assert_eq!(format_repo_list_markdown_v2(input), expected);
    }

    #[test]
    fn test_format_repo_list_markdown_v2_multiple_repos() {
        let input = "user/repo1\tFirst repository\nuser/repo2\tSecond repository";
        let expected = "• [user/repo1](https://github.com/user/repo1) \\- First repository\n• [user/repo2](https://github.com/user/repo2) \\- Second repository";
        assert_eq!(format_repo_list_markdown_v2(input), expected);
    }

    #[test]
    fn test_format_repo_list_markdown_v2_with_special_characters() {
        let input = "user/repo-test\tRepository with special chars: [test] (v1.0)";
        let expected = "• [user/repo\\-test](https://github.com/user/repo-test) \\- Repository with special chars: \\[test\\] \\(v1\\.0\\)";
        assert_eq!(format_repo_list_markdown_v2(input), expected);
    }

    #[test]
    fn test_format_repo_list_markdown_v2_space_separated() {
        let input = "user/repo1    First repository with spaces";
        let expected = "• [user/repo1](https://github.com/user/repo1) \\- First repository with spaces";
        assert_eq!(format_repo_list_markdown_v2(input), expected);
    }

    #[test]
    fn test_format_repo_list_markdown_v2_mixed_formatting() {
        let input = "owner/project1\tDescription 1\nowner/project2    Description 2\nowner/project3";
        let expected = "• [owner/project1](https://github.com/owner/project1) \\- Description 1\n• [owner/project2](https://github.com/owner/project2) \\- Description 2\n• [owner/project3](https://github.com/owner/project3)";
        assert_eq!(format_repo_list_markdown_v2(input), expected);
    }
}

#[cfg(test)]
mod auth_code_detection_tests {
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
        assert!(is_authentication_code("yHNxk8SH0fw861QGEXP80UeTIzJUbSg6BDQWvtN80ecoOGAf#ybFaWRHX0Y5YdJaM9ET8_06if-w9Rwg0X-4lEMdyT7I"));
        assert!(is_authentication_code("abcd1234567890abcd1234567890#efgh5678901234efgh5678901234_code-part"));
        assert!(is_authentication_code("long_part_with_underscores_123#another_long_part_with_more_data_456"));
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
        assert!(!is_authentication_code("short#this_is_long_enough_but_first_was_short")); // First part too short
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
