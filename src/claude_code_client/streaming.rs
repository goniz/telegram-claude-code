use serde::{Deserialize, Serialize};

/// Claude CLI streaming JSON message types
#[derive(Debug, Deserialize, Serialize)]
#[serde(tag = "type")]
pub enum ClaudeMessage {
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
pub struct AssistantMessage {
    #[serde(default)]
    pub id: Option<String>,
    #[serde(default)]
    pub content: Option<Vec<ContentBlock>>,
}

#[derive(Debug, Deserialize, Serialize)]
pub struct UserMessage {
    #[serde(default)]
    pub content: Option<Vec<ToolResult>>,
}

#[derive(Debug, Deserialize, Serialize)]
#[serde(tag = "type")]
pub enum ContentBlock {
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
pub struct ToolResult {
    #[serde(default)]
    pub tool_use_id: Option<String>,
    #[serde(default)]
    pub content: Option<String>,
}
