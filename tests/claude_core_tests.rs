//! # Claude Core Functionality Tests
//!
//! This file consolidates all core Claude functionality tests including:
//! - Claude status and availability checks
//! - Claude update commands and entrypoint script handling
//! - Integration tests for container management and Claude CLI
//! - Configuration persistence across sessions
//! - Enhanced workflow testing with authentication checks
//!
//! All tests are organized by functionality and use shared fixtures for consistency.

use bollard::Docker;
use rstest::*;
use telegram_bot::{
    container_utils, ClaudeCodeClient, ClaudeCodeConfig, GithubClient, GithubClientConfig,
};
use uuid::Uuid;

// =============================================================================
// SHARED TEST FIXTURES
// =============================================================================

/// Test fixture that provides a Docker client using local defaults
#[fixture]
pub fn docker() -> Docker {
    Docker::connect_with_local_defaults().expect("Failed to connect to Docker")
}

/// Test fixture that provides a Docker client using socket defaults
/// Used for tests that require socket-based connection
#[fixture]
pub fn docker_socket() -> Docker {
    Docker::connect_with_socket_defaults().expect("Failed to connect to Docker")
}

/// Test fixture that creates a test container for status tests
#[fixture]
pub async fn status_test_container(docker: Docker) -> (Docker, String, String) {
    let container_name = format!("test-claude-status-{}", Uuid::new_v4());
    let container_id = container_utils::create_test_container(&docker, &container_name)
        .await
        .expect("Failed to create test container");

    (docker, container_id, container_name)
}

/// Test fixture that creates a test container for update tests
#[fixture]
pub async fn update_test_container(docker: Docker) -> (Docker, String, String) {
    let container_name = format!("test-claude-update-{}", Uuid::new_v4());
    let container_id = container_utils::create_test_container(&docker, &container_name)
        .await
        .expect("Failed to create test container");

    (docker, container_id, container_name)
}

/// Test fixture that creates a test container for integration tests
#[fixture]
pub async fn integration_test_container(docker: Docker) -> (Docker, String, String) {
    let container_name = format!("test-claude-integration-{}", Uuid::new_v4());
    let container_id = container_utils::create_test_container(&docker, &container_name)
        .await
        .expect("Failed to create test container");

    (docker, container_id, container_name)
}

/// Test fixture that creates a coding session container using socket connection
#[fixture]
pub async fn socket_test_container(docker_socket: Docker) -> (Docker, String, String) {
    let container_name = format!("test-claude-update-{}", Uuid::new_v4());
    let client = container_utils::start_coding_session(
        &docker_socket,
        &container_name,
        ClaudeCodeConfig::default(),
        container_utils::CodingContainerConfig::default(),
    )
    .await
    .expect("Failed to start coding session");

    (
        docker_socket,
        client.container_id().to_string(),
        container_name,
    )
}

/// Cleanup fixture that ensures test containers are removed
pub async fn cleanup_container(docker: &Docker, container_name: &str) {
    let _ = container_utils::clear_coding_session(docker, container_name).await;
}

/// Cleanup fixture that ensures test containers and volumes are removed
pub async fn cleanup_test_resources(docker: &Docker, container_name: &str, user_id: i64) {
    // Clean up container
    let _ = container_utils::clear_coding_session(docker, container_name).await;

    // Clean up volume
    let volume_name = container_utils::generate_volume_name(&user_id.to_string());
    let _ = docker.remove_volume(&volume_name, None).await;
}

// =============================================================================
// CLAUDE STATUS TESTS
// =============================================================================

