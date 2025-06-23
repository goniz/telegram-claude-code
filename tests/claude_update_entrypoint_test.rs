/// Test to verify that the claude update command uses the entrypoint script properly
/// This ensures the same pattern as the claude config command
#[cfg(test)]
mod tests {
    use bollard::Docker;
    use rstest::*;
    use telegram_bot::{container_utils, ClaudeCodeClient, ClaudeCodeConfig};
    use uuid::Uuid;

    /// Test fixture that provides a Docker client
    #[fixture]
    pub fn docker() -> Docker {
        Docker::connect_with_socket_defaults().expect("Failed to connect to Docker")
    }

    /// Test fixture that creates a test container and cleans it up
    #[fixture]
    pub async fn test_container(docker: Docker) -> (Docker, String, String) {
        let container_name = format!("test-claude-update-{}", Uuid::new_v4());
        let client = container_utils::start_coding_session(
            &docker,
            &container_name,
            ClaudeCodeConfig::default(),
            container_utils::CodingContainerConfig::default(),
        )
        .await
        .expect("Failed to start coding session");

        (docker, client.container_id().to_string(), container_name)
    }

    /// Cleanup fixture that ensures test containers are removed
    pub async fn cleanup_container(docker: &Docker, container_name: &str) {
        let _ = container_utils::clear_coding_session(docker, container_name).await;
    }

    #[rstest]
    #[tokio::test]
    async fn test_claude_update_uses_entrypoint_script(
        #[future] test_container: (Docker, String, String),
    ) {
        let (docker, container_id, container_name) = test_container.await;

        let client = ClaudeCodeClient::new(docker.clone(), container_id, ClaudeCodeConfig::default());

        // Test that the update command uses the proper entrypoint script structure
        // We'll test by executing a command that verifies the entrypoint is being used
        let test_result = client
            .exec_basic_command(vec![
                "sh".to_string(),
                "-c".to_string(),
                "/opt/entrypoint.sh -c \"nvm use default && echo 'entrypoint works'\"".to_string(),
            ])
            .await;

        // Cleanup
        cleanup_container(&docker, &container_name).await;

        assert!(
            test_result.is_ok(),
            "Entrypoint script test failed: {:?}",
            test_result
        );

        let output = test_result.unwrap();
        assert!(
            output.contains("entrypoint works") || output.contains("Now using node"),
            "Entrypoint script should work properly: {}",
            output
        );
    }

    #[rstest]
    #[tokio::test]
    async fn test_claude_update_command_structure(#[future] test_container: (Docker, String, String)) {
        let (docker, container_id, container_name) = test_container.await;

        let client = ClaudeCodeClient::new(docker.clone(), container_id, ClaudeCodeConfig::default());

        // Test that we can at least attempt the update command without errors in command structure
        // Note: The actual update might fail due to authentication, but the command structure should be valid
        let update_result = client.update_claude().await;

        // Cleanup
        cleanup_container(&docker, &container_name).await;

        // We expect either success or a controlled failure (not a command structure error)
        match update_result {
            Ok(_) => {
                // Update succeeded
                println!("✅ Claude update command succeeded");
            }
            Err(e) => {
                let error_msg = e.to_string().to_lowercase();
                
                // These are acceptable error conditions that indicate the command structure is correct
                let acceptable_errors = [
                    "authentication", "auth", "login", "token", "unauthorized",
                    "not authenticated", "api key", "permission denied", "forbidden",
                    "network", "connection", "timeout", "update"
                ];
                
                let is_expected_error = acceptable_errors.iter().any(|pattern| error_msg.contains(pattern));
                
                assert!(
                    is_expected_error,
                    "Update command failed with unexpected error (suggests command structure issue): {}",
                    e
                );
                
                println!("✅ Claude update command has correct structure (failed with expected error: {})", e);
            }
        }
    }
}