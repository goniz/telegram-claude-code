use teloxide::{
    prelude::*,
    types::{InlineKeyboardButton, InlineKeyboardMarkup, ParseMode}
};
use crate::{
    escape_markdown_v2, BotState,
    claude_code_client::{container_utils, ClaudeCodeConfig}
};

/// Handle the /start command
pub async fn handle_start(bot: Bot, msg: Message, bot_state: BotState, chat_id: i64, user_id: i64) -> ResponseResult<()> {
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
            let message = format!(
                "✅ Coding session started successfully\\!\n\n*Container ID:* \
                 `{}`\n*Container Name:* `{}`\n\n🎯 Claude Code is pre\\-installed and \
                 ready to use\\!\n\nYou can now run code and manage your development \
                 environment\\.",
                escape_markdown_v2(&container_id_short),
                escape_markdown_v2(&container_name)
            );

            let keyboard = InlineKeyboardMarkup::new(vec![
                vec![
                    InlineKeyboardButton::switch_inline_query_current_chat(
                        "🔐 Auth Claude",
                        "/authenticateclaude",
                    ),
                    InlineKeyboardButton::switch_inline_query_current_chat(
                        "🐙 Auth GitHub",
                        "/githubauth",
                    ),
                ],
                vec![
                    InlineKeyboardButton::switch_inline_query_current_chat(
                        "📊 Claude Status",
                        "/claudestatus",
                    ),
                    InlineKeyboardButton::switch_inline_query_current_chat(
                        "📋 GitHub Status",
                        "/githubstatus",
                    ),
                ],
            ]);

            bot.send_message(msg.chat.id, message)
                .parse_mode(ParseMode::MarkdownV2)
                .reply_markup(keyboard)
                .await?;
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