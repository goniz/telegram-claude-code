use bollard::image::CreateImageOptions;
use bollard::Docker;
use futures_util::StreamExt;
use std::collections::HashMap;
use std::sync::Arc;
use teloxide::{prelude::*, utils::command::BotCommands};
use tokio::sync::Mutex;

mod claude_code_client;
use claude_code_client::{container_utils, ClaudeCodeClient, ClaudeCodeConfig, GithubClient, GithubClientConfig};

// Define the commands that your bot will handle
#[derive(BotCommands, Clone)]
#[command(
    rename_rule = "lowercase",
    description = "These commands are supported:"
)]
enum Command {
    #[command(description = "Display this help message")]
    Help,
    #[command(description = "Start the bot")]
    Start,
    #[command(description = "Start a new coding session (creates a new container)")]
    StartSession,
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
}

// Authentication session state
#[derive(Debug, Clone)]
#[allow(dead_code)]
struct AuthSession {
    container_name: String,
    state: claude_code_client::InteractiveLoginState,
    url: Option<String>,
}

// Global state for tracking authentication sessions
type AuthSessions = Arc<Mutex<HashMap<i64, AuthSession>>>;

#[derive(Clone)]
struct BotState {
    docker: Docker,
    auth_sessions: AuthSessions,
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

    // Pull the latest runtime image on startup
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

    // Initialize bot state
    let auth_sessions: AuthSessions = Arc::new(Mutex::new(HashMap::new()));
    let bot_state = BotState {
        docker: docker.clone(),
        auth_sessions: auth_sessions.clone(),
    };

    Command::repl(bot, move |bot, msg, cmd| {
        let bot_state = bot_state.clone();
        answer(bot, msg, cmd, bot_state)
    })
    .await;
}

