use bollard::Docker;
use rstest::*;
use telegram_bot::{container_utils, GithubClient, GithubClientConfig};

/// Test fixture that provides a Docker client
#[fixture]
pub fn docker() -> Docker {
    Docker::connect_with_socket_defaults().expect("Failed to connect to Docker")
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
async fn test_github_client_creation(#[future] test_container: (Docker, String, String)) {
    let (docker, container_id, container_name) = test_container.await;

    // Test basic GitHub client creation
    let client = GithubClient::new(
        docker.clone(),
        container_id.clone(),
        GithubClientConfig::default(),
    );

    // Verify container ID is set correctly
    assert_eq!(client.container_id(), container_id);

    // Cleanup
    cleanup_container(&docker, &container_name).await;
}

#[rstest]
#[tokio::test]
async fn test_gh_availability_check(#[future] test_container: (Docker, String, String)) {
    let (docker, container_id, container_name) = test_container.await;

    let client = GithubClient::new(docker.clone(), container_id, GithubClientConfig::default());

    // Check if gh CLI is available in the container
    let availability_result = client.check_availability().await;

    // Cleanup
    cleanup_container(&docker, &container_name).await;

    // The availability check should return a result
    assert!(
        availability_result.is_ok(),
        "gh CLI availability check should return a result: {:?}",
        availability_result
    );
    let version_output = availability_result.unwrap();

    // Should contain some output
    assert!(
        !version_output.is_empty(),
        "Version output should not be empty"
    );

    // Check that gh CLI is actually installed and working
    // If it's not found, the test should fail
    assert!(
        version_output.contains("gh version"),
        "gh CLI must be installed and working. Got: {}",
        version_output
    );
}

#[rstest]
#[tokio::test]
async fn test_github_auth_status_check(#[future] test_container: (Docker, String, String)) {
    let (docker, container_id, container_name) = test_container.await;

    let client = GithubClient::new(docker.clone(), container_id, GithubClientConfig::default());

    // Test GitHub authentication status check
    let auth_status_result = client.check_auth_status().await;

    // Cleanup
    cleanup_container(&docker, &container_name).await;

    assert!(
        auth_status_result.is_ok(),
        "Auth status check should return a result: {:?}",
        auth_status_result
    );
    let auth_result = auth_status_result.unwrap();

    // Should have a valid response structure
    assert!(
        !auth_result.message.is_empty(),
        "Auth status message should not be empty"
    );

    // gh CLI must be installed for this to work
    // If gh CLI is missing, we should fail the test
    assert!(
        !auth_result.message.contains("not found")
            && !auth_result.message.contains("executable file not found"),
        "gh CLI must be installed. Auth status failed with: {}",
        auth_result.message
    );

    println!("Auth status result: {:?}", auth_result);
}

#[rstest]
#[tokio::test]
async fn test_github_basic_command_execution(#[future] test_container: (Docker, String, String)) {
    let (docker, container_id, container_name) = test_container.await;

    let client = GithubClient::new(docker.clone(), container_id, GithubClientConfig::default());

    // Test basic command execution through the client
    let result = client
        .exec_basic_command(vec!["echo".to_string(), "test-github-client".to_string()])
        .await;

    // Cleanup
    cleanup_container(&docker, &container_name).await;

    assert!(
        result.is_ok(),
        "Basic command execution failed: {:?}",
        result
    );
    assert_eq!(result.unwrap().trim(), "test-github-client");
}

#[rstest]
#[tokio::test]
async fn test_github_login_oauth_url_generation(
    #[future] test_container: (Docker, String, String),
) {
    let (docker, container_id, container_name) = test_container.await;

    let client = GithubClient::new(docker.clone(), container_id, GithubClientConfig::default());

    // Test the login flow to verify it returns a valid OAuth2 URL
    let login_result = client.login().await;

    // Cleanup
    cleanup_container(&docker, &container_name).await;

    assert!(
        login_result.is_ok(),
        "Login method should return a result: {:?}",
        login_result
    );
    let auth_result = login_result.unwrap();

    // Should have a valid message
    assert!(
        !auth_result.message.is_empty(),
        "Login result message should not be empty"
    );

    // If not already authenticated, should provide OAuth details
    if !auth_result.authenticated {
        // Should have an OAuth URL for the user to visit
        assert!(
            auth_result.oauth_url.is_some(),
            "OAuth URL should be provided for login"
        );

        let oauth_url = auth_result.oauth_url.as_ref().unwrap();
        assert!(
            oauth_url.contains("github.com"),
            "OAuth URL should be a GitHub URL: {}",
            oauth_url
        );
        assert!(
            oauth_url.starts_with("https://"),
            "OAuth URL should be HTTPS: {}",
            oauth_url
        );

        // Should have a device code for the user
        assert!(
            auth_result.device_code.is_some(),
            "Device code should be provided for login"
        );

        let device_code = auth_result.device_code.as_ref().unwrap();
        assert!(!device_code.is_empty(), "Device code should not be empty");

        println!("OAuth URL: {}", oauth_url);
        println!("Device code: {}", device_code);
    } else {
        println!("Already authenticated: {:?}", auth_result.username);
    }

    println!("Login result: {:?}", auth_result);
}

#[rstest]
#[tokio::test]
async fn test_github_repo_clone_invalid_repo(#[future] test_container: (Docker, String, String)) {
    let (docker, container_id, container_name) = test_container.await;

    let client = GithubClient::new(docker.clone(), container_id, GithubClientConfig::default());

    // Test cloning with an invalid repository name
    let clone_result = client
        .repo_clone("invalid/nonexistent-repo-12345", None)
        .await;

    // Cleanup
    cleanup_container(&docker, &container_name).await;

    assert!(
        clone_result.is_ok(),
        "Clone method should return a result: {:?}",
        clone_result
    );
    let clone_response = clone_result.unwrap();

    // Should have failed but with proper error handling
    assert!(
        !clone_response.success,
        "Clone should fail for invalid repo"
    );
    assert_eq!(clone_response.repository, "invalid/nonexistent-repo-12345");
    assert!(
        !clone_response.message.is_empty(),
        "Clone result message should not be empty"
    );

    println!("Clone result: {:?}", clone_response);
}

#[rstest]
#[tokio::test]
async fn test_github_repo_clone_with_target_directory(
    #[future] test_container: (Docker, String, String),
) {
    let (docker, container_id, container_name) = test_container.await;

    let client = GithubClient::new(docker.clone(), container_id, GithubClientConfig::default());

    // Test cloning with a target directory specification
    let clone_result = client
        .repo_clone("octocat/Hello-World", Some("test-clone-dir"))
        .await;

    // Cleanup
    cleanup_container(&docker, &container_name).await;

    assert!(
        clone_result.is_ok(),
        "Clone method should return a result: {:?}",
        clone_result
    );
    let clone_response = clone_result.unwrap();

    // Verify the target directory is set correctly
    assert_eq!(clone_response.target_directory, "test-clone-dir");
    assert_eq!(clone_response.repository, "octocat/Hello-World");
    assert!(
        !clone_response.message.is_empty(),
        "Clone result message should not be empty"
    );

    // Note: This may succeed or fail depending on network access and authentication
    // The important thing is that the structure is correct
    println!("Clone result: {:?}", clone_response);
}

#[rstest]
#[tokio::test]
async fn test_github_client_working_directory_config(
    #[future] test_container: (Docker, String, String),
) {
    let (docker, container_id, container_name) = test_container.await;

    // Test with custom working directory
    let custom_config = GithubClientConfig {
        working_directory: Some("/tmp".to_string()),
        exec_timeout_secs: 30,
    };

    let client = GithubClient::new(docker.clone(), container_id, custom_config);

    // Test that commands execute in the correct working directory
    let result = client.exec_basic_command(vec!["pwd".to_string()]).await;

    // Cleanup
    cleanup_container(&docker, &container_name).await;

    assert!(result.is_ok(), "pwd command should work: {:?}", result);
    let pwd_output = result.unwrap();

    // Should show the working directory as /tmp
    assert_eq!(
        pwd_output.trim(),
        "/tmp",
        "Working directory should be /tmp but got: {}",
        pwd_output
    );
}

#[rstest]
#[tokio::test]
async fn test_github_repo_list(#[future] test_container: (Docker, String, String)) {
    let (docker, container_id, container_name) = test_container.await;

    let client = GithubClient::new(docker.clone(), container_id, GithubClientConfig::default());

    // Test GitHub repository list command
    let repo_list_result = client.repo_list().await;

    // Cleanup
    cleanup_container(&docker, &container_name).await;

    // The command should either succeed (if authenticated) or fail with auth error
    match repo_list_result {
        Ok(output) => {
            // Success case - user is authenticated and may have repos
            println!("Repo list output: {:?}", output);
        }
        Err(e) => {
            // Expected failure case - user is not authenticated
            let error_msg = e.to_string();
            assert!(
                error_msg.contains("gh auth login") || 
                error_msg.contains("authentication") ||
                error_msg.contains("GH_TOKEN"),
                "Error should be related to authentication, got: {}",
                error_msg
            );
            println!("Expected authentication error: {}", error_msg);
        }
    }
    
    // Note: The actual content depends on authentication status and available repositories
    // The important thing is that the command structure is correct and doesn't crash
}
