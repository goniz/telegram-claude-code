/// Default OAuth configuration for Claude Code
#[derive(Clone, Debug)]
pub struct Config {
    /// OAuth authorization URL
    pub authorize_url: String,
    /// OAuth token exchange URL  
    pub token_url: String,
    /// OAuth client ID
    pub client_id: String,
    /// OAuth redirect URI
    pub redirect_uri: String,
    /// OAuth scopes to request
    pub scopes: Vec<String>,
    /// State expiration time in seconds (default: 600 = 10 minutes)
    pub state_expiry_seconds: u64,
}

impl Default for Config {
    fn default() -> Self {
        Self {
            authorize_url: "https://claude.ai/oauth/authorize".to_string(),
            token_url: "https://console.anthropic.com/v1/oauth/token".to_string(),
            client_id: "9d1c250a-e61b-44d9-88ed-5944d1962f5e".to_string(),
            redirect_uri: "https://console.anthropic.com/oauth/code/callback".to_string(),
            scopes: vec![
                "org:create_api_key".to_string(),
                "user:profile".to_string(),
                "user:inference".to_string(),
            ],
            state_expiry_seconds: 600,
        }
    }
}
