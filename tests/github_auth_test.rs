use bollard::Docker;
use rstest::*;
use telegram_bot::{ClaudeCodeClient, GithubClient, GithubClientConfig, container_utils};
use uuid;

/// Test fixture that provides a Docker client
#[fixture]
pub fn docker() -> Docker {
    Docker::connect_with_socket_defaults().expect("Failed to connect to Docker")
}

/// Test fixture that creates a test container and cleans it up
#[fixture]
pub async fn test_container(docker: Docker) -> (Docker, String, String) {
    let container_name = format!("test-github-auth-{}", uuid::Uuid::new_v4());
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
async fn test_github_auth_command_workflow(
    #[future] test_container: (Docker, String, String)
) -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
    let (docker, _container_id, container_name) = test_container.await;
    
    println!("=== STEP 1: Creating Claude Code client session ===");
    
    // Step 1: Get ClaudeCodeClient session (simulating the session lookup in the command)
    let claude_client = ClaudeCodeClient::for_session(docker.clone(), &container_name).await;
    if claude_client.is_err() {
        return Err(format!("Failed to find session: {:?}", claude_client.unwrap_err()).into());
    }
    let claude_client = claude_client.unwrap();
    
    println!("=== STEP 2: Creating GitHub client from session ===");
    
    // Step 2: Create GitHub client using same pattern as the new command
    let github_client = GithubClient::new(
        docker.clone(), 
        claude_client.container_id().to_string(), 
        GithubClientConfig::default()
    );
    
    println!("=== STEP 3: Testing GitHub auth availability ===");
    
    // Step 3: Check that gh CLI is available (prerequisite for auth)
    let availability_result = github_client.check_availability().await;
    match availability_result {
        Ok(version_output) => {
            println!("✅ gh CLI availability check successful: {}", version_output);
            assert!(
                version_output.contains("gh version"), 
                "gh CLI must be installed and working. Got: {}", version_output
            );
        }
        Err(e) => {
            return Err(format!("gh CLI availability check failed: {}", e).into());
        }
    }
    
    println!("=== STEP 4: Testing GitHub authentication status check ===");
    
    // Step 4: Test authentication status check (part of login flow)
    let auth_status_result = github_client.check_auth_status().await;
    match auth_status_result {
        Ok(auth_result) => {
            println!("✅ GitHub auth status check successful");
            println!("Auth status: authenticated={}, username={:?}, message={}", 
                     auth_result.authenticated, auth_result.username, auth_result.message);
            
            // Should have a valid response structure
            assert!(!auth_result.message.is_empty(), "Auth status message should not be empty");
            
            // gh CLI must be working for this test to be valid
            assert!(!auth_result.message.contains("not found") && 
                   !auth_result.message.contains("executable file not found"), 
                   "gh CLI must be installed. Auth status failed with: {}", auth_result.message);
        }
        Err(e) => {
            return Err(format!("GitHub auth status check failed: {}", e).into());
        }
    }
    
    println!("=== STEP 5: Testing GitHub login initiation (OAuth flow) ===");
    
    // Step 5: Test the login method (core of the new command)
    // Note: In a test environment, this should initiate OAuth flow without completing it
    let login_result = github_client.login().await;
    match login_result {
        Ok(auth_result) => {
            println!("✅ GitHub login initiation successful");
            println!("Login result: authenticated={}, oauth_url={:?}, device_code={:?}", 
                     auth_result.authenticated, auth_result.oauth_url, auth_result.device_code);
            
            // Should return a valid response
            assert!(!auth_result.message.is_empty(), "Login result message should not be empty");
            
            // In test environment, either:
            // 1. Already authenticated (authenticated=true)
            // 2. OAuth flow initiated (oauth_url and device_code provided)
            // 3. Some other status message
            if auth_result.authenticated {
                println!("Already authenticated with GitHub");
            } else if auth_result.oauth_url.is_some() && auth_result.device_code.is_some() {
                println!("OAuth flow initiated successfully");
                assert!(auth_result.oauth_url.unwrap().starts_with("https://"), "OAuth URL should be valid HTTPS URL");
                assert!(!auth_result.device_code.unwrap().is_empty(), "Device code should not be empty");
            } else {
                println!("Login returned status: {}", auth_result.message);
            }
        }
        Err(e) => {
            // In CI/test environments, login might fail due to missing config or network issues
            // This is acceptable as long as the command structure works
            println!("⚠️  GitHub login failed (expected in test environment): {}", e);
            let error_msg = e.to_string();
            
            // Verify it's not a structural error (wrong command, missing gh CLI, etc.)
            assert!(!error_msg.contains("command not found"), 
                   "gh CLI command should exist: {}", error_msg);
            assert!(!error_msg.contains("executable file not found"), 
                   "gh CLI executable should exist: {}", error_msg);
        }
    }
    
    // Cleanup
    cleanup_container(&docker, &container_name).await;
    
    println!("✅ GitHub authentication command workflow test completed successfully");
    
    Ok(())
}

#[rstest]
#[tokio::test]
async fn test_github_auth_without_session() -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
    let docker = Docker::connect_with_socket_defaults()?;
    let non_existent_container = "non-existent-session";
    
    // Test the error handling when no session exists (simulating command behavior)
    let session_result = ClaudeCodeClient::for_session(docker, non_existent_container).await;
    
    // Should fail gracefully
    assert!(session_result.is_err(), "Should fail when session doesn't exist");
    
    let error = session_result.unwrap_err();
    println!("Expected error when no session exists: {}", error);
    
    // Error should be descriptive
    assert!(!error.to_string().is_empty(), "Error message should not be empty");
    
    Ok(())
}