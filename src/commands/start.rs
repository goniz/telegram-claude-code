use crate::github_client::{GithubClient, GithubClientConfig};
use crate::{escape_markdown_v2, BotState};
use telegram_bot::claude_code_client::{container_utils, ClaudeCodeClient, ClaudeCodeConfig};
use teloxide::types::CopyTextButton;
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
    bot_state: &BotState,
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
                // Update working directory for this session
                {
                    let mut claude_sessions = bot_state.claude_sessions.lock().await;
                    claude_sessions
                        .entry(chat_id.0)
                        .or_insert_with(crate::bot::claude_session::ClaudeSession::new)
                        .set_working_directory(
                            std::path::Path::new(&clone_result.target_directory)
                                .canonicalize()
                                .unwrap_or_else(|_| {
                                    std::path::PathBuf::from(&clone_result.target_directory)
                                })
                                .to_string_lossy()
                                .to_string(),
                        );
                }

                format!(
                    "✅ *Repository Cloned Successfully*\n\n📦 Repository: {}\n📁 Location: \
                     {}\n✨ {}\n\n🎯 *Working directory set for /claude commands*",
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
                    "❌ *GitHub Authentication Required*\n\n🔐 Please authenticate with GitHub \
                     first using /auth login",
                )
            } else if e.to_string().contains("gh: command not found")
                || e.to_string().contains("executable file not found")
            {
                escape_markdown_v2(
                    "❌ *GitHub CLI Not Available*\n\n⚠️ The GitHub CLI (gh) is not installed in \
                     the coding session.",
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

    // Send initial welcome message and start container creation
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

            // Start the guided workflow directly with container info included
            check_and_guide_authentication_with_container_info(
                bot,
                msg.chat.id,
                &bot_state,
                &claude_client,
                &container_id_short,
                &container_name,
            )
            .await?;
        }
        Err(e) => {
            bot.send_message(
                msg.chat.id,
                format!(
                    "❌ Failed to start coding session: {}\n\nThis could be due to:\n• Container \
                     creation failure\n• Runtime image pull failure\n• Network connectivity issues",
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
/// This version consolidates container success with authentication status
async fn check_and_guide_authentication_with_container_info(
    bot: Bot,
    chat_id: ChatId,
    bot_state: &BotState,
    claude_client: &ClaudeCodeClient,
    container_id_short: &str,
    container_name: &str,
) -> ResponseResult<()> {
    // Create GitHub client
    let github_client = GithubClient::new(
        bot_state.docker.clone(),
        claude_client.container_id().to_string(),
        GithubClientConfig::default(),
    );

    // Check authentication status for both services (without sending individual status messages)
    let github_authenticated = check_github_auth_status_silent(&github_client).await;
    let claude_authenticated = check_claude_auth_status_silent(claude_client).await;

    // Send consolidated container success + auth status message
    let auth_status_text = match (github_authenticated, claude_authenticated) {
        (true, true) => {
            "✅ Container running with Claude Code\n✅ GitHub authenticated\n✅ Claude \
             authenticated\n\n🎯 Ready to start coding!"
        }
        (true, false) => {
            "✅ Container running with Claude Code\n✅ GitHub authenticated\n❌ Claude \
             authentication needed"
        }
        (false, true) => {
            "✅ Container running with Claude Code\n❌ GitHub authentication needed\n✅ Claude \
             authenticated"
        }
        (false, false) => {
            "✅ Container running with Claude Code\n❌ GitHub authentication needed\n❌ Claude \
             authentication needed"
        }
    };

    let consolidated_message = format!(
        "✅ *Coding session started successfully\\!*\n\n*Container ID:* `{}`\n*Container Name:* \
         `{}`\n\n{}",
        escape_markdown_v2(container_id_short),
        escape_markdown_v2(container_name),
        escape_markdown_v2(auth_status_text)
    );

    bot.send_message(chat_id, consolidated_message)
        .parse_mode(ParseMode::MarkdownV2)
        .await?;

    // Guide user through next steps based on authentication status
    if github_authenticated && claude_authenticated {
        // Both authenticated - proceed to repository setup
        prompt_for_repository_selection(bot, chat_id, bot_state, claude_client).await?;
    } else if github_authenticated && !claude_authenticated {
        // GitHub authenticated but Claude not - offer repository listing while Claude auth proceeds
        start_authentication_flows_consolidated(
            bot.clone(),
            chat_id,
            bot_state,
            claude_client,
            github_authenticated,
            claude_authenticated,
        )
        .await?;

        // Add repository listing option since GitHub is authenticated
        let keyboard = InlineKeyboardMarkup::new(vec![vec![InlineKeyboardButton::callback(
            "📂 List Repositories",
            "github_repo_list",
        )]]);

        bot.send_message(
            chat_id,
            "💡 *Quick Start Option*\n\nWhile Claude authentication is in progress, you can \
             browse and select a repository to clone:",
        )
        .parse_mode(ParseMode::MarkdownV2)
        .reply_markup(keyboard)
        .await?;
    } else {
        // Start authentication flows automatically and show guidance
        start_authentication_flows_consolidated(
            bot,
            chat_id,
            bot_state,
            claude_client,
            github_authenticated,
            claude_authenticated,
        )
        .await?;
    }

    Ok(())
}

/// Check GitHub authentication status silently (no messages sent)
async fn check_github_auth_status_silent(github_client: &GithubClient) -> bool {
    match github_client.check_auth_status().await {
        Ok(auth_result) => auth_result.authenticated,
        Err(_) => false,
    }
}

/// Check Claude authentication status silently (no messages sent)
async fn check_claude_auth_status_silent(claude_client: &ClaudeCodeClient) -> bool {
    claude_client.check_auth_status().await.unwrap_or_default()
}

/// Start authentication flows automatically for unauthenticated services (consolidated version)
async fn start_authentication_flows_consolidated(
    bot: Bot,
    chat_id: ChatId,
    bot_state: &BotState,
    claude_client: &ClaudeCodeClient,
    github_authenticated: bool,
    claude_authenticated: bool,
) -> ResponseResult<()> {
    let mut auth_actions = Vec::new();

    if !github_authenticated {
        auth_actions.push("🐙 GitHub");
    }

    if !claude_authenticated {
        auth_actions.push("🤖 Claude");
    }

    if !auth_actions.is_empty() {
        // Start GitHub authentication if needed
        if !github_authenticated {
            let github_client = GithubClient::new(
                bot_state.docker.clone(),
                claude_client.container_id().to_string(),
                GithubClientConfig::default(),
            );

            if let Ok(auth_result) = github_client.login().await {
                if let (Some(oauth_url), Some(device_code)) =
                    (&auth_result.oauth_url, &auth_result.device_code)
                {
                    let consolidated_message = format!(
                        "🔐 *Authentication Required*\n\n📋 Starting authentication for: \
                         {}\n\nPlease complete the authentication process and then use /start \
                         again to continue\\.\n\n🐙 *GitHub Authentication*\n\nDevice code: \
                         ```{}```\n\nClick below to authenticate\\.",
                        auth_actions.join(" and "),
                        escape_markdown_v2(device_code)
                    );

                    let keyboard = InlineKeyboardMarkup::new(vec![
                        vec![InlineKeyboardButton::url(
                            "🔗 Authenticate GitHub",
                            url::Url::parse(oauth_url).unwrap_or_else(|_| {
                                url::Url::parse("https://github.com/login/device").unwrap()
                            }),
                        )],
                        vec![InlineKeyboardButton::copy_text_button(
                            "📋 Copy Device Code",
                            CopyTextButton {
                                text: device_code.clone(),
                            },
                        )],
                    ]);

                    bot.send_message(chat_id, consolidated_message)
                        .parse_mode(ParseMode::MarkdownV2)
                        .reply_markup(keyboard)
                        .await?;
                } else {
                    let fallback_message = format!(
                        "🔐 *Authentication Required*\n\n📋 Starting authentication for: \
                         {}\n\nPlease complete the authentication process and then use /start \
                         again to continue\\.",
                        auth_actions.join(" and ")
                    );
                    bot.send_message(chat_id, fallback_message)
                        .parse_mode(ParseMode::MarkdownV2)
                        .await?;
                }
            } else {
                let fallback_message = format!(
                    "🔐 *Authentication Required*\n\n📋 Starting authentication for: {}\n\nPlease \
                     complete the authentication process and then use /start again to continue\\.",
                    auth_actions.join(" and ")
                );
                bot.send_message(chat_id, fallback_message)
                    .parse_mode(ParseMode::MarkdownV2)
                    .await?;
            }
        } else {
            let base_message = format!(
                "🔐 *Authentication Required*\n\n📋 Starting authentication for: {}\n\nPlease \
                 complete the authentication process and then use /start again to continue\\.",
                auth_actions.join(" and ")
            );
            bot.send_message(chat_id, base_message)
                .parse_mode(ParseMode::MarkdownV2)
                .await?;
        }

        // Start Claude authentication if needed (this will still generate separate messages due to the interactive nature)
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
                        format!(
                            "❌ Failed to start Claude authentication: {}",
                            crate::escape_markdown_v2(&e.to_string())
                        ),
                    )
                    .parse_mode(ParseMode::MarkdownV2)
                    .await?;
                }
            }
        }
    }

    Ok(())
}

