use crate::{escape_markdown_v2, BotState};
use telegram_bot::claude_code_client::{ClaudeCodeClient, GithubClient, GithubClientConfig};
use teloxide::{
    prelude::*,
    types::{InlineKeyboardButton, InlineKeyboardMarkup, ParseMode},
};

/// Parse repository list into structured data
#[derive(Debug)]
pub struct Repository {
    pub full_name: String,
    pub name: String,
}

pub fn parse_repository_list(repo_list: &str) -> Vec<Repository> {
    let lines: Vec<&str> = repo_list.trim().lines().collect();
    let mut repos = Vec::new();

    for line in lines {
        let line = line.trim();
        if line.is_empty() {
            continue;
        }

        // gh repo list output format is typically: "repo_name    description"
        // Split by whitespace and take the first part as the repo name
        let parts: Vec<&str> = line.split_whitespace().collect();
        if let Some(full_name) = parts.first() {
            let name = full_name.split('/').last().unwrap_or(full_name).to_string();

            repos.push(Repository {
                full_name: full_name.to_string(),
                name,
            });
        }
    }

    repos
}

/// Perform the actual GitHub clone operation
pub async fn perform_github_clone(
    bot: &Bot,
    chat_id: teloxide::types::ChatId,
    github_client: &GithubClient,
    repository: &str,
) -> ResponseResult<()> {
    bot.send_message(
        chat_id,
        format!(
            "🔄 *Cloning Repository*\\n\\n📦 Repository: {}\\n⏳ Please wait\\.\\.\\.",
            escape_markdown_v2(repository)
        ),
    )
    .parse_mode(ParseMode::MarkdownV2)
    .await?;

    match github_client.repo_clone(repository, None).await {
        Ok(clone_result) => {
            let message = if clone_result.success {
                format!(
                    "✅ *Repository Cloned Successfully*\\n\\n📦 Repository: {}\\n📁 Location: {}\\n✨ {}",
                    escape_markdown_v2(&clone_result.repository),
                    escape_markdown_v2(&clone_result.target_directory),
                    escape_markdown_v2(&clone_result.message)
                )
            } else {
                format!(
                    "❌ *Repository Clone Failed*\\n\\n📦 Repository: {}\\n🔍 Error: {}",
                    escape_markdown_v2(&clone_result.repository),
                    escape_markdown_v2(&clone_result.message)
                )
            };

            bot.send_message(chat_id, message)
                .parse_mode(ParseMode::MarkdownV2)
                .await?;
        }
        Err(e) => {
            let error_message = if e.to_string().contains("authentication required")
                || e.to_string().contains("not authenticated")
            {
                escape_markdown_v2(
                    "❌ *GitHub Authentication Required*\n\n🔐 Please authenticate with GitHub first using /githubauth",
                )
            } else if e.to_string().contains("gh: command not found")
                || e.to_string().contains("executable file not found")
            {
                escape_markdown_v2(
                    "❌ *GitHub CLI Not Available*\n\n⚠️ The GitHub CLI (gh) is not installed in the coding session.",
                )
            } else {
                format!(
                    "❌ *Failed to clone repository*\\n\\n🔍 Error: {}",
                    escape_markdown_v2(&e.to_string())
                )
            };

            bot.send_message(chat_id, error_message)
                .parse_mode(ParseMode::MarkdownV2)
                .await?;
        }
    }

    Ok(())
}

