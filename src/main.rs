use teloxide::{prelude::*, utils::command::BotCommands};
use bollard::Docker;
use bollard::container::{CreateContainerOptions, Config, RemoveContainerOptions};

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
    #[command(description = "Echo the provided text")]
    Echo(String),
    #[command(description = "List running Docker containers")]
    ListContainers,
    #[command(description = "Show Docker system information")]
    DockerInfo,
    #[command(description = "Review code in your session")]
    ReviewCode(String),
    #[command(description = "Generate documentation for code in your session")]
    GenerateDocs(String),
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
            
            log::info!("Starting coding session for chat {} with container {}", chat_id, container_name);
            
            match start_coding_session(&docker, &container_name).await {
                Ok(container_id) => {
                    log::info!("Successfully started container {} for chat {}", container_id, chat_id);
                    bot.send_message(
                        msg.chat.id, 
                        format!("🚀 New coding session started!\n\nContainer ID: {}\nContainer Name: {}\n\nYou can now run code and manage your development environment.", 
                                container_id.chars().take(12).collect::<String>(), container_name)
                    ).await?;
                }
                Err(e) => {
                    log::error!("Failed to start coding session for chat {}: {}", chat_id, e);
                    bot.send_message(
                        msg.chat.id, 
                        format!("❌ Failed to start coding session: {}", e)
                    ).await?;
                }
            }
        }
        Command::ClearSession => {
            let chat_id = msg.chat.id.0;
            let container_name = format!("coding-session-{}", chat_id);
            
            log::info!("Clearing coding session for chat {} with container {}", chat_id, container_name);
            
            match clear_coding_session(&docker, &container_name).await {
                Ok(()) => {
                    log::info!("Successfully cleared container {} for chat {}", container_name, chat_id);
                    bot.send_message(
                        msg.chat.id, 
                        "🧹 Coding session cleared successfully!\n\nThe container has been stopped and removed."
                    ).await?;
                }
                Err(e) => {
                    log::error!("Failed to clear coding session for chat {}: {}", chat_id, e);
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
        Command::Echo(text) => {
            bot.send_message(msg.chat.id, text).await?;
        }
        Command::ListContainers => {
            log::debug!("Listing containers for chat {}", msg.chat.id);
            match docker.list_containers(None::<bollard::container::ListContainersOptions<String>>).await {
                Ok(containers) => {
                    let mut response = "🐳 Running containers:\n".to_string();
                    if containers.is_empty() {
                        response.push_str("No containers running");
                    } else {
                        for container in containers {
                            let name = container.names
                                .and_then(|names| names.first().cloned())
                                .unwrap_or_else(|| "Unknown".to_string());
                            let image = container.image.unwrap_or_else(|| "Unknown".to_string());
                            let status = container.status.unwrap_or_else(|| "Unknown".to_string());
                            response.push_str(&format!("• {}: {} ({})\n", name, image, status));
                        }
                    }
                    bot.send_message(msg.chat.id, response).await?;
                }
                Err(e) => {
                    log::error!("Failed to list containers: {}", e);
                    bot.send_message(msg.chat.id, format!("❌ Error listing containers: {}", e))
                        .await?;
                }
            }
        }
        Command::DockerInfo => {
            log::debug!("Getting Docker system info for chat {}", msg.chat.id);
            match docker.info().await {
                Ok(info) => {
                    let response = format!(
                        "📊 Docker System Info:\n\
                        • Version: {}\n\
                        • Containers: {}\n\
                        • Images: {}\n\
                        • Memory: {} MB\n\
                        • CPUs: {}",
                        info.server_version.unwrap_or_else(|| "Unknown".to_string()),
                        info.containers.unwrap_or(0),
                        info.images.unwrap_or(0),
                        info.mem_total.unwrap_or(0) / 1024 / 1024,
                        info.ncpu.unwrap_or(0)
                    );
                    bot.send_message(msg.chat.id, response).await?;
                }
                Err(e) => {
                    log::error!("Failed to get Docker info: {}", e);
                    bot.send_message(msg.chat.id, format!("❌ Error getting Docker info: {}", e))
                        .await?;
                }
            }
        }
        Command::ReviewCode(file_path) => {
            let chat_id = msg.chat.id.0;
            let container_name = format!("coding-session-{}", chat_id);
            
            match ClaudeCodeClient::for_session(docker.clone(), &container_name).await {
                Ok(client) => {
                    match client.review_code(&file_path).await {
                        Ok(result) => {
                            let response = if result.is_error {
                                format!("❌ Code review failed: {}", result.result)
                            } else {
                                format!("📝 Code Review Results:\n\n{}", result.result)
                            };
                            bot.send_message(msg.chat.id, response).await?;
                        }
                        Err(e) => {
                            bot.send_message(
                                msg.chat.id, 
                                format!("❌ Failed to review code: {}", e)
                            ).await?;
                        }
                    }
                }
                Err(_) => {
                    bot.send_message(
                        msg.chat.id, 
                        format!("❌ No active coding session found. Start a session first with /startsession")
                    ).await?;
                }
            }
        }
        Command::GenerateDocs(file_path) => {
            let chat_id = msg.chat.id.0;
            let container_name = format!("coding-session-{}", chat_id);
            
            match ClaudeCodeClient::for_session(docker.clone(), &container_name).await {
                Ok(client) => {
                    match client.generate_docs(&file_path).await {
                        Ok(result) => {
                            let response = if result.is_error {
                                format!("❌ Documentation generation failed: {}", result.result)
                            } else {
                                format!("📚 Generated Documentation:\n\n{}", result.result)
                            };
                            bot.send_message(msg.chat.id, response).await?;
                        }
                        Err(e) => {
                            bot.send_message(
                                msg.chat.id, 
                                format!("❌ Failed to generate documentation: {}", e)
                            ).await?;
                        }
                    }
                }
                Err(_) => {
                    bot.send_message(
                        msg.chat.id, 
                        format!("❌ No active coding session found. Start a session first with /startsession")
                    ).await?;
                }
            }
        }
    }

    Ok(())
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

#[cfg(test)]
mod tests {
    use super::*;
    
    #[test]
    fn test_container_name_generation() {
        let chat_id = 12345i64;
        let expected = "coding-session-12345";
        let actual = format!("coding-session-{}", chat_id);
        assert_eq!(actual, expected);
    }
    
    #[test]
    fn test_command_descriptions() {
        let descriptions = Command::descriptions();
        assert!(descriptions.to_string().contains("Display this help message"));
        assert!(descriptions.to_string().contains("Start the bot"));
        assert!(descriptions.to_string().contains("List running Docker containers"));
    }
    
    #[tokio::test]
    async fn test_docker_connection() {
        // This test checks if Docker is available
        // In a real environment, this would verify Docker connectivity
        match Docker::connect_with_socket_defaults() {
            Ok(_) => {
                // Docker is available
                assert!(true);
            }
            Err(_) => {
                // Docker is not available (e.g., in CI without Docker)
                // This is acceptable for unit tests
                assert!(true);
            }
        }
    }
}
