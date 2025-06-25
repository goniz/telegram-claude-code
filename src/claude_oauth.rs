//! Claude Code OAuth 2.0 Authentication Library
//! 
//! This library provides OAuth 2.0 authentication for Claude Code using the
//! Authorization Code flow with PKCE (Proof Key for Code Exchange).
//! 
//! # Examples
//! 
//! ```no_run
//! use claude_oauth::{ClaudeAuth, Config};
//! 
//! #[tokio::main]
//! async fn main() -> Result<(), Box<dyn std::error::Error>> {
//!     let config = Config::default();
//!     let auth = ClaudeAuth::new(config);
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

use std::collections::HashMap;
use std::env;
use std::fs;
use std::path::{Path, PathBuf};
use std::time::{SystemTime, UNIX_EPOCH};

use anyhow::{Context, Result};
use base64::{engine::general_purpose::URL_SAFE_NO_PAD, Engine as _};
use rand::RngCore;
use reqwest::Client;
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use tokio;
use url::Url;

/// Default OAuth configuration for Claude Code
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
    /// Directory to store state and credential files
    pub storage_dir: PathBuf,
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
            storage_dir: std::env::current_dir().unwrap_or_else(|_| PathBuf::from(".")),
            state_expiry_seconds: 600,
        }
    }
}

/// OAuth state data for PKCE verification
#[derive(Debug, Serialize, Deserialize, Clone)]
struct OAuthState {
    state: String,
    code_verifier: String,
    timestamp: u64,
    expires_at: u64,
}

/// Token response from OAuth provider
#[derive(Debug, Deserialize)]
struct TokenResponse {
    access_token: String,
    refresh_token: String,
    expires_in: u64,
    scope: Option<String>,
}

/// Claude OAuth credentials - maintains exact JSON compatibility with TypeScript implementation
/// 
/// When serialized to JSON, produces:
/// ```json
/// {
///   "accessToken": "...",
///   "refreshToken": "...", 
///   "expiresAt": 1234567890000,
///   "scopes": ["user:inference", "user:profile"],
///   "isMax": true
/// }
/// ```
#[derive(Debug, Serialize, Deserialize, Clone, PartialEq)]
pub struct Credentials {
    /// OAuth access token - serialized as "accessToken" in JSON
    #[serde(rename = "accessToken")]
    pub access_token: String,
    /// OAuth refresh token - serialized as "refreshToken" in JSON
    #[serde(rename = "refreshToken")]  
    pub refresh_token: String,
    /// Token expiration timestamp in milliseconds - serialized as "expiresAt" in JSON
    #[serde(rename = "expiresAt")]
    pub expires_at: u64,
    /// Granted OAuth scopes - serialized as "scopes" in JSON
    pub scopes: Vec<String>,
    /// Maximum scope flag - serialized as "isMax" in JSON
    #[serde(rename = "isMax")]
    pub is_max: bool,
}

impl Credentials {
    /// Check if the access token is expired
    pub fn is_expired(&self) -> bool {
        let current_time = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map(|d| d.as_millis() as u64)
            .unwrap_or(0);
        current_time >= self.expires_at
    }
    
    /// Get time until token expiration in seconds
    pub fn expires_in_seconds(&self) -> Option<u64> {
        let current_time = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map(|d| d.as_millis() as u64)
            .unwrap_or(0);
        
        if current_time < self.expires_at {
            Some((self.expires_at - current_time) / 1000)
        } else {
            None
        }
    }
}

/// Credentials file format - maintains exact JSON compatibility with TypeScript implementation
/// 
/// When serialized to JSON, produces:
/// ```json
/// {
///   "claudeAiOauth": {
///     "accessToken": "...",
///     "refreshToken": "...",
///     "expiresAt": 1234567890000,
///     "scopes": ["user:inference", "user:profile"],
///     "isMax": true
///   }
/// }
/// ```
#[derive(Debug, Serialize, Deserialize)]
struct CredentialsFile {
    /// OAuth credentials - serialized as "claudeAiOauth" in JSON to match TypeScript implementation
    #[serde(rename = "claudeAiOauth")]
    claude_ai_oauth: Credentials,
}

/// Token exchange request parameters
#[derive(Debug, Serialize)]
struct TokenExchangeRequest {
    grant_type: String,
    client_id: String,
    code: String,
    redirect_uri: String,
    code_verifier: String,
    state: String,
}

