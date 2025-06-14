use teloxide::{prelude::*, utils::command::BotCommands};
use bollard::Docker;

mod claude_code_client;
use claude_code_client::{ClaudeCodeClient, ClaudeCodeConfig, container_utils};

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
                "🚀 Starting new coding session...\n\n⏳ Creating container and installing Claude Code..."
            ).await?;
            
            match container_utils::start_coding_session(&docker, &container_name, ClaudeCodeConfig::default()).await {
                Ok(claude_client) => {
                    bot.send_message(
                        msg.chat.id, 
                        format!("✅ Coding session started successfully!\n\nContainer ID: {}\nContainer Name: {}\n\n🎯 Claude Code has been installed and is ready to use!\n\nYou can now run code and manage your development environment.", 
                                claude_client.container_id().chars().take(12).collect::<String>(), container_name)
                    ).await?;
                }
                Err(e) => {
                    bot.send_message(
                        msg.chat.id, 
                        format!("❌ Failed to start coding session: {}\n\nThis could be due to:\n• Container creation failure\n• Claude Code installation failure\n• Network connectivity issues", e)
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
                            let error_msg = e.to_string();
                            // Check if this is a container-related error
                            if error_msg.contains("Container health check failed") || 
                               error_msg.contains("container may have terminated") ||
                               error_msg.contains("Container is not running") ||
                               error_msg.contains("container may have terminated") {
                                bot.send_message(
                                    msg.chat.id, 
                                    format!("❌ Container issue detected during authentication: {}\n\n🔄 **Recommended actions:**\n• Try restarting your coding session with /clearsession followed by /startsession\n• Check if there are sufficient system resources available\n• If the issue persists, there may be Docker configuration problems", error_msg)
                                ).await?;
                            } else {
                                bot.send_message(
                                    msg.chat.id, 
                                    format!("❌ Failed to initiate Claude account authentication: {}\n\nPlease ensure:\n• Your coding session is active\n• Claude Code is properly installed\n• Network connectivity is available", e)
                                ).await?;
                            }
                        }
                    }
                }
                Err(e) => {
                    let error_msg = e.to_string();
                    if error_msg.contains("Container not found") {
                        bot.send_message(
                            msg.chat.id, 
                            "❌ No active coding session found.\n\nPlease start a coding session first using /startsession"
                        ).await?;
                    } else if error_msg.contains("Container health check failed") ||
                              error_msg.contains("Container is not running") {
                        bot.send_message(
                            msg.chat.id, 
                            format!("❌ Container health issue detected: {}\n\n🔄 **Recommended actions:**\n• Try restarting your coding session with /clearsession followed by /startsession\n• The container may have terminated unexpectedly due to resource constraints", error_msg)
                        ).await?;
                    } else {
                        bot.send_message(
                            msg.chat.id, 
                            format!("❌ Failed to connect to coding session: {}\n\nPlease try restarting your session with /clearsession followed by /startsession", e)
                        ).await?;
                    }
                }
            }
        }
    }

    Ok(())
}
