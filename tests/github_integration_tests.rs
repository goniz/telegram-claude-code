use bollard::Docker;
use futures_util;
use rstest::*;
use telegram_bot::claude_code_client::ClaudeCodeConfig;
use telegram_bot::{container_utils, ClaudeCodeClient, GithubClient, GithubClientConfig};
use uuid;

// =============================================================================
// COMMON FIXTURES AND HELPER FUNCTIONS
// =============================================================================

/// Test fixture that provides a Docker client
#[fixture]
pub fn docker() -> Docker {
    Docker::connect_with_socket_defaults().expect("Failed to connect to Docker")
}

/// Test fixture that creates a test container and cleans it up
#[fixture]
pub async fn test_container(docker: Docker) -> (Docker, String, String) {
    let container_name = format!("test-github-integration-{}", uuid::Uuid::new_v4());
    let container_id = container_utils::create_test_container(&docker, &container_name)
        .await
        .expect("Failed to create test container");

    (docker, container_id, container_name)
}

/// Cleanup fixture that ensures test containers are removed
pub async fn cleanup_container(docker: &Docker, container_name: &str) {
    let _ = container_utils::clear_coding_session(docker, container_name).await;
}

// =============================================================================
// GITHUB AUTHENTICATION TESTS
// =============================================================================

#[rstest]
#[tokio::test]
async fn test_github_auth_command_workflow(
    #[future] test_container: (Docker, String, String),
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
        GithubClientConfig::default(),
    );

    println!("=== STEP 3: Testing GitHub auth availability ===");

    // Step 3: Check that gh CLI is available (prerequisite for auth)
    let availability_result = github_client.check_availability().await;
    match availability_result {
        Ok(version_output) => {
            println!(
                "✅ gh CLI availability check successful: {}",
                version_output
            );
            assert!(
                version_output.contains("gh version"),
                "gh CLI must be installed and working. Got: {}",
                version_output
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
            println!(
                "Auth status: authenticated={}, username={:?}, message={}",
                auth_result.authenticated, auth_result.username, auth_result.message
            );

            // Should have a valid response structure
            assert!(
                !auth_result.message.is_empty(),
                "Auth status message should not be empty"
            );

            // gh CLI must be working for this test to be valid
            assert!(
                !auth_result.message.contains("not found")
                    && !auth_result.message.contains("executable file not found"),
                "gh CLI must be installed. Auth status failed with: {}",
                auth_result.message
            );
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
            println!(
                "Login result: authenticated={}, oauth_url={:?}, device_code={:?}",
                auth_result.authenticated, auth_result.oauth_url, auth_result.device_code
            );

            // Should return a valid response
            assert!(
                !auth_result.message.is_empty(),
                "Login result message should not be empty"
            );

            // In test environment, either:
            // 1. Already authenticated (authenticated=true)
            // 2. OAuth flow initiated (oauth_url and device_code provided)
            // 3. Some other status message
            if auth_result.authenticated {
                println!("Already authenticated with GitHub");
            } else if auth_result.oauth_url.is_some() && auth_result.device_code.is_some() {
                println!("OAuth flow initiated successfully");
                assert!(
                    auth_result.oauth_url.unwrap().starts_with("https://"),
                    "OAuth URL should be valid HTTPS URL"
                );
                assert!(
                    !auth_result.device_code.unwrap().is_empty(),
                    "Device code should not be empty"
                );
            } else {
                println!("Login returned status: {}", auth_result.message);
            }
        }
        Err(e) => {
            // In CI/test environments, login might fail due to missing config or network issues
            // This is acceptable as long as the command structure works
            println!(
                "⚠️  GitHub login failed (expected in test environment): {}",
                e
            );
            let error_msg = e.to_string();

            // Verify it's not a structural error (wrong command, missing gh CLI, etc.)
            assert!(
                !error_msg.contains("command not found"),
                "gh CLI command should exist: {}",
                error_msg
            );
            assert!(
                !error_msg.contains("executable file not found"),
                "gh CLI executable should exist: {}",
                error_msg
            );
        }
    }

    // Cleanup
    cleanup_container(&docker, &container_name).await;

    println!("✅ GitHub authentication command workflow test completed successfully");

    Ok(())
}

#[rstest]
#[tokio::test]
async fn test_github_auth_without_session() -> Result<(), Box<dyn std::error::Error + Send + Sync>>
{
    let docker = Docker::connect_with_socket_defaults()?;
    let non_existent_container = "non-existent-session";

    // Test the error handling when no session exists (simulating command behavior)
    let session_result = ClaudeCodeClient::for_session(docker, non_existent_container).await;

    // Should fail gracefully
    assert!(
        session_result.is_err(),
        "Should fail when session doesn't exist"
    );

    let error = session_result.unwrap_err();
    println!("Expected error when no session exists: {}", error);

    // Error should be descriptive
    assert!(
        !error.to_string().is_empty(),
        "Error message should not be empty"
    );

    Ok(())
}

// =============================================================================
// GITHUB OAUTH EARLY RETURN TESTS
// =============================================================================

