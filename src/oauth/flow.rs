use base64::{engine::general_purpose::URL_SAFE_NO_PAD, Engine as _};
use rand::RngCore;
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::time::{SystemTime, UNIX_EPOCH};

use crate::oauth::{
    credentials::{Credentials, TokenResponse},
    errors::OAuthError,
    storage::CredStorageOps,
};

/// OAuth state data for PKCE verification
#[derive(Debug, Serialize, Deserialize, Clone)]
pub(crate) struct OAuthState {
    pub state: String,
    pub code_verifier: String,
    pub timestamp: u64,
    pub expires_at: u64,
}

/// OAuth flow helper methods
pub struct OAuthFlow;

impl OAuthFlow {
    /// Generate secure random parameters for OAuth flow
    pub fn generate_secure_params() -> Result<(String, String), OAuthError> {
        let mut rng = rand::rng();
        let mut state_bytes = [0u8; 32];
        let mut code_verifier_bytes = [0u8; 32];

        rng.fill_bytes(&mut state_bytes);
        rng.fill_bytes(&mut code_verifier_bytes);

        let state = hex::encode(state_bytes);
        let code_verifier = URL_SAFE_NO_PAD.encode(code_verifier_bytes);

        Ok((state, code_verifier))
    }

    /// Create PKCE challenge from code verifier
    pub fn create_pkce_challenge(code_verifier: &str) -> Result<String, OAuthError> {
        let mut hasher = Sha256::new();
        hasher.update(code_verifier.as_bytes());
        Ok(URL_SAFE_NO_PAD.encode(hasher.finalize()))
    }

    /// Save OAuth state to storage
    pub async fn save_oauth_state(
        storage: &dyn CredStorageOps,
        state: &str,
        code_verifier: &str,
        state_expiry_seconds: u64,
    ) -> Result<(), OAuthError> {
        let current_time = SystemTime::now().duration_since(UNIX_EPOCH)?.as_secs();

        let oauth_state = OAuthState {
            state: state.to_string(),
            code_verifier: code_verifier.to_string(),
            timestamp: current_time,
            expires_at: current_time + state_expiry_seconds,
        };

        let json_content = serde_json::to_string_pretty(&oauth_state)?;
        storage.save_state(json_content.into_bytes()).await?;

        Ok(())
    }

    /// Load OAuth state from storage
    pub async fn load_oauth_state(storage: &dyn CredStorageOps) -> Result<OAuthState, OAuthError> {
        let content = match storage.load_state().await? {
            Some(bytes) => String::from_utf8(bytes)
                .map_err(|e| OAuthError::CustomHandlerError(format!("Invalid UTF-8: {}", e)))?,
            None => return Err(OAuthError::StateNotFound),
        };

        let oauth_state: OAuthState = serde_json::from_str(&content)?;
        Ok(oauth_state)
    }

    /// Verify OAuth state is valid and not expired
    pub fn verify_oauth_state(oauth_state: &OAuthState) -> Result<(), OAuthError> {
        let current_time = SystemTime::now().duration_since(UNIX_EPOCH)?.as_secs();

        if current_time > oauth_state.expires_at {
            return Err(OAuthError::InvalidState);
        }

        Ok(())
    }

    /// Clean authorization code by removing URL fragments and parameters
    pub fn clean_authorization_code(auth_code: &str) -> String {
        auth_code
            .split('#')
            .next()
            .unwrap_or(auth_code)
            .split('&')
            .next()
            .unwrap_or(auth_code)
            .to_string()
    }

    /// Create credentials from token response
    pub async fn create_credentials(
        token_response: TokenResponse,
    ) -> Result<Credentials, OAuthError> {
        let current_time = SystemTime::now().duration_since(UNIX_EPOCH)?.as_secs();

        let scopes = token_response
            .scope
            .map(|s| s.split(' ').map(|s| s.to_string()).collect())
            .unwrap_or_else(|| vec!["user:inference".to_string(), "user:profile".to_string()]);

        Ok(Credentials {
            access_token: token_response.access_token,
            refresh_token: token_response.refresh_token,
            expires_at: (current_time + token_response.expires_in) * 1000,
            scopes,
            // TODO: Handle subscription type properly
            subscription_type: "pro".to_string(),
            // OAuth account and organization information
            oauth_account: token_response.account,
            oauth_organization: token_response.organization,
        })
    }
}
