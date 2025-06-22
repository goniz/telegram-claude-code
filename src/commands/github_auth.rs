use teloxide::{
    prelude::*,
    types::{InlineKeyboardButton, InlineKeyboardMarkup, ParseMode}
};
use url::Url;
use crate::{
    escape_markdown_v2, BotState,
    claude_code_client::{ClaudeCodeClient, GithubClient, GithubClientConfig}
};

/// Handle the /githubauth command
pub async fn handle_github_authentication(
    bot: Bot,
    msg: Message,
    bot_state: BotState,
    chat_id: i64,
) -> ResponseResult<()> {
    let container_name = format!("coding-session-{}", chat_id);

    match ClaudeCodeClient::for_session(bot_state.docker.clone(), &container_name).await {
        Ok(client) => {
            // Send initial message
            bot.send_message(
                msg.chat.id,
                "🔐 Starting GitHub authentication process\\.\\.\\.\n\n⏳ Initiating OAuth \
                 flow\\.\\.\\.",
            )
            .parse_mode(ParseMode::MarkdownV2)
            .await?;

            // Create GitHub client using same docker instance and container ID
            let github_client = GithubClient::new(
                bot_state.docker.clone(),
                client.container_id().to_string(),
                GithubClientConfig::default(),
            );

            match github_client.login().await {
                Ok(auth_result) => {
                    let message = if auth_result.authenticated {
                        if let Some(username) = &auth_result.username {
                            format!(
                                "✅ GitHub authentication successful\\!\n\n👤 Logged in as: \
                                 {}\n\n🎯 You can now use GitHub features in your coding \
                                 session\\.",
                                escape_markdown_v2(username)
                            )
                        } else {
                            "✅ GitHub authentication successful\\!\n\n🎯 You can now use GitHub \
                             features in your coding session\\."
                                .to_string()
                        }
                    } else if let (Some(oauth_url), Some(device_code)) =
                        (&auth_result.oauth_url, &auth_result.device_code)
                    {
                        let message = format!(
                            "🔗 *GitHub OAuth Authentication Required*\n\n*Please follow these \
                             steps:*\n\n1️⃣ *Click the button below to visit the authentication \
                             URL*\n\n2️⃣ *Enter this device code:*\n```{}```\n\n3️⃣ *Sign in to \
                             your GitHub account* and authorize the application\n\n4️⃣ *Return \
                             here* \\- authentication will be completed automatically\n\n⏱️ This \
                             code will expire in a few minutes, so please complete the process \
                             promptly\\.\n\n💡 *Tip:* Use /githubstatus to check if \
                             authentication completed successfully\\.",
                            escape_markdown_v2(device_code)
                        );

                        let keyboard = InlineKeyboardMarkup::new(vec![
                            vec![InlineKeyboardButton::url(
                                "🔗 Open GitHub OAuth",
                                Url::parse(oauth_url)
                                    .unwrap_or_else(|_| Url::parse("https://github.com").unwrap()),
                            )],
                            vec![InlineKeyboardButton::switch_inline_query_current_chat(
                                "📋 Copy Device Code",
                                device_code,
                            )],
                        ]);

                        bot.send_message(msg.chat.id, message)
                            .parse_mode(ParseMode::MarkdownV2)
                            .reply_markup(keyboard)
                            .await?;
                        return Ok(());
                    } else {
                        format!(
                            "ℹ️ GitHub authentication status: {}",
                            escape_markdown_v2(&auth_result.message)
                        )
                    };

                    bot.send_message(msg.chat.id, message)
                        .parse_mode(ParseMode::MarkdownV2)
                        .await?;
                }
                Err(e) => {
                    let error_msg = e.to_string();
                    let user_message = if error_msg.contains("timed out after") {
                        format!(
                            "⏰ GitHub authentication timed out: {}\n\nThis usually means:\n• The \
                             authentication process is taking longer than expected\n• There may \
                             be network connectivity issues\n• The GitHub CLI might be \
                             unresponsive\n\nPlease try again in a few moments\\.",
                            escape_markdown_v2(&error_msg)
                        )
                    } else {
                        format!(
                            "❌ Failed to initiate GitHub authentication: {}\n\nPlease ensure:\n• \
                             Your coding session is active\n• GitHub CLI \\(gh\\) is properly \
                             installed\n• Network connectivity is available",
                            escape_markdown_v2(&error_msg)
                        )
                    };

                    bot.send_message(msg.chat.id, user_message)
                        .parse_mode(ParseMode::MarkdownV2)
                        .await?;
                }
            }
        }
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
        }
    }

    Ok(())
}