use teloxide::{prelude::*, utils::command::BotCommands};
use bollard::Docker;
use bollard::container::{CreateContainerOptions, Config, RemoveContainerOptions};
use bollard::exec::{CreateExecOptions, StartExecOptions};
use futures_util::StreamExt;

mod claude_code_client;
use claude_code_client::ClaudeCodeClient;

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
            
            match start_coding_session(&docker, &container_name).await {
                Ok(container_id) => {
                    bot.send_message(
                        msg.chat.id, 
                        format!("✅ Coding session started successfully!\n\nContainer ID: {}\nContainer Name: {}\n\n🎯 Claude Code has been installed and is ready to use!\n\nYou can now run code and manage your development environment.", 
                                container_id.chars().take(12).collect::<String>(), container_name)
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
            
            match clear_coding_session(&docker, &container_name).await {
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
    }

    Ok(())
}

// Helper function to execute a command in a container
async fn exec_command_in_container(docker: &Docker, container_id: &str, command: Vec<String>) -> Result<String, Box<dyn std::error::Error + Send + Sync>> {
    let exec_config = CreateExecOptions {
        cmd: Some(command),
        attach_stdout: Some(true),
        attach_stderr: Some(true),
        ..Default::default()
    };

    let exec = docker.create_exec(container_id, exec_config).await?;
    
    let start_config = StartExecOptions {
        detach: false,
        ..Default::default()
    };

    let mut output = String::new();
    
    match docker.start_exec(&exec.id, Some(start_config)).await? {
        bollard::exec::StartExecResults::Attached { output: mut output_stream, .. } => {
            while let Some(Ok(msg)) = output_stream.next().await {
                match msg {
                    bollard::container::LogOutput::StdOut { message } => {
                        output.push_str(&String::from_utf8_lossy(&message));
                    }
                    bollard::container::LogOutput::StdErr { message } => {
                        output.push_str(&String::from_utf8_lossy(&message));
                    }
                    _ => {}
                }
            }
        }
        bollard::exec::StartExecResults::Detached => {
            return Err("Unexpected detached execution".into());
        }
    }

    Ok(output.trim().to_string())
}

// Helper function to wait for container readiness
async fn wait_for_container_ready(docker: &Docker, container_id: &str) -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
    log::info!("Waiting for container to be ready...");
    
    // Try up to 30 times with 1 second delays (30 seconds total)
    for attempt in 1..=30 {
        match exec_command_in_container(docker, container_id, vec!["echo".to_string(), "ready".to_string()]).await {
            Ok(_) => {
                log::info!("Container is ready after {} attempts", attempt);
                return Ok(());
            }
            Err(e) => {
                log::debug!("Container readiness check failed (attempt {}): {}", attempt, e);
                tokio::time::sleep(tokio::time::Duration::from_secs(1)).await;
            }
        }
    }
    
    Err("Container failed to become ready after 30 seconds".into())
}

// Helper function to install Claude Code via npm
async fn install_claude_code(docker: &Docker, container_id: &str) -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
    log::info!("Starting Claude Code installation...");
    
    let install_command = vec![
        "npm".to_string(),
        "install".to_string(),
        "-g".to_string(),
        "@anthropic-ai/claude-code".to_string()
    ];
    
    match exec_command_in_container(docker, container_id, install_command).await {
        Ok(output) => {
            log::info!("Claude Code installation completed successfully");
            log::debug!("Installation output: {}", output);
            Ok(())
        }
        Err(e) => {
            log::error!("Claude Code installation failed: {}", e);
            Err(format!("Failed to install Claude Code: {}", e).into())
        }
    }
}

// Function to start a new coding session container
async fn start_coding_session(docker: &Docker, container_name: &str) -> Result<String, Box<dyn std::error::Error + Send + Sync>> {
    // First, try to remove any existing container with the same name
    let _ = clear_coding_session(docker, container_name).await;
    
    let options = CreateContainerOptions {
        name: container_name,
        ..Default::default()
    };
    
    let config = Config {
        image: Some("ghcr.io/openai/codex-universal:latest"),
        working_dir: Some("/workspace"),
        tty: Some(true),
        attach_stdin: Some(true),
        attach_stdout: Some(true),
        attach_stderr: Some(true),
        env: Some(vec![
            "CODEX_ENV_PYTHON_VERSION=3.12",
            "CODEX_ENV_NODE_VERSION=20", 
            "CODEX_ENV_RUST_VERSION=1.87.0",
            "CODEX_ENV_GO_VERSION=1.23.8",
            "CODEX_ENV_SWIFT_VERSION=6.1",
        ]),
        cmd: Some(vec!["/bin/bash"]),
        ..Default::default()
    };
    
    let container = docker.create_container(Some(options), config).await?;
    docker.start_container::<String>(&container.id, None).await?;
    
    // Wait for container to be ready
    wait_for_container_ready(docker, &container.id).await?;
    
    // Install Claude Code via npm
    install_claude_code(docker, &container.id).await?;
    
    Ok(container.id)
}

// Function to clear (stop and remove) a coding session container
async fn clear_coding_session(docker: &Docker, container_name: &str) -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
    // Try to stop the container first (ignore errors if it's not running)
    let _ = docker.stop_container(container_name, None).await;
    
    // Remove the container
    let remove_options = RemoveContainerOptions {
        force: true,
        ..Default::default()
    };
    
    match docker.remove_container(container_name, Some(remove_options)).await {
        Ok(()) => Ok(()),
        Err(e) => {
            // If container doesn't exist, that's fine
            if e.to_string().contains("No such container") {
                Ok(())
            } else {
                Err(e.into())
            }
        }
    }
}
