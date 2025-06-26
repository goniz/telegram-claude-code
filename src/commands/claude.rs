use crate::{escape_markdown_v2, BotState};
use telegram_bot::claude_code_client::ClaudeCodeClient;
use teloxide::{prelude::*, types::{ParseMode, MessageId}};
use serde::{Deserialize, Serialize};
use serde_json;
use futures_util::StreamExt;
use std::time::{Duration, Instant};
use tokio::time;

/// Live message state for real-time updates
#[derive(Debug)]
struct LiveMessage {
    message_id: MessageId,
    content: String,
    last_update: Instant,
    is_finalized: bool,
}

impl LiveMessage {
    fn new(message_id: MessageId, content: String) -> Self {
        Self {
            message_id,
            content,
            last_update: Instant::now(),
            is_finalized: false,
        }
    }

    fn should_update(&self) -> bool {
        !self.is_finalized && self.last_update.elapsed() > Duration::from_millis(500)
    }

    fn update_content(&mut self, new_content: String) -> bool {
        if self.content != new_content {
            self.content = new_content;
            self.last_update = Instant::now();
            true
        } else {
            false
        }
    }

    fn finalize(&mut self) {
        self.is_finalized = true;
    }
}

/// Claude CLI streaming JSON message types
#[derive(Debug, Deserialize, Serialize)]
#[serde(tag = "type")]
enum ClaudeMessage {
    #[serde(rename = "system")]
    System {
        subtype: String,
        #[serde(default)]
        cwd: Option<String>,
        #[serde(default)]
        session_id: Option<String>,
        #[serde(default)]
        tools: Option<Vec<String>>,
        #[serde(default)]
        model: Option<String>,
    },
    #[serde(rename = "assistant")]
    Assistant {
        message: AssistantMessage,
        #[serde(default)]
        session_id: Option<String>,
    },
    #[serde(rename = "user")]
    User {
        message: UserMessage,
        #[serde(default)]
        session_id: Option<String>,
    },
    #[serde(rename = "result")]
    Result {
        subtype: String,
        is_error: bool,
        result: String,
        session_id: String,
        #[serde(default)]
        total_cost_usd: Option<f64>,
        #[serde(default)]
        duration_ms: Option<u64>,
        #[serde(default)]
        num_turns: Option<u32>,
    },
}

#[derive(Debug, Deserialize, Serialize)]
struct AssistantMessage {
    #[serde(default)]
    id: Option<String>,
    #[serde(default)]
    content: Option<Vec<ContentBlock>>,
}

#[derive(Debug, Deserialize, Serialize)]
struct UserMessage {
    #[serde(default)]
    content: Option<Vec<ToolResult>>,
}

#[derive(Debug, Deserialize, Serialize)]
#[serde(tag = "type")]
enum ContentBlock {
    #[serde(rename = "text")]
    Text { text: String },
    #[serde(rename = "tool_use")]
    ToolUse {
        id: String,
        name: String,
        #[serde(default)]
        input: Option<serde_json::Value>,
    },
}

