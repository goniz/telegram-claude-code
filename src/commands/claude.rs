use crate::bot::markdown::{escape_markdown_v2, truncate_if_needed};
use crate::BotState;
use futures_util::StreamExt;
use std::time::{Duration, Instant};
use telegram_bot::claude_code_client::{
    ClaudeCodeClient, ClaudeExecutionResult, ClaudeMessageParser, LiveMessage, MessageType,
    ParseResult, ParsedClaudeMessage,
};
use teloxide::{
    prelude::*,
    types::{MessageId, ParseMode},
};
use tokio::time;

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
                "🤖 *Starting new Claude conversation\\!*\n\nYou can now send me any message \
                 \\(without a command\\) and I'll forward it to Claude\\.",
            )
            .parse_mode(ParseMode::MarkdownV2)
            .await?;
        }
        Err(e) => {
            let full_message = format!(
                "❌ No active coding session found: {}\n\nPlease start a coding session first \
                 using /start",
                escape_markdown_v2(&e.to_string())
            );
            let (message_to_send, _was_truncated) = truncate_if_needed(&full_message);
            
            bot.send_message(msg.chat.id, message_to_send)
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
    log::info!(
        "Executing Claude command for chat {}: prompt='{}'",
        chat_id.0,
        prompt
    );

    let container_name = format!("coding-session-{}", chat_id.0);

    // Get working directory from session state
    let working_directory = {
        let claude_sessions = bot_state.claude_sessions.lock().await;
        claude_sessions
            .get(&chat_id.0)
            .and_then(|session| session.get_working_directory().cloned())
    };

    let client = ClaudeCodeClient::for_session_with_working_dir(
        bot_state.docker.clone(),
        &container_name,
        working_directory,
    )
    .await?;

    // Execute Claude prompt with streaming or batch processing
    match client
        .execute_claude_prompt(prompt, conversation_id.as_deref())
        .await?
    {
        ClaudeExecutionResult::Streaming(mut stream) => {
            log::info!("Using streaming execution for Claude command");
            process_claude_streaming(bot, chat_id, &mut stream, bot_state.clone()).await?;
        }
        ClaudeExecutionResult::Batch(output) => {
            log::info!("Using batch processing for Claude command");
            process_claude_batch(bot, chat_id, output, bot_state.clone()).await?;
        }
    }

    Ok(())
}

