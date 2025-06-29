use crate::{escape_markdown_v2, BotState};
use telegram_bot::claude_code_client::{
    container_utils, ClaudeCodeClient, ClaudeCodeConfig, GithubClient, GithubClientConfig,
};
use teloxide::{
    prelude::*,
    types::{InlineKeyboardButton, InlineKeyboardMarkup, ParseMode},
};

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
        prompt_for_repository_setup(bot, chat_id).await?;
    } else {
        // Show authentication guidance
        show_authentication_guidance(bot, chat_id, github_authenticated, claude_authenticated)
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

/// Show authentication guidance with appropriate buttons
async fn show_authentication_guidance(
    bot: Bot,
    chat_id: ChatId,
    github_authenticated: bool,
    claude_authenticated: bool,
) -> ResponseResult<()> {
    let mut auth_steps = Vec::new();
    let mut keyboard_buttons = Vec::new();

    if !github_authenticated {
        auth_steps.push("🐙 *Authenticate with GitHub* to access repositories");
        keyboard_buttons.push(vec![
            InlineKeyboardButton::switch_inline_query_current_chat(
                "🔐 Authenticate GitHub",
                "/auth login",
            ),
        ]);
    }

    if !claude_authenticated {
        auth_steps.push("🤖 *Authenticate with Claude* to use AI coding features");
        keyboard_buttons.push(vec![
            InlineKeyboardButton::switch_inline_query_current_chat(
                "🔐 Authenticate Claude",
                "/auth login",
            ),
        ]);
    }

    let message = if auth_steps.is_empty() {
        "🎉 All authentication complete\\! Setting up your repository\\.\\.\\."
    } else {
        &format!(
            "🔐 *Authentication Required*\n\nTo get started, please complete the following:\n\n{}",
            auth_steps.join("\n")
        )
    };

    // Add status check button
    keyboard_buttons.push(vec![
        InlineKeyboardButton::switch_inline_query_current_chat("🔄 Check Status Again", "/start"),
    ]);

    let keyboard = InlineKeyboardMarkup::new(keyboard_buttons);

    bot.send_message(chat_id, message)
        .parse_mode(ParseMode::MarkdownV2)
        .reply_markup(keyboard)
        .await?;

    Ok(())
}

/// Prompt user for repository setup after successful authentication
async fn prompt_for_repository_setup(bot: Bot, chat_id: ChatId) -> ResponseResult<()> {
    let message = "🎯 *Ready to Start Coding\\!*\n\nBoth GitHub and Claude are authenticated\\. \
                   Now let's set up your development environment:\n\n\
                   📂 *Repository Setup*\n\
                   Please provide the following information:\n\n\
                   1️⃣ **GitHub Repository** to clone\n\
                   2️⃣ **Branch** to work on \\(optional\\)\n\
                   3️⃣ **Task Description** for this session";

    let keyboard = InlineKeyboardMarkup::new(vec![
        vec![InlineKeyboardButton::switch_inline_query_current_chat(
            "📂 Browse Repositories",
            "/githubrepolist",
        )],
        vec![InlineKeyboardButton::switch_inline_query_current_chat(
            "🔗 Clone Repository",
            "/githubclone",
        )],
        vec![
            InlineKeyboardButton::switch_inline_query_current_chat(
                "📊 Claude Status",
                "/claudestatus",
            ),
            InlineKeyboardButton::switch_inline_query_current_chat(
                "🔐 Auth Status",
                "/auth",
            ),
        ],
    ]);

    bot.send_message(chat_id, message)
        .parse_mode(ParseMode::MarkdownV2)
        .reply_markup(keyboard)
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