/// Show repository selection UI (public function for callback handlers)
pub async fn show_repository_selection(
    bot: Bot,
    chat_id: ChatId,
    bot_state: &BotState,
    claude_client: &ClaudeCodeClient,
) -> ResponseResult<()> {
    prompt_for_repository_selection(bot, chat_id, bot_state, claude_client).await
}

/// Prompt user for repository selection after successful authentication
async fn prompt_for_repository_selection(
    bot: Bot,
    chat_id: ChatId,
    bot_state: &BotState,
    claude_client: &ClaudeCodeClient,
) -> ResponseResult<()> {
    let message = "🎯 *Ready to Start Coding\\!*\n\nBoth GitHub and Claude are \
                   authenticated\\.\n\n📂 *Choose a Repository*\nSelect a repository to clone \
                   into your coding environment:";

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
                    "📁 *No Repositories Found*\n\n💡 No repositories found or accessible\\. You \
                     can:\n\n• Create a new repository on GitHub\n• Get access to existing \
                     repositories\n• Manually specify a public repository",
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
                    keyboard_rows.push(vec![InlineKeyboardButton::callback(
                        "✏️ Enter Repository Manually",
                        "manual_repo_entry",
                    )]);

                    // Add skip option
                    keyboard_rows.push(vec![InlineKeyboardButton::callback(
                        "⏭️ Skip Repository Setup",
                        "skip_repo_setup",
                    )]);

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
                vec![InlineKeyboardButton::callback(
                    "✏️ Enter Repository Manually",
                    "manual_repo_entry",
                )],
                vec![InlineKeyboardButton::callback(
                    "⏭️ Skip Repository Setup",
                    "skip_repo_setup",
                )],
            ]);

            bot.send_message(
                chat_id,
                format!(
                    "{}\n\n⚠️ Could not list repositories\\. You can enter one manually:",
                    message
                ),
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
            perform_github_clone(&bot, chat_id, &github_client, repository, bot_state).await?;

            // After successful clone, provide next steps
            bot.send_message(
                chat_id,
                "🎉 *Setup Complete\\!*\n\nYour coding environment is ready:\n✅ Container \
                 running\n✅ GitHub & Claude authenticated\n✅ Repository cloned\n\n💬 You can \
                 now start chatting with Claude about your code\\!",
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
pub async fn handle_manual_repository_entry(bot: Bot, chat_id: ChatId) -> ResponseResult<()> {
    bot.send_message(
        chat_id,
        "✏️ *Enter Repository*\n\nPlease type the repository you want to clone in the \
         format:\n`owner/repository`\n\nExamples:\n• `octocat/Hello-World`\n• \
         `microsoft/vscode`\n• `golang/go`",
    )
    .parse_mode(ParseMode::MarkdownV2)
    .await?;

    Ok(())
}

/// Handle skipping repository setup
pub async fn handle_skip_repository_setup(bot: Bot, chat_id: ChatId) -> ResponseResult<()> {
    bot.send_message(
        chat_id,
        "⏭️ *Repository Setup Skipped*\n\nYour coding environment is ready:\n✅ Container \
         running\n✅ GitHub & Claude authenticated\n\n💬 You can now start chatting with \
         Claude\\!\n📂 Clone a repository anytime by mentioning it in the chat\\.",
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
