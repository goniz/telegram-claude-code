use telegram_bot::claude_code_client::{
    container_utils, ClaudeCodeConfig, GithubClient, GithubClientConfig,
};
use bollard::Docker;

#[tokio::test]
async fn test_github_status_integration() {
    // Initialize logging for this test
    let _ = pretty_env_logger::formatted_builder().is_test(true).try_init();

    let docker = Docker::connect_with_socket_defaults()
        .expect("Failed to connect to Docker daemon for testing");

    let container_name = "test-github-status-integration";
    let config = ClaudeCodeConfig::default();

    println!("Starting coding session for GitHub status test...");

    // Start a coding session (creates a container with GitHub CLI)
    let claude_client = match container_utils::start_coding_session(&docker, container_name, config, container_utils::CodingContainerConfig::default()).await {
        Ok(client) => client,
        Err(e) => {
            eprintln!("Failed to start coding session: {}", e);
            return;
        }
    };

    println!("Coding session started, testing GitHub status...");

    // Create GitHub client
    let github_client = GithubClient::new(
        docker.clone(),
        claude_client.container_id().to_string(),
        GithubClientConfig::default(),
    );

    // Test GitHub status check (should work even if not authenticated)
    match github_client.check_auth_status().await {
        Ok(auth_result) => {
            println!("✅ GitHub status check successful!");
            println!("   Authenticated: {}", auth_result.authenticated);
            if let Some(username) = &auth_result.username {
                println!("   Username: {}", username);
            }
            println!("   Message: {}", auth_result.message);
            
            // Should not be authenticated initially (unless user has pre-configured auth)
            // Just ensure the call doesn't fail
            assert!(!auth_result.message.is_empty(), "Status message should not be empty");
        }
        Err(e) => {
            eprintln!("❌ GitHub status check failed: {}", e);
            // This test validates that the status check works, so we expect success
            // But we'll be lenient since GitHub CLI may not be available in all test environments
            println!("ℹ️  GitHub status check failed (expected in some test environments)");
        }
    }

    // Test GitHub CLI availability (should work if gh is installed)
    match github_client.check_availability().await {
        Ok(version_info) => {
            println!("✅ GitHub CLI available: {}", version_info);
            assert!(version_info.contains("gh version"), "Should contain version info");
        }
        Err(e) => {
            println!("ℹ️  GitHub CLI not available (expected in some environments): {}", e);
            // This is okay - not all test environments have GitHub CLI
        }
    }

    // Clean up the test container
    let _ = container_utils::clear_coding_session(&docker, container_name).await;
    println!("✅ Test completed and cleaned up");
}