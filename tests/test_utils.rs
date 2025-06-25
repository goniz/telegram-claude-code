//! Test utilities for container management and cleanup
//!
//! This module provides RAII-based container management that ensures proper cleanup
//! even when tests panic or return early.

use bollard::Docker;
use telegram_bot::{
    container_utils::{self, CodingContainerConfig},
    ClaudeCodeClient, ClaudeCodeConfig,
};
use uuid::Uuid;

/// Test container guard that automatically cleans up containers when dropped
/// This ensures cleanup happens even if tests panic or return early
pub struct TestContainerGuard {
    docker: Docker,
    container_name: String,
    user_id: Option<i64>,
}

#[allow(dead_code)]
impl TestContainerGuard {
    /// Create a new test container with automatic cleanup
    pub async fn new() -> Result<Self, Box<dyn std::error::Error + Send + Sync>> {
        let docker = Docker::connect_with_local_defaults()?;
        let container_name = Self::generate_unique_container_name("test");

        Ok(Self {
            docker,
            container_name,
            user_id: None,
        })
    }

    /// Create a new test container with socket-based Docker connection
    pub async fn new_with_socket() -> Result<Self, Box<dyn std::error::Error + Send + Sync>> {
        let docker = Docker::connect_with_socket_defaults()?;
        let container_name = Self::generate_unique_container_name("test");

        Ok(Self {
            docker,
            container_name,
            user_id: None,
        })
    }

    /// Create a test container with persistence (user ID for volume)
    pub async fn new_with_persistence(
        user_id: i64,
    ) -> Result<Self, Box<dyn std::error::Error + Send + Sync>> {
        let docker = Docker::connect_with_local_defaults()?;
        let container_name = Self::generate_unique_container_name("test-persist");

        Ok(Self {
            docker,
            container_name,
            user_id: Some(user_id),
        })
    }

    /// Create a test container with custom name prefix
    pub async fn new_with_prefix(
        prefix: &str,
    ) -> Result<Self, Box<dyn std::error::Error + Send + Sync>> {
        let docker = Docker::connect_with_local_defaults()?;
        let container_name = Self::generate_unique_container_name(prefix);

        Ok(Self {
            docker,
            container_name,
            user_id: None,
        })
    }

    /// Start a coding session in this container
    pub async fn start_coding_session(
        &self,
    ) -> Result<ClaudeCodeClient, Box<dyn std::error::Error + Send + Sync>> {
        let config = CodingContainerConfig {
            persistent_volume_key: self.user_id.map(|id| id.to_string()),
            force_pull: false,
            ..Default::default()
        };

        container_utils::start_coding_session(
            &self.docker,
            &self.container_name,
            ClaudeCodeConfig::default(),
            config,
        )
        .await
    }

    /// Get the container name
    pub fn container_name(&self) -> &str {
        &self.container_name
    }

    /// Get the Docker client
    pub fn docker(&self) -> &Docker {
        &self.docker
    }

    /// Get the user ID (if any)
    pub fn user_id(&self) -> Option<i64> {
        self.user_id
    }

    /// Generate a unique container name with timestamp and UUID for parallel test safety
    fn generate_unique_container_name(prefix: &str) -> String {
        let timestamp = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_millis();
        let uuid = Uuid::new_v4();
        format!("{}-{}-{}", prefix, timestamp, uuid.simple())
    }

    /// Manual cleanup (called automatically by Drop, but can be called explicitly)
    pub async fn cleanup(&self) {
        // Clean up container
        if let Err(e) =
            container_utils::clear_coding_session(&self.docker, &self.container_name).await
        {
            eprintln!(
                "Warning: Failed to cleanup container {}: {}",
                self.container_name, e
            );
        }

        // Clean up volume if persistent
        if let Some(user_id) = self.user_id {
            let volume_name = container_utils::generate_volume_name(&user_id.to_string());
            if let Err(e) = self.docker.remove_volume(&volume_name, None).await {
                eprintln!("Warning: Failed to cleanup volume {}: {}", volume_name, e);
            }
        }
    }
}

impl Drop for TestContainerGuard {
    fn drop(&mut self) {
        // Schedule cleanup on a blocking thread since Drop can't be async
        let docker = self.docker.clone();
        let container_name = self.container_name.clone();
        let user_id = self.user_id;

        // Use a blocking task to handle cleanup
        // Note: This is best-effort cleanup. For critical cleanup, use explicit cleanup() calls.
        std::thread::spawn(move || {
            let rt = match tokio::runtime::Runtime::new() {
                Ok(rt) => rt,
                Err(e) => {
                    eprintln!("Warning: Failed to create runtime for cleanup: {}", e);
                    return;
                }
            };

            rt.block_on(async {
                // Clean up container
                if let Err(e) =
                    container_utils::clear_coding_session(&docker, &container_name).await
                {
                    eprintln!(
                        "Warning: Failed to cleanup container {} in Drop: {}",
                        container_name, e
                    );
                }

                // Clean up volume if persistent
                if let Some(user_id) = user_id {
                    let volume_name = container_utils::generate_volume_name(&user_id.to_string());
                    if let Err(e) = docker.remove_volume(&volume_name, None).await {
                        eprintln!(
                            "Warning: Failed to cleanup volume {} in Drop: {}",
                            volume_name, e
                        );
                    }
                }
            });
        });
    }
}

/// Convenience macro for creating a test container guard with error propagation
#[macro_export]
macro_rules! test_container {
    () => {
        test_utils::TestContainerGuard::new().await?
    };
    (socket) => {
        test_utils::TestContainerGuard::new_with_socket().await?
    };
    (persist: $user_id:expr) => {
        test_utils::TestContainerGuard::new_with_persistence($user_id).await?
    };
    (prefix: $prefix:expr) => {
        test_utils::TestContainerGuard::new_with_prefix($prefix).await?
    };
}

/// Result type for test operations
pub type TestResult<T = ()> = Result<T, Box<dyn std::error::Error + Send + Sync>>;

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn test_container_guard_creation() {
        let guard = TestContainerGuard::new()
            .await
            .expect("Should create guard");
        assert!(!guard.container_name().is_empty());
        assert!(guard.container_name().starts_with("test-"));
        // Guard will auto-cleanup on drop
    }

    #[tokio::test]
    async fn test_unique_container_names() {
        let guard1 = TestContainerGuard::new()
            .await
            .expect("Should create guard1");
        let guard2 = TestContainerGuard::new()
            .await
            .expect("Should create guard2");

        assert_ne!(guard1.container_name(), guard2.container_name());
        // Guards will auto-cleanup on drop
    }
}
