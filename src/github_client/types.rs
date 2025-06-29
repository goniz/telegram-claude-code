use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct GithubAuthResult {
    pub authenticated: bool,
    pub username: Option<String>,
    pub message: String,
    pub oauth_url: Option<String>,
    pub device_code: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct GithubCloneResult {
    pub success: bool,
    pub repository: String,
    pub target_directory: String,
    pub message: String,
}

#[derive(Debug, Clone)]
pub struct GithubClientConfig {
    pub working_directory: Option<String>,
    #[allow(dead_code)]
    pub exec_timeout_secs: u64,
}

impl Default for GithubClientConfig {
    fn default() -> Self {
        Self {
            working_directory: Some("/workspace".to_string()),
            exec_timeout_secs: 60, // 60 seconds timeout for auth operations
        }
    }
}