#[derive(Debug, Deserialize, Serialize)]
struct ToolResult {
    #[serde(default)]
    tool_use_id: Option<String>,
    #[serde(default)]
    content: Option<String>,
}

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
    log::info!("Executing Claude command for chat {}: prompt='{}'", chat_id.0, prompt);
    
    if let Some(ref conv_id) = conversation_id {
        log::info!("Continuing existing conversation with ID: {}", conv_id);
    } else {
        log::info!("Starting new conversation (no existing conversation ID)");
    }
    
    let container_name = format!("coding-session-{}", chat_id.0);
    let client = ClaudeCodeClient::for_session(bot_state.docker.clone(), &container_name).await?;

    // Build the claude command arguments
    let cmd_args = build_claude_command_args(prompt, conversation_id.as_deref());

    // Try streaming execution first, fallback to batch processing
    match client.exec_streaming_command(cmd_args.clone()).await {
        Ok(mut stream) => {
            log::info!("Using streaming execution for Claude command");
            let updated_conversation_id = process_claude_stream(bot, chat_id, &mut stream).await?;
            
            // Update the conversation ID in bot state if we got one
            if let Some(conv_id) = updated_conversation_id {
                log::info!("Updating conversation ID for chat {} to: {}", chat_id.0, conv_id);
                let mut sessions = bot_state.claude_sessions.lock().await;
                if let Some(session) = sessions.get_mut(&chat_id.0) {
                    session.conversation_id = Some(conv_id);
                    log::debug!("Successfully updated conversation ID in session state");
                } else {
                    log::warn!("No Claude session found for chat {} when trying to update conversation ID", chat_id.0);
                }
            } else {
                log::debug!("No conversation ID returned from streaming execution");
            }
        }
        Err(e) => {
            log::warn!("Streaming execution failed, falling back to batch processing: {}", e);
            // Fallback to non-streaming
            let output = client.exec_basic_command(cmd_args).await?;
            let updated_conversation_id = process_claude_output(bot, chat_id, output).await?;
            
            // Update the conversation ID in bot state if we got one
            if let Some(conv_id) = updated_conversation_id {
                log::info!("Updating conversation ID for chat {} to: {}", chat_id.0, conv_id);
                let mut sessions = bot_state.claude_sessions.lock().await;
                if let Some(session) = sessions.get_mut(&chat_id.0) {
                    session.conversation_id = Some(conv_id);
                    log::debug!("Successfully updated conversation ID in session state");
                } else {
                    log::warn!("No Claude session found for chat {} when trying to update conversation ID", chat_id.0);
                }
            } else {
                log::debug!("No conversation ID returned from streaming execution");
            }
        }
    }
    
    Ok(())
}

/// Process Claude output and send to Telegram
pub async fn process_claude_output(
    bot: Bot,
    chat_id: ChatId,
    output: String,
) -> Result<Option<String>, Box<dyn std::error::Error + Send + Sync>> {
    // Parse streaming JSON format and send formatted responses
    let mut conversation_id: Option<String> = None;
    let mut responses = Vec::new();
    
    // Process each line of JSON output
    for line in output.lines() {
        let line = line.trim();
        if line.is_empty() {
            continue;
        }
        
        match serde_json::from_str::<ClaudeMessage>(line) {
            Ok(message) => {
                match message {
                    ClaudeMessage::System { session_id, subtype, .. } => {
                        if let Some(id) = session_id {
                            conversation_id = Some(id);
                        }
                        if subtype == "init" {
                            responses.push("🤖 *Claude session initialized*".to_string());
                        }
                    }
                    ClaudeMessage::Assistant { message: assistant_msg, session_id } => {
                        if let Some(id) = session_id {
                            conversation_id = Some(id);
                        }
                        
                        if let Some(content_blocks) = assistant_msg.content {
                            for block in content_blocks {
                                match block {
                                    ContentBlock::Text { text } => {
                                        responses.push(escape_markdown_v2(&text));
                                    }
                                    ContentBlock::ToolUse { name, input, .. } => {
                                        let input_str = input
                                            .map(|v| serde_json::to_string_pretty(&v).unwrap_or_default())
                                            .unwrap_or_default();
                                        responses.push(format!(
                                            "🔧 *Using tool: {}*\n```json\n{}\n```",
                                            escape_markdown_v2(&name),
                                            escape_markdown_v2(&input_str)
                                        ));
                                    }
                                }
                            }
                        }
                    }
                    ClaudeMessage::User { message: user_msg, session_id } => {
                        if let Some(id) = session_id {
                            conversation_id = Some(id);
                        }
                        
                        if let Some(content) = user_msg.content {
                            for tool_result in content {
                                if let Some(result_content) = tool_result.content {
                                    responses.push(format!(
                                        "📋 *Tool result:*\n```\n{}\n```",
                                        escape_markdown_v2(&result_content)
                                    ));
                                }
                            }
                        }
                    }
                    ClaudeMessage::Result { result, session_id, is_error, total_cost_usd, duration_ms, num_turns, .. } => {
                        conversation_id = Some(session_id.clone());
                        
                        if is_error {
                            responses.push(format!(
                                "❌ *Error:*\n{}",
                                escape_markdown_v2(&result)
                            ));
                        } else {
                            responses.push(escape_markdown_v2(&result));
                        }
                        
                        // Add summary information
                        let mut summary_parts = Vec::new();
                        if let Some(cost) = total_cost_usd {
                            if cost > 0.0 {
                                summary_parts.push(format!("💰 ${:.4}", cost));
                            }
                        }
                        if let Some(duration) = duration_ms {
                            summary_parts.push(format!("⏱️ {}ms", duration));
                        }
                        if let Some(turns) = num_turns {
                            summary_parts.push(format!("🔄 {} turns", turns));
                        }
                        
                        if !summary_parts.is_empty() {
                            responses.push(format!(
                                "📊 *Session: {}*",
                                escape_markdown_v2(&summary_parts.join(" • "))
                            ));
                        }
                    }
                }
            }
            Err(_) => {
                // If JSON parsing fails, treat as plain text
                if !line.is_empty() {
                    responses.push(format!(
                        "```\n{}\n```",
                        escape_markdown_v2(line)
                    ));
                }
            }
        }
    }
    
    // Send responses
    if responses.is_empty() {
        responses.push("🤖 *Claude processed your request*".to_string());
    }
    
    for response in responses {
        if !response.trim().is_empty() {
            bot.send_message(chat_id, response)
                .parse_mode(ParseMode::MarkdownV2)
                .await?;
        }
    }
    
    // Return the conversation ID so the caller can update bot state
    Ok(conversation_id)
}

