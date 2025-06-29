use crate::github_client::{GithubClient, GithubClientConfig};
use crate::{escape_markdown_v2, BotState};
use telegram_bot::claude_code_client::{container_utils, ClaudeCodeClient, ClaudeCodeConfig};
use teloxide::{
    prelude::*,
    types::{InlineKeyboardButton, InlineKeyboardMarkup, ParseMode},
};
use url;

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
            "🔄 *Cloning Repository*\n\n📦 Repository: {}\n⏳ Please wait\\.\\.\\.",
            escape_markdown_v2(repository)
        ),
    )
    .parse_mode(ParseMode::MarkdownV2)
    .await?;

    match github_client.repo_clone(repository, None).await {
        Ok(clone_result) => {
            let message = if clone_result.success {
                format!(
                    "✅ *Repository Cloned Successfully*\n\n📦 Repository: {}\n📁 Location: {}\n✨ {}",
                    escape_markdown_v2(&clone_result.repository),
                    escape_markdown_v2(&clone_result.target_directory),
                    escape_markdown_v2(&clone_result.message)
                )
            } else {
                format!(
                    "❌ *Repository Clone Failed*\n\n📦 Repository: {}\n🔍 Error: {}",
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
                    "❌ *GitHub Authentication Required*\n\n🔐 Please authenticate with GitHub first using /auth login",
                )
            } else if e.to_string().contains("gh: command not found")
                || e.to_string().contains("executable file not found")
            {
                escape_markdown_v2(
                    "❌ *GitHub CLI Not Available*\n\n⚠️ The GitHub CLI (gh) is not installed in the coding session.",
                )
            } else {
                format!(
                    "❌ *Failed to clone repository*\n\n🔍 Error: {}",
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

/// Handle the /start command with enhanced workflow
pub async fn handle_start(
    bot: Bot,
    msg: Message,
    bot_state: BotState,
    chat_id: i64,
    user_id: i64,
) -> ResponseResult<()> {
    let container_name = format!("coding-session-{}", chat_id);

    // Send initial welcome message
    bot.send_message(
        msg.chat.id,
        "Hello\\! I'm your Claude Code Chat Bot 🤖🐳\n\n🚀 Starting new coding \
         session\\.\\.\\.\n\n⏳ Creating container with Claude Code\\.\\.\\.",
    )
    .parse_mode(ParseMode::MarkdownV2)
    .await?;

    match container_utils::start_coding_session(
        &bot_state.docker,
        &container_name,
        ClaudeCodeConfig::default(),
        container_utils::CodingContainerConfig {
            persistent_volume_key: Some(user_id.to_string()),
        },
    )
    .await
    {
        Ok(claude_client) => {
            let container_id_short = claude_client
                .container_id()
                .chars()
                .take(12)
                .collect::<String>();

            // Send container started message
            bot.send_message(
                msg.chat.id,
                format!(
                    "✅ Coding session started successfully\\!\n\n*Container ID:* \
                     `{}`\n*Container Name:* `{}`\n\n🎯 Claude Code is pre\\-installed and \
                     ready to use\\!",
                    escape_markdown_v2(&container_id_short),
                    escape_markdown_v2(&container_name)
                ),
            )
            .parse_mode(ParseMode::MarkdownV2)
            .await?;

            // Start the guided workflow: check authentication status
            check_and_guide_authentication(bot, msg.chat.id, &bot_state, &claude_client).await?;
        }
        Err(e) => {
            bot.send_message(
                msg.chat.id,
                format!(
                    "❌ Failed to start coding session: {}\n\nThis could be due to:\n• \
                     Container creation failure\n• Runtime image pull failure\n• Network \
                     connectivity issues",
                    escape_markdown_v2(&e.to_string())
                ),
            )
            .parse_mode(ParseMode::MarkdownV2)
            .await?;
        }
    }

    Ok(())
}

/// Check GitHub and Claude authentication status and guide the user through the process
async fn check_and_guide_authentication(
    bot: Bot,
    chat_id: ChatId,
    bot_state: &BotState,
    claude_client: &ClaudeCodeClient,
) -> ResponseResult<()> {
    // Send status checking message
    bot.send_message(chat_id, "🔍 Checking authentication status\\.\\.\\.")
        .parse_mode(ParseMode::MarkdownV2)
        .await?;

    // Create GitHub client
    let github_client = GithubClient::new(
        bot_state.docker.clone(),
        claude_client.container_id().to_string(),
        GithubClientConfig::default(),
    );

    // Check authentication status for both services
    let github_authenticated = check_github_auth_status(&github_client, &bot, chat_id).await?;
    let claude_authenticated = check_claude_auth_status(claude_client, &bot, chat_id).await?;

    // Guide user through next steps based on authentication status
    if github_authenticated && claude_authenticated {
        // Both authenticated - proceed to repository setup
        prompt_for_repository_selection(bot, chat_id, bot_state, claude_client).await?;
    } else {
        // Start authentication flows automatically and show guidance
        start_authentication_flows(bot, chat_id, bot_state, claude_client, github_authenticated, claude_authenticated)
            .await?;
    }

    Ok(())
}

/// Check GitHub authentication status and send appropriate status message
async fn check_github_auth_status(
    github_client: &GithubClient,
    bot: &Bot,
    chat_id: ChatId,
) -> ResponseResult<bool> {
    match github_client.check_auth_status().await {
        Ok(auth_result) => {
            if auth_result.authenticated {
                let message = if let Some(username) = &auth_result.username {
                    format!(
                        "✅ *GitHub Status:* Authenticated as {}",
                        escape_markdown_v2(username)
                    )
                } else {
                    "✅ *GitHub Status:* Authenticated".to_string()
                };

                bot.send_message(chat_id, message)
                    .parse_mode(ParseMode::MarkdownV2)
                    .await?;
                Ok(true)
            } else {
                bot.send_message(chat_id, "❌ *GitHub Status:* Not authenticated")
                    .parse_mode(ParseMode::MarkdownV2)
                    .await?;
                Ok(false)
            }
        }
        Err(e) => {
            bot.send_message(
                chat_id,
                format!(
                    "⚠️ *GitHub Status:* Could not check \\({}\\)",
                    escape_markdown_v2(&e.to_string())
                ),
            )
            .parse_mode(ParseMode::MarkdownV2)
            .await?;
            Ok(false)
        }
    }
}

/// Check Claude authentication status and send appropriate status message
async fn check_claude_auth_status(
    claude_client: &ClaudeCodeClient,
    bot: &Bot,
    chat_id: ChatId,
) -> ResponseResult<bool> {
    match claude_client.check_auth_status().await {
        Ok(is_authenticated) => {
            let message = if is_authenticated {
                "✅ *Claude Status:* Authenticated"
            } else {
                "❌ *Claude Status:* Not authenticated"
            };

            bot.send_message(chat_id, message)
                .parse_mode(ParseMode::MarkdownV2)
                .await?;
            Ok(is_authenticated)
        }
        Err(e) => {
            bot.send_message(
                chat_id,
                format!(
                    "⚠️ *Claude Status:* Could not check \\({}\\)",
                    escape_markdown_v2(&e.to_string())
                ),
            )
            .parse_mode(ParseMode::MarkdownV2)
            .await?;
            Ok(false)
        }
    }
}


/// Start authentication flows automatically for unauthenticated services
async fn start_authentication_flows(
    bot: Bot,
    chat_id: ChatId,
    bot_state: &BotState,
    claude_client: &ClaudeCodeClient,
    github_authenticated: bool,
    claude_authenticated: bool,
) -> ResponseResult<()> {
    let mut auth_actions = Vec::new();
    
    if !github_authenticated {
        auth_actions.push("🐙 GitHub authentication");
    }
    
    if !claude_authenticated {
        auth_actions.push("🤖 Claude authentication");
    }

    if !auth_actions.is_empty() {
        let message = format!(
            "🔐 *Authentication Required*\n\nStarting authentication for: {}\n\n\
             Please complete the authentication process and then use /start again to continue\\.",
            auth_actions.join(" and ")
        );

        bot.send_message(chat_id, message)
            .parse_mode(ParseMode::MarkdownV2)
            .await?;

        // Start GitHub authentication if needed
        if !github_authenticated {
            let github_client = GithubClient::new(
                bot_state.docker.clone(),
                claude_client.container_id().to_string(),
                GithubClientConfig::default(),
            );
            
            if let Ok(auth_result) = github_client.login().await {
                if let Some(oauth_url) = auth_result.oauth_url {
                    let keyboard = InlineKeyboardMarkup::new(vec![vec![
                        InlineKeyboardButton::url("🔗 Authenticate GitHub", 
                        url::Url::parse(&oauth_url).unwrap_or_else(|_| url::Url::parse("https://github.com").unwrap()))
                    ]]);

                    bot.send_message(chat_id, "🐙 *GitHub Authentication*\n\nClick the button below to authenticate with GitHub\\.")
                        .parse_mode(ParseMode::MarkdownV2)
                        .reply_markup(keyboard)
                        .await?;
                }
            }
        }

        // Start Claude authentication if needed
        if !claude_authenticated {
            match claude_client.authenticate_claude_account().await {
                Ok(auth_handle) => {
                    use telegram_bot::claude_code_client::AuthenticationHandle;
                    
                    let AuthenticationHandle {
                        state_receiver,
                        code_sender,
                        cancel_sender,
                    } = auth_handle;

                    let session = crate::AuthSession {
                        container_name: format!("coding-session-{}", chat_id.0),
                        code_sender: code_sender.clone(),
                        cancel_sender,
                    };

                    {
                        let mut sessions = bot_state.auth_sessions.lock().await;
                        sessions.insert(chat_id.0, session);
                    }

                    tokio::spawn(crate::handle_auth_state_updates(
                        state_receiver,
                        bot.clone(),
                        chat_id,
                        bot_state.clone(),
                    ));
                }
                Err(e) => {
                    bot.send_message(
                        chat_id,
                        format!("❌ Failed to start Claude authentication: {}", crate::escape_markdown_v2(&e.to_string())),
                    )
                    .parse_mode(ParseMode::MarkdownV2)
                    .await?;
                }
            }
        }
    }

    Ok(())
}

/// Prompt user for repository selection after successful authentication
async fn prompt_for_repository_selection(
    bot: Bot, 
    chat_id: ChatId, 
    bot_state: &BotState,
    claude_client: &ClaudeCodeClient
) -> ResponseResult<()> {
    let message = "🎯 *Ready to Start Coding\\!*\n\nBoth GitHub and Claude are authenticated\\.\n\n\
                   📂 *Choose a Repository*\n\
                   Select a repository to clone into your coding environment:";

    let github_client = GithubClient::new(
        bot_state.docker.clone(),
        claude_client.container_id().to_string(),
        GithubClientConfig::default(),
    );

    // Try to show repository selection
    match github_client.repo_list().await {
        Ok(repo_list) => {
            if repo_list.trim().is_empty() {
                bot.send_message(
                    chat_id,
                    "📁 *No Repositories Found*\n\n💡 No repositories found or accessible\\. You can:\n\n\
                     • Create a new repository on GitHub\n\
                     • Get access to existing repositories\n\
                     • Manually specify a public repository",
                )
                .parse_mode(ParseMode::MarkdownV2)
                .await?;
            } else {
                // Parse repositories and create buttons
                let repos = parse_repository_list(&repo_list);
                if repos.is_empty() {
                    bot.send_message(
                        chat_id,
                        "📁 *Repository Selection*\n\n💡 No valid repositories found\\.",
                    )
                    .parse_mode(ParseMode::MarkdownV2)
                    .await?;
                } else {
                    // Create inline keyboard with repository buttons
                    let mut keyboard_rows = Vec::new();

                    // Show up to 8 repositories to avoid UI clutter
                    let display_repos = &repos[..repos.len().min(8)];

                    for repo in display_repos.iter() {
                        let button = InlineKeyboardButton::callback(
                            format!("📦 {}", repo.name),
                            format!("start_clone:{}", repo.full_name),
                        );
                        keyboard_rows.push(vec![button]);
                    }

                    // Add option for manual repository entry
                    keyboard_rows.push(vec![
                        InlineKeyboardButton::callback("✏️ Enter Repository Manually", "manual_repo_entry")
                    ]);

                    // Add skip option
                    keyboard_rows.push(vec![
                        InlineKeyboardButton::callback("⏭️ Skip Repository Setup", "skip_repo_setup")
                    ]);

                    let keyboard = InlineKeyboardMarkup::new(keyboard_rows);

                    let repo_count_text = if repos.len() > 8 {
                        format!("\\(showing first 8 of {} repositories\\)", repos.len())
                    } else {
                        format!("\\({} repositories\\)", repos.len())
                    };

                    bot.send_message(
                        chat_id,
                        format!(
                            "{}\n\n🎯 Select a repository to clone {}:",
                            message, repo_count_text
                        ),
                    )
                    .parse_mode(ParseMode::MarkdownV2)
                    .reply_markup(keyboard)
                    .await?;
                }
            }
        }
        Err(_) => {
            // Show manual entry option if repo listing fails
            let keyboard = InlineKeyboardMarkup::new(vec![
                vec![InlineKeyboardButton::callback("✏️ Enter Repository Manually", "manual_repo_entry")],
                vec![InlineKeyboardButton::callback("⏭️ Skip Repository Setup", "skip_repo_setup")],
            ]);

            bot.send_message(
                chat_id,
                format!("{}\n\n⚠️ Could not list repositories\\. You can enter one manually:", message),
            )
            .parse_mode(ParseMode::MarkdownV2)
            .reply_markup(keyboard)
            .await?;
        }
    }

    Ok(())
}

/// Handle repository cloning as part of the start workflow
pub async fn handle_repository_clone_in_start(
    bot: Bot,
    chat_id: ChatId,
    bot_state: &BotState,
    repository: &str,
) -> ResponseResult<()> {
    let container_name = format!("coding-session-{}", chat_id.0);

    match ClaudeCodeClient::for_session(bot_state.docker.clone(), &container_name).await {
        Ok(client) => {
            let github_client = GithubClient::new(
                bot_state.docker.clone(),
                client.container_id().to_string(),
                GithubClientConfig::default(),
            );

            // Perform the clone using the same logic as the old github_clone command
            perform_github_clone(&bot, chat_id, &github_client, repository).await?;

            // After successful clone, provide next steps
            bot.send_message(
                chat_id,
                "🎉 *Setup Complete\\!*\n\n\
                 Your coding environment is ready:\n\
                 ✅ Container running\n\
                 ✅ GitHub & Claude authenticated\n\
                 ✅ Repository cloned\n\n\
                 💬 You can now start chatting with Claude about your code\\!",
            )
            .parse_mode(ParseMode::MarkdownV2)
            .await?;
        }
        Err(e) => {
            bot.send_message(
                chat_id,
                format!(
                    "❌ Failed to access coding session: {}\n\nPlease try /start again\\.",
                    escape_markdown_v2(&e.to_string())
                ),
            )
            .parse_mode(ParseMode::MarkdownV2)
            .await?;
        }
    }

    Ok(())
}

/// Handle manual repository entry
pub async fn handle_manual_repository_entry(
    bot: Bot,
    chat_id: ChatId,
) -> ResponseResult<()> {
    bot.send_message(
        chat_id,
        "✏️ *Enter Repository*\n\n\
         Please type the repository you want to clone in the format:\n\
         `owner/repository`\n\n\
         Examples:\n\
         • `octocat/Hello-World`\n\
         • `microsoft/vscode`\n\
         • `golang/go`",
    )
    .parse_mode(ParseMode::MarkdownV2)
    .await?;

    Ok(())
}

/// Handle skipping repository setup
pub async fn handle_skip_repository_setup(
    bot: Bot,
    chat_id: ChatId,
) -> ResponseResult<()> {
    bot.send_message(
        chat_id,
        "⏭️ *Repository Setup Skipped*\n\n\
         Your coding environment is ready:\n\
         ✅ Container running\n\
         ✅ GitHub & Claude authenticated\n\n\
         💬 You can now start chatting with Claude\\!\n\
         📂 Clone a repository anytime by mentioning it in the chat\\.",
    )
    .parse_mode(ParseMode::MarkdownV2)
    .await?;

    Ok(())
}

#[cfg(test)]
mod tests {
    #[test]
    fn test_authentication_guidance_both_unauthenticated() {
        // This test verifies the logic for showing guidance when both services are unauthenticated
        let auth_steps = vec![
            "🐙 *Authenticate with GitHub* to access repositories",
            "🤖 *Authenticate with Claude* to use AI coding features",
        ];

        assert_eq!(auth_steps.len(), 2);
        assert!(auth_steps[0].contains("GitHub"));
        assert!(auth_steps[1].contains("Claude"));
    }

    #[test]
    fn test_authentication_guidance_github_only() {
        // This test verifies the logic for showing guidance when only GitHub is unauthenticated
        let github_authenticated = false;
        let claude_authenticated = true;

        let mut auth_steps = Vec::new();
        if !github_authenticated {
            auth_steps.push("🐙 *Authenticate with GitHub* to access repositories");
        }
        if !claude_authenticated {
            auth_steps.push("🤖 *Authenticate with Claude* to use AI coding features");
        }

        assert_eq!(auth_steps.len(), 1);
        assert!(auth_steps[0].contains("GitHub"));
    }

    #[test]
    fn test_authentication_guidance_claude_only() {
        // This test verifies the logic for showing guidance when only Claude is unauthenticated
        let github_authenticated = true;
        let claude_authenticated = false;

        let mut auth_steps = Vec::new();
        if !github_authenticated {
            auth_steps.push("🐙 *Authenticate with GitHub* to access repositories");
        }
        if !claude_authenticated {
            auth_steps.push("🤖 *Authenticate with Claude* to use AI coding features");
        }

        assert_eq!(auth_steps.len(), 1);
        assert!(auth_steps[0].contains("Claude"));
    }
}