/// Tests that Claude Code is available and responds correctly to status checks
#[rstest]
#[tokio::test]
async fn test_claude_status_command_with_preinstalled_claude(
    #[future] status_test_container: (Docker, String, String),
) {
    let (docker, container_id, container_name) = status_test_container.await;

    let client = ClaudeCodeClient::new(docker.clone(), container_id, ClaudeCodeConfig::default());

    // Claude Code should be pre-installed in the runtime image
    // Simulate the /claudestatus workflow - check availability
    println!("Checking Claude availability...");
    let availability_result = client.check_availability().await;
    assert!(
        availability_result.is_ok(),
        "Claude availability check should succeed: {:?}",
        availability_result
    );

    let version_output = availability_result.unwrap();
    println!("Claude version output: {}", version_output);

    // The output should contain version information or some success indicator
    assert!(
        !version_output.is_empty(),
        "Claude version output should not be empty"
    );
    assert!(
        !version_output.contains("not found"),
        "Should not contain 'not found' error"
    );
    assert!(
        !version_output.contains("OCI runtime exec failed"),
        "Should not contain Docker exec error"
    );

    // Cleanup
    cleanup_container(&docker, &container_name).await;
}

// =============================================================================
// CLAUDE UPDATE TESTS
// =============================================================================

/// Tests that the Claude update command executes without panicking
#[rstest]
#[tokio::test]
async fn test_claude_update_command_execution(
    #[future] update_test_container: (Docker, String, String),
) {
    let (docker, container_id, container_name) = update_test_container.await;

    let client = ClaudeCodeClient::new(docker.clone(), container_id, ClaudeCodeConfig::default());

    // Simulate the /update-claude workflow
    println!("Testing Claude update command...");
    let update_result = client.update_claude().await;

    // The update command should either succeed or fail gracefully
    // We can't guarantee it will always succeed (network issues, etc.)
    // but we can test that the method exists and executes without panicking
    match update_result {
        Ok(output) => {
            println!("Update succeeded with output: {}", output);
            // If successful, output should not be empty
            assert!(
                !output.is_empty(),
                "Update output should not be empty when successful"
            );
        }
        Err(e) => {
            println!("Update failed (expected in test environment): {}", e);
            // Error should be a proper error message, not a panic
            let error_msg = e.to_string();
            assert!(!error_msg.is_empty(), "Error message should not be empty");
        }
    }

    // Cleanup
    cleanup_container(&docker, &container_name).await;
}

/// Tests that the update_claude method exists and is callable
#[rstest]
#[tokio::test]
async fn test_claude_update_command_method_exists(
    #[future] update_test_container: (Docker, String, String),
) {
    let (docker, container_id, container_name) = update_test_container.await;

    let client = ClaudeCodeClient::new(docker.clone(), container_id, ClaudeCodeConfig::default());

    // Test that the method exists and can be called
    // This is a compilation test - if this compiles, the method exists
    let _result = client.update_claude().await;

    // We don't assert on the result because in a test environment
    // the update might fail due to network issues, but the method should exist
    println!("update_claude method exists and is callable");

    // Cleanup
    cleanup_container(&docker, &container_name).await;
}

// =============================================================================
// CLAUDE UPDATE ENTRYPOINT TESTS
// =============================================================================

/// Tests that the Claude update command uses the entrypoint script properly
#[rstest]
#[tokio::test]
async fn test_claude_update_uses_entrypoint_script(
    #[future] socket_test_container: (Docker, String, String),
) {
    let (docker, container_id, container_name) = socket_test_container.await;

    let client = ClaudeCodeClient::new(docker.clone(), container_id, ClaudeCodeConfig::default());

    // Test that the update command uses the proper entrypoint script structure
    // We'll test by executing a command that verifies the entrypoint is being used
    let test_result = client
        .exec_basic_command(vec![
            "sh".to_string(),
            "-c".to_string(),
            "/opt/entrypoint.sh -c \"nvm use default && echo 'entrypoint works'\"".to_string(),
        ])
        .await;

    // Cleanup
    cleanup_container(&docker, &container_name).await;

    assert!(
        test_result.is_ok(),
        "Entrypoint script test failed: {:?}",
        test_result
    );

    let output = test_result.unwrap();
    assert!(
        output.contains("entrypoint works") || output.contains("Now using node"),
        "Entrypoint script should work properly: {}",
        output
    );
}