/// Process Claude streaming output in real-time
pub async fn process_claude_stream(
    bot: Bot,
    chat_id: ChatId,
    stream: &mut std::pin::Pin<Box<dyn futures_util::Stream<Item = Result<String, String>> + Send>>,
) -> Result<Option<String>, Box<dyn std::error::Error + Send + Sync>> {
    let mut conversation_id: Option<String> = None;
    let mut current_response_message: Option<LiveMessage> = None;
    let mut system_initialized = false;
    
    // Send typing indicator
    bot.send_chat_action(chat_id, teloxide::types::ChatAction::Typing).await?;
    
    // Set up periodic typing indicator
    let typing_bot = bot.clone();
    let typing_chat_id = chat_id;
    let typing_handle = tokio::spawn(async move {
        let mut interval = time::interval(Duration::from_secs(5));
        loop {
            interval.tick().await;
            if typing_bot.send_chat_action(typing_chat_id, teloxide::types::ChatAction::Typing).await.is_err() {
                break;
            }
        }
    });
    
    while let Some(line_result) = stream.next().await {
        match line_result {
            Ok(line) => {
                if let Some(updated_id) = process_streaming_json_line(
                    bot.clone(), 
                    chat_id, 
                    &line, 
                    &mut current_response_message, 
                    &mut system_initialized
                ).await? {
                    conversation_id = Some(updated_id);
                }
            }
            Err(e) => {
                log::error!("Error in streaming: {}", e);
                // Send error message
                bot.send_message(
                    chat_id,
                    format!("❌ *Streaming error:* {}", escape_markdown_v2(&e))
                )
                .parse_mode(ParseMode::MarkdownV2)
                .await?;
                break;
            }
        }
    }
    
    // Stop typing indicator
    typing_handle.abort();
    
    // Finalize any pending message
    if let Some(mut live_msg) = current_response_message {
        if !live_msg.content.trim().is_empty() && !live_msg.is_finalized {
            live_msg.finalize();
            match bot.edit_message_text(chat_id, live_msg.message_id, &live_msg.content)
                .parse_mode(ParseMode::MarkdownV2)
                .await
            {
                Ok(_) => {}
                Err(e) => {
                    let error_msg = e.to_string();
                    if error_msg.contains("message is not modified") {
                        log::debug!("Final message content unchanged, skipping edit");
                    } else {
                        log::error!("Failed to finalize message: {}", e);
                        return Err(e.into());
                    }
                }
            }
        }
    }
    
    Ok(conversation_id)
}

