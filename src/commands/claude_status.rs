use crate::{BotState, escape_markdown_v2};
use telegram_bot::claude_code_client::ClaudeCodeClient;
use teloxide::{prelude::*, types::ParseMode};

/// Handle the /claudestatus command
pub async fn handle_claude_status(
    bot: Bot,
    msg: Message,
    bot_state: BotState,
    chat_id: i64,
) -> ResponseResult<()> {
    let container_name = format!("coding-session-{}", chat_id);

    match ClaudeCodeClient::for_session(bot_state.docker.clone(), &container_name).await {
        Ok(client) => {
            // Check Claude Code availability
            let version_result = client.check_availability().await;
            // Check Claude authentication status
            let auth_result = client.get_auth_info().await;

            match (version_result, auth_result) {
                (Ok(version), Ok(auth_info)) => {
                    bot.send_message(
                        msg.chat.id,
                        format!(
                            "✅ *Claude Code Status*\n\n*Version:* `{}`\n\n*Authentication:* {}",
                            escape_markdown_v2(&version),
                            escape_markdown_v2(&auth_info)
                        ),
                    )
                    .parse_mode(ParseMode::MarkdownV2)
                    .await?;
                }
                (Ok(version), Err(auth_err)) => {
                    bot.send_message(
                        msg.chat.id,
                        format!(
                            "✅ *Claude Code Status*\n\n*Version:* `{}`\n\n❌ *Authentication Error:* {}",
                            escape_markdown_v2(&version),
                            escape_markdown_v2(&auth_err.to_string())
                        ),
                    )
                    .parse_mode(ParseMode::MarkdownV2)
                    .await?;
                }
                (Err(version_err), Ok(auth_info)) => {
                    bot.send_message(
                        msg.chat.id,
                        format!(
                            "❌ *Claude Code Status*\n\n*Version Check Failed:* {}\n\n*Authentication:* {}",
                            escape_markdown_v2(&version_err.to_string()),
                            escape_markdown_v2(&auth_info)
                        ),
                    )
                    .parse_mode(ParseMode::MarkdownV2)
                    .await?;
                }
                (Err(version_err), Err(auth_err)) => {
                    bot.send_message(
                        msg.chat.id,
                        format!(
                            "❌ *Claude Code Status*\n\n*Version Check Failed:* {}\n\n*Authentication Check Failed:* {}",
                            escape_markdown_v2(&version_err.to_string()),
                            escape_markdown_v2(&auth_err.to_string())
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
                    "❌ No active coding session found: {}",
                    escape_markdown_v2(&e.to_string())
                ),
            )
            .parse_mode(ParseMode::MarkdownV2)
            .await?;
        }
    }

    Ok(())
}
