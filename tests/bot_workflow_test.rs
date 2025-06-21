use bollard::Docker;
use rstest::*;
use telegram_bot::{container_utils, ClaudeCodeClient, ClaudeCodeConfig};

/// Test fixture that provides a Docker client
#[fixture]
pub fn docker() -> Docker {
    Docker::connect_with_local_defaults().expect("Failed to connect to Docker")
}

/// Cleanup fixture that ensures test containers are removed
pub async fn cleanup_container(docker: &Docker, container_name: &str) {
    let _ = container_utils::clear_coding_session(docker, container_name).await;
}

#[rstest]
#[tokio::test]
async fn test_complete_start_claudestatus_workflow(docker: Docker) {
    let container_name = format!("test-workflow-{}", uuid::Uuid::new_v4());

    // Test the complete workflow as it would happen in the bot

    // Step 1: Simulate /start command
    println!("=== STEP 1: Starting coding session (simulating /start) ===");
    let claude_client_result = container_utils::start_coding_session(
        &docker,
        &container_name,
        ClaudeCodeConfig::default(),
        container_utils::CodingContainerConfig::default(),
    )
    .await;

    assert!(
        claude_client_result.is_ok(),
        "start_coding_session should succeed: {:?}",
        claude_client_result
    );
    let claude_client = claude_client_result.unwrap();

    println!(
        "✅ Coding session started successfully! Container ID: {}",
        claude_client
            .container_id()
            .chars()
            .take(12)
            .collect::<String>()
    );

    // Step 2: Simulate /claudestatus command
    println!("=== STEP 2: Checking Claude status (simulating /claudestatus) ===");

    // Create a new client instance for the existing container (simulating finding the session)
    let status_client_result = ClaudeCodeClient::for_session(docker.clone(), &container_name).await;
    assert!(
        status_client_result.is_ok(),
        "for_session should find the container: {:?}",
        status_client_result
    );
    let status_client = status_client_result.unwrap();

    // This is what the /claudestatus command actually calls
    let availability_result = status_client.check_availability().await;

    assert!(
        availability_result.is_ok(),
        "check_availability should succeed after session start: {:?}",
        availability_result
    );

    let version_output = availability_result.unwrap();
    println!("✅ Claude Code is available! Version: {}", version_output);

    // Verify the output looks correct
    assert!(
        !version_output.is_empty(),
        "Version output should not be empty"
    );
    assert!(
        !version_output.contains("not found"),
        "Should not contain 'not found' error"
    );
    assert!(
        !version_output.contains("OCI runtime exec failed"),
        "Should not contain Docker exec error"
    );

    // The version should contain some version information
    assert!(
        version_output.contains("Claude Code")
            || version_output.chars().any(|c| c.is_ascii_digit()),
        "Version output should contain 'Claude Code' or version numbers: {}",
        version_output
    );

    // Cleanup
    cleanup_container(&docker, &container_name).await;

    println!("🎉 Complete workflow test passed!");
}
