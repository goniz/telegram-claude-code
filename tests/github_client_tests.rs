use bollard::Docker;
use rstest::*;
use telegram_bot::{GithubClient, GithubClientConfig, container_utils};

/// Test fixture that provides a Docker client
#[fixture]
pub fn docker() -> Docker {
    #[cfg(unix)]
    {
        Docker::connect_with_socket_defaults().expect("Failed to connect to Docker")
    }
    #[cfg(windows)]
    {
        Docker::connect_with_named_pipe_defaults().expect("Failed to connect to Docker")
    }
}

/// Test fixture that creates a test container and cleans it up
#[fixture]
pub async fn test_container(docker: Docker) -> (Docker, String, String) {
    let container_name = format!("test-github-{}", uuid::Uuid::new_v4());
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
async fn test_github_client_creation(
    #[future] test_container: (Docker, String, String)
) {
    let (docker, container_id, container_name) = test_container.await;
    
    // Test basic GitHub client creation
    let client = GithubClient::new(docker.clone(), container_id.clone(), GithubClientConfig::default());
    
    // Verify container ID is set correctly
    assert_eq!(client.container_id(), container_id);
    
    // Cleanup
    cleanup_container(&docker, &container_name).await;
}

#[rstest]
#[tokio::test]
async fn test_gh_availability_check(
    #[future] test_container: (Docker, String, String)
) {
    let (docker, container_id, container_name) = test_container.await;
    
    let client = GithubClient::new(docker.clone(), container_id, GithubClientConfig::default());
    
    // Check if gh CLI is available in the container
    let availability_result = client.check_availability().await;
    
    // Cleanup
    cleanup_container(&docker, &container_name).await;
    
    // The availability check should return a result (either success or error)
    assert!(availability_result.is_ok(), "gh CLI availability check should return a result: {:?}", availability_result);
    let version_output = availability_result.unwrap();
    
    // Should contain some output (either version info if gh is installed, or error message if not)
    assert!(!version_output.is_empty(), "Version output should not be empty");
    
    // In the test container, gh may not be installed, which is expected
    // We're testing that the method works and handles both cases gracefully
    println!("gh availability result: {}", version_output);
    
    // The test passes as long as we get a coherent response
    assert!(
        version_output.contains("gh version") || 
        version_output.contains("usage") || 
        version_output.contains("not found") ||
        version_output.contains("executable file not found"),
        "Output should contain expected patterns: {}", version_output
    );
}

#[rstest]
#[tokio::test]
async fn test_github_auth_status_check(
    #[future] test_container: (Docker, String, String)
) {
    let (docker, container_id, container_name) = test_container.await;
    
    let client = GithubClient::new(docker.clone(), container_id, GithubClientConfig::default());
    
    // Test GitHub authentication status check
    let auth_status_result = client.check_auth_status().await;
    
    // Cleanup
    cleanup_container(&docker, &container_name).await;
    
    assert!(auth_status_result.is_ok(), "Auth status check should return a result: {:?}", auth_status_result);
    let auth_result = auth_status_result.unwrap();
    
    // Should have a valid response structure
    assert!(!auth_result.message.is_empty(), "Auth status message should not be empty");
    
    // In test environment without gh CLI, we expect authentication to be false
    // and this is the correct behavior to test
    println!("Auth status result: {:?}", auth_result);
    
    // The method should handle missing gh CLI gracefully
    if auth_result.message.contains("not found") || auth_result.message.contains("executable file not found") {
        assert!(!auth_result.authenticated, "Should not be authenticated when gh CLI is missing");
        assert_eq!(auth_result.username, None, "Username should be None when gh CLI is missing");
    }
}

#[rstest]
#[tokio::test]
async fn test_github_basic_command_execution(
    #[future] test_container: (Docker, String, String)
) {
    let (docker, container_id, container_name) = test_container.await;
    
    let client = GithubClient::new(docker.clone(), container_id, GithubClientConfig::default());
    
    // Test basic command execution through the client
    let result = client.exec_basic_command(vec!["echo".to_string(), "test-github-client".to_string()]).await;
    
    // Cleanup
    cleanup_container(&docker, &container_name).await;
    
    assert!(result.is_ok(), "Basic command execution failed: {:?}", result);
    assert_eq!(result.unwrap().trim(), "test-github-client");
}

#[rstest]
#[tokio::test]
async fn test_github_login_interactive_flow(
    #[future] test_container: (Docker, String, String)
) {
    let (docker, container_id, container_name) = test_container.await;
    
    let client = GithubClient::new(docker.clone(), container_id, GithubClientConfig::default());
    
    // Test the login flow - this will likely fail in CI without user interaction
    // but tests that the method works and handles errors gracefully
    let login_result = client.login().await;
    
    // Cleanup
    cleanup_container(&docker, &container_name).await;
    
    assert!(login_result.is_ok(), "Login method should return a result: {:?}", login_result);
    let auth_result = login_result.unwrap();
    
    // In automated tests, we expect this to fail (no user interaction)
    // but the structure should be valid
    assert!(!auth_result.message.is_empty(), "Login result message should not be empty");
    
    // The login likely failed due to no user interaction in CI
    // This is expected behavior - we're testing the method structure and error handling
    println!("Login result: {:?}", auth_result);
}

#[rstest]
#[tokio::test]
async fn test_github_repo_clone_invalid_repo(
    #[future] test_container: (Docker, String, String)
) {
    let (docker, container_id, container_name) = test_container.await;
    
    let client = GithubClient::new(docker.clone(), container_id, GithubClientConfig::default());
    
    // Test cloning with an invalid repository name
    let clone_result = client.repo_clone("invalid/nonexistent-repo-12345", None).await;
    
    // Cleanup
    cleanup_container(&docker, &container_name).await;
    
    assert!(clone_result.is_ok(), "Clone method should return a result: {:?}", clone_result);
    let clone_response = clone_result.unwrap();
    
    // Should have failed but with proper error handling
    assert!(!clone_response.success, "Clone should fail for invalid repo");
    assert_eq!(clone_response.repository, "invalid/nonexistent-repo-12345");
    assert!(!clone_response.message.is_empty(), "Clone result message should not be empty");
    
    println!("Clone result: {:?}", clone_response);
}

#[rstest]
#[tokio::test]
async fn test_github_repo_clone_with_target_directory(
    #[future] test_container: (Docker, String, String)
) {
    let (docker, container_id, container_name) = test_container.await;
    
    let client = GithubClient::new(docker.clone(), container_id, GithubClientConfig::default());
    
    // Test cloning with a target directory specification
    let clone_result = client.repo_clone("octocat/Hello-World", Some("test-clone-dir")).await;
    
    // Cleanup
    cleanup_container(&docker, &container_name).await;
    
    assert!(clone_result.is_ok(), "Clone method should return a result: {:?}", clone_result);
    let clone_response = clone_result.unwrap();
    
    // Verify the target directory is set correctly
    assert_eq!(clone_response.target_directory, "test-clone-dir");
    assert_eq!(clone_response.repository, "octocat/Hello-World");
    assert!(!clone_response.message.is_empty(), "Clone result message should not be empty");
    
    // Note: This may succeed or fail depending on network access and authentication
    // The important thing is that the structure is correct
    println!("Clone result: {:?}", clone_response);
}

#[rstest]
#[tokio::test]
async fn test_github_client_working_directory_config(
    #[future] test_container: (Docker, String, String)
) {
    let (docker, container_id, container_name) = test_container.await;
    
    // Test with custom working directory
    let custom_config = GithubClientConfig {
        working_directory: Some("/tmp".to_string()),
    };
    
    let client = GithubClient::new(docker.clone(), container_id, custom_config);
    
    // Test that commands execute in the correct working directory
    let result = client.exec_basic_command(vec!["pwd".to_string()]).await;
    
    // Cleanup
    cleanup_container(&docker, &container_name).await;
    
    assert!(result.is_ok(), "pwd command should work: {:?}", result);
    let pwd_output = result.unwrap();
    
    // Should show the working directory as /tmp
    assert_eq!(pwd_output.trim(), "/tmp", "Working directory should be /tmp but got: {}", pwd_output);
}