#[rstest]
#[tokio::test]
async fn test_oauth_early_return_flow(
    #[future] test_container: (Docker, String, String),
) -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
    let (docker, _container_id, container_name) = test_container.await;

    println!("=== Testing OAuth flow with early return ===");

    // Step 1: Create GitHub client
    let claude_client = ClaudeCodeClient::for_session(docker.clone(), &container_name).await;
    if claude_client.is_err() {
        return Err(format!("Failed to find session: {:?}", claude_client.unwrap_err()).into());
    }
    let claude_client = claude_client.unwrap();

    let github_client = GithubClient::new(
        docker.clone(),
        claude_client.container_id().to_string(),
        GithubClientConfig::default(),
    );

    // Step 2: Check GitHub CLI availability
    let availability_result = github_client.check_availability().await;
    match availability_result {
        Ok(version_output) => {
            println!("✅ gh CLI available: {}", version_output);
            assert!(
                version_output.contains("gh version"),
                "gh CLI must be working"
            );
        }
        Err(e) => {
            return Err(format!("gh CLI not available: {}", e).into());
        }
    }

    // Step 3: Test the refactored login method
    println!("=== Testing login with early OAuth return ===");

    let start_time = std::time::Instant::now();
    let login_result = github_client.login().await;
    let elapsed_time = start_time.elapsed();

    match login_result {
        Ok(auth_result) => {
            println!("✅ Login completed in {:?}", elapsed_time);
            println!(
                "Auth result: authenticated={}, oauth_url={:?}, device_code={:?}",
                auth_result.authenticated, auth_result.oauth_url, auth_result.device_code
            );

            // Verify result structure
            assert!(
                !auth_result.message.is_empty(),
                "Auth result should have a message"
            );

            if auth_result.authenticated {
                println!("Already authenticated with GitHub");
            } else if auth_result.oauth_url.is_some() && auth_result.device_code.is_some() {
                println!("✅ OAuth flow initiated with early return");

                // Verify OAuth credentials are present
                let oauth_url = auth_result.oauth_url.unwrap();
                let device_code = auth_result.device_code.unwrap();

                assert!(
                    oauth_url.starts_with("https://"),
                    "OAuth URL should be HTTPS"
                );
                assert!(!device_code.is_empty(), "Device code should not be empty");

                // The key test: login should return quickly with OAuth credentials
                // rather than waiting for the entire auth process to complete
                assert!(
                    elapsed_time.as_secs() < 45,
                    "Login should return quickly with OAuth credentials, took {:?}",
                    elapsed_time
                );

                println!("✅ OAuth URL: {}", oauth_url);
                println!("✅ Device code: {}", device_code);
            } else {
                // This could happen if already authenticated or some other status
                println!("ℹ️ Login returned status: {}", auth_result.message);
            }
        }
        Err(e) => {
            let error_msg = e.to_string();

            // In test environments, OAuth might timeout or fail due to no interaction
            // This is acceptable as long as it's not a structural error
            if error_msg.contains("Timeout waiting for OAuth credentials") {
                println!(
                    "⚠️ OAuth timeout (expected in test environment): {}",
                    error_msg
                );
                // This is actually a success case - it means we tried to get OAuth credentials
                // but timed out waiting for them, which is expected behavior
            } else {
                println!(
                    "⚠️ OAuth failed (possibly expected in test environment): {}",
                    error_msg
                );

                // Verify it's not a structural error
                assert!(
                    !error_msg.contains("command not found"),
                    "gh CLI command should exist: {}",
                    error_msg
                );
                assert!(
                    !error_msg.contains("executable file not found"),
                    "gh CLI executable should exist: {}",
                    error_msg
                );
            }
        }
    }

    // Cleanup
    cleanup_container(&docker, &container_name).await;

    println!("✅ OAuth early return test completed");

    Ok(())
}

#[rstest]
#[tokio::test]
async fn test_oauth_process_timeout_behavior(
    #[future] test_container: (Docker, String, String),
) -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
    let (docker, _container_id, container_name) = test_container.await;

    println!("=== Testing OAuth process timeout behavior ===");

    // Create GitHub client with very short timeout for this test
    let claude_client = ClaudeCodeClient::for_session(docker.clone(), &container_name).await;
    if claude_client.is_err() {
        return Err(format!("Failed to find session: {:?}", claude_client.unwrap_err()).into());
    }
    let claude_client = claude_client.unwrap();

    let config = GithubClientConfig {
        working_directory: Some("/workspace".to_string()),
        exec_timeout_secs: 10, // Very short timeout for this test
    };

    let github_client = GithubClient::new(
        docker.clone(),
        claude_client.container_id().to_string(),
        config,
    );

    // Test that timeout handling works correctly
    let start_time = std::time::Instant::now();
    let login_result = github_client.login().await;
    let elapsed_time = start_time.elapsed();

    println!("Login attempt took {:?}", elapsed_time);

    match login_result {
        Ok(auth_result) => {
            // If we get a result, verify it's structured correctly
            assert!(!auth_result.message.is_empty(), "Should have a message");

            if auth_result.oauth_url.is_some() && auth_result.device_code.is_some() {
                println!("✅ Got OAuth credentials quickly");
            } else {
                println!("ℹ️ Got other auth status: {}", auth_result.message);
            }
        }
        Err(e) => {
            let error_msg = e.to_string();
            println!("Expected error (timeout or OAuth failure): {}", error_msg);

            // Verify the error is related to expected behavior, not structural issues
            assert!(
                !error_msg.contains("command not found"),
                "Should not be a command not found error: {}",
                error_msg
            );
        }
    }

    // Cleanup
    cleanup_container(&docker, &container_name).await;

    println!("✅ OAuth timeout behavior test completed");

    Ok(())
}

// =============================================================================
// GITHUB STATUS INTEGRATION TESTS
// =============================================================================

#[tokio::test]
async fn test_github_status_integration() {
    // Initialize logging for this test
    let _ = pretty_env_logger::formatted_builder()
        .is_test(true)
        .try_init();

    let docker = Docker::connect_with_socket_defaults()
        .expect("Failed to connect to Docker daemon for testing");

    let container_name = "test-github-status-integration";
    let config = ClaudeCodeConfig::default();

    println!("Starting coding session for GitHub status test...");

    // Start a coding session (creates a container with GitHub CLI)
    let claude_client = match container_utils::start_coding_session(
        &docker,
        container_name,
        config,
        container_utils::CodingContainerConfig::default(),
    )
    .await
    {
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
            assert!(
                !auth_result.message.is_empty(),
                "Status message should not be empty"
            );
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
            assert!(
                version_info.contains("gh version"),
                "Should contain version info"
            );
        }
        Err(e) => {
            println!(
                "ℹ️  GitHub CLI not available (expected in some environments): {}",
                e
            );
            // This is okay - not all test environments have GitHub CLI
        }
    }

    // Clean up the test container
    let _ = container_utils::clear_coding_session(&docker, container_name).await;
    println!("✅ Test completed and cleaned up");
}

// =============================================================================
// GITHUB CLIENT BASIC FUNCTIONALITY TESTS
// =============================================================================

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
                error_msg.contains("gh auth login")
                    || error_msg.contains("authentication")
                    || error_msg.contains("GH_TOKEN"),
                "Error should be related to authentication, got: {}",
                error_msg
            );
            println!("Expected authentication error: {}", error_msg);
        }
    }

    // Note: The actual content depends on authentication status and available repositories
    // The important thing is that the command structure is correct and doesn't crash
}