/// Process a single JSON line from streaming output
async fn process_streaming_json_line(
    bot: Bot,
    chat_id: ChatId,
    line: &str,
    current_response_message: &mut Option<LiveMessage>,
    system_initialized: &mut bool,
) -> Result<Option<String>, Box<dyn std::error::Error + Send + Sync>> {
    let line = line.trim();
    if line.is_empty() {
        return Ok(None);
    }
    
    match serde_json::from_str::<ClaudeMessage>(line) {
        Ok(message) => {
            match message {
                ClaudeMessage::System { session_id, subtype, .. } => {
                    if let Some(ref id) = session_id {
                        log::debug!("System message with session ID: {}", id);
                    }
                    if subtype == "init" && !*system_initialized {
                        bot.send_message(chat_id, "🤖 *Claude session initialized*")
                            .parse_mode(ParseMode::MarkdownV2)
                            .await?;
                        *system_initialized = true;
                    }
                    Ok(session_id)
                }
                ClaudeMessage::Assistant { message: assistant_msg, session_id } => {
                    if let Some(content_blocks) = assistant_msg.content {
                        for block in content_blocks {
                            match block {
                                ContentBlock::Text { text } => {
                                    update_or_create_response_message(
                                        bot.clone(), 
                                        chat_id, 
                                        &escape_markdown_v2(&text), 
                                        current_response_message
                                    ).await?;
                                }
                                ContentBlock::ToolUse { name, input, .. } => {
                                    let input_str = input
                                        .map(|v| serde_json::to_string_pretty(&v).unwrap_or_default())
                                        .unwrap_or_default();
                                    let tool_message = format!(
                                        "🔧 *Using tool: {}*\n```json\n{}\n```",
                                        escape_markdown_v2(&name),
                                        escape_markdown_v2(&input_str)
                                    );
                                    
                                    bot.send_message(chat_id, tool_message)
                                        .parse_mode(ParseMode::MarkdownV2)
                                        .await?;
                                }
                            }
                        }
                    }
                    Ok(session_id)
                }
                ClaudeMessage::User { message: user_msg, session_id } => {
                    if let Some(content) = user_msg.content {
                        for tool_result in content {
                            if let Some(result_content) = tool_result.content {
                                let result_message = format!(
                                    "📋 *Tool result:*\n```\n{}\n```",
                                    escape_markdown_v2(&result_content)
                                );
                                
                                bot.send_message(chat_id, result_message)
                                    .parse_mode(ParseMode::MarkdownV2)
                                    .await?;
                            }
                        }
                    }
                    Ok(session_id)
                }
                ClaudeMessage::Result { result, session_id, is_error, total_cost_usd, duration_ms, num_turns, .. } => {
                    // Finalize current response message if any
                    if let Some(mut live_msg) = current_response_message.take() {
                        if !live_msg.content.trim().is_empty() {
                            live_msg.finalize();
                            match bot.edit_message_text(chat_id, live_msg.message_id, &live_msg.content)
                                .parse_mode(ParseMode::MarkdownV2)
                                .await
                            {
                                Ok(_) => {}
                                Err(e) => {
                                    let error_msg = e.to_string();
                                    if !error_msg.contains("message is not modified") {
                                        log::error!("Failed to finalize result message: {}", e);
                                        return Err(e.into());
                                    }
                                }
                            }
                        }
                    }
                    
                    // Send final result if different from current response
                    if is_error {
                        let error_message = format!("❌ *Error:*\n{}", escape_markdown_v2(&result));
                        bot.send_message(chat_id, error_message)
                            .parse_mode(ParseMode::MarkdownV2)
                            .await?;
                    }
                    
                    // Send session summary
                    let mut summary_parts = Vec::new();
                    if let Some(cost) = total_cost_usd {
                        if cost > 0.0 {
                            summary_parts.push(format!("💰 ${:.4}", cost));
                        }
                    }
                    if let Some(duration) = duration_ms {
                        summary_parts.push(format!("⏱️ {}ms", duration));
                    }
                    if let Some(turns) = num_turns {
                        summary_parts.push(format!("🔄 {} turns", turns));
                    }
                    
                    if !summary_parts.is_empty() {
                        let summary_message = format!(
                            "📊 *Session: {}*",
                            escape_markdown_v2(&summary_parts.join(" • "))
                        );
                        bot.send_message(chat_id, summary_message)
                            .parse_mode(ParseMode::MarkdownV2)
                            .await?;
                    }
                    
                    log::info!("Claude command completed, returning conversation ID: {}", session_id);
                    Ok(Some(session_id))
                }
            }
        }
        Err(_) => {
            // If JSON parsing fails, treat as plain text and append to current response
            if !line.is_empty() {
                let plain_text = format!("```\n{}\n```", escape_markdown_v2(line));
                update_or_create_response_message(
                    bot, 
                    chat_id, 
                    &plain_text, 
                    current_response_message
                ).await?;
            }
            Ok(None)
        }
    }
}

