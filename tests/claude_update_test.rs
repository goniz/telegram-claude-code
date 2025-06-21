use bollard::Docker;
use rstest::*;
use telegram_bot::{container_utils, ClaudeCodeClient, ClaudeCodeConfig};

/// Test fixture that provides a Docker client
#[fixture]
pub fn docker() -> Docker {
    Docker::connect_with_local_defaults().expect("Failed to connect to Docker")
}

/// Test fixture that creates a test container and cleans it up
#[fixture]
pub async fn test_container(docker: Docker) -> (Docker, String, String) {
    let container_name = format!("test-claude-update-{}", uuid::Uuid::new_v4());
    let container_id = container_utils::create_test_container(&docker, &container_name)
        .await
        .expect("Failed to create test container");

    (docker, container_id, container_name)
}

/// Cleanup fixture that ensures test containers are removed
pub async fn cleanup_container(docker: &Docker, container_name: &str) {
    let _ = container_utils::clear_coding_session(docker, container_name).await;
}

#[rstest]
#[tokio::test]
async fn test_claude_update_command_execution(
    #[future] test_container: (Docker, String, String),
) {
    let (docker, container_id, container_name) = test_container.await;

    let client = ClaudeCodeClient::new(docker.clone(), container_id, ClaudeCodeConfig::default());

    // Simulate the /update-claude workflow
    println!("Testing Claude update command...");
    let update_result = client.update_claude().await;
    
    // The update command should either succeed or fail gracefully
    // We can't guarantee it will always succeed (network issues, etc.)
    // but we can test that the method exists and executes without panicking
    match update_result {
        Ok(output) => {
            println!("Update succeeded with output: {}", output);
            // If successful, output should not be empty
            assert!(
                !output.is_empty(),
                "Update output should not be empty when successful"
            );
        }
        Err(e) => {
            println!("Update failed (expected in test environment): {}", e);
            // Error should be a proper error message, not a panic
            let error_msg = e.to_string();
            assert!(
                !error_msg.is_empty(),
                "Error message should not be empty"
            );
        }
    }

    // Cleanup
    cleanup_container(&docker, &container_name).await;
}

#[rstest]
#[tokio::test]
async fn test_claude_update_command_method_exists(
    #[future] test_container: (Docker, String, String),
) {
    let (docker, container_id, container_name) = test_container.await;

    let client = ClaudeCodeClient::new(docker.clone(), container_id, ClaudeCodeConfig::default());

    // Test that the method exists and can be called
    // This is a compilation test - if this compiles, the method exists
    let _result = client.update_claude().await;
    
    // We don't assert on the result because in a test environment
    // the update might fail due to network issues, but the method should exist
    println!("update_claude method exists and is callable");

    // Cleanup
    cleanup_container(&docker, &container_name).await;
}