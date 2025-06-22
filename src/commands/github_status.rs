use teloxide::{
    prelude::*,
    types::{InlineKeyboardButton, InlineKeyboardMarkup, ParseMode}
};
use crate::{
    escape_markdown_v2, BotState,
    claude_code_client::{ClaudeCodeClient, GithubClient, GithubClientConfig}
};

/// Handle the /githubstatus command
pub async fn handle_github_status(
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

            match github_client.check_auth_status().await {
                Ok(auth_result) => {
                    let message = if auth_result.authenticated {
                        if let Some(username) = &auth_result.username {
                            format!(
                                "✅ *GitHub Authentication Status: Authenticated*\n\n👤 *Logged \
                                 in as:* {}\n\n🎯 You can now use GitHub features like:\n• \
                                 Repository cloning\n• Git operations\n• GitHub CLI commands",
                                escape_markdown_v2(username)
                            )
                        } else {
                            "✅ *GitHub Authentication Status: Authenticated*\n\n🎯 You can now \
                             use GitHub features like:\n• Repository cloning\n• Git operations\n• \
                             GitHub CLI commands"
                                .to_string()
                        }
                    } else {
                        "❌ *GitHub Authentication Status: Not Authenticated*\n\n🔐 Use \
                         `/githubauth` to start the authentication process\\.\n\nYou'll receive an \
                         OAuth URL and device code to complete authentication in your browser\\."
                            .to_string()
                    };

                    let keyboard = if auth_result.authenticated {
                        InlineKeyboardMarkup::new(vec![vec![
                            InlineKeyboardButton::switch_inline_query_current_chat(
                                "📂 List Repositories",
                                "/githubrepolist",
                            ),
                        ]])
                    } else {
                        InlineKeyboardMarkup::new(vec![vec![
                            InlineKeyboardButton::switch_inline_query_current_chat(
                                "🔐 Start Authentication",
                                "/githubauth",
                            ),
                        ]])
                    };

                    bot.send_message(msg.chat.id, message)
                        .parse_mode(ParseMode::MarkdownV2)
                        .reply_markup(keyboard)
                        .await?;
                }
                Err(e) => {
                    bot.send_message(
                        msg.chat.id,
                        format!(
                            "❌ Failed to check GitHub authentication status: {}\n\nThis could be \
                             due to:\n• GitHub CLI not being available\n• Network connectivity \
                             issues\n• Container problems",
                            escape_markdown_v2(&e.to_string())
                        ),
                    )
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