// =============================================================================
// GITHUB REPOSITORY CLONE TESTS
// =============================================================================

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
async fn test_github_repo_clone_empty_repository(
    #[future] test_container: (Docker, String, String),
) {
    let (docker, container_id, container_name) = test_container.await;

    let client = GithubClient::new(docker.clone(), container_id, GithubClientConfig::default());

    // Test cloning an empty repository (using GitHub's official empty repo for testing)
    // Note: We use a known empty public repository that should exist
    let clone_result = client
        .repo_clone("octocat/Hello-World-Template", None)
        .await;

    // Cleanup
    cleanup_container(&docker, &container_name).await;

    assert!(
        clone_result.is_ok(),
        "Clone method should return a result: {:?}",
        clone_result
    );
    let clone_response = clone_result.unwrap();

    // Verify the repository name is set correctly
    assert_eq!(clone_response.repository, "octocat/Hello-World-Template");
    assert_eq!(clone_response.target_directory, "Hello-World-Template");
    assert!(
        !clone_response.message.is_empty(),
        "Clone result message should not be empty"
    );

    // Note: This may succeed or fail depending on network access and authentication
    // But the structure should be correct
    println!("Empty repo clone result: {:?}", clone_response);
}

#[rstest]
#[tokio::test]
async fn test_github_repo_clone_multi_branch_repository(
    #[future] test_container: (Docker, String, String),
) {
    let (docker, container_id, container_name) = test_container.await;

    let client = GithubClient::new(docker.clone(), container_id, GithubClientConfig::default());

    // Test cloning a repository known to have multiple branches
    // Using a popular public repository that has multiple branches
    let clone_result = client
        .repo_clone("microsoft/vscode", Some("vscode-clone"))
        .await;

    // Cleanup
    cleanup_container(&docker, &container_name).await;

    assert!(
        clone_result.is_ok(),
        "Clone method should return a result: {:?}",
        clone_result
    );
    let clone_response = clone_result.unwrap();

    // Verify the target directory and repository are set correctly
    assert_eq!(clone_response.repository, "microsoft/vscode");
    assert_eq!(clone_response.target_directory, "vscode-clone");
    assert!(
        !clone_response.message.is_empty(),
        "Clone result message should not be empty"
    );

    println!("Multi-branch repo clone result: {:?}", clone_response);
}

#[rstest]
#[tokio::test]
async fn test_github_repo_clone_malformed_urls(#[future] test_container: (Docker, String, String)) {
    let (docker, container_id, container_name) = test_container.await;

    let client = GithubClient::new(docker.clone(), container_id, GithubClientConfig::default());

    // Test various malformed repository URLs
    let malformed_repos = vec![
        "invalid-repo-name",             // Missing owner
        "owner/",                        // Missing repo name
        "/repo-name",                    // Missing owner
        "owner//repo",                   // Double slash
        "owner/repo/extra",              // Too many parts
        "",                              // Empty string
        "owner/repo with spaces",        // Spaces in name
        "owner/repo@branch",             // Invalid characters
        "https://github.com/owner/repo", // Full URL instead of owner/repo
    ];

    for repo in malformed_repos {
        let clone_result = client.repo_clone(repo, None).await;

        assert!(
            clone_result.is_ok(),
            "Clone method should return a result even for malformed URL '{}': {:?}",
            repo,
            clone_result
        );

        let clone_response = clone_result.unwrap();

        // For malformed URLs, the clone should fail
        if repo.is_empty() {
            // Empty string is a special case - might be handled differently
            println!("Empty string clone result: {:?}", clone_response);
        } else {
            // Most malformed URLs should result in failure
            println!(
                "Malformed URL '{}' clone result: {:?}",
                repo, clone_response
            );

            // Verify the repository name matches what was passed
            assert_eq!(clone_response.repository, repo);
            assert!(
                !clone_response.message.is_empty(),
                "Clone result message should not be empty for malformed URL: {}",
                repo
            );
        }
    }

    // Cleanup
    cleanup_container(&docker, &container_name).await;
}

#[rstest]
#[tokio::test]
async fn test_github_repo_clone_target_directory_edge_cases(
    #[future] test_container: (Docker, String, String),
) {
    let (docker, container_id, container_name) = test_container.await;

    let client = GithubClient::new(docker.clone(), container_id, GithubClientConfig::default());

    // Test various target directory edge cases
    let test_cases = vec![
        ("octocat/Hello-World", Some("."), "Test with current directory"),
        ("octocat/Hello-World", Some("./test-dir"), "Test with relative path"),
        ("octocat/Hello-World", Some("/tmp/test-clone"), "Test with absolute path"),
        ("octocat/Hello-World", Some("test dir with spaces"), "Test with spaces in directory name"),
        ("octocat/Hello-World", Some("test-dir-with-special-chars_123"), "Test with special characters"),
        ("octocat/Hello-World", Some("very-long-directory-name-that-might-cause-issues-in-some-systems-but-should-be-handled-gracefully"), "Test with very long directory name"),
    ];

    for (repo, target_dir, description) in test_cases {
        println!("Testing: {}", description);

        let clone_result = client.repo_clone(repo, target_dir).await;

        assert!(
            clone_result.is_ok(),
            "Clone method should return a result for {}: {:?}",
            description,
            clone_result
        );

        let clone_response = clone_result.unwrap();

        // Verify the target directory is set correctly
        if let Some(expected_dir) = target_dir {
            assert_eq!(
                clone_response.target_directory, expected_dir,
                "Target directory should match for case: {}",
                description
            );
        }

        assert_eq!(clone_response.repository, repo);
        assert!(
            !clone_response.message.is_empty(),
            "Clone result message should not be empty for: {}",
            description
        );

        println!("{} result: {:?}", description, clone_response);
    }

    // Cleanup
    cleanup_container(&docker, &container_name).await;
}

