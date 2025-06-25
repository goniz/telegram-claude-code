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

use telegram_bot::{ClaudeCodeClient, ClaudeCodeConfig, GithubClient, GithubClientConfig};

mod test_utils;

// =============================================================================
// All tests now use TestContainerGuard for safe, parallel container management
// =============================================================================

// =============================================================================
// CLAUDE STATUS TESTS
// =============================================================================

/// Tests that Claude Code is available and responds correctly to status checks
#[tokio::test]
async fn test_claude_status_command_with_preinstalled_claude() -> test_utils::TestResult {
    let guard = test_utils::TestContainerGuard::new_with_prefix("claude-status").await?;
    let client = guard.start_coding_session().await?;

    // Claude Code should be pre-installed in the runtime image
    // Simulate the /claudestatus workflow - check availability
    println!("Checking Claude availability...");
    let availability_result = client.check_availability().await;
    if let Err(e) = availability_result {
        return Err(format!("Claude availability check should succeed: {}", e).into());
    }

    let version_output = availability_result.unwrap();
    println!("Claude version output: {}", version_output);

    // The output should contain version information or some success indicator
    if version_output.is_empty() {
        return Err("Claude version output should not be empty".into());
    }
    if version_output.contains("not found") {
        return Err("Should not contain 'not found' error".into());
    }
    if version_output.contains("OCI runtime exec failed") {
        return Err("Should not contain Docker exec error".into());
    }

    // Cleanup
    guard.cleanup().await;
    Ok(())
}

// =============================================================================
// CLAUDE UPDATE TESTS
// =============================================================================

/// Tests that the Claude update command executes without panicking
#[tokio::test]
async fn test_claude_update_command_execution() -> test_utils::TestResult {
    let guard = test_utils::TestContainerGuard::new_with_prefix("claude-update").await?;
    let client = guard.start_coding_session().await?;

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
            if output.is_empty() {
                return Err("Update output should not be empty when successful".into());
            }
        }
        Err(e) => {
            println!("Update failed (expected in test environment): {}", e);
            // Error should be a proper error message, not a panic
            let error_msg = e.to_string();
            if error_msg.is_empty() {
                return Err("Error message should not be empty".into());
            }
        }
    }

    // Cleanup
    guard.cleanup().await;
    Ok(())
}

/// Tests that the update_claude method exists and is callable
#[tokio::test]
async fn test_claude_update_command_method_exists() -> test_utils::TestResult {
    let guard = test_utils::TestContainerGuard::new_with_prefix("claude-method").await?;
    let client = guard.start_coding_session().await?;

    // Test that the method exists and can be called
    // This is a compilation test - if this compiles, the method exists
    let _result = client.update_claude().await;
    
    // We don't assert on the result because in a test environment
    // the update might fail due to network issues, but the method should exist
    println!("update_claude method exists and is callable");

    // Cleanup
    guard.cleanup().await;
    Ok(())
}

// =============================================================================
// CLAUDE UPDATE ENTRYPOINT TESTS
// =============================================================================

/// Tests that the Claude update command uses the entrypoint script properly
#[tokio::test]
async fn test_claude_update_uses_entrypoint_script() -> test_utils::TestResult {
    let guard = test_utils::TestContainerGuard::new_with_socket().await?;
    let client = guard.start_coding_session().await?;

    // Test that the update command uses the proper entrypoint script structure
    // We'll test by executing a command that verifies the entrypoint is being used
    let test_result = client
        .exec_basic_command(vec![
            "sh".to_string(),
            "-c".to_string(),
            "/opt/entrypoint.sh -c \"nvm use default && echo 'entrypoint works'\"".to_string(),
        ])
        .await;

    if let Err(e) = test_result {
        guard.cleanup().await;
        return Err(format!("Entrypoint script test failed: {}", e).into());
    }

    let output = test_result.unwrap();
    if !output.contains("entrypoint works") && !output.contains("Now using node") {
        guard.cleanup().await;
        return Err(format!("Entrypoint script should work properly: {}", output).into());
    }

    // Cleanup
    guard.cleanup().await;
    Ok(())
}

/// Tests that the Claude update command has correct structure and handles errors gracefully
#[tokio::test]
async fn test_claude_update_command_structure() -> test_utils::TestResult {
    let guard = test_utils::TestContainerGuard::new_with_socket().await?;
    let client = guard.start_coding_session().await?;

    // Test that we can at least attempt the update command without errors in command structure
    // Note: The actual update might fail due to authentication, but the command structure should be valid
    let update_result = client.update_claude().await;

    // We expect either success or a controlled failure (not a command structure error)
    let result = match update_result {
        Ok(_) => {
            // Update succeeded
            println!("✅ Claude update command succeeded");
            Ok(())
        }
        Err(e) => {
            let error_msg = e.to_string().to_lowercase();
            
            // These are acceptable error conditions that indicate the command structure is correct
            let acceptable_errors = [
                "authentication", "auth", "login", "token", "unauthorized",
                "not authenticated", "api key", "permission denied", "forbidden",
                "network", "connection", "timeout", "update"
            ];
            
            let is_expected_error = acceptable_errors.iter().any(|pattern| error_msg.contains(pattern));
            
            if !is_expected_error {
                return Err(format!("Update command failed with unexpected error (suggests command structure issue): {}", e).into());
            }
            
            println!("✅ Claude update command has correct structure (failed with expected error: {})", e);
            Ok(())
        }
    };

    // Cleanup
    guard.cleanup().await;
    result
}