/// Tests that the Claude update command has correct structure and handles errors gracefully
#[rstest]
#[tokio::test]
async fn test_claude_update_command_structure(
    #[future] socket_test_container: (Docker, String, String),
) {
    let (docker, container_id, container_name) = socket_test_container.await;

    let client = ClaudeCodeClient::new(docker.clone(), container_id, ClaudeCodeConfig::default());

    // Test that we can at least attempt the update command without errors in command structure
    // Note: The actual update might fail due to authentication, but the command structure should be valid
    let update_result = client.update_claude().await;

    // Cleanup
    cleanup_container(&docker, &container_name).await;

    // We expect either success or a controlled failure (not a command structure error)
    match update_result {
        Ok(_) => {
            // Update succeeded
            println!("✅ Claude update command succeeded");
        }
        Err(e) => {
            let error_msg = e.to_string().to_lowercase();

            // These are acceptable error conditions that indicate the command structure is correct
            let acceptable_errors = [
                "authentication",
                "auth",
                "login",
                "token",
                "unauthorized",
                "not authenticated",
                "api key",
                "permission denied",
                "forbidden",
                "network",
                "connection",
                "timeout",
                "update",
            ];

            let is_expected_error = acceptable_errors
                .iter()
                .any(|pattern| error_msg.contains(pattern));

            assert!(
                is_expected_error,
                "Update command failed with unexpected error (suggests command structure issue): {}",
                e
            );

            println!(
                "✅ Claude update command has correct structure (failed with expected error: {})",
                e
            );
        }
    }
}

// =============================================================================
// CLAUDE INTEGRATION TESTS
// =============================================================================

/// Tests basic container launch and connectivity
#[rstest]
#[tokio::test]
async fn test_container_launch_and_connectivity(
    #[future] integration_test_container: (Docker, String, String),
) {
    let (docker, container_id, container_name) = integration_test_container.await;

    // Test basic connectivity
    let client = ClaudeCodeClient::new(docker.clone(), container_id, ClaudeCodeConfig::default());

    // Try to execute a simple command to verify container is working
    let result = client
        .exec_basic_command(vec!["echo".to_string(), "Hello World".to_string()])
        .await;

    // Cleanup
    cleanup_container(&docker, &container_name).await;

    assert!(
        result.is_ok(),
        "Container connectivity test failed: {:?}",
        result
    );
    assert_eq!(result.unwrap().trim(), "Hello World");
}

/// Tests that Claude Code is pre-installed and available
#[rstest]
#[tokio::test]
async fn test_claude_code_preinstalled(
    #[future] integration_test_container: (Docker, String, String),
) {
    let (docker, container_id, container_name) = integration_test_container.await;

    let client = ClaudeCodeClient::new(docker.clone(), container_id, ClaudeCodeConfig::default());

    // Test that Claude Code is pre-installed and available
    let availability_result = client.check_availability().await;

    // Cleanup
    cleanup_container(&docker, &container_name).await;

    assert!(
        availability_result.is_ok(),
        "Claude Code should be pre-installed and available: {:?}",
        availability_result
    );
}

/// Tests Claude availability check functionality
#[rstest]
#[tokio::test]
async fn test_claude_availability_check(
    #[future] integration_test_container: (Docker, String, String),
) {
    let (docker, container_id, container_name) = integration_test_container.await;

    let client = ClaudeCodeClient::new(docker.clone(), container_id, ClaudeCodeConfig::default());

    // Claude Code should be pre-installed in the runtime image
    // Test Claude availability check
    let availability_result = client.check_availability().await;

    // Cleanup
    cleanup_container(&docker, &container_name).await;

    assert!(
        availability_result.is_ok(),
        "Claude availability check failed: {:?}",
        availability_result
    );
    let version_output = availability_result.unwrap();
    // Should contain version information or help text
    assert!(
        !version_output.is_empty(),
        "Version output should not be empty"
    );
}

