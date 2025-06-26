use async_trait::async_trait;
use bollard::Docker;

use super::container_utils::{container_get_file, container_put_file};
use crate::claude_oauth::{CredStorageOps, OAuthError};

/// Container-based credential storage implementation
///
/// This storage implementation reads and writes credentials and OAuth state
/// to files inside the Docker container using the same paths as configured
/// in the init_volume_structure function.
pub struct ContainerCredStorage {
    docker: Docker,
    container_id: String,
}

impl ContainerCredStorage {
    /// Create a new container credential storage instance
    pub fn new(docker: Docker, container_id: String) -> Self {
        Self {
            docker,
            container_id,
        }
    }

    /// Get the path for credentials file in the container
    fn credentials_path() -> &'static str {
        "/volume_data/claude/.credentials.json"
    }

    /// Get the path for OAuth state file in the container
    fn state_path() -> &'static str {
        "/volume_data/claude/claude_oauth_state.json"
    }
}

#[async_trait]
impl CredStorageOps for ContainerCredStorage {
    /// Load credentials from the container storage
    async fn load_credentials(&self) -> Result<Option<Vec<u8>>, OAuthError> {
        match container_get_file(&self.docker, &self.container_id, Self::credentials_path()).await {
            Ok(content) => Ok(Some(content)),
            Err(e) => {
                // Check if file not found - this is expected for first-time auth
                let error_msg = e.to_string().to_lowercase();
                if error_msg.contains("no such file") || error_msg.contains("not found") {
                    Ok(None)
                } else {
                    Err(OAuthError::CustomHandlerError(format!(
                        "Failed to load credentials from container: {}",
                        e
                    )))
                }
            }
        }
    }

    /// Save credentials to the container storage
    async fn save_credentials(&self, data: Vec<u8>) -> Result<(), OAuthError> {
        container_put_file(
            &self.docker,
            &self.container_id,
            Self::credentials_path(),
            &data,
            Some(0o644),
        )
        .await
        .map_err(|e| {
            OAuthError::CustomHandlerError(format!(
                "Failed to save credentials to container: {}",
                e
            ))
        })
    }

    /// Load OAuth state from the container storage
    async fn load_state(&self) -> Result<Option<Vec<u8>>, OAuthError> {
        match container_get_file(&self.docker, &self.container_id, Self::state_path()).await {
            Ok(content) => Ok(Some(content)),
            Err(e) => {
                // Check if file not found - this is expected for first-time auth
                let error_msg = e.to_string().to_lowercase();
                if error_msg.contains("no such file") || error_msg.contains("not found") {
                    Ok(None)
                } else {
                    Err(OAuthError::CustomHandlerError(format!(
                        "Failed to load OAuth state from container: {}",
                        e
                    )))
                }
            }
        }
    }

    /// Save OAuth state to the container storage
    async fn save_state(&self, data: Vec<u8>) -> Result<(), OAuthError> {
        container_put_file(
            &self.docker,
            &self.container_id,
            Self::state_path(),
            &data,
            Some(0o644),
        )
        .await
        .map_err(|e| {
            OAuthError::CustomHandlerError(format!(
                "Failed to save OAuth state to container: {}",
                e
            ))
        })
    }

    /// Remove OAuth state from the container storage
    async fn remove_state(&self) -> Result<(), OAuthError> {
        use super::container_utils::exec_command_in_container;

        // Use rm command to remove the state file
        match exec_command_in_container(
            &self.docker,
            &self.container_id,
            vec![
                "rm".to_string(),
                "-f".to_string(),
                Self::state_path().to_string(),
            ],
        )
        .await
        {
            Ok(_) => Ok(()),
            Err(e) => {
                // rm -f won't fail if file doesn't exist, but just in case
                let error_msg = e.to_string().to_lowercase();
                if error_msg.contains("no such file") || error_msg.contains("not found") {
                    Ok(())
                } else {
                    Err(OAuthError::CustomHandlerError(format!(
                        "Failed to remove OAuth state from container: {}",
                        e
                    )))
                }
            }
        }
    }
}
