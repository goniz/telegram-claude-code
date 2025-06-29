use bollard::image::CreateImageOptions;
use bollard::Docker;
use futures_util::StreamExt;
use std::collections::HashMap;
use std::sync::Arc;
use teloxide::{dispatching::UpdateFilterExt, dptree, prelude::*, utils::command::BotCommands};
use tokio::sync::Mutex;

mod bot;
mod commands;
mod github_client;

use bot::{
    escape_markdown_v2, handle_auth_state_updates, handle_callback_query, handle_text_message,
    AuthSession, AuthSessions, BotState, ClaudeSessions,
};
use telegram_bot::claude_code_client::container_utils;

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
    #[command(description = "Unified authentication management for GitHub and Claude")]
    Auth(String),
    #[command(description = "List GitHub repositories for the authenticated user")]
    GitHubRepoList,
    #[command(description = "Clone a GitHub repository")]
    GitHubClone(String),
    #[command(description = "Update Claude CLI to latest version")]
    UpdateClaude,
    #[command(description = "Start a new Claude conversation")]
    Claude,
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
    let claude_sessions: ClaudeSessions = Arc::new(Mutex::new(HashMap::new()));
    let bot_state = BotState {
        docker: docker.clone(),
        auth_sessions: auth_sessions.clone(),
        claude_sessions: claude_sessions.clone(),
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
        .branch(Update::filter_callback_query().endpoint(move |bot, query| {
            let bot_state = bot_state_clone3.clone();
            handle_callback_query(bot, query, bot_state)
        }));

    Dispatcher::builder(bot, handler)
        .enable_ctrlc_handler()
        .build()
        .dispatch()
        .await;
}

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
        Command::Auth(args) => {
            let auth_args = if args.trim().is_empty() {
                None
            } else {
                Some(args)
            };
            commands::handle_auth(bot, msg, bot_state, chat_id, auth_args).await?;
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
        Command::UpdateClaude => {
            commands::handle_update_claude(bot, msg, bot_state, chat_id).await?;
        }
        Command::Claude => {
            commands::handle_claude(bot, msg, bot_state, chat_id).await?;
        }
    }

    Ok(())
}