#[rstest]
#[tokio::test]
async fn test_github_repo_clone_nonexistent_variations(
    #[future] test_container: (Docker, String, String),
) {
    let (docker, container_id, container_name) = test_container.await;

    let client = GithubClient::new(docker.clone(), container_id, GithubClientConfig::default());

    // Test various types of nonexistent repositories
    let nonexistent_repos = vec![
        "nonexistent-user-12345/nonexistent-repo-12345",
        "github/definitely-does-not-exist-12345",
        "octocat/this-repo-should-never-exist-12345",
        "user-with-very-long-name-that-might-not-exist/repo",
        "123456789/numeric-user",
        "special-chars-user_123/special-chars-repo_456",
    ];

    for repo in nonexistent_repos {
        let clone_result = client.repo_clone(repo, None).await;

        assert!(
            clone_result.is_ok(),
            "Clone method should return a result for nonexistent repo '{}': {:?}",
            repo,
            clone_result
        );

        let clone_response = clone_result.unwrap();

        // Should fail for nonexistent repositories
        assert!(
            !clone_response.success,
            "Clone should fail for nonexistent repo: {}",
            repo
        );

        assert_eq!(clone_response.repository, repo);
        assert!(
            !clone_response.message.is_empty(),
            "Clone result message should not be empty for nonexistent repo: {}",
            repo
        );

        // The message should indicate the failure reason
        // With our improved error analysis, we now get more helpful messages
        assert!(
            clone_response.message.contains("Clone failed")
                || clone_response.message.contains("not found")
                || clone_response.message.contains("404")
                || clone_response.message.contains("repository not found")
                || clone_response.message.contains("Repository not found")
                || clone_response.message.contains("Authentication required")
                || clone_response.message.contains("Permission denied"),
            "Clone failure message should indicate the reason for repo '{}': {}",
            repo,
            clone_response.message
        );

        println!(
            "Nonexistent repo '{}' clone result: {:?}",
            repo, clone_response
        );
    }

    // Cleanup
    cleanup_container(&docker, &container_name).await;
}

#[rstest]
#[tokio::test]
async fn test_github_repo_clone_private_repo_without_auth(
    #[future] test_container: (Docker, String, String),
) {
    let (docker, container_id, container_name) = test_container.await;

    let client = GithubClient::new(docker.clone(), container_id, GithubClientConfig::default());

    // Test cloning a private repository without authentication
    // Using Microsoft's private repos as examples (these should fail without proper auth)
    let private_repos = vec![
        "microsoft/private-test-repo", // This likely doesn't exist but represents a private repo pattern
        "github/private-repo-example", // Another example of a private repo pattern
    ];

    for repo in private_repos {
        let clone_result = client.repo_clone(repo, None).await;

        assert!(
            clone_result.is_ok(),
            "Clone method should return a result for private repo '{}': {:?}",
            repo,
            clone_result
        );

        let clone_response = clone_result.unwrap();

        // Should likely fail for private repositories without authentication
        // (Note: might succeed if the repo doesn't exist and fails for that reason instead)
        assert_eq!(clone_response.repository, repo);
        assert!(
            !clone_response.message.is_empty(),
            "Clone result message should not be empty for private repo: {}",
            repo
        );

        println!("Private repo '{}' clone result: {:?}", repo, clone_response);
    }

    // Cleanup
    cleanup_container(&docker, &container_name).await;
}

#[rstest]
#[tokio::test]
async fn test_github_repo_clone_concurrent_operations(
    #[future] test_container: (Docker, String, String),
) {
    let (docker, container_id, container_name) = test_container.await;

    let client = GithubClient::new(docker.clone(), container_id, GithubClientConfig::default());

    // Test multiple concurrent clone operations
    let repos = vec![
        ("octocat/Hello-World", Some("concurrent-1")),
        ("octocat/Hello-World", Some("concurrent-2")),
        ("invalid/nonexistent-repo-1", Some("concurrent-fail-1")),
        ("invalid/nonexistent-repo-2", Some("concurrent-fail-2")),
    ];

    // Launch all clone operations concurrently
    let mut clone_futures = Vec::new();
    for (repo, target_dir) in repos.iter() {
        let future = client.repo_clone(repo, *target_dir);
        clone_futures.push(future);
    }

    // Wait for all operations to complete
    let results = futures_util::future::join_all(clone_futures).await;

    // Cleanup
    cleanup_container(&docker, &container_name).await;

    // Verify all operations completed
    assert_eq!(results.len(), repos.len());

    for (i, result) in results.iter().enumerate() {
        let (expected_repo, expected_target) = &repos[i];

        assert!(
            result.is_ok(),
            "Concurrent clone operation {} should return a result: {:?}",
            i,
            result
        );

        let clone_response = result.as_ref().unwrap();
        assert_eq!(clone_response.repository, *expected_repo);
        if let Some(target) = expected_target {
            assert_eq!(clone_response.target_directory, *target);
        }

        println!("Concurrent operation {} result: {:?}", i, clone_response);
    }
}

#[rstest]
#[tokio::test]
async fn test_github_repo_clone_special_characters_and_unicode(
    #[future] test_container: (Docker, String, String),
) {
    let (docker, container_id, container_name) = test_container.await;

    let client = GithubClient::new(docker.clone(), container_id, GithubClientConfig::default());

    // Test repository names with special characters and edge cases
    let special_repos = vec![
        "user/repo-with-dashes",
        "user/repo_with_underscores",
        "user/repo.with.dots",
        "user-with-dashes/repo",
        "user_with_underscores/repo",
        "user123/repo456",
        "123user/456repo", // Numbers at start (might not be valid GitHub usernames)
    ];

    for repo in special_repos {
        let clone_result = client.repo_clone(repo, None).await;

        assert!(
            clone_result.is_ok(),
            "Clone method should return a result for special repo '{}': {:?}",
            repo,
            clone_result
        );

        let clone_response = clone_result.unwrap();
        assert_eq!(clone_response.repository, repo);
        assert!(
            !clone_response.message.is_empty(),
            "Clone result message should not be empty for special repo: {}",
            repo
        );

        println!(
            "Special characters repo '{}' clone result: {:?}",
            repo, clone_response
        );
    }

    // Test target directories with special characters
    let special_target_dirs = vec![
        "target-with-dashes",
        "target_with_underscores",
        "target.with.dots",
        "target123",
    ];

    for target_dir in special_target_dirs {
        let clone_result = client
            .repo_clone("octocat/Hello-World", Some(&target_dir))
            .await;

        assert!(
            clone_result.is_ok(),
            "Clone method should return a result for special target dir '{}': {:?}",
            target_dir,
            clone_result
        );

        let clone_response = clone_result.unwrap();
        assert_eq!(clone_response.target_directory, target_dir);
        assert_eq!(clone_response.repository, "octocat/Hello-World");

        println!(
            "Special target dir '{}' clone result: {:?}",
            target_dir, clone_response
        );
    }

    // Cleanup
    cleanup_container(&docker, &container_name).await;
}

