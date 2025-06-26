pub mod claude_code_client;
pub mod oauth;

#[allow(unused_imports)]
pub use claude_code_client::{
    container_utils, AuthState, AuthenticationHandle, ClaudeCodeClient, ClaudeCodeConfig,
    ClaudeCodeResult, GithubAuthResult, GithubClient, GithubClientConfig, GithubCloneResult,
};

// Re-export OAuth types for backward compatibility
pub use oauth::{ClaudeAuth, Config as OAuthConfig, CredStorageOps, Credentials, OAuthError};
