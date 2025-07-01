use crate::bot::markdown::{escape_markdown_v2, truncate_if_needed};
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

    // Check if a coding session exists by trying to create a basic client
    if ClaudeCodeClient::for_session(bot_state.docker.clone(), &container_name).await.is_err() {
        bot.send_message(
            msg.chat.id,
            "❌ No active coding session found\\.\n\nPlease start a coding session first using /start",
        )
        .parse_mode(ParseMode::MarkdownV2)
        .await?;
        return Ok(());
    }

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
            let full_message = format!(
                "❌ Failed to create client with working directory: {}",
                escape_markdown_v2(&e.to_string())
            );
            let (message_to_send, _was_truncated) = truncate_if_needed(&full_message);
            
            bot.send_message(msg.chat.id, message_to_send)
                .parse_mode(ParseMode::MarkdownV2)
                .await?;
            return Ok(());
        }
    };

    // Execute `git status --porcelain` up-front so we can later inspect for untracked files
    let git_status_raw = match client_with_dir
        .exec_basic_command(vec![
            "git".to_string(),
            "status".to_string(),
            "--porcelain".to_string(),
        ])
        .await
    {
        Ok(out) => out,
        Err(e) => {
            let full_message = format!(
                "❌ *Failed to check git status:*\n```\n{}\n```",
                escape_markdown_v2(&e.to_string())
            );
            let (message_to_send, _) = truncate_if_needed(&full_message);
            bot.send_message(msg.chat.id, message_to_send)
                .parse_mode(ParseMode::MarkdownV2)
                .await?;
            return Ok(());
        }
    };

    // If absolutely nothing reported by porcelain, we can early-exit.
    if git_status_raw.trim().is_empty() {
        bot.send_message(
            msg.chat.id,
            "ℹ️ *No changes to commit*\n\nThe working directory is clean\\.",
        )
        .parse_mode(ParseMode::MarkdownV2)
        .await?;
        return Ok(());
    }

    // Otherwise, attempt to collect a meaningful diff / file list.
    let git_diff_opt = get_git_diff(&client_with_dir, &git_status_raw).await;

    let git_diff = match git_diff_opt {
        Ok(Some(diff)) => diff,
        Ok(None) => {
            // We had a non-empty `git status`, but nothing to commit.
            bot.send_message(
                msg.chat.id,
                "ℹ️ *No changes to commit*\n\nAll changes are already committed\\.",
            )
            .parse_mode(ParseMode::MarkdownV2)
            .await?;
            return Ok(());
        }
        Err(e) => {
            let full_message = format!(
                "❌ *Failed to get git diff:*\n```\n{}\n```",
                escape_markdown_v2(&e.to_string())
            );
            let (message_to_send, _) = truncate_if_needed(&full_message);
            bot.send_message(msg.chat.id, message_to_send)
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
            // If Claude returned an *empty* string we fall back to a plain
            // "Add changes" message *without* the checkpoint prefix.  We only
            // attach the "Claude Code Checkpoint:" prefix when Claude
            // produced a non-empty summary.
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
        let full_message = format!(
            "❌ *Failed to stage changes:*\n```\n{}\n```",
            escape_markdown_v2(&e.to_string())
        );
        let (message_to_send, _was_truncated) = truncate_if_needed(&full_message);
        
        bot.send_message(msg.chat.id, message_to_send)
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
            let full_message = format!(
                "✅ *Commit successful\\!*\n\n*Message:*\n```\n{}\n```\n\n*Git output:*\n```\n{}\n```",
                escape_markdown_v2(&commit_message),
                escape_markdown_v2(&output)
            );
            let (message_to_send, _was_truncated) = truncate_if_needed(&full_message);
            
            bot.send_message(msg.chat.id, message_to_send)
                .parse_mode(ParseMode::MarkdownV2)
                .await?;
        }
        Err(e) => {
            let full_message = format!(
                "❌ *Commit failed:*\n```\n{}\n```\n\n*Attempted message:*\n```\n{}\n```",
                escape_markdown_v2(&e.to_string()),
                escape_markdown_v2(&commit_message)
            );
            let (message_to_send, _was_truncated) = truncate_if_needed(&full_message);
            
            bot.send_message(msg.chat.id, message_to_send)
                .parse_mode(ParseMode::MarkdownV2)
                .await?;
        }
    }

    Ok(())
}

/// Determine the effective diff for the current working directory.
/// 
/// Returns:
/// * `Ok(Some(diff))` – when there is a meaningful diff (unstaged / staged / new files).
/// * `Ok(None)` – when there are *no* changes that need committing (already committed).
/// * `Err(_)` – if a fatal error occurs when running the git commands.
///
/// The function tries, in order:
/// 1. `git diff HEAD` – regular unstaged changes.
/// 2. `git diff --cached` – staged changes (even if the above is empty).
/// 3. Looks for untracked files from the previously-retrieved `git status --porcelain` output.
///
/// The `git diff --cached` command *may* exit with a non-zero status if there are no commits yet.
/// In that case we treat it the same as an empty diff instead of bubbling the error up – this
/// replicates the fallback behaviour that existed before the regression noted in PR review.
async fn get_git_diff(
    client: &ClaudeCodeClient,
    git_status_raw: &str,
) -> Result<Option<String>, anyhow::Error> {
    // 1. Unstaged diff
    let head_diff = client
        .exec_basic_command(vec![
            "git".into(),
            "diff".into(),
            "HEAD".into(),
        ])
        .await
        .unwrap_or_default(); // non-critical – treat failure like empty diff

    if !head_diff.trim().is_empty() {
        return Ok(Some(head_diff));
    }

    // 2. Staged diff – allow error fall-through (e.g., fresh repo with no commits)
    if let Ok(staged_diff) = client
        .exec_basic_command(vec![
            "git".into(),
            "diff".into(),
            "--cached".into(),
        ])
        .await
    {
        if !staged_diff.trim().is_empty() {
            return Ok(Some(staged_diff));
        }
    }

    // 3. Untracked files
    let untracked_files: Vec<_> = git_status_raw
        .lines()
        .filter(|l| l.starts_with("??"))
        .map(|l| l.trim_start_matches("?? ").to_string())
        .collect();

    if !untracked_files.is_empty() {
        let list = untracked_files.join("\n");
        return Ok(Some(format!("New untracked files:\n{}", list)));
    }

    // Nothing to commit
    Ok(None)
}