// =============================================================================
// CLAUDE INTEGRATION TESTS
// =============================================================================

/// Tests basic container launch and connectivity
#[tokio::test]
async fn test_container_launch_and_connectivity() -> test_utils::TestResult {
    let guard = test_utils::TestContainerGuard::new_with_prefix("connectivity").await?;
    let client = guard.start_coding_session().await?;

    // Try to execute a simple command to verify container is working
    let result = client
        .exec_basic_command(vec!["echo".to_string(), "Hello World".to_string()])
        .await;

    if let Err(e) = result {
        guard.cleanup().await;
        return Err(format!("Container connectivity test failed: {}", e).into());
    }

    let output = result.unwrap();
    if output.trim() != "Hello World" {
        guard.cleanup().await;
        return Err(format!("Expected 'Hello World', got '{}'", output.trim()).into());
    }

    // Cleanup
    guard.cleanup().await;
    Ok(())
}

/// Tests that Claude Code is pre-installed and available
#[tokio::test]
async fn test_claude_code_preinstalled() -> test_utils::TestResult {
    let guard = test_utils::TestContainerGuard::new_with_prefix("preinstalled").await?;
    let client = guard.start_coding_session().await?;

    // Test that Claude Code is pre-installed and available
    let availability_result = client.check_availability().await;

    if let Err(e) = availability_result {
        guard.cleanup().await;
        return Err(format!("Claude Code should be pre-installed and available: {}", e).into());
    }

    // Cleanup
    guard.cleanup().await;
    Ok(())
}

/// Tests Claude availability check functionality
#[tokio::test]
async fn test_claude_availability_check() -> test_utils::TestResult {
    let guard = test_utils::TestContainerGuard::new_with_prefix("availability").await?;
    let client = guard.start_coding_session().await?;

    // Claude Code should be pre-installed in the runtime image
    // Test Claude availability check
    let availability_result = client.check_availability().await;

    if let Err(e) = availability_result {
        guard.cleanup().await;
        return Err(format!("Claude availability check failed: {}", e).into());
    }

    let version_output = availability_result.unwrap();
    // Should contain version information or help text
    if version_output.is_empty() {
        guard.cleanup().await;
        return Err("Version output should not be empty".into());
    }

    // Cleanup
    guard.cleanup().await;
    Ok(())
}

/// Tests Claude CLI basic invocation and binary presence
#[tokio::test]
async fn test_claude_cli_basic_invocation_and_binary_presence() -> test_utils::TestResult {
    let guard = test_utils::TestContainerGuard::new_with_prefix("cli-binary").await?;
    let client = guard.start_coding_session().await?;

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

    if claude_path.is_empty() {
        guard.cleanup().await;
        return Err("claude binary path should not be empty".into());
    }
    
    if !claude_path.contains("claude") {
        guard.cleanup().await;
        return Err(format!("Path should contain 'claude': {}", claude_path).into());
    }

    // Test that the claude binary is executable (only if we found a path)
    if !claude_path.is_empty() {
        let test_exec_result = client
            .exec_basic_command(vec![
                "test".to_string(),
                "-x".to_string(),
                claude_path.trim().to_string(),
            ])
            .await;
        if test_exec_result.is_err() {
            guard.cleanup().await;
            return Err(format!("claude binary is not executable: {:?}", test_exec_result).into());
        }

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
        let is_valid_result = help_result.is_ok()
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
                .contains("login");
        
        if !is_valid_result {
            guard.cleanup().await;
            return Err(format!("Claude CLI basic invocation failed unexpectedly: {:?}", help_result).into());
        }
    }

    // Cleanup
    guard.cleanup().await;
    Ok(())
}

// =============================================================================
// CLAUDE CONFIG PERSISTENCE TESTS
// =============================================================================

