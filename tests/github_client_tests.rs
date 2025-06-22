use bollard::Docker;
use rstest::*;
use telegram_bot::{container_utils, GithubClient, GithubClientConfig};
use futures_util;
use uuid;

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

// =============================================================================
// COMPREHENSIVE GITHUB CLONE FUNCTIONALITY TESTS
// =============================================================================

#[rstest]
#[tokio::test]
async fn test_github_repo_clone_empty_repository(#[future] test_container: (Docker, String, String)) {
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
async fn test_github_repo_clone_multi_branch_repository(#[future] test_container: (Docker, String, String)) {
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
        "invalid-repo-name",           // Missing owner
        "owner/",                      // Missing repo name
        "/repo-name",                  // Missing owner
        "owner//repo",                 // Double slash
        "owner/repo/extra",            // Too many parts
        "",                            // Empty string
        "owner/repo with spaces",      // Spaces in name
        "owner/repo@branch",           // Invalid characters
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
            println!("Malformed URL '{}' clone result: {:?}", repo, clone_response);
            
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
async fn test_github_repo_clone_target_directory_edge_cases(#[future] test_container: (Docker, String, String)) {
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
                clone_response.target_directory, 
                expected_dir,
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
async fn test_github_repo_clone_nonexistent_variations(#[future] test_container: (Docker, String, String)) {
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
        assert!(
            clone_response.message.contains("Clone failed") ||
            clone_response.message.contains("not found") ||
            clone_response.message.contains("404") ||
            clone_response.message.contains("repository not found"),
            "Clone failure message should indicate the reason for repo '{}': {}",
            repo,
            clone_response.message
        );

        println!("Nonexistent repo '{}' clone result: {:?}", repo, clone_response);
    }

    // Cleanup
    cleanup_container(&docker, &container_name).await;
}

#[rstest]
#[tokio::test]
async fn test_github_repo_clone_private_repo_without_auth(#[future] test_container: (Docker, String, String)) {
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
async fn test_github_repo_clone_concurrent_operations(#[future] test_container: (Docker, String, String)) {
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
async fn test_github_repo_clone_special_characters_and_unicode(#[future] test_container: (Docker, String, String)) {
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

        println!("Special characters repo '{}' clone result: {:?}", repo, clone_response);
    }

    // Test target directories with special characters
    let special_target_dirs = vec![
        "target-with-dashes",
        "target_with_underscores",
        "target.with.dots",
        "target123",
    ];

    for target_dir in special_target_dirs {
        let clone_result = client.repo_clone("octocat/Hello-World", Some(&target_dir)).await;

        assert!(
            clone_result.is_ok(),
            "Clone method should return a result for special target dir '{}': {:?}",
            target_dir,
            clone_result
        );
        
        let clone_response = clone_result.unwrap();
        assert_eq!(clone_response.target_directory, target_dir);
        assert_eq!(clone_response.repository, "octocat/Hello-World");

        println!("Special target dir '{}' clone result: {:?}", target_dir, clone_response);
    }

    // Cleanup
    cleanup_container(&docker, &container_name).await;
}

#[rstest]
#[tokio::test]
async fn test_github_repo_clone_large_repository(#[future] test_container: (Docker, String, String)) {
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
async fn test_github_repo_clone_case_sensitivity(#[future] test_container: (Docker, String, String)) {
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
async fn test_github_repo_clone_target_directory_default_behavior(#[future] test_container: (Docker, String, String)) {
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
            clone_response.target_directory, 
            expected_target,
            "Default target directory should be repo name for: {}",
            repo
        );

        println!("Default target dir for '{}' result: {:?}", repo, clone_response);
    }

    // Cleanup
    cleanup_container(&docker, &container_name).await;
}
