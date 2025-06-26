use super::auth_session::AuthSessions;
use bollard::Docker;

#[derive(Clone)]
pub struct BotState {
    pub docker: Docker,
    pub auth_sessions: AuthSessions,
}
