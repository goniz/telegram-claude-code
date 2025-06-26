use crate::{escape_markdown_v2, find_claude_auth_log_file};
use std::path::Path;
use teloxide::{
    prelude::*,
    types::{InputFile, ParseMode},
};

/// Handle the /debugclaudelogin command
pub async fn handle_debug_claude_login(
    bot: Bot,
    msg: Message,
    _chat_id: i64,
) -> ResponseResult<()> {
    // Send initial message
    bot.send_message(
        msg.chat.id,
        "🔍 Searching for Claude authentication debug logs\\.\\.\\.",
    )
    .parse_mode(ParseMode::MarkdownV2)
    .await?;

    match find_claude_auth_log_file().await {
        Some(log_file_path) => {
            // Check if file exists and get its size
            match tokio::fs::metadata(&log_file_path).await {
                Ok(metadata) => {
                    let file_size = metadata.len();
                    let file_name = Path::new(&log_file_path)
                        .file_name()
                        .unwrap_or_default()
                        .to_string_lossy()
                        .to_string();

                    // Telegram has a 50MB file size limit
                    if file_size > 50 * 1024 * 1024 {
                        bot.send_message(
                            msg.chat.id,
                            format!(
                                "📁 Found debug log file: `{}`\n\n⚠️ File is too large to send \
                                 via Telegram \\({}\\)\\. Log files are automatically cleaned up \
                                 periodically\\.",
                                escape_markdown_v2(&file_name),
                                escape_markdown_v2(&format!(
                                    "{:.1} MB",
                                    file_size as f64 / (1024.0 * 1024.0)
                                ))
                            ),
                        )
                        .parse_mode(ParseMode::MarkdownV2)
                        .await?;
                        return Ok(());
                    }

                    // Send the file as an attachment
                    let input_file = InputFile::file(&log_file_path);
                    let caption = format!(
                        "🔍 *Claude Authentication Debug Log*\n\n📁 File: `{}`\n📊 Size: {} \
                         \n\n💡 This log contains detailed information about the Claude \
                         authentication process\\.",
                        escape_markdown_v2(&file_name),
                        escape_markdown_v2(&format!("{:.1} KB", file_size as f64 / 1024.0))
                    );

                    bot.send_document(msg.chat.id, input_file)
                        .caption(caption)
                        .parse_mode(ParseMode::MarkdownV2)
                        .await?;
                }
                Err(e) => {
                    bot.send_message(
                        msg.chat.id,
                        format!(
                            "❌ Found log file but couldn't access it: {}",
                            escape_markdown_v2(&e.to_string())
                        ),
                    )
                    .parse_mode(ParseMode::MarkdownV2)
                    .await?;
                }
            }
        }
        None => {
            bot.send_message(
                msg.chat.id,
                "📂 *No Claude authentication debug logs found*\n\n💡 Debug logs are created \
                 when:\n• Claude authentication is attempted\n• An authentication session fails \
                 or encounters errors\n\n🔄 Try running `/authenticateclaude` first to generate \
                 debug logs\\.",
            )
            .parse_mode(ParseMode::MarkdownV2)
            .await?;
        }
    }

    Ok(())
}
