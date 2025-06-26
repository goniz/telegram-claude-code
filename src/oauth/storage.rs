use crate::oauth::errors::OAuthError;
use std::path::PathBuf;

/// Trait for credential storage operations
#[async_trait::async_trait]
pub trait CredStorageOps: Send + Sync {
    /// Load credentials from storage
    async fn load_credentials(&self) -> Result<Option<Vec<u8>>, OAuthError>;

    /// Save credentials to storage
    async fn save_credentials(&self, data: Vec<u8>) -> Result<(), OAuthError>;

    /// Load OAuth state from storage
    async fn load_state(&self) -> Result<Option<Vec<u8>>, OAuthError>;

    /// Save OAuth state to storage
    async fn save_state(&self, data: Vec<u8>) -> Result<(), OAuthError>;

    /// Remove OAuth state from storage
    async fn remove_state(&self) -> Result<(), OAuthError>;
}

/// File-based storage implementation
pub struct FileStorage {
    storage_dir: PathBuf,
}

impl FileStorage {
    pub fn new(storage_dir: PathBuf) -> Self {
        Self { storage_dir }
    }
}

#[async_trait::async_trait]
impl CredStorageOps for FileStorage {
    async fn load_credentials(&self) -> Result<Option<Vec<u8>>, OAuthError> {
        let file_path = self.storage_dir.join("credentials.json");
        match tokio::fs::read(&file_path).await {
            Ok(content) => Ok(Some(content)),
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(None),
            Err(e) => Err(OAuthError::IoError(e)),
        }
    }

    async fn save_credentials(&self, data: Vec<u8>) -> Result<(), OAuthError> {
        let file_path = self.storage_dir.join("credentials.json");
        tokio::fs::write(&file_path, data).await?;
        Ok(())
    }

    async fn load_state(&self) -> Result<Option<Vec<u8>>, OAuthError> {
        let file_path = self.storage_dir.join("claude_oauth_state.json");
        match tokio::fs::read(&file_path).await {
            Ok(content) => Ok(Some(content)),
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(None),
            Err(e) => Err(OAuthError::IoError(e)),
        }
    }

    async fn save_state(&self, data: Vec<u8>) -> Result<(), OAuthError> {
        let file_path = self.storage_dir.join("claude_oauth_state.json");
        tokio::fs::write(&file_path, data).await?;
        Ok(())
    }

    async fn remove_state(&self) -> Result<(), OAuthError> {
        let file_path = self.storage_dir.join("claude_oauth_state.json");
        match tokio::fs::remove_file(&file_path).await {
            Ok(_) => Ok(()),
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(()),
            Err(e) => Err(OAuthError::IoError(e)),
        }
    }
}