/// Tests Claude CLI basic invocation and binary presence
#[rstest]
#[tokio::test]
async fn test_claude_cli_basic_invocation_and_binary_presence(
    #[future] integration_test_container: (Docker, String, String),
) {
    let (docker, container_id, container_name) = integration_test_container.await;

    let client = ClaudeCodeClient::new(docker.clone(), container_id, ClaudeCodeConfig::default());

    // Claude Code should be pre-installed in the runtime image
    // Debug: Check what's in the PATH and npm global bin
    let npm_bin_result = client
        .exec_basic_command(vec!["npm".to_string(), "bin".to_string(), "-g".to_string()])
        .await;
    println!("npm global bin directory: {:?}", npm_bin_result);

    let path_result = client
        .exec_basic_command(vec!["echo".to_string(), "$PATH".to_string()])
        .await;
    println!("Current PATH: {:?}", path_result);

    // Test that claude binary is present and reachable via PATH
    let which_result = client
        .exec_basic_command(vec!["which".to_string(), "claude".to_string()])
        .await;

    // If which fails, try to find claude in common npm locations
    let claude_path = if which_result.is_ok() && !which_result.as_ref().unwrap().is_empty() {
        which_result.unwrap()
    } else {
        println!("which claude failed: {:?}", which_result);
        // Try common npm global bin locations
        let npm_global_result = client
            .exec_basic_command(vec![
                "ls".to_string(),
                "-la".to_string(),
                "/usr/local/bin/claude".to_string(),
            ])
            .await;
        if npm_global_result.is_ok() {
            "/usr/local/bin/claude".to_string()
        } else {
            // Try node_modules location with a simpler approach
            let node_modules_result = client
                .exec_basic_command(vec![
                    "sh".to_string(),
                    "-c".to_string(),
                    "find / -name claude -type f -executable 2>/dev/null | head -1".to_string(),
                ])
                .await;
            if node_modules_result.is_ok() && !node_modules_result.as_ref().unwrap().is_empty() {
                node_modules_result.unwrap()
            } else {
                println!("Could not find claude binary anywhere");
                // Check if the npm package was actually installed
                let npm_list_result = client
                    .exec_basic_command(vec![
                        "npm".to_string(),
                        "list".to_string(),
                        "-g".to_string(),
                        "@anthropic-ai/claude-code".to_string(),
                    ])
                    .await;
                println!("npm list result: {:?}", npm_list_result);

                String::new() // Return empty string to trigger proper failure message
            }
        }
    };

    assert!(
        !claude_path.is_empty(),
        "claude binary path should not be empty"
    );
    assert!(
        claude_path.contains("claude"),
        "Path should contain 'claude': {}",
        claude_path
    );

    // Test that the claude binary is executable (only if we found a path)
    if !claude_path.is_empty() {
        let test_exec_result = client
            .exec_basic_command(vec![
                "test".to_string(),
                "-x".to_string(),
                claude_path.trim().to_string(),
            ])
            .await;
        assert!(
            test_exec_result.is_ok(),
            "claude binary is not executable: {:?}",
            test_exec_result
        );

        // Test basic Claude CLI invocation (help command)
        let help_result = client
            .exec_basic_command(vec!["claude".to_string(), "--help".to_string()])
            .await;
        println!(
            "Claude help output: '{}'",
            help_result.as_ref().unwrap_or(&"failed".to_string())
        );

        // The help command might fail if Claude requires authentication, so we just verify it produces some output
        // or fails with an expected authentication error
        assert!(
            help_result.is_ok()
                || help_result
                    .as_ref()
                    .err()
                    .unwrap()
                    .to_string()
                    .contains("auth")
                || help_result
                    .as_ref()
                    .err()
                    .unwrap()
                    .to_string()
                    .contains("login"),
            "Claude CLI basic invocation failed unexpectedly: {:?}",
            help_result
        );
    }

    // Cleanup
    cleanup_container(&docker, &container_name).await;
}

