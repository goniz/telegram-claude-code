use crate::{BotState, escape_markdown_v2};
use telegram_bot::claude_code_client::{ClaudeCodeClient, GithubClient, GithubClientConfig};
use teloxide::{prelude::*, types::ParseMode};

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

/// Format repository list for display in MarkdownV2 format
fn format_repo_list_markdown_v2(repo_list: &str) -> String {
    let lines: Vec<&str> = repo_list.trim().lines().collect();
    if lines.is_empty() {
        return "💡 No repositories found or no repositories accessible with current \
                authentication\\."
            .to_string();
    }

    let mut formatted_repos = Vec::new();

    for line in lines {
        let line = line.trim();
        if line.is_empty() {
            continue;
        }

        // gh repo list output format is typically: "repo_name    description"
        // Split by whitespace and take the first part as the repo name
        let parts: Vec<&str> = line.split_whitespace().collect();
        if let Some(repo_name) = parts.first() {
            // Create GitHub URL for the repository
            let github_url = format!("https://github.com/{}", repo_name);

            // Extract description (everything after the first whitespace-separated token)
            let description = if parts.len() > 1 {
                parts[1..].join(" ")
            } else {
                String::new()
            };

            // Create MarkdownV2 list item with hyperlink
            // Format: • [repo_name](https://github.com/repo_name) - description
            let escaped_repo_name = escape_markdown_v2(repo_name);
            let escaped_description = if !description.is_empty() {
                format!(" \\- {}", escape_markdown_v2(&description))
            } else {
                String::new()
            };

            let formatted_item = format!(
                "• [{}]({}){}",
                escaped_repo_name, github_url, escaped_description
            );
            formatted_repos.push(formatted_item);
        }
    }

    if formatted_repos.is_empty() {
        "💡 No repositories found or no repositories accessible with current authentication\\."
            .to_string()
    } else {
        formatted_repos.join("\n")
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_format_repo_list_markdown_v2_empty_input() {
        assert_eq!(
            format_repo_list_markdown_v2(""),
            "💡 No repositories found or no repositories accessible with current authentication\\."
        );
        assert_eq!(
            format_repo_list_markdown_v2("   \n  \n  "),
            "💡 No repositories found or no repositories accessible with current authentication\\."
        );
    }

    #[test]
    fn test_format_repo_list_markdown_v2_single_repo() {
        let input = "user/repo1\tA sample repository";
        let expected = "• [user/repo1](https://github.com/user/repo1) \\- A sample repository";
        assert_eq!(format_repo_list_markdown_v2(input), expected);
    }

    #[test]
    fn test_format_repo_list_markdown_v2_single_repo_no_description() {
        let input = "user/repo1";
        let expected = "• [user/repo1](https://github.com/user/repo1)";
        assert_eq!(format_repo_list_markdown_v2(input), expected);
    }

    #[test]
    fn test_format_repo_list_markdown_v2_multiple_repos() {
        let input = "user/repo1\tFirst repository\nuser/repo2\tSecond repository";
        let expected = "• [user/repo1](https://github.com/user/repo1) \\- First repository\n• [user/repo2](https://github.com/user/repo2) \\- Second repository";
        assert_eq!(format_repo_list_markdown_v2(input), expected);
    }

    #[test]
    fn test_format_repo_list_markdown_v2_with_special_characters() {
        let input = "user/repo-test\tRepository with special chars: [test] (v1.0)";
        let expected = "• [user/repo\\-test](https://github.com/user/repo-test) \\- Repository \
                        with special chars: \\[test\\] \\(v1\\.0\\)";
        assert_eq!(format_repo_list_markdown_v2(input), expected);
    }

    #[test]
    fn test_format_repo_list_markdown_v2_space_separated() {
        let input = "user/repo1    First repository with spaces";
        let expected =
            "• [user/repo1](https://github.com/user/repo1) \\- First repository with spaces";
        assert_eq!(format_repo_list_markdown_v2(input), expected);
    }

    #[test]
    fn test_format_repo_list_markdown_v2_mixed_formatting() {
        let input =
            "owner/project1\tDescription 1\nowner/project2    Description 2\nowner/project3";
        let expected = "• [owner/project1](https://github.com/owner/project1) \\- Description 1\n• [owner/project2](https://github.com/owner/project2) \\- Description 2\n• [owner/project3](https://github.com/owner/project3)";
        assert_eq!(format_repo_list_markdown_v2(input), expected);
    }
}
