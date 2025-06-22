use teloxide::{prelude::*, types::ParseMode};
use crate::{escape_markdown_v2, Command, BotState};
use teloxide::utils::command::BotCommands;

/// Generate help text in BotFather format from command descriptions
pub fn generate_help_text() -> String {
    let descriptions = Command::descriptions().to_string();

    // Parse the default descriptions and convert to BotFather format
    descriptions
        .lines()
        .skip(2) // Skip the header "These commands are supported:" and empty line
        .map(|line| {
            // Remove the leading slash and replace em dash with hyphen
            line.trim()
                .strip_prefix('/')
                .unwrap_or(line.trim())
                .replace(" — ", " - ")
        })
        .collect::<Vec<String>>()
        .join("\n")
}

/// Handle the /help command
pub async fn handle_help(bot: Bot, msg: Message, _bot_state: BotState) -> ResponseResult<()> {
    let help_text = generate_help_text();
    bot.send_message(msg.chat.id, escape_markdown_v2(&help_text))
        .parse_mode(ParseMode::MarkdownV2)
        .await?;
    Ok(())
}