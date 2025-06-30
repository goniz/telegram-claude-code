use crate::bot::markdown::{escape_markdown_v2, truncate_if_needed};
use crate::BotState;
use telegram_bot::claude_code_client::ClaudeCodeClient;
use teloxide::{prelude::*, types::ParseMode};
use teloxide::types::ChatId;
use std::error::Error;

/// Handle the /commit command
pub async fn handle_commit(
    bot: Bot,
    msg: Message,
    bot_state: BotState,
    chat_id: i64,
) -> ResponseResult<()> {
    let container_name = format!("coding-session-{}", chat_id);

    // 1. Validate session
    if !session_exists(&bot_state, &container_name).await {
        send_md(
            &bot,
            msg.chat.id,
            "❌ No active coding session found\\.\n\nPlease start a coding session first using /start",
        )
        .await?;
        return Ok(());
    }

    // 2. Inform the user that we started processing.
    send_md(
        &bot,
        msg.chat.id,
        "🔄 *Generating commit message\\.\\.\\.*\n\nChecking git status and diff\\.\\.\\.",
    )
    .await?;

    // 3. Retrieve working directory stored in session state (if any).
    let working_directory = {
        let claude_sessions = bot_state.claude_sessions.lock().await;
        claude_sessions
            .get(&chat_id)
            .and_then(|s| s.get_working_directory().cloned())
    };

    // 4. Create Claude client scoped to working directory.
    let client = match get_claude_client(bot_state.clone(), &container_name, working_directory).await {
        Ok(c) => c,
        Err(e) => {
            send_md(&bot, msg.chat.id, error_block("❌ Failed to create client with working directory:", &e)).await?;
            return Ok(());
        }
    };

    // 5. Compute git diff, early-return if no changes.
    let git_diff = match get_git_diff(&client).await {
        Ok(Some(d)) => d,
        Ok(None) => {
            send_md(&bot, msg.chat.id, "ℹ️ *No changes to commit*\n\nThe working directory is clean\\.").await?;
            return Ok(());
        }
        Err(e) => {
            send_md(&bot, msg.chat.id, error_block("❌ *Failed to get git diff:*", &e)).await?;
            return Ok(());
        }
    };

    // 6. Generate commit message with Claude.
    let commit_message = generate_commit_message(&client, &git_diff).await;

    // 7. Stage changes.
    if let Err(e) = stage_changes(&client).await {
        send_md(&bot, msg.chat.id, error_block("❌ *Failed to stage changes:*", &e)).await?;
        return Ok(());
    }

    // 8. Commit.
    match commit_changes(&client, &commit_message).await {
        Ok(output) => {
            send_md(
                &bot,
                msg.chat.id,
                format!(
                    "✅ *Commit successful\\!*\n\n*Message:*\n```\n{}\n```\n\n*Git output:*\n```\n{}\n```",
                    escape_markdown_v2(&commit_message),
                    escape_markdown_v2(&output)
                ),
            )
            .await?;
        }
        Err(e) => {
            send_md(
                &bot,
                msg.chat.id,
                format!(
                    "❌ *Commit failed:*\n```\n{}\n```\n\n*Attempted message:*\n```\n{}\n```",
                    escape_markdown_v2(&e.to_string()),
                    escape_markdown_v2(&commit_message)
                ),
            )
            .await?;
        }
    }

    Ok(())
}

/// Helper: send a MarkdownV2-formatted message, automatically truncating if needed.
async fn send_md<B: AsRef<str>>(bot: &Bot, chat_id: ChatId, text: B) -> ResponseResult<()> {
    let (message_to_send, _) = truncate_if_needed(text.as_ref());
    bot.send_message(chat_id, message_to_send)
        .parse_mode(ParseMode::MarkdownV2)
        .await?;
    Ok(())
}

/// Helper: format an error block with triple-backtick fencing and MarkdownV2 escaping.
fn error_block(prefix: &str, err: &dyn std::fmt::Display) -> String {
    format!(
        "{}\n```\n{}\n```",
        prefix,
        escape_markdown_v2(&err.to_string())
    )
}

/// Check that a coding session container exists.
async fn session_exists(bot_state: &BotState, container_name: &str) -> bool {
    ClaudeCodeClient::for_session(bot_state.docker.clone(), container_name)
        .await
        .is_ok()
}

/// Create a Claude client scoped to the (optional) working directory.
async fn get_claude_client(
    bot_state: BotState,
    container_name: &str,
    working_directory: Option<String>,
) -> Result<ClaudeCodeClient, Box<dyn Error + Send + Sync>> {
    ClaudeCodeClient::for_session_with_working_dir(
        bot_state.docker.clone(),
        container_name,
        working_directory,
    )
    .await
}

/// Retrieve a meaningful Git diff, or `None` if there are no changes worth committing.
async fn get_git_diff(
    client: &ClaudeCodeClient,
) -> Result<Option<String>, Box<dyn Error + Send + Sync>> {
    let git_status = client
        .exec_basic_command(vec![
            "git".to_string(),
            "status".to_string(),
            "--porcelain".to_string(),
        ])
        .await?;

    if git_status.trim().is_empty() {
        return Ok(None);
    }

    // Unstaged diff first.
    let diff_head = client
        .exec_basic_command(vec!["git".into(), "diff".into(), "HEAD".into()])
        .await?;

    let diff = if diff_head.trim().is_empty() {
        // Staged diff.
        let staged = client
            .exec_basic_command(vec!["git".into(), "diff".into(), "--cached".into()])
            .await?;

        if !staged.trim().is_empty() {
            staged
        } else {
            // Untracked files.
            let untracked: String = git_status
                .lines()
                .filter(|l| l.starts_with("??"))
                .map(|l| l.trim_start_matches("?? "))
                .collect::<Vec<_>>()
                .join("\n");

            if untracked.is_empty() {
                return Ok(None);
            }
            format!("New untracked files:\n{}", untracked)
        }
    } else {
        diff_head
    };

    Ok(Some(diff))
}

/// Use Claude to generate a commit message from the given diff.
async fn generate_commit_message(
    client: &ClaudeCodeClient,
    git_diff: &str,
) -> String {
    let prompt = format!(
        "generate a commit message for the following working state diff:\n\n{}",
        git_diff
    );

    match client
        .exec_basic_command(vec![
            "claude".into(),
            "--print".into(),
            "--model".into(),
            "claude-3-5-haiku-20241022".into(),
            prompt,
        ])
        .await
    {
        Ok(output) => {
            let msg = output.trim();
            if msg.is_empty() {
                "Claude Code Checkpoint: Add changes".into()
            } else {
                format!("Claude Code Checkpoint: {}", msg)
            }
        }
        Err(e) => {
            log::warn!("Failed to generate commit message with Claude: {}", e);
            "Claude Code Checkpoint: Add changes".into()
        }
    }
}

/// Stage all changes (git add -A).
async fn stage_changes(client: &ClaudeCodeClient) -> Result<(), Box<dyn Error + Send + Sync>> {
    client
        .exec_basic_command(vec!["git".into(), "add".into(), "-A".into()])
        .await
        .map(|_| ())
}

/// Commit using the provided commit message and return git output.
async fn commit_changes(
    client: &ClaudeCodeClient,
    message: &str,
) -> Result<String, Box<dyn Error + Send + Sync>> {
    client
        .exec_basic_command(vec![
            "git".into(),
            "commit".into(),
            "-m".into(),
            message.into(),
        ])
        .await
}