/// Process Claude streaming output
async fn process_claude_streaming(
    bot: Bot,
    chat_id: ChatId,
    stream: &mut std::pin::Pin<
        Box<dyn futures_util::Stream<Item = Result<ParsedClaudeMessage, String>> + Send>,
    >,
    bot_state: BotState,
) -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
    let mut current_live_message: Option<(MessageId, LiveMessage)> = None;

    // Send typing indicator
    bot.send_chat_action(chat_id, teloxide::types::ChatAction::Typing)
        .await?;

    // Set up periodic typing indicator
    let typing_bot = bot.clone();
    let typing_chat_id = chat_id;
    let typing_handle = tokio::spawn(async move {
        let mut interval = time::interval(Duration::from_secs(5));
        loop {
            interval.tick().await;
            if typing_bot
                .send_chat_action(typing_chat_id, teloxide::types::ChatAction::Typing)
                .await
                .is_err()
            {
                break;
            }
        }
    });

    // Process streaming events (now already parsed)
    while let Some(message_result) = stream.next().await {
        match message_result {
            Ok(parsed) => {
                // Update conversation ID if available
                if let Some(conversation_id) = &parsed.conversation_id {
                    update_conversation_id(&bot_state, chat_id.0, conversation_id.clone()).await;
                }

                // Handle real-time events that need immediate processing
                match &parsed.message_type {
                    MessageType::SystemInit { .. } => {
                        bot.send_message(chat_id, "🤖 *Claude session initialized*")
                            .parse_mode(ParseMode::MarkdownV2)
                            .await?;
                    }
                    MessageType::AssistantText { text, .. } => {
                        update_live_message(
                            bot.clone(),
                            chat_id,
                            &escape_markdown_v2(text),
                            &mut current_live_message,
                        )
                        .await?;
                    }
                    MessageType::AssistantToolUse { name, input, .. } => {
                        let input_str = input
                            .as_ref()
                            .map(|v| serde_json::to_string_pretty(v).unwrap_or_default())
                            .unwrap_or_default();
                        let tool_message = format!(
                            "🔧 *Using tool: {}*\n```json\n{}\n```",
                            escape_markdown_v2(name),
                            escape_markdown_v2(&input_str)
                        );
                        let (message_to_send, _was_truncated) = truncate_if_needed(&tool_message);

                        bot.send_message(chat_id, message_to_send)
                            .parse_mode(ParseMode::MarkdownV2)
                            .await?;
                    }
                    MessageType::Result {
                        is_error,
                        cost,
                        duration_ms,
                        num_turns,
                        usage,
                        ..
                    } => {
                        let mut summary_parts = Vec::new();

                        // Cost summary
                        if let Some(c) = cost {
                            if c > 0.0 {
                                summary_parts.push(format!("Cost: ${:.4}", c));
                            }
                        }

                        // Duration summary
                        if let Some(d) = duration_ms {
                            summary_parts.push(format!("Duration: {}ms", d));
                        }

                        // Turns summary
                        if let Some(t) = num_turns {
                            summary_parts.push(format!("Turns: {}", t));
                        }

                        // Token usage summary
                        if let Some(u) = usage {
                            summary_parts.push(format!("Tokens: {} in / {} out", u.input_tokens, u.output_tokens));

                            if let Some(cache_create) = u.cache_creation_input_tokens {
                                summary_parts.push(format!("Cache create: {}", cache_create));
                            }

                            if let Some(cache_read) = u.cache_read_input_tokens {
                                summary_parts.push(format!("Cache read: {}", cache_read));
                            }
                        }

                        let summary_body = if summary_parts.is_empty() {
                            "Run completed".to_string()
                        } else {
                            summary_parts.join(" • ")
                        };

                        let status_emoji = if *is_error { "❌" } else { "✅" };

                        let summary_message = format!(
                            "{} *Claude Run Summary*\n{}",
                            status_emoji,
                            escape_markdown_v2(&summary_body)
                        );

                        let (message_to_send, _was_truncated) = truncate_if_needed(&summary_message);

                        bot.send_message(chat_id, message_to_send)
                            .parse_mode(ParseMode::MarkdownV2)
                            .await?;
                    }
                    MessageType::UserToolResult { .. } => {
                        // Tool results are no longer sent to chat
                    }
                    _ => {}
                }
            }
            Err(e) => {
                log::error!("Error in streaming: {}", e);
                let full_message = format!("❌ *Streaming error:* {}", escape_markdown_v2(&e.to_string()));
                let (message_to_send, _was_truncated) = truncate_if_needed(&full_message);
                
                bot.send_message(chat_id, message_to_send)
                    .parse_mode(ParseMode::MarkdownV2)
                    .await?;
                break;
            }
        }
    }

    // Stop typing indicator
    typing_handle.abort();

    // Finalize any pending live message
    if let Some((message_id, mut live_msg)) = current_live_message {
        if !live_msg.content.trim().is_empty() && !live_msg.is_finalized {
            live_msg.finalize();
            if let Err(e) = bot
                .edit_message_text(chat_id, message_id, &live_msg.content)
                .parse_mode(ParseMode::MarkdownV2)
                .await
            {
                if !e.to_string().contains("message is not modified") {
                    log::error!("Failed to finalize message: {}", e);
                }
            }
        }
    }

    // Final processing complete

    Ok(())
}

