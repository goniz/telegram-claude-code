use serde::{Deserialize, Serialize};
use std::time::{SystemTime, UNIX_EPOCH};

/// Organization information from OAuth response
#[derive(Debug, Serialize, Deserialize, Clone, PartialEq, Default)]
pub struct Organization {
    pub uuid: String,
    pub name: String,
}

/// Account information from OAuth response
#[derive(Debug, Serialize, Deserialize, Clone, PartialEq, Default)]
pub struct Account {
    pub uuid: String,
    pub email_address: String,
}

/// Token response from OAuth provider
#[derive(Debug, Serialize, Deserialize, Clone)]
pub(crate) struct TokenResponse {
    pub token_type: String,
    pub access_token: String,
    pub expires_in: u64,
    pub refresh_token: String,
    pub scope: Option<String>,
    pub organization: Organization,
    pub account: Account,
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
///   "subscriptionType": "pro"
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
    /// Subscription type - serialized as "subscriptionType" in JSON
    #[serde(rename = "subscriptionType")]
    pub subscription_type: String,

    /// Optional OAuth account information
    #[serde(skip)]
    pub oauth_account: Account,

    #[serde(skip)]
    pub oauth_organization: Organization,
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
///     "subscriptionType": "pro"
///   }
/// }
/// ```
#[derive(Debug, Serialize, Deserialize)]
pub(crate) struct CredentialsFile {
    /// OAuth credentials - serialized as "claudeAiOauth" in JSON to match TypeScript implementation
    #[serde(rename = "claudeAiOauth")]
    pub claude_ai_oauth: Credentials,
}

/// Token exchange request parameters
#[derive(Debug, Serialize)]
pub(crate) struct TokenExchangeRequest {
    pub grant_type: String,
    pub client_id: String,
    pub code: String,
    pub redirect_uri: String,
    pub code_verifier: String,
    pub state: String,
}
