//! Claude Code OAuth 2.0 Authentication Library
//!
//! This library provides OAuth 2.0 authentication for Claude Code using the
//! Authorization Code flow with PKCE (Proof Key for Code Exchange).
//!
//! # Examples
//!
//! ```no_run
//! use oauth::{ClaudeAuth, Config};
//!
//! #[tokio::main]
//! async fn main() -> Result<(), Box<dyn std::error::Error>> {
//!     let config = Config::default();
//!     let auth = ClaudeAuth::new_default();
//!     
//!     // Generate login URL
//!     let url = auth.generate_login_url().await?;
//!     println!("Visit: {}", url);
//!     
//!     // Exchange authorization code for tokens
//!     let auth_code = "your_auth_code_here";
//!     let credentials = auth.exchange_code(auth_code).await?;
//!     
//!     // Save credentials
//!     auth.save_credentials(&credentials).await?;
//!     
//!     Ok(())
//! }
//! ```

use reqwest::Client;
use std::path::PathBuf;
use url::Url;

// Internal modules
mod config;
mod credentials;
mod errors;
mod flow;
mod storage;

// Public re-exports
pub use config::Config;
pub use credentials::{Account, Credentials, Organization};
pub use errors::OAuthError;
pub use storage::{CredStorageOps, FileStorage};

// Internal re-exports
use credentials::{CredentialsFile, TokenExchangeRequest, TokenResponse};
use flow::OAuthFlow;

/// Main OAuth authentication client
pub struct ClaudeAuth {
    config: Config,
    http_client: Client,
    storage: Box<dyn CredStorageOps>,
}

impl std::fmt::Debug for ClaudeAuth {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("ClaudeAuth")
            .field("config", &self.config)
            .field("http_client", &"<http_client>")
            .field("storage", &"<storage>")
            .finish()
    }
}

impl Default for ClaudeAuth {
    fn default() -> Self {
        Self::new_default()
    }
}

impl ClaudeAuth {
    /// Create a new OAuth client with the given configuration and storage
    pub fn new(config: Config, storage: Box<dyn CredStorageOps>) -> Self {
        let http_client = Client::builder()
            .user_agent(
                "Mozilla/5.0 (Macintosh; Intel Mac OS X 10_15_7) AppleWebKit/537.36 (KHTML, like \
                 Gecko) Chrome/131.0.0.0 Safari/537.36",
            )
            .build()
            .expect("Failed to create HTTP client");

        Self {
            config,
            http_client,
            storage,
        }
    }

    /// Create a new OAuth client with default configuration and file storage
    pub fn new_default() -> Self {
        let storage_dir = std::env::current_dir().unwrap_or_else(|_| PathBuf::from("."));
        Self::with_file_storage(Config::default(), storage_dir)
    }

    /// Create a new OAuth client with file-based storage
    pub fn with_file_storage(config: Config, storage_dir: PathBuf) -> Self {
        Self::new(config, Box::new(FileStorage::new(storage_dir)))
    }

    /// Create a new OAuth client with custom storage
    pub fn with_custom_storage(config: Config, storage: Box<dyn CredStorageOps>) -> Self {
        Self::new(config, storage)
    }

    /// Generate a secure OAuth login URL
    pub async fn generate_login_url(&self) -> Result<String, OAuthError> {
        let (state, code_verifier) = OAuthFlow::generate_secure_params()?;
        let code_challenge = OAuthFlow::create_pkce_challenge(&code_verifier)?;

        OAuthFlow::save_oauth_state(
            self.storage.as_ref(),
            &state,
            &code_verifier,
            self.config.state_expiry_seconds,
        )
        .await?;

        let mut url = Url::parse(&self.config.authorize_url)?;

        {
            let mut query = url.query_pairs_mut();
            query.append_pair("code", "true");
            query.append_pair("client_id", &self.config.client_id);
            query.append_pair("response_type", "code");
            query.append_pair("redirect_uri", &self.config.redirect_uri);
            query.append_pair("scope", &self.config.scopes.join(" "));
            query.append_pair("code_challenge", &code_challenge);
            query.append_pair("code_challenge_method", "S256");
            query.append_pair("state", &state);
        }

        Ok(url.to_string())
    }

    /// Exchange authorization code for access tokens
    pub async fn exchange_code(&self, authorization_code: &str) -> Result<Credentials, OAuthError> {
        let cleaned_code = OAuthFlow::clean_authorization_code(authorization_code);
        let oauth_state = OAuthFlow::load_oauth_state(self.storage.as_ref()).await?;

        OAuthFlow::verify_oauth_state(&oauth_state)?;

        let request = TokenExchangeRequest {
            grant_type: "authorization_code".to_string(),
            client_id: self.config.client_id.clone(),
            code: cleaned_code,
            redirect_uri: self.config.redirect_uri.clone(),
            code_verifier: oauth_state.code_verifier,
            state: oauth_state.state,
        };

        let response = self
            .http_client
            .post(&self.config.token_url)
            .header("Content-Type", "application/json")
            .header("Accept", "application/json, text/plain, */*")
            .header("Accept-Language", "en-US,en;q=0.9")
            .header("Referer", "https://claude.ai/")
            .header("Origin", "https://claude.ai")
            .json(&request)
            .send()
            .await?;

        if !response.status().is_success() {
            let status = response.status();
            let error_text = response.text().await.unwrap_or_default();
            return Err(OAuthError::TokenExchangeFailed(format!(
                "{} - {}",
                status, error_text
            )));
        }

        let response_text = response.text().await?;
        log::debug!("Raw OAuth token response: {}", response_text);
        let token_response: TokenResponse = serde_json::from_str(&response_text)?;

        OAuthFlow::create_credentials(token_response).await
    }

    /// Save credentials
    pub async fn save_credentials(&self, credentials: &Credentials) -> Result<(), OAuthError> {
        let credentials_file = CredentialsFile {
            claude_ai_oauth: credentials.clone(),
        };

        let json_content = serde_json::to_string_pretty(&credentials_file)?;
        self.storage
            .save_credentials(json_content.into_bytes())
            .await?;

        Ok(())
    }

    /// Load credentials
    pub async fn load_credentials(&self) -> Result<Option<Credentials>, OAuthError> {
        let content = match self.storage.load_credentials().await? {
            Some(bytes) => String::from_utf8(bytes)
                .map_err(|e| OAuthError::CustomHandlerError(format!("Invalid UTF-8: {}", e)))?,
            None => return Ok(None),
        };

        let credentials_file: CredentialsFile = serde_json::from_str(&content)?;
        Ok(Some(credentials_file.claude_ai_oauth))
    }

    /// Clean up OAuth state file after successful authentication
    pub async fn cleanup_state(&self) -> Result<(), OAuthError> {
        self.storage.remove_state().await?;
        Ok(())
    }
}
