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
    let container_name = format!("test-claude-status-{}", uuid::Uuid::new_v4());
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
async fn test_claude_status_command_with_preinstalled_claude(
    #[future] test_container: (Docker, String, String),
) {
    let (docker, container_id, container_name) = test_container.await;

    let client = ClaudeCodeClient::new(docker.clone(), container_id, ClaudeCodeConfig::default());

    // Claude Code should be pre-installed in the runtime image
    // Simulate the /claudestatus workflow - check availability
    println!("Checking Claude availability...");
    let availability_result = client.check_availability().await;
    assert!(
        availability_result.is_ok(),
        "Claude availability check should succeed: {:?}",
        availability_result
    );

    let version_output = availability_result.unwrap();
    println!("Claude version output: {}", version_output);

    // The output should contain version information or some success indicator
    assert!(
        !version_output.is_empty(),
        "Claude version output should not be empty"
    );
    assert!(
        !version_output.contains("not found"),
        "Should not contain 'not found' error"
    );
    assert!(
        !version_output.contains("OCI runtime exec failed"),
        "Should not contain Docker exec error"
    );

    // Cleanup
    cleanup_container(&docker, &container_name).await;
}