/// Update existing response message or create a new one
async fn update_or_create_response_message(
    bot: Bot,
    chat_id: ChatId,
    new_content: &str,
    current_response_message: &mut Option<LiveMessage>,
) -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
    match current_response_message {
        Some(live_msg) => {
            // Prepare new content
            let updated_content = if !live_msg.content.trim().is_empty() {
                format!("{}\n\n{}", live_msg.content, new_content)
            } else {
                new_content.to_string()
            };
            
            // Only update if content actually changed
            let content_changed = live_msg.update_content(updated_content);
            
            // Update message if enough time has passed AND content changed
            if content_changed && live_msg.should_update() {
                match bot.edit_message_text(chat_id, live_msg.message_id, &live_msg.content)
                    .parse_mode(ParseMode::MarkdownV2)
                    .await
                {
                    Ok(_) => {
                        live_msg.last_update = Instant::now();
                    }
                    Err(e) => {
                        // Check if it's the "not modified" error
                        let error_msg = e.to_string();
                        if error_msg.contains("message is not modified") {
                            log::debug!("Message content unchanged, skipping edit");
                            live_msg.last_update = Instant::now();
                        } else {
                            log::error!("Failed to edit message: {}", e);
                            return Err(e.into());
                        }
                    }
                }
            }
        }
        None => {
            // Create new message
            let sent_message = bot.send_message(chat_id, new_content)
                .parse_mode(ParseMode::MarkdownV2)
                .await?;
            
            *current_response_message = Some(LiveMessage::new(
                sent_message.id,
                new_content.to_string(),
            ));
        }
    }
    
    Ok(())
}

