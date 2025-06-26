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
    #[error("URL parsing error: {0}")]
    UrlError(#[from] url::ParseError),
    #[error("General error: {0}")]
    GeneralError(#[from] anyhow::Error),
    #[error("Custom handler error: {0}")]
    CustomHandlerError(String),
}
