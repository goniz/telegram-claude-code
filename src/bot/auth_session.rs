use std::collections::HashMap;
use std::sync::Arc;
use tokio::sync::{mpsc, oneshot, Mutex};

/// Authentication session state
#[derive(Debug)]
#[allow(dead_code)]
pub struct AuthSession {
    pub container_name: String,
    pub code_sender: mpsc::UnboundedSender<String>,
    pub cancel_sender: oneshot::Sender<()>,
}

/// Global state for tracking authentication sessions
pub type AuthSessions = Arc<Mutex<HashMap<i64, AuthSession>>>;
