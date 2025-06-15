use teloxide::{prelude::*, utils::command::BotCommands};
use bollard::Docker;
use bollard::image::CreateImageOptions;
use futures_util::StreamExt;

mod claude_code_client;
use claude_code_client::{ClaudeCodeClient, ClaudeCodeConfig, container_utils, GithubClient, GithubClientConfig};

// Define the commands that your bot will handle
#[derive(BotCommands, Clone)]
#[command(rename_rule = "lowercase", description = "These commands are supported:")]
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
    #[command(description = "Authenticate Claude using your Claude account credentials (OAuth flow)")]
    AuthenticateClaude,
    #[command(description = "Authenticate with GitHub using OAuth flow")]
    GitHubAuth,
}

// Main bot logic
#[tokio::main]
async fn main() {
    pretty_env_logger::init();
    log::info!("Starting Telegram bot...");

    let bot = Bot::from_env();

    // Initialize Docker client
    let docker = Docker::connect_with_socket_defaults()
        .expect("Failed to connect to Docker daemon");

    log::info!("Connected to Docker daemon");

    // Pull the latest runtime image on startup
    log::info!("Pulling latest runtime image: {}", container_utils::MAIN_CONTAINER_IMAGE);
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

    Command::repl(bot, move |bot, msg, cmd| {
        let docker = docker.clone();
        answer(bot, msg, cmd, docker)
    }).await;
}

// Handler function for bot commands
async fn answer(bot: Bot, msg: Message, cmd: Command, docker: Docker) -> ResponseResult<()> {
    match cmd {
        Command::Help => {
            bot.send_message(msg.chat.id, Command::descriptions().to_string())
                .await?;
        }
        Command::StartSession => {
            let chat_id = msg.chat.id.0;
            let container_name = format!("coding-session-{}", chat_id);
            
            // Send initial message
            bot.send_message(
                msg.chat.id,
                "🚀 Starting new coding session...\n\n⏳ Creating container with pre-installed Claude Code..."
            ).await?;
            
            match container_utils::start_coding_session(&docker, &container_name, ClaudeCodeConfig::default()).await {
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
            let chat_id = msg.chat.id.0;
            let container_name = format!("coding-session-{}", chat_id);
            
            match container_utils::clear_coding_session(&docker, &container_name).await {
                Ok(()) => {
                    bot.send_message(
                        msg.chat.id, 
                        "🧹 Coding session cleared successfully!\n\nThe container has been stopped and removed."
                    ).await?;
                }
                Err(e) => {
                    bot.send_message(
                        msg.chat.id, 
                        format!("❌ Failed to clear session: {}", e)
                    ).await?;
                }
            }
        }
        Command::Start => {
            bot.send_message(msg.chat.id, "Hello! I'm your Telegram bot with Docker support 🤖🐳")
                .await?;
        }
        Command::ClaudeStatus => {
            let chat_id = msg.chat.id.0;
            let container_name = format!("coding-session-{}", chat_id);
            
            match ClaudeCodeClient::for_session(docker.clone(), &container_name).await {
                Ok(client) => {
                    match client.check_availability().await {
                        Ok(version) => {
                            bot.send_message(
                                msg.chat.id, 
                                format!("✅ Claude Code is available!\n\nVersion: {}", version)
                            ).await?;
                        }
                        Err(e) => {
                            bot.send_message(
                                msg.chat.id, 
                                format!("❌ Claude Code check failed: {}", e)
                            ).await?;
                        }
                    }
                }
                Err(e) => {
                    bot.send_message(
                        msg.chat.id, 
                        format!("❌ No active coding session found: {}", e)
                    ).await?;
                }
            }
        }
        Command::AuthenticateClaude => {
            let chat_id = msg.chat.id.0;
            let container_name = format!("coding-session-{}", chat_id);
            
            match ClaudeCodeClient::for_session(docker.clone(), &container_name).await {
                Ok(client) => {
                    // Send initial message
                    bot.send_message(
                        msg.chat.id,
                        "🔐 Starting Claude account authentication process...\n\n⏳ Initiating OAuth flow..."
                    ).await?;
                    
                    match client.authenticate_claude_account().await {
                        Ok(auth_info) => {
                            bot.send_message(
                                msg.chat.id, 
                                auth_info
                            ).await?;
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
            let chat_id = msg.chat.id.0;
            let container_name = format!("coding-session-{}", chat_id);
            
            match ClaudeCodeClient::for_session(docker.clone(), &container_name).await {
                Ok(client) => {
                    // Send initial message
                    bot.send_message(
                        msg.chat.id,
                        "🔐 Starting GitHub authentication process...\n\n⏳ Initiating OAuth flow..."
                    ).await?;
                    
                    // Create GitHub client using same docker instance and container ID
                    let github_client = GithubClient::new(
                        docker.clone(), 
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
                                format!("🔗 **GitHub OAuth Authentication Required**\n\n**Please follow these steps:**\n\n1️⃣ **Visit this URL:** {}\n\n2️⃣ **Enter this device code:** `{}`\n\n3️⃣ **Sign in to your GitHub account** and authorize the application\n\n4️⃣ **Return here** - authentication will be completed automatically\n\n⏱️ This code will expire in a few minutes, so please complete the process promptly.", oauth_url, device_code)
                            } else {
                                format!("ℹ️ GitHub authentication status: {}", auth_result.message)
                            };
                            
                            bot.send_message(msg.chat.id, message).await?;
                        }
                        Err(e) => {
                            bot.send_message(
                                msg.chat.id, 
                                format!("❌ Failed to initiate GitHub authentication: {}\n\nPlease ensure:\n• Your coding session is active\n• GitHub CLI (gh) is properly installed\n• Network connectivity is available", e)
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