/// Tests that Claude configuration persists between container sessions
#[tokio::test]
async fn test_claude_config_persistence_between_sessions() -> test_utils::TestResult {
    let test_user_id = 888888; // Test user ID for config persistence
    
    // Step 1: Start first coding session with persistent volume
    println!("=== STEP 1: Starting first coding session ===");
    let guard1 = test_utils::TestContainerGuard::new_with_persistence(test_user_id).await?;
    let first_client = guard1.start_coding_session().await?;
    
    // Step 2: Check initial Claude config value and set a custom value
    println!("=== STEP 2: Setting Claude config for persistence test ===");
    let config_key = "hasCompletedProjectOnboarding";
    
    // Check initial value (should be undefined)
    let initial_config_result = first_client.exec_basic_command(vec![
        "sh".to_string(),
        "-c".to_string(),
        format!("/opt/entrypoint.sh -c \"nvm use default && claude config get {}\"", config_key),
    ]).await;
    
    if let Err(e) = initial_config_result {
        guard1.cleanup().await;
        return Err(format!("Getting initial config should succeed: {}", e).into());
    }
    let initial_value = initial_config_result.unwrap();
    println!("Initial config value: {}", initial_value.trim());
    
    // Set the config to true for testing
    let set_config_result = first_client.exec_basic_command(vec![
        "sh".to_string(),
        "-c".to_string(),
        format!("/opt/entrypoint.sh -c \"nvm use default && claude config set {} true\"", config_key),
    ]).await;
    
    if let Err(e) = set_config_result {
        guard1.cleanup().await;
        return Err(format!("Setting config should succeed: {}", e).into());
    }
    println!("Config set successfully");
    
    // Step 3: Verify the configuration was set
    println!("=== STEP 3: Verifying config was set ===");
    let verify_config_result = first_client.exec_basic_command(vec![
        "sh".to_string(),
        "-c".to_string(),
        format!("/opt/entrypoint.sh -c \"nvm use default && claude config get {}\"", config_key),
    ]).await;
    
    if let Err(e) = verify_config_result {
        guard1.cleanup().await;
        return Err(format!("Getting config should succeed: {}", e).into());
    }
    let config_output = verify_config_result.unwrap();
    // Extract the last line which contains the actual config value
    let config_value = config_output.lines().last().unwrap_or("").trim();
    if config_value != "true" {
        guard1.cleanup().await;
        return Err(format!("Config should be set to true. Expected: true, Got: {}", config_value).into());
    }
    
    // Step 4: Stop the first session
    println!("=== STEP 4: Stopping first session ===");
    guard1.cleanup().await;
    
    // Step 5: Start second coding session with same user ID (should reuse volume)
    println!("=== STEP 5: Starting second coding session with same user ===");
    let guard2 = test_utils::TestContainerGuard::new_with_persistence(test_user_id).await?;
    let second_client = guard2.start_coding_session().await?;
    
    // Step 6: Verify the Claude config persisted in the new session
    println!("=== STEP 6: Verifying Claude config persisted in new session ===");
    let persisted_config_result = second_client.exec_basic_command(vec![
        "sh".to_string(),
        "-c".to_string(),
        format!("/opt/entrypoint.sh -c \"nvm use default && claude config get {}\"", config_key),
    ]).await;
    
    if let Err(e) = persisted_config_result {
        guard2.cleanup().await;
        return Err(format!("Getting config in second session should succeed: {}", e).into());
    }
    let persisted_output = persisted_config_result.unwrap();
    // Extract the last line which contains the actual config value
    let persisted_value = persisted_output.lines().last().unwrap_or("").trim();
    if persisted_value != "true" {
        guard2.cleanup().await;
        return Err(format!("Claude config should persist between sessions. Expected: true, Got: {}", persisted_value).into());
    }
    
    // Cleanup
    guard2.cleanup().await;
    
    println!("✅ Claude configuration successfully persisted between sessions!");
    Ok(())
}

