use teloxide::{prelude::*, types::ParseMode};
use crate::{escape_markdown_v2, BotState, claude_code_client::ClaudeCodeClient};

/// Handle the /claudestatus command
pub async fn handle_claude_status(bot: Bot, msg: Message, bot_state: BotState, chat_id: i64) -> ResponseResult<()> {
    let container_name = format!("coding-session-{}", chat_id);

    match ClaudeCodeClient::for_session(bot_state.docker.clone(), &container_name).await {
        Ok(client) => match client.check_availability().await {
            Ok(version) => {
                bot.send_message(
                    msg.chat.id,
                    format!(
                        "✅ Claude Code is available\\!\n\n*Version:* `{}`",
                        escape_markdown_v2(&version)
                    ),
                )
                .parse_mode(ParseMode::MarkdownV2)
                .await?;
            }
            Err(e) => {
                bot.send_message(
                    msg.chat.id,
                    format!(
                        "❌ Claude Code check failed: {}",
                        escape_markdown_v2(&e.to_string())
                    ),
                )
                .parse_mode(ParseMode::MarkdownV2)
                .await?;
            }
        },
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