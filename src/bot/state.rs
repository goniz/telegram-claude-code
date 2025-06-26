use super::auth_session::AuthSessions;
use super::claude_session::ClaudeSessions;
use bollard::Docker;

#[derive(Clone)]
pub struct BotState {
    pub docker: Docker,
    pub auth_sessions: AuthSessions,
    pub claude_sessions: ClaudeSessions,
}