// =============================================================================
// CLAUDE CONFIG PERSISTENCE TESTS
// =============================================================================

/// Tests that Claude configuration persists between container sessions
#[rstest]
#[tokio::test]
async fn test_claude_config_persistence_between_sessions(docker_socket: Docker) {
    let test_user_id = 888888; // Test user ID for config persistence
    let container_name_1 = format!("test-config-persistence-1-{}", Uuid::new_v4());
    let container_name_2 = format!("test-config-persistence-2-{}", Uuid::new_v4());

    // Clean up any existing volume before starting test
    let volume_name = container_utils::generate_volume_name(&test_user_id.to_string());
    let _ = docker_socket.remove_volume(&volume_name, None).await;

    // Step 1: Start first coding session with persistent volume
    println!("=== STEP 1: Starting first coding session ===");
    let first_session = container_utils::start_coding_session(
        &docker_socket,
        &container_name_1,
        ClaudeCodeConfig::default(),
        container_utils::CodingContainerConfig {
            persistent_volume_key: Some(test_user_id.to_string()),
        },
    )
    .await;

    assert!(
        first_session.is_ok(),
        "First session should start successfully"
    );
    let first_client = first_session.unwrap();

    // Step 2: Check initial Claude config value and set a custom value
    println!("=== STEP 2: Setting Claude config for persistence test ===");
    let config_key = "hasCompletedProjectOnboarding";

    // Check initial value (should be undefined)
    let initial_config_result = first_client
        .exec_basic_command(vec![
            "sh".to_string(),
            "-c".to_string(),
            format!(
                "/opt/entrypoint.sh -c \"nvm use default && claude config get {}\"",
                config_key
            ),
        ])
        .await;

    assert!(
        initial_config_result.is_ok(),
        "Getting initial config should succeed: {:?}",
        initial_config_result
    );
    let initial_value = initial_config_result.unwrap();
    println!("Initial config value: {}", initial_value.trim());

    // Set the config to true for testing
    let set_config_result = first_client
        .exec_basic_command(vec![
            "sh".to_string(),
            "-c".to_string(),
            format!(
                "/opt/entrypoint.sh -c \"nvm use default && claude config set {} true\"",
                config_key
            ),
        ])
        .await;

    assert!(
        set_config_result.is_ok(),
        "Setting config should succeed: {:?}",
        set_config_result
    );
    println!("Config set successfully");

    // Step 3: Verify the configuration was set
    println!("=== STEP 3: Verifying config was set ===");
    let verify_config_result = first_client
        .exec_basic_command(vec![
            "sh".to_string(),
            "-c".to_string(),
            format!(
                "/opt/entrypoint.sh -c \"nvm use default && claude config get {}\"",
                config_key
            ),
        ])
        .await;

    assert!(
        verify_config_result.is_ok(),
        "Getting config should succeed: {:?}",
        verify_config_result
    );
    let config_output = verify_config_result.unwrap();
    // Extract the last line which contains the actual config value
    let config_value = config_output.lines().last().unwrap_or("").trim();
    assert!(
        config_value == "true",
        "Config should be set to true. Expected: true, Got: {}",
        config_value
    );

    // Step 4: Stop the first session
    println!("=== STEP 4: Stopping first session ===");
    container_utils::clear_coding_session(&docker_socket, &container_name_1)
        .await
        .expect("Should clear session successfully");

    // Step 5: Start second coding session with same user ID (should reuse volume)
    println!("=== STEP 5: Starting second coding session with same user ===");
    let second_session = container_utils::start_coding_session(
        &docker_socket,
        &container_name_2,
        ClaudeCodeConfig::default(),
        container_utils::CodingContainerConfig {
            persistent_volume_key: Some(test_user_id.to_string()),
        },
    )
    .await;

    assert!(
        second_session.is_ok(),
        "Second session should start successfully"
    );
    let second_client = second_session.unwrap();

    // Step 6: Verify the Claude config persisted in the new session
    println!("=== STEP 6: Verifying Claude config persisted in new session ===");
    let persisted_config_result = second_client
        .exec_basic_command(vec![
            "sh".to_string(),
            "-c".to_string(),
            format!(
                "/opt/entrypoint.sh -c \"nvm use default && claude config get {}\"",
                config_key
            ),
        ])
        .await;

    // Cleanup
    cleanup_test_resources(&docker_socket, &container_name_2, test_user_id).await;

    assert!(
        persisted_config_result.is_ok(),
        "Getting config in second session should succeed: {:?}",
        persisted_config_result
    );
    let persisted_output = persisted_config_result.unwrap();
    // Extract the last line which contains the actual config value
    let persisted_value = persisted_output.lines().last().unwrap_or("").trim();
    assert!(
        persisted_value == "true",
        "Claude config should persist between sessions. Expected: true, Got: {}",
        persisted_value
    );

    println!("✅ Claude configuration successfully persisted between sessions!");
}

