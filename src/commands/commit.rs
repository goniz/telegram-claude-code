use crate::bot::markdown::escape_markdown_v2;
use crate::BotState;
use telegram_bot::claude_code_client::ClaudeCodeClient;
use teloxide::{prelude::*, types::ParseMode};

/// Handle the /commit command
pub async fn handle_commit(
    bot: Bot,
    msg: Message,
    bot_state: BotState,
    chat_id: i64,
) -> ResponseResult<()> {
    let container_name = format!("coding-session-{}", chat_id);

    // Check if Claude Code client is available
    let client = match ClaudeCodeClient::for_session(bot_state.docker.clone(), &container_name).await {
        Ok(client) => client,
        Err(e) => {
            bot.send_message(
                msg.chat.id,
                format!(
                    "❌ No active coding session found: {}\n\nPlease start a coding session first \
                     using /start",
                    escape_markdown_v2(&e.to_string())
                ),
            )
            .parse_mode(ParseMode::MarkdownV2)
            .await?;
            return Ok(());
        }
    };

    // Send initial message
    bot.send_message(
        msg.chat.id,
        "🔄 *Generating commit message\\.\\.\\.*\n\nChecking git status and diff\\.\\.\\.",
    )
    .parse_mode(ParseMode::MarkdownV2)
    .await?;

    // Get working directory from session state
    let working_directory = {
        let claude_sessions = bot_state.claude_sessions.lock().await;
        claude_sessions
            .get(&chat_id)
            .and_then(|session| session.get_working_directory().cloned())
    };

    let client_with_dir = match ClaudeCodeClient::for_session_with_working_dir(
        bot_state.docker.clone(),
        &container_name,
        working_directory,
    )
    .await {
        Ok(client) => client,
        Err(e) => {
            bot.send_message(
                msg.chat.id,
                format!(
                    "❌ Failed to create client with working directory: {}",
                    escape_markdown_v2(&e.to_string())
                ),
            )
            .parse_mode(ParseMode::MarkdownV2)
            .await?;
            return Ok(());
        }
    };

    // Execute git status to check if there are changes
    let git_status_result = client_with_dir.exec_basic_command(vec!["git".to_string(), "status".to_string(), "--porcelain".to_string()]).await;
    let git_status = match git_status_result {
        Ok(output) if output.trim().is_empty() => {
            bot.send_message(
                msg.chat.id,
                "ℹ️ *No changes to commit*\n\nThe working directory is clean\\.",
            )
            .parse_mode(ParseMode::MarkdownV2)
            .await?;
            return Ok(());
        }
        Ok(output) => output,
        Err(e) => {
            bot.send_message(
                msg.chat.id,
                format!(
                    "❌ *Failed to check git status:*\n```\n{}\n```",
                    escape_markdown_v2(&e.to_string())
                ),
            )
            .parse_mode(ParseMode::MarkdownV2)
            .await?;
            return Ok(());
        }
    };

    // Get git diff
    let git_diff_result = client_with_dir.exec_basic_command(vec!["git".to_string(), "diff".to_string(), "HEAD".to_string()]).await;
    let git_diff = match git_diff_result {
        Ok(output) => {
            if output.trim().is_empty() {
                // Check for staged changes
                match client_with_dir.exec_basic_command(vec!["git".to_string(), "diff".to_string(), "--cached".to_string()]).await {
                    Ok(staged_diff) if !staged_diff.trim().is_empty() => staged_diff,
                    _ => {
                        // No staged or unstaged changes, check for untracked files
                        let untracked_files = git_status
                            .lines()
                            .filter(|line| line.starts_with("??"))
                            .map(|line| line.trim_start_matches("?? "))
                            .collect::<Vec<_>>()
                            .join("\n");
                        
                        if untracked_files.is_empty() {
                            bot.send_message(
                                msg.chat.id,
                                "ℹ️ *No changes to commit*\n\nAll changes are already committed\\.",
                            )
                            .parse_mode(ParseMode::MarkdownV2)
                            .await?;
                            return Ok(());
                        }
                        
                        format!("New untracked files:\n{}", untracked_files)
                    }
                }
            } else {
                output
            }
        }
        Err(e) => {
            bot.send_message(
                msg.chat.id,
                format!(
                    "❌ *Failed to get git diff:*\n```\n{}\n```",
                    escape_markdown_v2(&e.to_string())
                ),
            )
            .parse_mode(ParseMode::MarkdownV2)
            .await?;
            return Ok(());
        }
    };

    // Build prompt for Claude
    let prompt = format!(
        "generate a commit message for the following working state diff:\n\n{}",
        git_diff
    );

    // Generate commit message using Claude
    let claude_result = client_with_dir
        .exec_basic_command(vec![
            "claude".to_string(),
            "--print".to_string(),
            "--model".to_string(),
            "claude-3-5-haiku-20241022".to_string(),
            prompt,
        ])
        .await;

    let commit_message = match claude_result {
        Ok(output) => {
            let generated_message = output.trim();
            if generated_message.is_empty() {
                "Add changes".to_string()
            } else {
                format!("Claude Code Checkpoint: {}", generated_message)
            }
        }
        Err(e) => {
            log::warn!("Failed to generate commit message with Claude: {}", e);
            "Claude Code Checkpoint: Add changes".to_string()
        }
    };

    // Stage all changes
    let stage_result = client_with_dir.exec_basic_command(vec!["git".to_string(), "add".to_string(), "-A".to_string()]).await;
    if let Err(e) = stage_result {
        bot.send_message(
            msg.chat.id,
            format!(
                "❌ *Failed to stage changes:*\n```\n{}\n```",
                escape_markdown_v2(&e.to_string())
            ),
        )
        .parse_mode(ParseMode::MarkdownV2)
        .await?;
        return Ok(());
    }

    // Commit changes
    let commit_result = client_with_dir
        .exec_basic_command(vec![
            "git".to_string(),
            "commit".to_string(),
            "-m".to_string(),
            commit_message.clone(),
        ])
        .await;

    match commit_result {
        Ok(output) => {
            bot.send_message(
                msg.chat.id,
                format!(
                    "✅ *Commit successful\\!*\n\n*Message:*\n```\n{}\n```\n\n*Git output:*\n```\n{}\n```",
                    escape_markdown_v2(&commit_message),
                    escape_markdown_v2(&output)
                ),
            )
            .parse_mode(ParseMode::MarkdownV2)
            .await?;
        }
        Err(e) => {
            bot.send_message(
                msg.chat.id,
                format!(
                    "❌ *Commit failed:*\n```\n{}\n```\n\n*Attempted message:*\n```\n{}\n```",
                    escape_markdown_v2(&e.to_string()),
                    escape_markdown_v2(&commit_message)
                ),
            )
            .parse_mode(ParseMode::MarkdownV2)
            .await?;
        }
    }

    Ok(())
}