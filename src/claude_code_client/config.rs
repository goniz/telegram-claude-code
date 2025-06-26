use crate::oauth::Config as OAuthConfig;

#[derive(Debug, Clone)]
#[allow(dead_code)]
pub struct ClaudeCodeConfig {
    pub model: String,
    pub max_tokens: Option<u32>,
    pub temperature: Option<f32>,
    pub working_directory: Option<String>,
    /// OAuth configuration for Claude authentication
    pub oauth_config: OAuthConfig,
}

impl Default for ClaudeCodeConfig {
    fn default() -> Self {
        Self {
            model: "claude-sonnet-4".to_string(),
            max_tokens: None,
            temperature: None,
            working_directory: Some("/workspace".to_string()),
            oauth_config: OAuthConfig::default(),
        }
    }
}
