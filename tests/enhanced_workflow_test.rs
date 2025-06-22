use bollard::Docker;
use rstest::*;
use telegram_bot::{container_utils, ClaudeCodeConfig, GithubClient, GithubClientConfig};
use uuid::Uuid;

/// Test fixture that provides a Docker client
#[fixture]
pub fn docker() -> Docker {
    Docker::connect_with_socket_defaults().expect("Failed to connect to Docker")
}

/// Cleanup fixture that ensures test containers are removed
pub async fn cleanup_container(docker: &Docker, container_name: &str) {
    let _ = container_utils::clear_coding_session(docker, container_name).await;
}

#[rstest]
#[tokio::test]
async fn test_enhanced_start_workflow_with_auth_checks(docker: Docker) {
    let container_name = format!("test-enhanced-workflow-{}", Uuid::new_v4());

    // Test the enhanced start workflow that includes authentication checking

    // Step 1: Simulate /start command (container creation)
    println!("=== STEP 1: Starting coding session (enhanced /start workflow) ===");
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

    // Step 2: Simulate the authentication checks that the enhanced /start command performs
    println!("=== STEP 2: Testing authentication status checking ===");

    // Test GitHub authentication check
    let github_client = GithubClient::new(
        docker.clone(),
        claude_client.container_id().to_string(),
        GithubClientConfig::default(),
    );

    let github_auth_result = github_client.check_auth_status().await;
    // We expect this to work (either authenticated or not, but not error)
    // In test environment, it will likely be not authenticated, which is fine
    println!("GitHub auth check completed: {:?}", github_auth_result);

    // Test Claude authentication check
    let claude_auth_result = claude_client.check_auth_status().await;
    // We expect this to work (either authenticated or not, but not error)
    println!("Claude auth check completed: {:?}", claude_auth_result);

    // Step 3: Verify Claude status works (as in the original workflow test)
    println!("=== STEP 3: Verifying Claude Code availability ===");
    let availability_result = claude_client.check_availability().await;

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

    println!("🎉 Enhanced workflow test passed!");
}

#[rstest]
#[tokio::test]
async fn test_authentication_check_resilience(docker: Docker) {
    let container_name = format!("test-auth-resilience-{}", Uuid::new_v4());

    // Test that authentication checks handle errors gracefully

    println!("=== Testing authentication check error handling ===");
    let claude_client_result = container_utils::start_coding_session(
        &docker,
        &container_name,
        ClaudeCodeConfig::default(),
        container_utils::CodingContainerConfig::default(),
    )
    .await;

    assert!(claude_client_result.is_ok());
    let claude_client = claude_client_result.unwrap();

    // Test GitHub authentication check resilience
    let github_client = GithubClient::new(
        docker.clone(),
        claude_client.container_id().to_string(),
        GithubClientConfig::default(),
    );

    // This should not panic even if GitHub CLI is not set up
    let github_auth_result = github_client.check_auth_status().await;
    println!("GitHub auth result (expected to be error or not authenticated): {:?}", github_auth_result);
    
    // We don't assert the result because in test environment it might fail,
    // but we verify it doesn't panic and returns a Result

    // Test Claude authentication check resilience
    let claude_auth_result = claude_client.check_auth_status().await;
    println!("Claude auth result: {:?}", claude_auth_result);
    
    // Claude check should succeed (returning true or false)
    assert!(claude_auth_result.is_ok(), "Claude auth check should not error");

    // Cleanup
    cleanup_container(&docker, &container_name).await;

    println!("✅ Authentication resilience test passed!");
}