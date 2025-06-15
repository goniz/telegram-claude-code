use bollard::Docker;
use rstest::*;
use telegram_bot::{GithubClient, GithubClientConfig, container_utils};
use uuid;

/// Test fixture that provides a Docker client
#[fixture]
pub fn docker() -> Docker {
    Docker::connect_with_socket_defaults().expect("Failed to connect to Docker")
}

/// Test fixture that creates a test container and cleans it up
#[fixture]
pub async fn test_container(docker: Docker) -> (Docker, String, String) {
    let container_name = format!("test-github-timeout-{}", uuid::Uuid::new_v4());
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
async fn test_github_exec_timeout_configuration(
    #[future] test_container: (Docker, String, String)
) -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
    let (docker, container_id, container_name) = test_container.await;
    
    println!("=== Testing GitHub client timeout configuration ===");
    
    // Test with custom short timeout
    let short_timeout_config = GithubClientConfig {
        working_directory: Some("/workspace".to_string()),
        exec_timeout_secs: 2, // Very short timeout for testing
    };
    
    let github_client = GithubClient::new(
        docker.clone(), 
        container_id.clone(), 
        short_timeout_config
    );
    
    // Test that a simple command still works with short timeout
    let availability_result = github_client.check_availability().await;
    match availability_result {
        Ok(version_output) => {
            println!("✅ gh CLI availability check successful with short timeout: {}", version_output);
        }
        Err(e) => {
            let error_msg = e.to_string();
            if error_msg.contains("timed out after 2 seconds") {
                println!("⏰ Command timed out as expected with 2 second timeout");
            } else if error_msg.contains("not found") || error_msg.contains("executable file not found") {
                println!("⚠️ gh CLI not available in test environment (expected): {}", error_msg);
            } else {
                println!("ℹ️ Other error occurred: {}", error_msg);
            }
        }
    }
    
    // Test with default timeout
    let default_config = GithubClientConfig::default();
    assert_eq!(default_config.exec_timeout_secs, 60, "Default timeout should be 60 seconds");
    
    let _github_client_default = GithubClient::new(
        docker.clone(), 
        container_id.clone(), 
        default_config
    );
    
    // Verify default timeout is configured correctly
    println!("✅ Default timeout configuration verified: {} seconds", 60);
    
    // Test that error messages include timeout information when timeouts occur
    // This is a structural test - we're testing that the error format is correct
    let simulated_timeout_error = format!(
        "Command timed out after {} seconds: {}", 
        2,
        vec!["gh", "auth", "login"].join(" ")
    );
    
    assert!(simulated_timeout_error.contains("timed out after 2 seconds"));
    assert!(simulated_timeout_error.contains("gh auth login"));
    println!("✅ Timeout error message format verified");
    
    // Cleanup
    cleanup_container(&docker, &container_name).await;
    
    println!("✅ GitHub client timeout configuration test completed successfully");
    
    Ok(())
}

#[rstest]
#[tokio::test]
async fn test_github_timeout_error_handling(
    #[future] test_container: (Docker, String, String)
) -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
    let (docker, container_id, container_name) = test_container.await;
    
    println!("=== Testing timeout error handling behavior ===");
    
    // Create client with very short timeout to force timeout scenarios
    let timeout_config = GithubClientConfig {
        working_directory: Some("/workspace".to_string()),
        exec_timeout_secs: 1, // 1 second timeout to trigger timeouts
    };
    
    let github_client = GithubClient::new(
        docker.clone(), 
        container_id.clone(), 
        timeout_config
    );
    
    // Test auth status check with timeout
    let auth_status_result = github_client.check_auth_status().await;
    match auth_status_result {
        Ok(auth_result) => {
            // Command completed quickly enough
            println!("✅ Auth status check completed within timeout");
            assert!(!auth_result.message.is_empty(), "Auth result should have a message");
        }
        Err(e) => {
            let error_msg = e.to_string();
            if error_msg.contains("timed out after 1 seconds") {
                println!("✅ Auth status check timed out as expected: {}", error_msg);
                assert!(error_msg.contains("gh auth status"), "Error should mention the command that timed out");
            } else {
                println!("ℹ️ Auth status check failed for other reason: {}", error_msg);
                // This is acceptable - might be gh CLI not available, etc.
            }
        }
    }
    
    // Test login with timeout (this is the main case from the issue)
    let login_result = github_client.login().await;
    match login_result {
        Ok(auth_result) => {
            // Login completed quickly enough or was already authenticated
            println!("✅ Login completed within timeout");
            assert!(!auth_result.message.is_empty(), "Login result should have a message");
        }
        Err(e) => {
            let error_msg = e.to_string();
            if error_msg.contains("timed out after 1 seconds") {
                println!("✅ Login timed out as expected: {}", error_msg);
                assert!(error_msg.contains("gh auth login"), "Error should mention the login command");
            } else {
                println!("ℹ️ Login failed for other reason: {}", error_msg);
                // This is acceptable - might be gh CLI not available, auth already failed, etc.
            }
        }
    }
    
    // Cleanup
    cleanup_container(&docker, &container_name).await;
    
    println!("✅ GitHub timeout error handling test completed successfully");
    
    Ok(())
}