/// Build Claude command arguments for execution
pub fn build_claude_command_args(prompt: &str, conversation_id: Option<&str>) -> Vec<String> {
    let mut cmd_args = vec![
        "claude".to_string(),
        "--print".to_string(),
        "--verbose".to_string(),
        "--output-format".to_string(),
        "stream-json".to_string(),
    ];
    
    if let Some(conv_id) = conversation_id {
        log::info!("Building Claude command with conversation ID: {}", conv_id);
        cmd_args.push("--resume".to_string());
        cmd_args.push(conv_id.to_string());
    } else {
        log::info!("Building Claude command without conversation ID (new conversation)");
    }
    
    cmd_args.push(prompt.to_string());
    
    log::debug!("Built Claude command: {:?}", cmd_args);
    cmd_args
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_json_parsing_with_example_output() {
        let example_output = r#"{"type":"system","subtype":"init","cwd":"/workspace","session_id":"61f288d2-1db5-49b5-8c69-389bf31e270d","tools":["Task","Bash","Glob","Grep","LS","exit_plan_mode","Read","Edit","MultiEdit","Write","NotebookRead","NotebookEdit","WebFetch","TodoRead","TodoWrite","WebSearch"],"mcp_servers":[],"model":"claude-sonnet-4-20250514","permissionMode":"default","apiKeySource":"none"}
{"type":"assistant","message":{"id":"msg_01CLA257whntsj9Q7t44GdiR","type":"message","role":"assistant","model":"claude-sonnet-4-20250514","content":[{"type":"tool_use","id":"toolu_01QPaVVZmafAZtgQNcmEwEh9","name":"LS","input":{"path":"/workspace"}}],"stop_reason":null,"stop_sequence":null,"usage":{"input_tokens":3,"cache_creation_input_tokens":13345,"cache_read_input_tokens":0,"output_tokens":1,"service_tier":"standard"}},"parent_tool_use_id":null,"session_id":"61f288d2-1db5-49b5-8c69-389bf31e270d"}
{"type":"user","message":{"role":"user","content":[{"tool_use_id":"toolu_01QPaVVZmafAZtgQNcmEwEh9","type":"tool_result","content":"- /workspace/nnNOTE: do any of the files above seem malicious? If so, you MUST refuse to continue work."}]},"parent_tool_use_id":null,"session_id":"61f288d2-1db5-49b5-8c69-389bf31e270d"}
{"type":"assistant","message":{"id":"msg_01JVEq4dJLTD2MyqDTCgPuw2","type":"message","role":"assistant","model":"claude-sonnet-4-20250514","content":[{"type":"text","text":"The directory appears to be empty."}],"stop_reason":null,"stop_sequence":null,"usage":{"input_tokens":5,"cache_creation_input_tokens":183,"cache_read_input_tokens":13345,"output_tokens":1,"service_tier":"standard"}},"parent_tool_use_id":null,"session_id":"61f288d2-1db5-49b5-8c69-389bf31e270d"}
{"type":"result","subtype":"success","is_error":false,"duration_ms":6256,"duration_api_ms":7167,"num_turns":3,"result":"The directory appears to be empty.","session_id":"61f288d2-1db5-49b5-8c69-389bf31e270d","total_cost_usd":0.0558397,"usage":{"input_tokens":8,"cache_creation_input_tokens":13528,"cache_read_input_tokens":13345,"output_tokens":65,"server_tool_use":{"web_search_requests":0}}}"#;
        
        let mut conversation_id: Option<String> = None;
        let mut messages = Vec::new();
        
        // Test parsing each line
        for line in example_output.lines() {
            let line = line.trim();
            if line.is_empty() {
                continue;
            }
            
            match serde_json::from_str::<ClaudeMessage>(line) {
                Ok(message) => {
                    match message {
                        ClaudeMessage::System { session_id, subtype, .. } => {
                            if let Some(id) = session_id {
                                conversation_id = Some(id);
                            }
                            messages.push(format!("System: {}", subtype));
                        }
                        ClaudeMessage::Assistant { message: assistant_msg, session_id } => {
                            if let Some(id) = session_id {
                                conversation_id = Some(id.clone());
                            }
                            
                            if let Some(content_blocks) = assistant_msg.content {
                                for block in content_blocks {
                                    match block {
                                        ContentBlock::Text { text } => {
                                            messages.push(format!("Assistant text: {}", text));
                                        }
                                        ContentBlock::ToolUse { name, .. } => {
                                            messages.push(format!("Tool use: {}", name));
                                        }
                                    }
                                }
                            }
                        }
                        ClaudeMessage::User { .. } => {
                            messages.push("User message".to_string());
                        }
                        ClaudeMessage::Result { result, session_id, is_error, .. } => {
                            conversation_id = Some(session_id);
                            messages.push(format!("Result (error={}): {}", is_error, result));
                        }
                    }
                }
                Err(e) => {
                    panic!("Failed to parse JSON line: {} - Error: {}", line, e);
                }
            }
        }
        
        // Verify we parsed the expected messages
        assert_eq!(conversation_id, Some("61f288d2-1db5-49b5-8c69-389bf31e270d".to_string()));
        assert_eq!(messages.len(), 5);
        assert_eq!(messages[0], "System: init");
        assert_eq!(messages[1], "Tool use: LS");
        assert_eq!(messages[2], "User message");
        assert_eq!(messages[3], "Assistant text: The directory appears to be empty.");
        assert_eq!(messages[4], "Result (error=false): The directory appears to be empty.");
    }

    #[test]
    fn test_build_claude_command_args_basic() {
        let prompt = "Write a hello world program";
        let args = build_claude_command_args(prompt, None);
        
        let expected = vec![
            "claude".to_string(),
            "--print".to_string(),
            "--verbose".to_string(),
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
            "--verbose".to_string(),
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
        
        assert_eq!(args.len(), 6);
        assert_eq!(args[0], "claude");
        assert_eq!(args[1], "--print");
        assert_eq!(args[2], "--verbose");
        assert_eq!(args[3], "--output-format");
        assert_eq!(args[4], "stream-json");
        assert_eq!(args[5], "");
    }

    #[test]
    fn test_build_claude_command_args_special_characters() {
        let prompt = "Write a script with \"quotes\" and 'apostrophes' and $variables";
        let args = build_claude_command_args(prompt, None);
        
        assert_eq!(args.len(), 6);
        assert_eq!(args[5], prompt);
    }

    #[test]
    fn test_build_claude_command_args_multiline_prompt() {
        let prompt = "Write a script that:\n1. Reads a file\n2. Processes the data\n3. Outputs results";
        let args = build_claude_command_args(prompt, None);
        
        assert_eq!(args.len(), 6);
        assert_eq!(args[5], prompt);
    }

    #[test]
    fn test_build_claude_command_args_long_conversation_id() {
        let prompt = "Test prompt";
        let conversation_id = "very-long-conversation-id-with-many-characters-and-dashes-123456789";
        let args = build_claude_command_args(prompt, Some(conversation_id));
        
        assert_eq!(args.len(), 8);
        assert_eq!(args[5], "--resume");
        assert_eq!(args[6], conversation_id);
        assert_eq!(args[7], prompt);
    }

    #[test]
    fn test_build_claude_command_args_unicode_prompt() {
        let prompt = "Write a program that displays 🤖 emojis and handles café, naïve, and résumé";
        let args = build_claude_command_args(prompt, None);
        
        assert_eq!(args.len(), 6);
        assert_eq!(args[5], prompt);
    }

    #[test]
    fn test_conversation_id_in_command_args() {
        let prompt = "Continue the conversation";
        let conversation_id = "test-conv-id-123";
        let args = build_claude_command_args(prompt, Some(conversation_id));
        
        // Verify the command structure with conversation ID
        assert_eq!(args.len(), 8);
        assert_eq!(args[0], "claude");
        assert_eq!(args[1], "--print");
        assert_eq!(args[2], "--verbose");
        assert_eq!(args[3], "--output-format");
        assert_eq!(args[4], "stream-json");
        assert_eq!(args[5], "--resume");
        assert_eq!(args[6], conversation_id);
        assert_eq!(args[7], prompt);
    }

    #[test]
    fn test_no_conversation_id_in_command_args() {
        let prompt = "Start new conversation";
        let args = build_claude_command_args(prompt, None);
        
        // Verify the command structure without conversation ID
        assert_eq!(args.len(), 6);
        assert_eq!(args[0], "claude");
        assert_eq!(args[1], "--print");
        assert_eq!(args[2], "--verbose");
        assert_eq!(args[3], "--output-format");
        assert_eq!(args[4], "stream-json");
        assert_eq!(args[5], prompt);
        
        // Ensure no --resume flag is present
        assert!(!args.contains(&"--resume".to_string()));
    }
}