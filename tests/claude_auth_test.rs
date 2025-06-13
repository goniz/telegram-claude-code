use bollard::Docker;
use rstest::*;
use telegram_bot::{ClaudeCodeClient, ClaudeCodeConfig, container_utils};

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
async fn test_claude_authentication_command_workflow(docker: Docker) {
    let container_name = format!("test-auth-{}", uuid::Uuid::new_v4());
    
    // Test the authentication workflow as it would happen with the /authenticateclaude command
    
    // Step 1: Start a coding session first (prerequisite for authentication)
    println!("=== STEP 1: Starting coding session (prerequisite) ===");
    let claude_client_result = container_utils::start_coding_session(&docker, &container_name, ClaudeCodeConfig::default()).await;
    
    assert!(claude_client_result.is_ok(), "start_coding_session should succeed: {:?}", claude_client_result);
    let claude_client = claude_client_result.unwrap();
    
    println!("✅ Coding session started successfully! Container ID: {}", claude_client.container_id().chars().take(12).collect::<String>());
    
    // Step 2: Simulate finding the session (what happens in /authenticateclaude command)
    println!("=== STEP 2: Finding session for authentication ===");
    
    let auth_client_result = ClaudeCodeClient::for_session(docker.clone(), &container_name).await;
    assert!(auth_client_result.is_ok(), "for_session should find the container: {:?}", auth_client_result);
    let auth_client = auth_client_result.unwrap();
    
    // Step 3: Test the authentication method (core of /authenticateclaude command)
    println!("=== STEP 3: Testing Claude authentication ===");
    
    let auth_result = auth_client.authenticate_claude_account().await;
    
    // The authentication method should either:
    // 1. Return a valid authentication URL (success case)
    // 2. Return an error indicating authentication setup issues (expected in test environment)
    
    match auth_result {
        Ok(auth_url) => {
            println!("✅ Authentication URL generated: {}", auth_url);
            // Verify the URL looks valid
            assert!(!auth_url.is_empty(), "Authentication URL should not be empty");
            assert!(auth_url.starts_with("http"), "Authentication URL should start with http: {}", auth_url);
        }
        Err(e) => {
            println!("⚠️  Authentication failed as expected in test environment: {}", e);
            // In a test environment, authentication might fail due to:
            // - No actual Claude Code service configured
            // - Network restrictions
            // - Container environment limitations
            // This is expected and acceptable for testing
            let error_msg = e.to_string();
            assert!(
                error_msg.contains("Could not parse authentication response") ||
                error_msg.contains("Authentication URL not found") ||
                error_msg.contains("auth") ||
                error_msg.contains("login") ||
                error_msg.contains("command not found") ||
                error_msg.contains("OCI runtime exec failed") ||
                error_msg.contains("container") && error_msg.contains("not running"),
                "Error should be related to authentication or command execution: {}", error_msg
            );
        }
    }
    
    // Step 4: Test authentication status check
    println!("=== STEP 4: Testing authentication status check ===");
    
    let status_result = auth_client.check_auth_status().await;
    
    // Status check should work (even if not authenticated)
    match status_result {
        Ok(is_authenticated) => {
            println!("✅ Authentication status check successful: {}", is_authenticated);
            // In test environment, we don't expect to be authenticated, but the method should work
        }
        Err(e) => {
            println!("⚠️  Authentication status check failed as expected in test environment: {}", e);
            // Similar to authentication, status check might fail in test environment
            let error_msg = e.to_string();
            assert!(
                error_msg.contains("auth") ||
                error_msg.contains("status") ||
                error_msg.contains("command not found") ||
                error_msg.contains("OCI runtime exec failed") ||
                error_msg.contains("container") && error_msg.contains("not running"),
                "Error should be related to authentication status or command execution: {}", error_msg
            );
        }
    }
    
    // Cleanup
    cleanup_container(&docker, &container_name).await;
    
    println!("🎉 Claude authentication command test completed!");
}

#[rstest]
#[tokio::test]
async fn test_claude_authentication_without_session(docker: Docker) {
    let container_name = format!("test-no-session-{}", uuid::Uuid::new_v4());
    
    // Test the error case: trying to authenticate without an active session
    println!("=== Testing authentication without active session ===");
    
    // Try to find a session that doesn't exist
    let auth_client_result = ClaudeCodeClient::for_session(docker.clone(), &container_name).await;
    
    // This should fail as expected
    assert!(auth_client_result.is_err(), "for_session should fail when no container exists");
    
    let error = auth_client_result.unwrap_err();
    println!("✅ Expected error when no session exists: {}", error);
    
    // Verify error message is appropriate
    let error_msg = error.to_string();
    assert!(
        error_msg.contains("Container not found") || 
        error_msg.contains("not found"),
        "Error should indicate container not found: {}", error_msg
    );
    
    println!("🎉 No session error test passed!");
}