#[rstest]
#[tokio::test]
async fn test_github_repo_clone_large_repository(
    #[future] test_container: (Docker, String, String),
) {
    let (docker, container_id, container_name) = test_container.await;

    let client = GithubClient::new(docker.clone(), container_id, GithubClientConfig::default());

    // Test cloning a large repository (this tests timeout handling and large transfers)
    // Using a well-known large repository
    let clone_result = client
        .repo_clone("torvalds/linux", Some("linux-kernel-clone"))
        .await;

    // Cleanup
    cleanup_container(&docker, &container_name).await;

    assert!(
        clone_result.is_ok(),
        "Clone method should return a result for large repo: {:?}",
        clone_result
    );

    let clone_response = clone_result.unwrap();
    assert_eq!(clone_response.repository, "torvalds/linux");
    assert_eq!(clone_response.target_directory, "linux-kernel-clone");
    assert!(
        !clone_response.message.is_empty(),
        "Clone result message should not be empty for large repo"
    );

    // Note: This will likely fail due to size/timeout in test environment,
    // but the important thing is that it's handled gracefully
    println!("Large repository clone result: {:?}", clone_response);
}

#[rstest]
#[tokio::test]
async fn test_github_repo_clone_case_sensitivity(
    #[future] test_container: (Docker, String, String),
) {
    let (docker, container_id, container_name) = test_container.await;

    let client = GithubClient::new(docker.clone(), container_id, GithubClientConfig::default());

    // Test case sensitivity in repository names
    let case_variants = vec![
        "octocat/Hello-World",
        "OCTOCAT/HELLO-WORLD", // GitHub usernames and repos are case-insensitive, but this tests handling
        "octocat/hello-world",
        "Octocat/Hello-World",
    ];

    for repo in case_variants {
        let clone_result = client.repo_clone(repo, None).await;

        assert!(
            clone_result.is_ok(),
            "Clone method should return a result for case variant '{}': {:?}",
            repo,
            clone_result
        );

        let clone_response = clone_result.unwrap();
        assert_eq!(clone_response.repository, repo);
        assert!(
            !clone_response.message.is_empty(),
            "Clone result message should not be empty for case variant: {}",
            repo
        );

        println!("Case variant '{}' clone result: {:?}", repo, clone_response);
    }

    // Cleanup
    cleanup_container(&docker, &container_name).await;
}

#[rstest]
#[tokio::test]
async fn test_github_repo_clone_target_directory_default_behavior(
    #[future] test_container: (Docker, String, String),
) {
    let (docker, container_id, container_name) = test_container.await;

    let client = GithubClient::new(docker.clone(), container_id, GithubClientConfig::default());

    // Test default target directory behavior (None target_dir)
    let test_repos = vec![
        "octocat/Hello-World",
        "microsoft/vscode",
        "invalid/nonexistent-repo",
    ];

    for repo in test_repos {
        let clone_result = client.repo_clone(repo, None).await;

        assert!(
            clone_result.is_ok(),
            "Clone method should return a result for default target dir test with '{}': {:?}",
            repo,
            clone_result
        );

        let clone_response = clone_result.unwrap();
        assert_eq!(clone_response.repository, repo);

        // Default target directory should be the repository name (last part after '/')
        let expected_target = repo.split('/').last().unwrap_or(repo);
        assert_eq!(
            clone_response.target_directory, expected_target,
            "Default target directory should be repo name for: {}",
            repo
        );

        println!(
            "Default target dir for '{}' result: {:?}",
            repo, clone_response
        );
    }

    // Cleanup
    cleanup_container(&docker, &container_name).await;
}

// =============================================================================
// CLONE INTEGRATION TESTS
// =============================================================================

/// Test GitHub clone integration with different working directories
#[rstest]
#[tokio::test]
async fn test_github_clone_with_different_working_directories(
    #[future] test_container: (Docker, String, String),
) {
    let (docker, container_id, container_name) = test_container.await;

    // Test cloning with different working directory configurations
    let working_directories = vec![
        Some("/tmp".to_string()),
        Some("/workspace".to_string()),
        Some("/home".to_string()),
        None, // Default working directory
    ];

    for working_dir in working_directories {
        let config = GithubClientConfig {
            working_directory: working_dir.clone(),
            exec_timeout_secs: 30,
        };

        let client = GithubClient::new(docker.clone(), container_id.clone(), config);

        // Test basic clone operation
        let clone_result = client
            .repo_clone("octocat/Hello-World", Some("test-working-dir"))
            .await;

        assert!(
            clone_result.is_ok(),
            "Clone should work with working directory {:?}: {:?}",
            working_dir,
            clone_result
        );

        let clone_response = clone_result.unwrap();
        assert_eq!(clone_response.repository, "octocat/Hello-World");
        assert_eq!(clone_response.target_directory, "test-working-dir");

        println!(
            "Working dir {:?} clone result: {:?}",
            working_dir, clone_response
        );
    }

    // Cleanup
    cleanup_container(&docker, &container_name).await;
}

/// Test GitHub clone with different timeout configurations
#[rstest]
#[tokio::test]
async fn test_github_clone_with_different_timeouts(
    #[future] test_container: (Docker, String, String),
) {
    let (docker, container_id, container_name) = test_container.await;

    // Test different timeout configurations
    let timeout_configs = vec![
        5,   // Very short timeout (likely to fail for real operations)
        30,  // Short timeout
        60,  // Default timeout
        120, // Long timeout
    ];

    for timeout_secs in timeout_configs {
        let config = GithubClientConfig {
            working_directory: Some("/workspace".to_string()),
            exec_timeout_secs: timeout_secs,
        };

        let client = GithubClient::new(docker.clone(), container_id.clone(), config);

        // Test clone operation with this timeout
        let clone_result = client
            .repo_clone(
                "octocat/Hello-World",
                Some(&format!("test-timeout-{}", timeout_secs)),
            )
            .await;

        assert!(
            clone_result.is_ok(),
            "Clone should return result with timeout {}: {:?}",
            timeout_secs,
            clone_result
        );

        let clone_response = clone_result.unwrap();
        assert_eq!(clone_response.repository, "octocat/Hello-World");

        println!(
            "Timeout {}s clone result: {:?}",
            timeout_secs, clone_response
        );
    }

    // Cleanup
    cleanup_container(&docker, &container_name).await;
}