// Handler function for bot commands
async fn answer(bot: Bot, msg: Message, cmd: Command, bot_state: BotState) -> ResponseResult<()> {
    let chat_id = msg.chat.id.0;

    match cmd {
        Command::Help => {
            bot.send_message(msg.chat.id, Command::descriptions().to_string())
                .await?;
        }
        Command::StartSession => {
            let container_name = format!("coding-session-{}", chat_id);

            // Send initial message
            bot.send_message(
                msg.chat.id,
                "🚀 Starting new coding session...\n\n⏳ Creating container with pre-installed Claude Code..."
            ).await?;

            match container_utils::start_coding_session(
                &bot_state.docker,
                &container_name,
                ClaudeCodeConfig::default(),
            )
            .await
            {
                Ok(claude_client) => {
                    bot.send_message(
                        msg.chat.id,
                        format!("✅ Coding session started successfully!\n\nContainer ID: {}\nContainer Name: {}\n\n🎯 Claude Code is pre-installed and ready to use!\n\nYou can now run code and manage your development environment.",
                                claude_client.container_id().chars().take(12).collect::<String>(), container_name)
                    ).await?;
                }
                Err(e) => {
                    bot.send_message(
                        msg.chat.id,
                        format!("❌ Failed to start coding session: {}\n\nThis could be due to:\n• Container creation failure\n• Runtime image pull failure\n• Network connectivity issues", e)
                    ).await?;
                }
            }
        }
        Command::ClearSession => {
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
                        "🧹 Coding session cleared successfully!\n\nThe container has been stopped and removed."
                    ).await?;
                }
                Err(e) => {
                    bot.send_message(msg.chat.id, format!("❌ Failed to clear session: {}", e))
                        .await?;
                }
            }
        }
        Command::Start => {
            bot.send_message(
                msg.chat.id,
                "Hello! I'm your Telegram bot with Docker support 🤖🐳",
            )
            .await?;
        }
        Command::ClaudeStatus => {
            let container_name = format!("coding-session-{}", chat_id);

            match ClaudeCodeClient::for_session(bot_state.docker.clone(), &container_name).await {
                Ok(client) => match client.check_availability().await {
                    Ok(version) => {
                        bot.send_message(
                            msg.chat.id,
                            format!("✅ Claude Code is available!\n\nVersion: {}", version),
                        )
                        .await?;
                    }
                    Err(e) => {
                        bot.send_message(
                            msg.chat.id,
                            format!("❌ Claude Code check failed: {}", e),
                        )
                        .await?;
                    }
                },
                Err(e) => {
                    bot.send_message(
                        msg.chat.id,
                        format!("❌ No active coding session found: {}", e),
                    )
                    .await?;
                }
            }
        }
        Command::AuthenticateClaude => {
            let container_name = format!("coding-session-{}", chat_id);

            match ClaudeCodeClient::for_session(bot_state.docker.clone(), &container_name).await {
                Ok(client) => {
                    // Check if there's already an authentication session in progress
                    {
                        let sessions = bot_state.auth_sessions.lock().await;
                        if let Some(session) = sessions.get(&chat_id) {
                            match &session.state {
                                claude_code_client::InteractiveLoginState::ProvideUrl(url) => {
                                    bot.send_message(
                                        msg.chat.id,
                                        format!("🔐 **Authentication Already in Progress**\n\n\
                                               You have an ongoing authentication session.\n\n\
                                               **Please visit this URL to continue:**\n{}\n\n\
                                               After completing the OAuth flow, use `/authcode <your_code>` if a code is required.",
                                               url)
                                    ).await?;
                                    return Ok(());
                                }
                                claude_code_client::InteractiveLoginState::WaitingForCode => {
                                    bot.send_message(
                                        msg.chat.id,
                                        "🔐 **Authentication Code Required**\n\n\
                                        Please send your authentication code using:\n\
                                        `/authcode <your_code>`",
                                    )
                                    .await?;
                                    return Ok(());
                                }
                                _ => {
                                    // Clear the session and start fresh
                                }
                            }
                        }
                    }

                    // Send initial message
                    bot.send_message(
                        msg.chat.id,
                        "🔐 Starting Claude account authentication process...\n\n⏳ Initiating OAuth flow..."
                    ).await?;

                    match client.authenticate_claude_account().await {
                        Ok(auth_info) => {
                            // Check if the authentication returned a URL or requires code
                            if auth_info.contains("Visit this authentication URL") {
                                // Extract URL and create session
                                if let Some(url_start) = auth_info.find("https://") {
                                    let url_part = &auth_info[url_start..];
                                    let url = if let Some(url_end) = url_part.find('\n') {
                                        &url_part[..url_end]
                                    } else {
                                        url_part
                                    }
                                    .trim();

                                    // Store authentication session
                                    let session = AuthSession {
                                        container_name: container_name.clone(),
                                        state:
                                            claude_code_client::InteractiveLoginState::ProvideUrl(
                                                url.to_string(),
                                            ),
                                        url: Some(url.to_string()),
                                    };

                                    {
                                        let mut sessions = bot_state.auth_sessions.lock().await;
                                        sessions.insert(chat_id, session);
                                    }

                                    bot.send_message(
                                        msg.chat.id,
                                        format!("{}\n\n💡 **After completing authentication, use `/authcode <code>` if prompted for a code.**", auth_info)
                                    ).await?;
                                } else {
                                    bot.send_message(msg.chat.id, auth_info).await?;
                                }
                            } else {
                                // Authentication completed or other status - this includes URL provision
                                bot.send_message(msg.chat.id, auth_info).await?;
                            }
                        }
                        Err(e) => {
                            bot.send_message(
                                msg.chat.id,
                                format!("❌ Failed to initiate Claude account authentication: {}\n\nPlease ensure:\n• Your coding session is active\n• Claude Code is properly installed\n• Network connectivity is available", e)
                            ).await?;
                        }
                    }
                }
                Err(e) => {
                    bot.send_message(
                        msg.chat.id,
                        format!("❌ No active coding session found: {}\n\nPlease start a coding session first using /startsession", e)
                    ).await?;
                }
            }
        }

        Command::GitHubAuth => {
            let container_name = format!("coding-session-{}", chat_id);
            
            match ClaudeCodeClient::for_session(bot_state.docker.clone(), &container_name).await {
                Ok(client) => {
                    // Send initial message
                    bot.send_message(
                        msg.chat.id,
                        "🔐 Starting GitHub authentication process...\n\n⏳ Initiating OAuth flow..."
                    ).await?;
                    
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
                                    format!("✅ GitHub authentication successful!\n\n👤 Logged in as: {}\n\n🎯 You can now use GitHub features in your coding session.", username)
                                } else {
                                    "✅ GitHub authentication successful!\n\n🎯 You can now use GitHub features in your coding session.".to_string()
                                }
                            } else if let (Some(oauth_url), Some(device_code)) = (&auth_result.oauth_url, &auth_result.device_code) {
                                format!("🔗 **GitHub OAuth Authentication Required**\n\n**Please follow these steps:**\n\n1️⃣ **Visit this URL:** {}\n\n2️⃣ **Enter this device code:**\n```\n{}\n```\n\n3️⃣ **Sign in to your GitHub account** and authorize the application\n\n4️⃣ **Return here** - authentication will be completed automatically\n\n⏱️ This code will expire in a few minutes, so please complete the process promptly.\n\n💡 **Tip:** Use `/githubstatus` to check if authentication completed successfully.", oauth_url, device_code)
                            } else {
                                format!("ℹ️ GitHub authentication status: {}", auth_result.message)
                            };
                            
                            bot.send_message(msg.chat.id, message).await?;
                        }
                        Err(e) => {
                            let error_msg = e.to_string();
                            let user_message = if error_msg.contains("timed out after") {
                                format!("⏰ GitHub authentication timed out: {}\n\nThis usually means:\n• The authentication process is taking longer than expected\n• There may be network connectivity issues\n• The GitHub CLI might be unresponsive\n\nPlease try again in a few moments.", error_msg)
                            } else {
                                format!("❌ Failed to initiate GitHub authentication: {}\n\nPlease ensure:\n• Your coding session is active\n• GitHub CLI (gh) is properly installed\n• Network connectivity is available", error_msg)
                            };
                            
                            bot.send_message(msg.chat.id, user_message).await?;
                        }
                    }
                }
                Err(e) => {
                    bot.send_message(
                        msg.chat.id, 
                        format!("❌ No active coding session found: {}\n\nPlease start a coding session first using /startsession", e)
                    ).await?;
                }
            }
        }
        Command::GitHubStatus => {
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
                                    format!("✅ **GitHub Authentication Status: Authenticated**\n\n👤 **Logged in as:** {}\n\n🎯 You can now use GitHub features like:\n• Repository cloning\n• Git operations\n• GitHub CLI commands", username)
                                } else {
                                    "✅ **GitHub Authentication Status: Authenticated**\n\n🎯 You can now use GitHub features like:\n• Repository cloning\n• Git operations\n• GitHub CLI commands".to_string()
                                }
                            } else {
                                "❌ **GitHub Authentication Status: Not Authenticated**\n\n🔐 Use `/githubauth` to start the authentication process.\n\nYou'll receive an OAuth URL and device code to complete authentication in your browser.".to_string()
                            };
                            
                            bot.send_message(msg.chat.id, message).await?;
                        }
                        Err(e) => {
                            bot.send_message(
                                msg.chat.id,
                                format!("❌ Failed to check GitHub authentication status: {}\n\nThis could be due to:\n• GitHub CLI not being available\n• Network connectivity issues\n• Container problems", e)
                            ).await?;
                        }
                    }
                }
                Err(e) => {
                    bot.send_message(
                        msg.chat.id, 
                        format!("❌ No active coding session found: {}\n\nPlease start a coding session first using /startsession", e)
                    ).await?;
                }
            }
        }
    }

    Ok(())
}
