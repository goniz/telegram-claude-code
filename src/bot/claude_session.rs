use std::collections::HashMap;
use std::sync::Arc;
use tokio::process::Child;
use tokio::sync::Mutex;

/// Claude conversation session state
#[derive(Debug)]
pub struct ClaudeSession {
    pub conversation_id: Option<String>,
    pub process_handle: Option<Child>,
    pub is_active: bool,
}

impl ClaudeSession {
    pub fn new() -> Self {
        Self {
            conversation_id: None,
            process_handle: None,
            is_active: false,
        }
    }

    pub fn start_conversation(&mut self, conversation_id: String, process: Child) {
        self.conversation_id = Some(conversation_id);
        self.process_handle = Some(process);
        self.is_active = true;
    }

    pub fn stop_conversation(&mut self) {
        if let Some(mut process) = self.process_handle.take() {
            let _ = process.kill();
        }
        self.is_active = false;
    }

    pub fn reset_conversation(&mut self) {
        self.stop_conversation();
        self.conversation_id = None;
    }
}

/// Global state for tracking Claude conversation sessions
pub type ClaudeSessions = Arc<Mutex<HashMap<i64, ClaudeSession>>>;