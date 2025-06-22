use bollard::Docker;
use rstest::*;
use telegram_bot::{container_utils, GithubClient, GithubClientConfig};
use uuid;

/// Test fixture that provides a Docker client
#[fixture]
pub fn docker() -> Docker {
    Docker::connect_with_socket_defaults().expect("Failed to connect to Docker")
}

/// Test fixture that creates a test container and cleans it up
#[fixture]
pub async fn test_container(docker: Docker) -> (Docker, String, String) {
    let container_name = format!("test-github-clone-integration-{}", uuid::Uuid::new_v4());
    let container_id = container_utils::create_test_container(&docker, &container_name)
        .await
        .expect("Failed to create test container");

    (docker, container_id, container_name)
}

/// Cleanup fixture that ensures test containers are removed
pub async fn cleanup_container(docker: &Docker, container_name: &str) {
    let _ = container_utils::clear_coding_session(docker, container_name).await;
}

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

        println!("Working dir {:?} clone result: {:?}", working_dir, clone_response);
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
            .repo_clone("octocat/Hello-World", Some(&format!("test-timeout-{}", timeout_secs)))
            .await;

        assert!(
            clone_result.is_ok(),
            "Clone should return result with timeout {}: {:?}",
            timeout_secs,
            clone_result
        );

        let clone_response = clone_result.unwrap();
        assert_eq!(clone_response.repository, "octocat/Hello-World");

        println!("Timeout {}s clone result: {:?}", timeout_secs, clone_response);
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
        ("torvalds/linux", "Very large repository (should handle gracefully)"),
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
            println!("✅ Step 3: Repository list retrieved: {} chars", repos.len());
        }
        Err(e) => {
            println!("ℹ️ Step 3: Repository list failed (expected if not authenticated): {}", e);
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