/// Show repository selection with clickable buttons
pub async fn show_repository_selection(
    bot: &Bot,
    msg: &Message,
    github_client: &GithubClient,
) -> ResponseResult<()> {
    match github_client.repo_list().await {
        Ok(repo_list) => {
            if repo_list.trim().is_empty() {
                bot.send_message(
                    msg.chat.id,
                    "📁 *GitHub Repository Selection*\\n\\n💡 No repositories found or no \
                     repositories accessible with current authentication\\\\.",
                )
                .parse_mode(ParseMode::MarkdownV2)
                .await?;
            } else {
                // Parse repositories and create buttons
                let repos = parse_repository_list(&repo_list);
                if repos.is_empty() {
                    bot.send_message(
                        msg.chat.id,
                        "📁 *GitHub Repository Selection*\\n\\n💡 No valid repositories found\\\\.",
                    )
                    .parse_mode(ParseMode::MarkdownV2)
                    .await?;
                } else {
                    // Create inline keyboard with repository buttons
                    let mut keyboard_rows = Vec::new();

                    // Show up to 10 repositories to avoid UI clutter
                    let display_repos = &repos[..repos.len().min(10)];

                    for repo in display_repos.iter() {
                        let button = InlineKeyboardButton::callback(
                            format!("📦 {}", repo.name),
                            format!("clone:{}", repo.full_name),
                        );
                        keyboard_rows.push(vec![button]);
                    }

                    let keyboard = InlineKeyboardMarkup::new(keyboard_rows);

                    let repo_count_text = if repos.len() > 10 {
                        format!("\\(showing first 10 of {} repositories\\)", repos.len())
                    } else {
                        format!("\\({} repositories\\)", repos.len())
                    };

                    bot.send_message(
                        msg.chat.id,
                        format!(
                            "📁 *GitHub Repository Selection*\\n\\n🎯 Select a repository to clone {}\\n\\n💡 Click a repository button below to clone it:",
                            repo_count_text
                        ),
                    )
                    .parse_mode(ParseMode::MarkdownV2)
                    .reply_markup(keyboard)
                    .await?;
                }
            }
        }
        Err(e) => {
            let error_message = if e.to_string().contains("authentication required")
                || e.to_string().contains("not authenticated")
            {
                "❌ *GitHub Authentication Required*\\n\\n🔐 Please authenticate with GitHub \
                 first using /githubauth"
            } else if e.to_string().contains("gh: command not found")
                || e.to_string().contains("executable file not found")
            {
                "❌ *GitHub CLI Not Available*\\n\\n⚠️ The GitHub CLI \\\\(gh\\\\) is not \
                 installed in the coding session\\\\."
            } else {
                &format!(
                    "❌ *Failed to list repositories*\\n\\n🔍 Error: {}",
                    escape_markdown_v2(&e.to_string())
                )
            };

            bot.send_message(msg.chat.id, error_message)
                .parse_mode(ParseMode::MarkdownV2)
                .await?;
        }
    }

    Ok(())
}

/// Handle the /githubclone command
pub async fn handle_github_clone(
    bot: Bot,
    msg: Message,
    bot_state: BotState,
    chat_id: i64,
    repository: Option<String>,
) -> ResponseResult<()> {
    let container_name = format!("coding-session-{}", chat_id);

    match ClaudeCodeClient::for_session(bot_state.docker.clone(), &container_name).await {
        Ok(client) => {
            let github_client = GithubClient::new(
                bot_state.docker.clone(),
                client.container_id().to_string(),
                GithubClientConfig::default(),
            );

            if let Some(repo) = repository {
                // Direct clone with provided repository name
                perform_github_clone(&bot, msg.chat.id, &github_client, &repo).await?;
            } else {
                // Show repository selection UI
                show_repository_selection(&bot, &msg, &github_client).await?;
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
#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_parse_repository_list_single_repo() {
        let repo_list = "owner/repo1\tFirst repository";
        let repos = parse_repository_list(repo_list);

        assert_eq!(repos.len(), 1);
        assert_eq!(repos[0].full_name, "owner/repo1");
        assert_eq!(repos[0].name, "repo1");
    }

    #[test]
    fn test_parse_repository_list_multiple_repos() {
        let repo_list =
            "owner/repo1\tFirst repository\nowner/repo2\tSecond repository\nowner/repo3";
        let repos = parse_repository_list(repo_list);

        assert_eq!(repos.len(), 3);
        assert_eq!(repos[0].full_name, "owner/repo1");
        assert_eq!(repos[0].name, "repo1");
        assert_eq!(repos[1].full_name, "owner/repo2");
        assert_eq!(repos[1].name, "repo2");
        assert_eq!(repos[2].full_name, "owner/repo3");
        assert_eq!(repos[2].name, "repo3");
    }

    #[test]
    fn test_parse_repository_list_empty_input() {
        let repo_list = "";
        let repos = parse_repository_list(repo_list);

        assert_eq!(repos.len(), 0);
    }

    #[test]
    fn test_parse_repository_list_whitespace_only() {
        let repo_list = "   \n\t\n   ";
        let repos = parse_repository_list(repo_list);

        assert_eq!(repos.len(), 0);
    }

    #[test]
    fn test_parse_repository_list_mixed_formatting() {
        let repo_list =
            "owner/project1\tDescription 1\nowner/project2    Description 2\nowner/project3";
        let repos = parse_repository_list(repo_list);

        assert_eq!(repos.len(), 3);
        assert_eq!(repos[0].full_name, "owner/project1");
        assert_eq!(repos[0].name, "project1");
        assert_eq!(repos[1].full_name, "owner/project2");
        assert_eq!(repos[1].name, "project2");
        assert_eq!(repos[2].full_name, "owner/project3");
        assert_eq!(repos[2].name, "project3");
    }
}
