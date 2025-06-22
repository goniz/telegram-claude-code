use teloxide::{prelude::*, types::ParseMode};
use crate::{
    escape_markdown_v2, BotState, format_repo_list_markdown_v2,
    claude_code_client::{ClaudeCodeClient, GithubClient, GithubClientConfig}
};

/// Handle the /githubrepolist command
pub async fn handle_github_repo_list(
    bot: Bot,
    msg: Message,
    bot_state: BotState,
    chat_id: i64,
) -> ResponseResult<()> {
    let container_name = format!("coding-session-{}", chat_id);

    match ClaudeCodeClient::for_session(bot_state.docker.clone(), &container_name).await {
        Ok(client) => {
            let github_client = GithubClient::new(
                bot_state.docker.clone(),
                client.container_id().to_string(),
                GithubClientConfig::default(),
            );

            match github_client.repo_list().await {
                Ok(repo_list) => {
                    if repo_list.trim().is_empty() {
                        bot.send_message(
                            msg.chat.id,
                            "📁 *GitHub Repository List*\n\n💡 No repositories found or no \
                             repositories accessible with current authentication\\."
                                .to_string(),
                        )
                        .parse_mode(ParseMode::MarkdownV2)
                        .await?;
                    } else {
                        let formatted_repo_list = format_repo_list_markdown_v2(&repo_list);
                        bot.send_message(
                            msg.chat.id,
                            format!("📁 *GitHub Repository List*\n\n{}", formatted_repo_list),
                        )
                        .parse_mode(ParseMode::MarkdownV2)
                        .await?;
                    }
                }
                Err(e) => {
                    let error_message = if e.to_string().contains("authentication required")
                        || e.to_string().contains("not authenticated")
                    {
                        "❌ *GitHub Authentication Required*\n\n🔐 Please authenticate with GitHub \
                         first using /githubauth"
                    } else if e.to_string().contains("gh: command not found")
                        || e.to_string().contains("executable file not found")
                    {
                        "❌ *GitHub CLI Not Available*\n\n⚠️ The GitHub CLI \\(gh\\) is not \
                         installed in the coding session\\."
                    } else {
                        &format!(
                            "❌ *Failed to list repositories*\n\n🔍 Error: {}",
                            escape_markdown_v2(&e.to_string())
                        )
                    };

                    bot.send_message(msg.chat.id, error_message)
                        .parse_mode(ParseMode::MarkdownV2)
                        .await?;
                }
            }
        }
        Err(e) => {
            bot.send_message(
                msg.chat.id,
                format!(
                    "❌ No active coding session found: {}\\n\\nPlease start a coding session \
                     first using /start",
                    escape_markdown_v2(&e.to_string())
                ),
            )
            .parse_mode(ParseMode::MarkdownV2)
            .await?;
        }
    }

    Ok(())
}