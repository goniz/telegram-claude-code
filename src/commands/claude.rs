use crate::{escape_markdown_v2, BotState};
use telegram_bot::claude_code_client::ClaudeCodeClient;
use teloxide::{prelude::*, types::ParseMode};

/// Handle the /claude command
pub async fn handle_claude(
    bot: Bot,
    msg: Message,
    bot_state: BotState,
    chat_id: i64,
) -> ResponseResult<()> {
    let container_name = format!("coding-session-{}", chat_id);

    // Check if Claude Code client is available
    match ClaudeCodeClient::for_session(bot_state.docker.clone(), &container_name).await {
        Ok(_client) => {
            // Reset any existing Claude conversation for this chat
            {
                let mut sessions = bot_state.claude_sessions.lock().await;
                if let Some(session) = sessions.get_mut(&chat_id) {
                    session.reset_conversation();
                } else {
                    sessions.insert(chat_id, crate::bot::ClaudeSession::new());
                }
                
                // Mark the session as active
                if let Some(session) = sessions.get_mut(&chat_id) {
                    session.is_active = true;
                }
            }

            // Send confirmation message
            bot.send_message(
                msg.chat.id,
                "🤖 *Starting new Claude conversation\\!*\n\nYou can now send me any message \\(without a command\\) and I'll forward it to Claude\\.",
            )
            .parse_mode(ParseMode::MarkdownV2)
            .await?;
        }
        Err(e) => {
            bot.send_message(
                msg.chat.id,
                format!(
                    "❌ No active coding session found: {}\n\nPlease start a coding session first using /start",
                    escape_markdown_v2(&e.to_string())
                ),
            )
            .parse_mode(ParseMode::MarkdownV2)
            .await?;
        }
    }

    Ok(())
}

/// Execute Claude command with streaming output
pub async fn execute_claude_command(
    bot: Bot,
    chat_id: ChatId,
    bot_state: BotState,
    prompt: &str,
    conversation_id: Option<String>,
) -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
    let container_name = format!("coding-session-{}", chat_id.0);
    let client = ClaudeCodeClient::for_session(bot_state.docker.clone(), &container_name).await?;

    // Build the claude command arguments
    let cmd_args = build_claude_command_args(prompt, conversation_id.as_deref());

    // For now, execute and get the complete output
    // TODO: Implement streaming execution
    let output = client.exec_basic_command(cmd_args).await?;
    
    // Process and send the output
    process_claude_output(bot, chat_id, output).await?;
    
    Ok(())
}

/// Process Claude output and send to Telegram
pub async fn process_claude_output(
    bot: Bot,
    chat_id: ChatId,
    output: String,
) -> ResponseResult<()> {
    // For now, send the raw output
    // TODO: Parse streaming JSON format and send formatted responses
    let escaped_output = escape_markdown_v2(&output);
    
    bot.send_message(chat_id, format!("```\n{}\n```", escaped_output))
        .parse_mode(ParseMode::MarkdownV2)
        .await?;

    Ok(())
}

/// Build Claude command arguments for execution
pub fn build_claude_command_args(prompt: &str, conversation_id: Option<&str>) -> Vec<String> {
    let mut cmd_args = vec![
        "claude".to_string(),
        "--print".to_string(),
        "--output-format".to_string(),
        "stream-json".to_string(),
    ];
    
    if let Some(conv_id) = conversation_id {
        cmd_args.push("--resume".to_string());
        cmd_args.push(conv_id.to_string());
    }
    
    cmd_args.push(prompt.to_string());
    cmd_args
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_build_claude_command_args_basic() {
        let prompt = "Write a hello world program";
        let args = build_claude_command_args(prompt, None);
        
        let expected = vec![
            "claude".to_string(),
            "--print".to_string(),
            "--output-format".to_string(),
            "stream-json".to_string(),
            prompt.to_string(),
        ];
        
        assert_eq!(args, expected);
    }

    #[test]
    fn test_build_claude_command_args_with_resume() {
        let prompt = "Continue the previous task";
        let conversation_id = "test-conversation-123";
        let args = build_claude_command_args(prompt, Some(conversation_id));
        
        let expected = vec![
            "claude".to_string(),
            "--print".to_string(),
            "--output-format".to_string(),
            "stream-json".to_string(),
            "--resume".to_string(),
            conversation_id.to_string(),
            prompt.to_string(),
        ];
        
        assert_eq!(args, expected);
    }

    #[test]
    fn test_build_claude_command_args_empty_prompt() {
        let prompt = "";
        let args = build_claude_command_args(prompt, None);
        
        assert_eq!(args.len(), 5);
        assert_eq!(args[0], "claude");
        assert_eq!(args[1], "--print");
        assert_eq!(args[2], "--output-format");
        assert_eq!(args[3], "stream-json");
        assert_eq!(args[4], "");
    }

    #[test]
    fn test_build_claude_command_args_special_characters() {
        let prompt = "Write a script with \"quotes\" and 'apostrophes' and $variables";
        let args = build_claude_command_args(prompt, None);
        
        assert_eq!(args.len(), 5);
        assert_eq!(args[4], prompt);
    }

    #[test]
    fn test_build_claude_command_args_multiline_prompt() {
        let prompt = "Write a script that:\n1. Reads a file\n2. Processes the data\n3. Outputs results";
        let args = build_claude_command_args(prompt, None);
        
        assert_eq!(args.len(), 5);
        assert_eq!(args[4], prompt);
    }

    #[test]
    fn test_build_claude_command_args_long_conversation_id() {
        let prompt = "Test prompt";
        let conversation_id = "very-long-conversation-id-with-many-characters-and-dashes-123456789";
        let args = build_claude_command_args(prompt, Some(conversation_id));
        
        assert_eq!(args.len(), 7);
        assert_eq!(args[4], "--resume");
        assert_eq!(args[5], conversation_id);
        assert_eq!(args[6], prompt);
    }

    #[test]
    fn test_build_claude_command_args_unicode_prompt() {
        let prompt = "Write a program that displays 🤖 emojis and handles café, naïve, and résumé";
        let args = build_claude_command_args(prompt, None);
        
        assert_eq!(args.len(), 5);
        assert_eq!(args[4], prompt);
    }
}