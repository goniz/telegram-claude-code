use teloxide::{prelude::*, types::ParseMode};
use crate::{escape_markdown_v2, BotState, claude_code_client::ClaudeCodeClient};

/// Handle the /claudestatus command
pub async fn handle_claude_status(bot: Bot, msg: Message, bot_state: BotState, chat_id: i64) -> ResponseResult<()> {
    let container_name = format!("coding-session-{}", chat_id);

    match ClaudeCodeClient::for_session(bot_state.docker.clone(), &container_name).await {
        Ok(client) => {
            let version_result = client.check_availability().await;
            let auth_result = client.check_auth_status().await;
            
            match (version_result, auth_result) {
                (Ok(version), Ok(is_authenticated)) => {
                    let auth_status = if is_authenticated {
                        "✅ Authenticated"
                    } else {
                        "❌ Not authenticated"
                    };
                    
                    bot.send_message(
                        msg.chat.id,
                        format!(
                            "✅ Claude Code is available\\!\n\n*Version:* `{}`\n*Authentication:* {}",
                            escape_markdown_v2(&version),
                            escape_markdown_v2(auth_status)
                        ),
                    )
                    .parse_mode(ParseMode::MarkdownV2)
                    .await?;
                }
                (Ok(version), Err(_)) => {
                    bot.send_message(
                        msg.chat.id,
                        format!(
                            "✅ Claude Code is available\\!\n\n*Version:* `{}`\n*Authentication:* ⚠️ Unable to check auth status",
                            escape_markdown_v2(&version)
                        ),
                    )
                    .parse_mode(ParseMode::MarkdownV2)
                    .await?;
                }
                (Err(e), _) => {
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