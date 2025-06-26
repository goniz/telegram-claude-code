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

    #[allow(dead_code)]
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

#[cfg(test)]
mod tests {
    use super::*;
    use tokio::process::Command;

    #[test]
    fn test_claude_session_new() {
        let session = ClaudeSession::new();
        assert!(session.conversation_id.is_none());
        assert!(session.process_handle.is_none());
        assert!(!session.is_active);
    }

    #[tokio::test]
    async fn test_claude_session_start_conversation() {
        let mut session = ClaudeSession::new();
        let conversation_id = "test-conversation-123".to_string();
        
        // Create a dummy process (this won't actually run)
        let process = Command::new("echo")
            .arg("test")
            .spawn()
            .expect("Failed to spawn test process");

        session.start_conversation(conversation_id.clone(), process);
        
        assert_eq!(session.conversation_id, Some(conversation_id));
        assert!(session.process_handle.is_some());
        assert!(session.is_active);
    }

    #[tokio::test]
    async fn test_claude_session_stop_conversation() {
        let mut session = ClaudeSession::new();
        let conversation_id = "test-conversation-123".to_string();
        
        // Create a dummy process
        let process = Command::new("echo")
            .arg("test")
            .spawn()
            .expect("Failed to spawn test process");

        session.start_conversation(conversation_id, process);
        assert!(session.is_active);
        assert!(session.process_handle.is_some());

        session.stop_conversation();
        
        assert!(session.process_handle.is_none());
        assert!(!session.is_active);
        // Conversation ID should remain for potential resume
        assert!(session.conversation_id.is_some());
    }

    #[tokio::test]
    async fn test_claude_session_reset_conversation() {
        let mut session = ClaudeSession::new();
        let conversation_id = "test-conversation-123".to_string();
        
        // Create a dummy process
        let process = Command::new("echo")
            .arg("test")
            .spawn()
            .expect("Failed to spawn test process");

        session.start_conversation(conversation_id, process);
        assert!(session.is_active);
        assert!(session.process_handle.is_some());
        assert!(session.conversation_id.is_some());

        session.reset_conversation();
        
        assert!(session.process_handle.is_none());
        assert!(!session.is_active);
        assert!(session.conversation_id.is_none());
    }

    #[test]
    fn test_claude_session_state_transitions() {
        // Test state without process creation
        let mut session = ClaudeSession::new();
        
        // Initial state
        assert!(!session.is_active);
        assert!(session.conversation_id.is_none());
        assert!(session.process_handle.is_none());
        
        // Simulate state changes without actual process
        session.is_active = true;
        session.conversation_id = Some("test-conv".to_string());
        
        assert!(session.is_active);
        assert_eq!(session.conversation_id, Some("test-conv".to_string()));
        
        // Reset state
        session.is_active = false;
        session.conversation_id = None;
        
        assert!(!session.is_active);
        assert!(session.conversation_id.is_none());
    }

    #[tokio::test]
    async fn test_claude_sessions_global_state() {
        let sessions: ClaudeSessions = Arc::new(Mutex::new(HashMap::new()));
        let chat_id = 12345i64;
        
        // Test inserting a new session
        {
            let mut sessions_lock = sessions.lock().await;
            sessions_lock.insert(chat_id, ClaudeSession::new());
        }
        
        // Test retrieving the session
        {
            let sessions_lock = sessions.lock().await;
            let session = sessions_lock.get(&chat_id);
            assert!(session.is_some());
            assert!(!session.unwrap().is_active);
        }
        
        // Test modifying the session
        {
            let mut sessions_lock = sessions.lock().await;
            if let Some(session) = sessions_lock.get_mut(&chat_id) {
                session.is_active = true;
            }
        }
        
        // Verify the modification
        {
            let sessions_lock = sessions.lock().await;
            let session = sessions_lock.get(&chat_id);
            assert!(session.is_some());
            assert!(session.unwrap().is_active);
        }
    }
}