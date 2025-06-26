use teloxide::{prelude::*, types::ParseMode};
use crate::{escape_markdown_v2, BotState};
use telegram_bot::claude_code_client::container_utils;

/// Handle the /clearsession command
pub async fn handle_clear_session(
    bot: Bot,
    msg: Message,
    bot_state: BotState,
    chat_id: i64,
) -> ResponseResult<()> {
    let container_name = format!("coding-session-{}", chat_id);

    // Also clear any pending authentication session
    {
        let mut sessions = bot_state.auth_sessions.lock().await;
        sessions.remove(&chat_id);
    }

    match container_utils::clear_coding_session(&bot_state.docker, &container_name).await {
        Ok(()) => {
            bot.send_message(
                msg.chat.id,
                "🧹 Coding session cleared successfully\\!\n\nThe container has been stopped and \
                 removed\\.",
            )
            .parse_mode(ParseMode::MarkdownV2)
            .await?;
        }
        Err(e) => {
            bot.send_message(
                msg.chat.id,
                format!(
                    "❌ Failed to clear session: {}",
                    escape_markdown_v2(&e.to_string())
                ),
            )
            .parse_mode(ParseMode::MarkdownV2)
            .await?;
        }
    }

    Ok(())
}