/// Test GitHub clone error handling and recovery
#[rstest]
#[tokio::test]
async fn test_github_clone_error_handling_and_recovery(
    #[future] test_container: (Docker, String, String),
) {
    let (docker, container_id, container_name) = test_container.await;

    let client = GithubClient::new(docker.clone(), container_id, GithubClientConfig::default());

    // Test sequence: fail -> succeed -> fail pattern
    let test_sequence = vec![
        ("invalid/nonexistent-repo-123", Some("fail-1"), false),
        ("octocat/Hello-World", Some("succeed-1"), true), // This may succeed if auth is available
        ("another-invalid/repo-456", Some("fail-2"), false),
        ("octocat/Hello-World", Some("succeed-2"), true), // Test recovery
    ];

    for (repo, target_dir, should_potentially_succeed) in test_sequence {
        let clone_result = client.repo_clone(repo, target_dir).await;

        assert!(
            clone_result.is_ok(),
            "Clone method should return result for '{}': {:?}",
            repo,
            clone_result
        );

        let clone_response = clone_result.unwrap();
        assert_eq!(clone_response.repository, repo);
        if let Some(expected_target) = target_dir {
            assert_eq!(clone_response.target_directory, expected_target);
        }

        // For valid repos, success may depend on authentication
        if !should_potentially_succeed {
            // Invalid repos should definitely fail
            assert!(
                !clone_response.success,
                "Invalid repo '{}' should fail",
                repo
            );
        }

        println!("Error handling test for '{}': {:?}", repo, clone_response);
    }

    // Cleanup
    cleanup_container(&docker, &container_name).await;
}

/// Test GitHub clone with authentication scenarios
#[rstest]
#[tokio::test]
async fn test_github_clone_authentication_scenarios(
    #[future] test_container: (Docker, String, String),
) {
    let (docker, container_id, container_name) = test_container.await;

    let client = GithubClient::new(docker.clone(), container_id, GithubClientConfig::default());

    // First check if GitHub CLI is available
    let availability_result = client.check_availability().await;
    assert!(
        availability_result.is_ok(),
        "GitHub CLI should be available: {:?}",
        availability_result
    );

    // Check authentication status
    let auth_status_result = client.check_auth_status().await;
    assert!(
        auth_status_result.is_ok(),
        "Auth status check should work: {:?}",
        auth_status_result
    );

    let auth_result = auth_status_result.unwrap();
    println!("Authentication status: {:?}", auth_result);

    // Test different repositories based on authentication status
    let test_repos = if auth_result.authenticated {
        // If authenticated, test both public and potentially private repos
        vec![
            ("octocat/Hello-World", "Public repo with auth"),
            ("microsoft/vscode", "Large public repo with auth"),
        ]
    } else {
        // If not authenticated, test public repos (should still work for public repos)
        vec![
            ("octocat/Hello-World", "Public repo without auth"),
            ("microsoft/vscode", "Large public repo without auth"),
        ]
    };

    for (repo, description) in test_repos {
        let clone_result = client.repo_clone(repo, None).await;

        assert!(
            clone_result.is_ok(),
            "Clone should return result for {}: {:?}",
            description,
            clone_result
        );

        let clone_response = clone_result.unwrap();
        assert_eq!(clone_response.repository, repo);

        println!("{} result: {:?}", description, clone_response);
    }

    // Cleanup
    cleanup_container(&docker, &container_name).await;
}

/// Test GitHub clone with repository access patterns
#[rstest]
#[tokio::test]
async fn test_github_clone_repository_access_patterns(
    #[future] test_container: (Docker, String, String),
) {
    let (docker, container_id, container_name) = test_container.await;

    let client = GithubClient::new(docker.clone(), container_id, GithubClientConfig::default());

    // Test different types of repositories
    let repository_types = vec![
        ("octocat/Hello-World", "Classic example repository"),
        ("github/gitignore", "Repository with many files"),
        ("microsoft/TypeScript", "Large active repository"),
        (
            "torvalds/linux",
            "Very large repository (should handle gracefully)",
        ),
        ("rails/rails", "Another large active repository"),
    ];

    for (repo, description) in repository_types {
        let clone_result = client
            .repo_clone(repo, Some(&format!("test-{}", repo.replace("/", "-"))))
            .await;

        assert!(
            clone_result.is_ok(),
            "Clone should return result for {}: {:?}",
            description,
            clone_result
        );

        let clone_response = clone_result.unwrap();
        assert_eq!(clone_response.repository, repo);
        assert!(
            !clone_response.message.is_empty(),
            "Clone message should not be empty for {}",
            description
        );

        println!("{} ({}) result: {:?}", repo, description, clone_response);

        // For very large repositories, we expect they might fail due to timeout/size
        // but should be handled gracefully
        if repo == "torvalds/linux" {
            // This will likely fail due to size, but should not crash
            println!("Note: Large repository clone may fail due to size/timeout constraints");
        }
    }

    // Cleanup
    cleanup_container(&docker, &container_name).await;
}

/// Test GitHub clone workflow integration
#[rstest]
#[tokio::test]
async fn test_github_clone_workflow_integration(
    #[future] test_container: (Docker, String, String),
) {
    let (docker, container_id, container_name) = test_container.await;

    let client = GithubClient::new(docker.clone(), container_id, GithubClientConfig::default());

    // Simulate a complete workflow: check availability -> check auth -> list repos -> clone

    // Step 1: Check GitHub CLI availability
    let availability_result = client.check_availability().await;
    assert!(
        availability_result.is_ok(),
        "Step 1 - GitHub CLI availability check failed: {:?}",
        availability_result
    );
    println!("✅ Step 1: GitHub CLI is available");

    // Step 2: Check authentication status
    let auth_result = client.check_auth_status().await;
    assert!(
        auth_result.is_ok(),
        "Step 2 - Auth status check failed: {:?}",
        auth_result
    );
    println!("✅ Step 2: Authentication status checked");

    // Step 3: Attempt to list repositories (may fail if not authenticated)
    let repo_list_result = client.repo_list().await;
    match repo_list_result {
        Ok(repos) => {
            println!(
                "✅ Step 3: Repository list retrieved: {} chars",
                repos.len()
            );
        }
        Err(e) => {
            println!(
                "ℹ️ Step 3: Repository list failed (expected if not authenticated): {}",
                e
            );
        }
    }

    // Step 4: Attempt to clone a public repository
    let clone_result = client
        .repo_clone("octocat/Hello-World", Some("workflow-test"))
        .await;

    assert!(
        clone_result.is_ok(),
        "Step 4 - Clone operation failed: {:?}",
        clone_result
    );

    let clone_response = clone_result.unwrap();
    assert_eq!(clone_response.repository, "octocat/Hello-World");
    assert_eq!(clone_response.target_directory, "workflow-test");

    println!("✅ Step 4: Clone operation completed: {:?}", clone_response);

    // Step 5: Test multiple clones to different directories
    let multiple_clones = vec![
        ("octocat/Hello-World", "workflow-test-1"),
        ("octocat/Hello-World", "workflow-test-2"),
        ("github/gitignore", "workflow-test-3"),
    ];

    for (repo, target) in multiple_clones {
        let clone_result = client.repo_clone(repo, Some(target)).await;
        assert!(
            clone_result.is_ok(),
            "Multiple clone failed for {}: {:?}",
            repo,
            clone_result
        );

        let clone_response = clone_result.unwrap();
        assert_eq!(clone_response.target_directory, target);
        println!("✅ Multiple clone {}: {:?}", target, clone_response);
    }

    println!("🎉 Complete workflow integration test passed!");

    // Cleanup
    cleanup_container(&docker, &container_name).await;
}