/// Tests that Claude configuration is properly isolated between different users
#[rstest]
#[tokio::test]
async fn test_claude_config_isolation_between_users(docker_socket: Docker) {
    let test_user_id_1 = 777777;
    let test_user_id_2 = 777778;
    let container_name_1 = format!("test-config-isolation-1-{}", Uuid::new_v4());
    let container_name_2 = format!("test-config-isolation-2-{}", Uuid::new_v4());

    // Clean up any existing volumes
    let volume_name_1 = container_utils::generate_volume_name(&test_user_id_1.to_string());
    let volume_name_2 = container_utils::generate_volume_name(&test_user_id_2.to_string());
    let _ = docker_socket.remove_volume(&volume_name_1, None).await;
    let _ = docker_socket.remove_volume(&volume_name_2, None).await;

    // Step 1: Start session for user 1
    println!("=== STEP 1: Starting session for user 1 ===");
    let session_1 = container_utils::start_coding_session(
        &docker_socket,
        &container_name_1,
        ClaudeCodeConfig::default(),
        container_utils::CodingContainerConfig {
            persistent_volume_key: Some(test_user_id_1.to_string()),
        },
    )
    .await;

    assert!(
        session_1.is_ok(),
        "User 1 session should start successfully"
    );
    let client_1 = session_1.unwrap();

    // Step 2: Start session for user 2
    println!("=== STEP 2: Starting session for user 2 ===");
    let session_2 = container_utils::start_coding_session(
        &docker_socket,
        &container_name_2,
        ClaudeCodeConfig::default(),
        container_utils::CodingContainerConfig {
            persistent_volume_key: Some(test_user_id_2.to_string()),
        },
    )
    .await;

    assert!(
        session_2.is_ok(),
        "User 2 session should start successfully"
    );
    let client_2 = session_2.unwrap();

    // Step 3: Set different Claude config for each user
    println!("=== STEP 3: Setting different Claude config for each user ===");
    let config_key = "hasCompletedProjectOnboarding";

    let set_config_1 = client_1
        .exec_basic_command(vec![
            "sh".to_string(),
            "-c".to_string(),
            format!(
                "/opt/entrypoint.sh -c \"nvm use default && claude config set {} true\"",
                config_key
            ),
        ])
        .await;

    let set_config_2 = client_2
        .exec_basic_command(vec![
            "sh".to_string(),
            "-c".to_string(),
            format!(
                "/opt/entrypoint.sh -c \"nvm use default && claude config set {} false\"",
                config_key
            ),
        ])
        .await;

    assert!(
        set_config_1.is_ok(),
        "Setting config for user 1 should succeed"
    );
    assert!(
        set_config_2.is_ok(),
        "Setting config for user 2 should succeed"
    );

    // Step 4: Verify each user has their own isolated Claude config
    println!("=== STEP 4: Verifying Claude config isolation ===");
    let get_config_1 = client_1
        .exec_basic_command(vec![
            "sh".to_string(),
            "-c".to_string(),
            format!(
                "/opt/entrypoint.sh -c \"nvm use default && claude config get {}\"",
                config_key
            ),
        ])
        .await;

    let get_config_2 = client_2
        .exec_basic_command(vec![
            "sh".to_string(),
            "-c".to_string(),
            format!(
                "/opt/entrypoint.sh -c \"nvm use default && claude config get {}\"",
                config_key
            ),
        ])
        .await;

    // Cleanup
    cleanup_test_resources(&docker_socket, &container_name_1, test_user_id_1).await;
    cleanup_test_resources(&docker_socket, &container_name_2, test_user_id_2).await;

    assert!(
        get_config_1.is_ok(),
        "Getting config for user 1 should succeed"
    );
    assert!(
        get_config_2.is_ok(),
        "Getting config for user 2 should succeed"
    );

    let config_output_1 = get_config_1.unwrap();
    let config_output_2 = get_config_2.unwrap();

    // Extract the last line which contains the actual config value
    let config_1 = config_output_1.lines().last().unwrap_or("").trim();
    let config_2 = config_output_2.lines().last().unwrap_or("").trim();

    assert!(
        config_1 == "true",
        "User 1 should have config set to true. Expected: true, Got: {}",
        config_1
    );
    assert!(
        config_2 == "false",
        "User 2 should have config set to false. Expected: false, Got: {}",
        config_2
    );

    println!("✅ Claude configuration properly isolated between users!");
}