/// Tests that Claude configuration is properly isolated between different users
#[tokio::test]
async fn test_claude_config_isolation_between_users() -> test_utils::TestResult {
    let test_user_id_1 = 777777;
    let test_user_id_2 = 777778;
    
    // Step 1: Start session for user 1
    println!("=== STEP 1: Starting session for user 1 ===");
    let guard1 = test_utils::TestContainerGuard::new_with_persistence(test_user_id_1).await?;
    let client_1 = guard1.start_coding_session().await?;
    
    // Step 2: Start session for user 2
    println!("=== STEP 2: Starting session for user 2 ===");
    let guard2 = test_utils::TestContainerGuard::new_with_persistence(test_user_id_2).await?;
    let client_2 = guard2.start_coding_session().await?;
    
    // Step 3: Set different Claude config for each user
    println!("=== STEP 3: Setting different Claude config for each user ===");
    let config_key = "hasCompletedProjectOnboarding";
    
    let set_config_1 = client_1.exec_basic_command(vec![
        "sh".to_string(),
        "-c".to_string(),
        format!("/opt/entrypoint.sh -c \"nvm use default && claude config set {} true\"", config_key),
    ]).await;
    
    let set_config_2 = client_2.exec_basic_command(vec![
        "sh".to_string(),
        "-c".to_string(),
        format!("/opt/entrypoint.sh -c \"nvm use default && claude config set {} false\"", config_key),
    ]).await;
    
    if set_config_1.is_err() || set_config_2.is_err() {
        guard1.cleanup().await;
        guard2.cleanup().await;
        return Err("Setting config for both users should succeed".into());
    }
    
    // Step 4: Verify each user has their own isolated Claude config
    println!("=== STEP 4: Verifying Claude config isolation ===");
    let get_config_1 = client_1.exec_basic_command(vec![
        "sh".to_string(),
        "-c".to_string(),
        format!("/opt/entrypoint.sh -c \"nvm use default && claude config get {}\"", config_key),
    ]).await;
    
    let get_config_2 = client_2.exec_basic_command(vec![
        "sh".to_string(),
        "-c".to_string(),
        format!("/opt/entrypoint.sh -c \"nvm use default && claude config get {}\"", config_key),
    ]).await;
    
    if get_config_1.is_err() || get_config_2.is_err() {
        guard1.cleanup().await;
        guard2.cleanup().await;
        return Err("Getting config for both users should succeed".into());
    }
    
    let config_output_1 = get_config_1.unwrap();
    let config_output_2 = get_config_2.unwrap();
    
    // Extract the last line which contains the actual config value
    let config_1 = config_output_1.lines().last().unwrap_or("").trim();
    let config_2 = config_output_2.lines().last().unwrap_or("").trim();
    
    if config_1 != "true" {
        guard1.cleanup().await;
        guard2.cleanup().await;
        return Err(format!("User 1 should have config set to true. Expected: true, Got: {}", config_1).into());
    }
    
    if config_2 != "false" {
        guard1.cleanup().await;
        guard2.cleanup().await;
        return Err(format!("User 2 should have config set to false. Expected: false, Got: {}", config_2).into());
    }
    
    // Cleanup
    guard1.cleanup().await;
    guard2.cleanup().await;
    
    println!("✅ Claude configuration properly isolated between users!");
    Ok(())
}

// =============================================================================
// ENHANCED WORKFLOW TESTS
// =============================================================================

/// Tests the enhanced start workflow that includes authentication checking
#[tokio::test]
async fn test_enhanced_start_workflow_with_auth_checks() -> test_utils::TestResult {
    let guard = test_utils::TestContainerGuard::new_with_socket().await?;
    let claude_client = guard.start_coding_session().await?;

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
        guard.docker().clone(),
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

    if let Err(e) = availability_result {
        guard.cleanup().await;
        return Err(format!("check_availability should succeed after session start: {}", e).into());
    }

    let version_output = availability_result.unwrap();
    println!("✅ Claude Code is available! Version: {}", version_output);

    // Verify the output looks correct
    if version_output.is_empty() {
        guard.cleanup().await;
        return Err("Version output should not be empty".into());
    }
    if version_output.contains("not found") {
        guard.cleanup().await;
        return Err("Should not contain 'not found' error".into());
    }
    if version_output.contains("OCI runtime exec failed") {
        guard.cleanup().await;
        return Err("Should not contain Docker exec error".into());
    }

    // The version should contain some version information
    if !version_output.contains("Claude Code") && !version_output.chars().any(|c| c.is_ascii_digit()) {
        guard.cleanup().await;
        return Err(format!("Version output should contain 'Claude Code' or version numbers: {}", version_output).into());
    }

    // Cleanup
    guard.cleanup().await;

    println!("🎉 Enhanced workflow test passed!");
    Ok(())
}

/// Tests that authentication checks handle errors gracefully
#[tokio::test]
async fn test_authentication_check_resilience() -> test_utils::TestResult {
    let guard = test_utils::TestContainerGuard::new_with_socket().await?;
    let claude_client = guard.start_coding_session().await?;

    println!("=== Testing authentication check error handling ===");

    // Test GitHub authentication check resilience
    let github_client = GithubClient::new(
        guard.docker().clone(),
        claude_client.container_id().to_string(),
        GithubClientConfig::default(),
    );

    // This should not panic even if GitHub CLI is not set up
    let github_auth_result = github_client.check_auth_status().await;
    println!("GitHub auth result (expected to be error or not authenticated): {:?}", github_auth_result);
    
    // We don't assert the result because in test environment it might fail,
    // but we verify it doesn't panic and returns a Result

    // Test Claude authentication check resilience
    let claude_auth_result = claude_client.check_auth_status().await;
    println!("Claude auth result: {:?}", claude_auth_result);
    
    // Claude check should succeed (returning true or false)
    if claude_auth_result.is_err() {
        guard.cleanup().await;
        return Err("Claude auth check should not error".into());
    }

    // Cleanup
    guard.cleanup().await;

    println!("✅ Authentication resilience test passed!");
    Ok(())
}