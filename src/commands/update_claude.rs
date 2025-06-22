use teloxide::{prelude::*, types::ParseMode};
use crate::{escape_markdown_v2, BotState, claude_code_client::ClaudeCodeClient};

/// Handle the /updateclaude command
pub async fn handle_update_claude(
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
                "🔄 Updating Claude CLI to latest version\\.\\.\\.",
            )
            .parse_mode(ParseMode::MarkdownV2)
            .await?;

            match client.update_claude().await {
                Ok(output) => {
                    let message = format!(
                        "✅ Claude CLI Update Complete\n\n{}", 
                        output
                    );
                    
                    bot.send_message(msg.chat.id, escape_markdown_v2(&message))
                        .parse_mode(ParseMode::MarkdownV2)
                        .await?;
                }
                Err(e) => {
                    bot.send_message(
                        msg.chat.id,
                        format!(
                            "❌ Failed to update Claude CLI: {}\n\nThis could be due to:\n• \
                             Network connectivity issues\n• Claude CLI not installed\n• \
                             Insufficient permissions",
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