/// Process Claude batch output
async fn process_claude_batch(
    bot: Bot,
    chat_id: ChatId,
    output: String,
    bot_state: BotState,
) -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
    // Process batch output directly

    // Parse all lines using the message parser
    let parse_results = ClaudeMessageParser::parse_lines(&output);

    for parse_result in parse_results {
        match parse_result {
            ParseResult::Message(parsed) => {
                // Update conversation ID if available
                if let Some(conversation_id) = &parsed.conversation_id {
                    update_conversation_id(&bot_state, chat_id.0, conversation_id.clone()).await;
                }

                // If this is a result message, send summary
                if let MessageType::Result {
                    is_error,
                    cost,
                    duration_ms,
                    num_turns,
                    usage,
                    ..
                } = parsed.message_type
                {
                    let mut summary_parts = Vec::new();

                    if let Some(c) = cost {
                        if c > 0.0 {
                            summary_parts.push(format!("Cost: ${:.4}", c));
                        }
                    }

                    if let Some(d) = duration_ms {
                        summary_parts.push(format!("Duration: {}ms", d));
                    }

                    if let Some(t) = num_turns {
                        summary_parts.push(format!("Turns: {}", t));
                    }

                    if let Some(u) = usage {
                        summary_parts.push(format!("Tokens: {} in / {} out", u.input_tokens, u.output_tokens));
                    }

                    let summary_body = if summary_parts.is_empty() {
                        "Run completed".to_string()
                    } else {
                        summary_parts.join(" • ")
                    };

                    let status_emoji = if is_error { "❌" } else { "✅" };

                    let summary_message = format!(
                        "{} *Claude Run Summary*\n{}",
                        status_emoji,
                        escape_markdown_v2(&summary_body)
                    );

                    let (message_to_send, _was_truncated) = truncate_if_needed(&summary_message);

                    bot.send_message(chat_id, message_to_send)
                        .parse_mode(ParseMode::MarkdownV2)
                        .await?;
                }
            }
            ParseResult::PlainText(_) => {}
            ParseResult::Empty => {}
        }
    }

    log::info!("Batch processing completed for chat {}", chat_id.0);

    Ok(())
}

/// Update live message for streaming
async fn update_live_message(
    bot: Bot,
    chat_id: ChatId,
    new_content: &str,
    current_live_message: &mut Option<(MessageId, LiveMessage)>,
) -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
    match current_live_message {
        Some((message_id, live_msg)) => {
            // Prepare new content
            let updated_content = if !live_msg.content.trim().is_empty() {
                format!("{}\n\n{}", live_msg.content, new_content)
            } else {
                new_content.to_string()
            };

            // Truncate if needed to fit Telegram's message limit
            let (truncated_content, _was_truncated) = truncate_if_needed(&updated_content);

            // Only update if content actually changed
            let content_changed = live_msg.update_content(truncated_content);

            // Update message if enough time has passed AND content changed
            if content_changed && live_msg.should_update() {
                match bot
                    .edit_message_text(chat_id, *message_id, &live_msg.content)
                    .parse_mode(ParseMode::MarkdownV2)
                    .await
                {
                    Ok(_) => {
                        live_msg.last_update = Instant::now();
                    }
                    Err(e) => {
                        if !e.to_string().contains("message is not modified") {
                            log::error!("Failed to edit message: {}", e);
                            return Err(e.into());
                        }
                        live_msg.last_update = Instant::now();
                    }
                }
            }
        }
        None => {
            // Create new message with truncation if needed
            let (content_to_send, _was_truncated) = truncate_if_needed(new_content);
            let sent_message = bot
                .send_message(chat_id, &content_to_send)
                .parse_mode(ParseMode::MarkdownV2)
                .await?;

            *current_live_message =
                Some((sent_message.id, LiveMessage::new(content_to_send)));
        }
    }

    Ok(())
}

/// Update conversation ID in bot state
async fn update_conversation_id(bot_state: &BotState, chat_id: i64, conversation_id: String) {
    let mut sessions = bot_state.claude_sessions.lock().await;
    if let Some(session) = sessions.get_mut(&chat_id) {
        session.conversation_id = Some(conversation_id.clone());
        log::info!(
            "Updated conversation ID for chat {} to: {}",
            chat_id,
            conversation_id
        );
    } else {
        log::warn!(
            "No Claude session found for chat {} when updating conversation ID",
            chat_id
        );
    }
}