// =============================================================================
// ENHANCED WORKFLOW TESTS
// =============================================================================

/// Tests the enhanced start workflow that includes authentication checking
#[rstest]
#[tokio::test]
async fn test_enhanced_start_workflow_with_auth_checks(docker_socket: Docker) {
    let container_name = format!("test-enhanced-workflow-{}", Uuid::new_v4());

    // Test the enhanced start workflow that includes authentication checking

    // Step 1: Simulate /start command (container creation)
    println!("=== STEP 1: Starting coding session (enhanced /start workflow) ===");
    let claude_client_result = container_utils::start_coding_session(
        &docker_socket,
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
        docker_socket.clone(),
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
    cleanup_container(&docker_socket, &container_name).await;

    println!("🎉 Enhanced workflow test passed!");
}

/// Tests that authentication checks handle errors gracefully
#[rstest]
#[tokio::test]
async fn test_authentication_check_resilience(docker_socket: Docker) {
    let container_name = format!("test-auth-resilience-{}", Uuid::new_v4());

    // Test that authentication checks handle errors gracefully

    println!("=== Testing authentication check error handling ===");
    let claude_client_result = container_utils::start_coding_session(
        &docker_socket,
        &container_name,
        ClaudeCodeConfig::default(),
        container_utils::CodingContainerConfig::default(),
    )
    .await;

    assert!(claude_client_result.is_ok());
    let claude_client = claude_client_result.unwrap();

    // Test GitHub authentication check resilience
    let github_client = GithubClient::new(
        docker_socket.clone(),
        claude_client.container_id().to_string(),
        GithubClientConfig::default(),
    );

    // This should not panic even if GitHub CLI is not set up
    let github_auth_result = github_client.check_auth_status().await;
    println!(
        "GitHub auth result (expected to be error or not authenticated): {:?}",
        github_auth_result
    );

    // We don't assert the result because in test environment it might fail,
    // but we verify it doesn't panic and returns a Result

    // Test Claude authentication check resilience
    let claude_auth_result = claude_client.check_auth_status().await;
    println!("Claude auth result: {:?}", claude_auth_result);

    // Claude check should succeed (returning true or false)
    assert!(
        claude_auth_result.is_ok(),
        "Claude auth check should not error"
    );

    // Cleanup
    cleanup_container(&docker_socket, &container_name).await;

    println!("✅ Authentication resilience test passed!");
}
