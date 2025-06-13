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
    #[command(description = "Check Claude Code authentication setup and get API key instructions")]
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
                        "🔐 Checking Claude Code authentication setup..."
                    ).await?;
                    
                    match client.setup_authentication().await {
                        Ok(instructions) => {
                            bot.send_message(
                                msg.chat.id, 
                                instructions
                            ).await?;
                        }
                        Err(e) => {
                            bot.send_message(
                                msg.chat.id, 
                                format!("❌ Failed to check authentication setup: {}\n\nPlease ensure:\n• Your coding session is active\n• Claude Code is properly installed\n• Network connectivity is available", e)
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