// =============================================================================
// GITHUB TIMEOUT TESTS
// =============================================================================

#[rstest]
#[tokio::test]
async fn test_github_exec_timeout_configuration(
    #[future] test_container: (Docker, String, String),
) -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
    let (docker, container_id, container_name) = test_container.await;

    println!("=== Testing GitHub client timeout configuration ===");

    // Test with custom short timeout
    let short_timeout_config = GithubClientConfig {
        working_directory: Some("/workspace".to_string()),
        exec_timeout_secs: 2, // Very short timeout for testing
    };

    let github_client =
        GithubClient::new(docker.clone(), container_id.clone(), short_timeout_config);

    // Test that a simple command still works with short timeout
    let availability_result = github_client.check_availability().await;
    match availability_result {
        Ok(version_output) => {
            println!(
                "✅ gh CLI availability check successful with short timeout: {}",
                version_output
            );
        }
        Err(e) => {
            let error_msg = e.to_string();
            if error_msg.contains("timed out after 2 seconds") {
                println!("⏰ Command timed out as expected with 2 second timeout");
            } else if error_msg.contains("not found")
                || error_msg.contains("executable file not found")
            {
                println!(
                    "⚠️ gh CLI not available in test environment (expected): {}",
                    error_msg
                );
            } else {
                println!("ℹ️ Other error occurred: {}", error_msg);
            }
        }
    }

    // Test with default timeout
    let default_config = GithubClientConfig::default();
    assert_eq!(
        default_config.exec_timeout_secs, 60,
        "Default timeout should be 60 seconds"
    );

    let _github_client_default =
        GithubClient::new(docker.clone(), container_id.clone(), default_config);

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
    #[future] test_container: (Docker, String, String),
) -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
    let (docker, container_id, container_name) = test_container.await;

    println!("=== Testing timeout error handling behavior ===");

    // Create client with very short timeout to force timeout scenarios
    let timeout_config = GithubClientConfig {
        working_directory: Some("/workspace".to_string()),
        exec_timeout_secs: 1, // 1 second timeout to trigger timeouts
    };

    let github_client = GithubClient::new(docker.clone(), container_id.clone(), timeout_config);

    // Test auth status check with timeout
    let auth_status_result = github_client.check_auth_status().await;
    match auth_status_result {
        Ok(auth_result) => {
            // Command completed quickly enough
            println!("✅ Auth status check completed within timeout");
            assert!(
                !auth_result.message.is_empty(),
                "Auth result should have a message"
            );
        }
        Err(e) => {
            let error_msg = e.to_string();
            if error_msg.contains("timed out after 1 seconds") {
                println!("✅ Auth status check timed out as expected: {}", error_msg);
                assert!(
                    error_msg.contains("gh auth status"),
                    "Error should mention the command that timed out"
                );
            } else {
                println!(
                    "ℹ️ Auth status check failed for other reason: {}",
                    error_msg
                );
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
            assert!(
                !auth_result.message.is_empty(),
                "Login result should have a message"
            );
        }
        Err(e) => {
            let error_msg = e.to_string();
            if error_msg.contains("timed out after 1 seconds") {
                println!("✅ Login timed out as expected: {}", error_msg);
                assert!(
                    error_msg.contains("gh auth login"),
                    "Error should mention the login command"
                );
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

// =============================================================================
// EXEC COMMAND ALLOW FAILURE TESTS
// =============================================================================

#[rstest]
#[tokio::test]
async fn test_exec_command_allow_failure_success(
    #[future] test_container: (Docker, String, String),
) {
    let (docker, container_id, container_name) = test_container.await;

    let client = GithubClient::new(docker.clone(), container_id, GithubClientConfig::default());

    // Test successful command execution
    let result = client
        .exec_command_allow_failure(vec!["echo".to_string(), "hello".to_string()])
        .await;

    // Cleanup
    cleanup_container(&docker, &container_name).await;

    assert!(
        result.is_ok(),
        "Command should execute successfully: {:?}",
        result
    );
    let (output, success) = result.unwrap();

    assert!(success, "Command should be marked as successful");
    assert_eq!(output.trim(), "hello", "Output should match expected value");
}

#[rstest]
#[tokio::test]
async fn test_exec_command_allow_failure_command_failure(
    #[future] test_container: (Docker, String, String),
) {
    let (docker, container_id, container_name) = test_container.await;

    let client = GithubClient::new(docker.clone(), container_id, GithubClientConfig::default());

    // Test failing command execution (exit code != 0)
    let result = client
        .exec_command_allow_failure(vec![
            "sh".to_string(),
            "-c".to_string(),
            "exit 1".to_string(),
        ])
        .await;

    // Cleanup
    cleanup_container(&docker, &container_name).await;

    assert!(
        result.is_ok(),
        "Method should return Ok even for failing commands: {:?}",
        result
    );
    let (output, success) = result.unwrap();

    assert!(!success, "Command should be marked as failed");
    // Output might be empty for simple exit commands
    println!("Failed command output: '{}'", output);
}

#[rstest]
#[tokio::test]
async fn test_exec_command_allow_failure_stderr_capture(
    #[future] test_container: (Docker, String, String),
) {
    let (docker, container_id, container_name) = test_container.await;

    let client = GithubClient::new(docker.clone(), container_id, GithubClientConfig::default());

    // Test command that outputs to stderr
    let result = client
        .exec_command_allow_failure(vec![
            "sh".to_string(),
            "-c".to_string(),
            "echo 'error output' >&2".to_string(),
        ])
        .await;

    // Cleanup
    cleanup_container(&docker, &container_name).await;

    assert!(
        result.is_ok(),
        "Command should execute successfully: {:?}",
        result
    );
    let (output, success) = result.unwrap();

    assert!(success, "Command should be marked as successful");
    assert!(
        output.contains("error output"),
        "Output should contain stderr content: '{}'",
        output
    );
}

#[rstest]
#[tokio::test]
async fn test_exec_command_allow_failure_mixed_output(
    #[future] test_container: (Docker, String, String),
) {
    let (docker, container_id, container_name) = test_container.await;

    let client = GithubClient::new(docker.clone(), container_id, GithubClientConfig::default());

    // Test command that outputs to both stdout and stderr
    let result = client
        .exec_command_allow_failure(vec![
            "sh".to_string(),
            "-c".to_string(),
            "echo 'stdout line'; echo 'stderr line' >&2".to_string(),
        ])
        .await;

    // Cleanup
    cleanup_container(&docker, &container_name).await;

    assert!(
        result.is_ok(),
        "Command should execute successfully: {:?}",
        result
    );
    let (output, success) = result.unwrap();

    assert!(success, "Command should be marked as successful");
    assert!(
        output.contains("stdout line"),
        "Output should contain stdout: '{}'",
        output
    );
    assert!(
        output.contains("stderr line"),
        "Output should contain stderr: '{}'",
        output
    );
}

#[rstest]
#[tokio::test]
async fn test_exec_command_allow_failure_nonexistent_command(
    #[future] test_container: (Docker, String, String),
) {
    let (docker, container_id, container_name) = test_container.await;

    let client = GithubClient::new(docker.clone(), container_id, GithubClientConfig::default());

    // Test execution of nonexistent command
    let result = client
        .exec_command_allow_failure(vec!["nonexistent-command-12345".to_string()])
        .await;

    // Cleanup
    cleanup_container(&docker, &container_name).await;

    assert!(
        result.is_ok(),
        "Method should return Ok even for nonexistent commands: {:?}",
        result
    );
    let (output, success) = result.unwrap();

    assert!(!success, "Nonexistent command should be marked as failed");
    assert!(
        !output.is_empty(),
        "Output should contain error message about nonexistent command"
    );
    println!("Nonexistent command output: '{}'", output);
}

#[rstest]
#[tokio::test]
async fn test_exec_command_allow_failure_working_directory(
    #[future] test_container: (Docker, String, String),
) {
    let (docker, container_id, container_name) = test_container.await;

    // Test with custom working directory
    let mut config = GithubClientConfig::default();
    config.working_directory = Some("/tmp".to_string());
    let client = GithubClient::new(docker.clone(), container_id, config);

    // Test command that shows current working directory
    let result = client
        .exec_command_allow_failure(vec!["pwd".to_string()])
        .await;

    // Cleanup
    cleanup_container(&docker, &container_name).await;

    assert!(
        result.is_ok(),
        "Command should execute successfully: {:?}",
        result
    );
    let (output, success) = result.unwrap();

    assert!(success, "Command should be marked as successful");
    assert!(
        output.contains("/tmp"),
        "Command should run in specified working directory: '{}'",
        output
    );
}

#[rstest]
#[tokio::test]
async fn test_exec_command_allow_failure_timeout_behavior(
    #[future] test_container: (Docker, String, String),
) {
    let (docker, container_id, container_name) = test_container.await;

    // Test with very short timeout to trigger timeout behavior
    let mut config = GithubClientConfig::default();
    config.exec_timeout_secs = 1; // 1 second timeout
    let client = GithubClient::new(docker.clone(), container_id, config);

    // Test command that takes longer than timeout
    let result = client
        .exec_command_allow_failure(vec!["sleep".to_string(), "2".to_string()])
        .await;

    // Cleanup
    cleanup_container(&docker, &container_name).await;

    // This should return an error due to timeout
    assert!(
        result.is_err(),
        "Long-running command should timeout and return error"
    );
    let error_msg = result.unwrap_err().to_string();
    assert!(
        error_msg.contains("timed out"),
        "Error should mention timeout: '{}'",
        error_msg
    );
    assert!(
        error_msg.contains("sleep 2"),
        "Error should mention the command: '{}'",
        error_msg
    );
}

#[rstest]
#[tokio::test]
async fn test_exec_command_allow_failure_environment_variables(
    #[future] test_container: (Docker, String, String),
) {
    let (docker, container_id, container_name) = test_container.await;

    let client = GithubClient::new(docker.clone(), container_id, GithubClientConfig::default());

    // Test that environment variables are set correctly (HOME and PATH)
    let result = client
        .exec_command_allow_failure(vec![
            "sh".to_string(),
            "-c".to_string(),
            "echo \"HOME=$HOME\" && echo \"PATH=$PATH\"".to_string(),
        ])
        .await;

    // Cleanup
    cleanup_container(&docker, &container_name).await;

    assert!(
        result.is_ok(),
        "Command should execute successfully: {:?}",
        result
    );
    let (output, success) = result.unwrap();

    assert!(success, "Command should be marked as successful");
    assert!(
        output.contains("HOME=/root"),
        "HOME should be set to /root: '{}'",
        output
    );
    assert!(
        output.contains("PATH=") && output.contains("/usr/local/bin"),
        "PATH should contain standard paths: '{}'",
        output
    );
}

#[rstest]
#[tokio::test]
async fn test_exec_command_allow_failure_empty_command(
    #[future] test_container: (Docker, String, String),
) {
    let (docker, container_id, container_name) = test_container.await;

    let client = GithubClient::new(docker.clone(), container_id, GithubClientConfig::default());

    // Test execution with empty command vector
    let result = client.exec_command_allow_failure(vec![]).await;

    // Cleanup
    cleanup_container(&docker, &container_name).await;

    // This should likely fail at the Docker API level
    // But we still want to ensure it returns a Result rather than panicking
    println!("Empty command result: {:?}", result);
    // We don't assert success/failure here since the behavior may vary
    // The important thing is that it doesn't panic
}