/// Errors that can occur during OAuth flow
#[derive(Debug, thiserror::Error)]
pub enum OAuthError {
    #[error("Invalid or expired OAuth state")]
    InvalidState,
    #[error("OAuth state file not found")]
    StateNotFound,
    #[error("Failed to exchange authorization code: {0}")]
    TokenExchangeFailed(String),
    #[error("Invalid authorization code format")]
    InvalidAuthCode,
    #[error("HTTP request failed: {0}")]
    HttpError(#[from] reqwest::Error),
    #[error("JSON serialization error: {0}")]
    JsonError(#[from] serde_json::Error),
    #[error("File I/O error: {0}")]
    IoError(#[from] std::io::Error),
    #[error("System time error: {0}")]
    SystemTimeError(#[from] std::time::SystemTimeError),
}

/// Main OAuth authentication client
pub struct ClaudeAuth {
    config: Config,
    http_client: Client,
}

impl ClaudeAuth {
    /// Create a new OAuth client with the given configuration
    pub fn new(config: Config) -> Self {
        let http_client = Client::builder()
            .user_agent("Mozilla/5.0 (Macintosh; Intel Mac OS X 10_15_7) AppleWebKit/537.36 (KHTML, like Gecko) Chrome/131.0.0.0 Safari/537.36")
            .build()
            .expect("Failed to create HTTP client");
            
        Self { config, http_client }
    }
    
    /// Create a new OAuth client with default configuration
    pub fn default() -> Self {
        Self::new(Config::default())
    }
    
    /// Generate a secure OAuth login URL
    pub async fn generate_login_url(&self) -> Result<String, OAuthError> {
        let (state, code_verifier) = self.generate_secure_params()?;
        let code_challenge = self.create_pkce_challenge(&code_verifier)?;
        
        self.save_oauth_state(&state, &code_verifier).await?;
        
        let mut url = Url::parse(&self.config.authorize_url)
            .context("Invalid authorize URL")?;
            
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
        let cleaned_code = self.clean_authorization_code(authorization_code);
        let oauth_state = self.load_oauth_state().await?;
        
        self.verify_oauth_state(&oauth_state)?;
        
        let request = TokenExchangeRequest {
            grant_type: "authorization_code".to_string(),
            client_id: self.config.client_id.clone(),
            code: cleaned_code,
            redirect_uri: self.config.redirect_uri.clone(),
            code_verifier: oauth_state.code_verifier,
            state: oauth_state.state,
        };
        
        let response = self.http_client
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
            let error_text = response.text().await.unwrap_or_default();
            return Err(OAuthError::TokenExchangeFailed(format!(
                "{} - {}", response.status(), error_text
            )));
        }
        
        let token_response: TokenResponse = response.json().await?;
        self.create_credentials(token_response).await
    }
    
    /// Save credentials to file
    pub async fn save_credentials(&self, credentials: &Credentials) -> Result<(), OAuthError> {
        let credentials_file = CredentialsFile {
            claude_ai_oauth: credentials.clone(),
        };
        
        let file_path = self.config.storage_dir.join("credentials.json");
        let json_content = serde_json::to_string_pretty(&credentials_file)?;
        
        tokio::fs::write(&file_path, json_content).await?;
        Ok(())
    }
    
    /// Load credentials from file  
    pub async fn load_credentials(&self) -> Result<Option<Credentials>, OAuthError> {
        let file_path = self.config.storage_dir.join("credentials.json");
        
        match tokio::fs::read_to_string(&file_path).await {
            Ok(content) => {
                let credentials_file: CredentialsFile = serde_json::from_str(&content)?;
                Ok(Some(credentials_file.claude_ai_oauth))
            }
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(None),
            Err(e) => Err(OAuthError::IoError(e)),
        }
    }
    
    /// Clean up OAuth state file after successful authentication
    pub async fn cleanup_state(&self) -> Result<(), OAuthError> {
        let state_file = self.config.storage_dir.join("claude_oauth_state.json");
        match tokio::fs::remove_file(&state_file).await {
            Ok(_) => Ok(()),
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(()),
            Err(e) => Err(OAuthError::IoError(e)),
        }
    }
    
    // Private helper methods
    
    fn generate_secure_params(&self) -> Result<(String, String), OAuthError> {
        let mut rng = rand::thread_rng();
        let mut state_bytes = [0u8; 32];
        let mut code_verifier_bytes = [0u8; 32];
        
        rng.fill_bytes(&mut state_bytes);
        rng.fill_bytes(&mut code_verifier_bytes);
        
        let state = hex::encode(state_bytes);
        let code_verifier = URL_SAFE_NO_PAD.encode(code_verifier_bytes);
        
        Ok((state, code_verifier))
    }
    
    fn create_pkce_challenge(&self, code_verifier: &str) -> Result<String, OAuthError> {
        let mut hasher = Sha256::new();
        hasher.update(code_verifier.as_bytes());
        Ok(URL_SAFE_NO_PAD.encode(hasher.finalize()))
    }
    
    async fn save_oauth_state(&self, state: &str, code_verifier: &str) -> Result<(), OAuthError> {
        let current_time = SystemTime::now().duration_since(UNIX_EPOCH)?.as_secs();
        
        let oauth_state = OAuthState {
            state: state.to_string(),
            code_verifier: code_verifier.to_string(),
            timestamp: current_time,
            expires_at: current_time + self.config.state_expiry_seconds,
        };
        
        let state_file = self.config.storage_dir.join("claude_oauth_state.json");
        let json_content = serde_json::to_string_pretty(&oauth_state)?;
        
        tokio::fs::write(&state_file, json_content).await?;
        Ok(())
    }
    
    async fn load_oauth_state(&self) -> Result<OAuthState, OAuthError> {
        let state_file = self.config.storage_dir.join("claude_oauth_state.json");
        
        let content = tokio::fs::read_to_string(&state_file).await
            .map_err(|_| OAuthError::StateNotFound)?;
            
        let oauth_state: OAuthState = serde_json::from_str(&content)?;
        Ok(oauth_state)
    }
    
    fn verify_oauth_state(&self, oauth_state: &OAuthState) -> Result<(), OAuthError> {
        let current_time = SystemTime::now().duration_since(UNIX_EPOCH)?.as_secs();
        
        if current_time > oauth_state.expires_at {
            return Err(OAuthError::InvalidState);
        }
        
        Ok(())
    }
    
    fn clean_authorization_code(&self, auth_code: &str) -> String {
        auth_code
            .split('#')
            .next()
            .unwrap_or(auth_code)
            .split('&')
            .next()
            .unwrap_or(auth_code)
            .to_string()
    }
    
    async fn create_credentials(&self, token_response: TokenResponse) -> Result<Credentials, OAuthError> {
        let current_time = SystemTime::now().duration_since(UNIX_EPOCH)?.as_secs();
        
        let scopes = token_response.scope
            .map(|s| s.split(' ').map(|s| s.to_string()).collect())
            .unwrap_or_else(|| vec!["user:inference".to_string(), "user:profile".to_string()]);
        
        Ok(Credentials {
            access_token: token_response.access_token,
            refresh_token: token_response.refresh_token,
            expires_at: (current_time + token_response.expires_in) * 1000,
            scopes,
            is_max: true,
        })
    }
}

// CLI Implementation
#[cfg(feature = "cli")]
mod cli {
    use super::*;
    
    pub async fn run_cli() -> Result<(), Box<dyn std::error::Error>> {
        let args: Vec<String> = env::args().collect();
        let program_name = &args[0];
        
        if args.len() > 1 && (args[1] == "--help" || args[1] == "-h") {
            print_help(program_name);
            return Ok(());
        }
        
        let auth = ClaudeAuth::default();
        let authorization_code = args.get(1);
        
        if let Some(auth_code) = authorization_code {
            handle_code_exchange(&auth, auth_code).await
        } else {
            handle_url_generation(&auth).await
        }
    }
    
    async fn handle_code_exchange(auth: &ClaudeAuth, auth_code: &str) -> Result<(), Box<dyn std::error::Error>> {
        println!("Exchanging authorization code for tokens...");
        
        match auth.exchange_code(auth_code).await {
            Ok(credentials) => {
                println!("\nOAuth token exchange successful!");
                println!("Received scopes: {}", credentials.scopes.join(", "));
                
                if auth.save_credentials(&credentials).await.is_ok() {
                    let _ = auth.cleanup_state().await;
                    
                    println!("\n=== SUCCESS ===");
                    println!("OAuth login successful!");
                    println!("Credentials saved to: credentials.json");
                    
                    if let Some(expires_in) = credentials.expires_in_seconds() {
                        let expires_at = SystemTime::UNIX_EPOCH + 
                            std::time::Duration::from_millis(credentials.expires_at);
                        let datetime = chrono::DateTime::<chrono::Local>::from(expires_at);
                        println!("Token expires at: {}", datetime.format("%Y-%m-%d %H:%M:%S"));
                    }
                    
                    println!("===============");
                    std::process::exit(0);
                }
            }
            Err(e) => {
                eprintln!("Token exchange failed: {}", e);
            }
        }
        
        eprintln!("Login failed!");
        std::process::exit(1);
    }
    
    async fn handle_url_generation(auth: &ClaudeAuth) -> Result<(), Box<dyn std::error::Error>> {
        match auth.generate_login_url().await {
            Ok(url) => {
                println!("{}", url);
                Ok(())
            }
            Err(e) => {
                eprintln!("Failed to generate login URL: {}", e);
                std::process::exit(1);
            }
        }
    }
    
    fn print_help(program_name: &str) {
        println!("Usage: {} [authorization_code]", program_name);
        println!();
        println!("  Without code: Generates an OAuth login URL for Claude Code authentication");
        println!("  With code: Completes OAuth login and exchanges code for tokens");
        println!();
        println!("Arguments:");
        println!("  authorization_code  The code received from the OAuth callback");
        println!();
        println!("Options:");
        println!("  --help, -h          Show this help message");
    }
}

#[cfg(feature = "cli")]
#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    cli::run_cli().await
}