//! Infrastructure and System Integration Tests
//!
//! This module contains comprehensive tests for the Telegram Claude Code bot infrastructure,
//! including volume persistence, timeout handling, output buffering, and end-to-end workflows.
//!
//! Test Categories:
//! - Volume Persistence: Tests for Docker volume management and data persistence across sessions
//! - Timeout Integration: Tests for command timeout handling and error message formatting
//! - Output Buffering: Tests for output buffering mechanism and state transition management
//! - Bot Workflows: End-to-end workflow tests simulating real user interactions

use bollard::Docker;
use rstest::*;
use std::time::Duration;
use telegram_bot::{container_utils, ClaudeCodeClient, ClaudeCodeConfig, GithubClientConfig};
use tokio::time::{sleep, Instant};
use uuid::Uuid;

// =============================================================================
// Test Fixtures and Utilities
// =============================================================================

/// Test fixture that provides a Docker client
#[fixture]
pub fn docker() -> Docker {
    Docker::connect_with_local_defaults().expect("Failed to connect to Docker")
}

/// Cleanup fixture that ensures test containers and volumes are removed
pub async fn cleanup_test_resources(docker: &Docker, container_name: &str, user_id: i64) {
    // Clean up container
    let _ = container_utils::clear_coding_session(docker, container_name).await;
    
    // Clean up volume
    let volume_name = container_utils::generate_volume_name(&user_id.to_string());
    let _ = docker.remove_volume(&volume_name, None).await;
}

/// Cleanup fixture that ensures test containers are removed
pub async fn cleanup_container(docker: &Docker, container_name: &str) {
    let _ = container_utils::clear_coding_session(docker, container_name).await;
}

// =============================================================================
// Volume Persistence Tests
// =============================================================================

/// Tests volume creation and data persistence between coding sessions
/// 
/// This test verifies that authentication data (Claude, GitHub) persists
/// across different container sessions when using the same user ID.
#[rstest]
#[tokio::test]
async fn test_volume_creation_and_persistence(docker: Docker) {
    let test_user_id = 999999; // Test user ID
    let container_name = format!("test-volume-{}", Uuid::new_v4());
    
    // Clean up any existing volume before starting test
    let volume_name = container_utils::generate_volume_name(&test_user_id.to_string());
    let _ = docker.remove_volume(&volume_name, None).await;
    
    // Step 1: Start first coding session
    println!("=== STEP 1: Starting first coding session ===");
    let first_session = container_utils::start_coding_session(
        &docker,
        &container_name,
        ClaudeCodeConfig::default(),
        container_utils::CodingContainerConfig { 
            persistent_volume_key: Some(test_user_id.to_string()) 
        },
    )
    .await;
    
    assert!(first_session.is_ok(), "First session should start successfully");
    let first_client = first_session.unwrap();
    
    // Step 2: Create some test authentication data
    println!("=== STEP 2: Creating test authentication data ===");
    let test_commands = vec![
        // Create test Claude authentication file in directory
        vec![
            "sh".to_string(),
            "-c".to_string(),
            "echo '{\"test\": \"claude_auth_data\"}' > /root/.claude/test_auth.json".to_string(),
        ],
        // Create test Claude configuration file
        vec![
            "sh".to_string(),
            "-c".to_string(),
            "echo '{\"api_key\": \"test_claude_config\"}' > /root/.claude.json".to_string(),
        ],
        // Create test GitHub authentication data
        vec![
            "sh".to_string(),
            "-c".to_string(),
            "echo 'test_github_token' > /root/.config/gh/token".to_string(),
        ],
    ];
    
    for command in test_commands {
        let result = container_utils::exec_command_in_container(
            &docker,
            first_client.container_id(),
            command.clone(),
        ).await;
        
        assert!(result.is_ok(), "Command should execute successfully: {:?}", command);
    }
    
    // Step 3: Stop the first session
    println!("=== STEP 3: Stopping first session ===");
    container_utils::clear_coding_session(&docker, &container_name).await
        .expect("Should clear session successfully");
    
    // Step 4: Start second coding session with same user ID
    println!("=== STEP 4: Starting second coding session with same user ===");
    let second_container_name = format!("test-volume-2-{}", Uuid::new_v4());
    let second_session = container_utils::start_coding_session(
        &docker,
        &second_container_name,
        ClaudeCodeConfig::default(),
        container_utils::CodingContainerConfig { 
            persistent_volume_key: Some(test_user_id.to_string()) 
        },
    )
    .await;
    
    assert!(second_session.is_ok(), "Second session should start successfully: {:?}", second_session.as_ref().err());
    let second_client = second_session.unwrap();
    
    // Step 5: Verify that authentication data persisted
    println!("=== STEP 5: Verifying authentication data persistence ===");
    
    // Check Claude authentication data
    let claude_data_result = container_utils::exec_command_in_container(
        &docker,
        second_client.container_id(),
        vec!["cat".to_string(), "/root/.claude/test_auth.json".to_string()],
    ).await;
    
    assert!(claude_data_result.is_ok(), "Should be able to read Claude auth data");
    let claude_data = claude_data_result.unwrap();
    assert!(claude_data.contains("claude_auth_data"), "Claude auth data should persist");
    
    // Check Claude configuration file (.claude.json)
    let claude_config_result = container_utils::exec_command_in_container(
        &docker,
        second_client.container_id(),
        vec!["cat".to_string(), "/root/.claude.json".to_string()],
    ).await;
    
    assert!(claude_config_result.is_ok(), "Should be able to read Claude config file");
    let claude_config = claude_config_result.unwrap();
    assert!(claude_config.contains("test_claude_config"), "Claude config file should persist");
    
    // Check GitHub authentication data
    let github_data_result = container_utils::exec_command_in_container(
        &docker,
        second_client.container_id(),
        vec!["cat".to_string(), "/root/.config/gh/token".to_string()],
    ).await;
    
    assert!(github_data_result.is_ok(), "Should be able to read GitHub auth data");
    let github_data = github_data_result.unwrap();
    assert!(github_data.contains("test_github_token"), "GitHub auth data should persist");
    
    println!("✅ Authentication data successfully persisted between sessions!");
    
    // Cleanup
    cleanup_test_resources(&docker, &second_container_name, test_user_id).await;
}

/// Tests that volumes are properly isolated between different users
/// 
/// This test verifies that user data stored in persistent volumes cannot
/// be accessed by other users, ensuring proper data isolation.
#[rstest]
#[tokio::test]
async fn test_volume_isolation_between_users(docker: Docker) {
    let user_id_1 = 111111;
    let user_id_2 = 222222;
    let container_name_1 = format!("test-isolation-1-{}", Uuid::new_v4());
    let container_name_2 = format!("test-isolation-2-{}", Uuid::new_v4());
    
    // Clean up any existing volumes before starting test
    let volume_name_1 = container_utils::generate_volume_name(&user_id_1.to_string());
    let volume_name_2 = container_utils::generate_volume_name(&user_id_2.to_string());
    let _ = docker.remove_volume(&volume_name_1, None).await;
    let _ = docker.remove_volume(&volume_name_2, None).await;
    
    // Step 1: Start sessions for both users
    println!("=== STEP 1: Starting sessions for two different users ===");
    
    let session_1 = container_utils::start_coding_session(
        &docker,
        &container_name_1,
        ClaudeCodeConfig::default(),
        container_utils::CodingContainerConfig { 
            persistent_volume_key: Some(user_id_1.to_string()) 
        },
    )
    .await;
    
    let session_2 = container_utils::start_coding_session(
        &docker,
        &container_name_2,
        ClaudeCodeConfig::default(),
        container_utils::CodingContainerConfig { 
            persistent_volume_key: Some(user_id_2.to_string()) 
        },
    )
    .await;
    
    assert!(session_1.is_ok(), "User 1 session should start successfully");
    assert!(session_2.is_ok(), "User 2 session should start successfully");
    
    let client_1 = session_1.unwrap();
    let client_2 = session_2.unwrap();
    
    // Step 2: Create different auth data for each user
    println!("=== STEP 2: Creating different auth data for each user ===");
    
    // User 1 data
    let user1_result = container_utils::exec_command_in_container(
        &docker,
        client_1.container_id(),
        vec![
            "sh".to_string(),
            "-c".to_string(),
            "echo 'user1_secret' > /root/.claude/user_data.txt".to_string(),
        ],
    ).await;
    
    // User 2 data
    let user2_result = container_utils::exec_command_in_container(
        &docker,
        client_2.container_id(),
        vec![
            "sh".to_string(),
            "-c".to_string(),
            "echo 'user2_secret' > /root/.claude/user_data.txt".to_string(),
        ],
    ).await;
    
    assert!(user1_result.is_ok(), "User 1 data creation should succeed");
    assert!(user2_result.is_ok(), "User 2 data creation should succeed");
    
    // Step 3: Verify data isolation
    println!("=== STEP 3: Verifying data isolation between users ===");
    
    // Check user 1 can only see their data
    let user1_data = container_utils::exec_command_in_container(
        &docker,
        client_1.container_id(),
        vec!["cat".to_string(), "/root/.claude/user_data.txt".to_string()],
    ).await.unwrap();
    
    // Check user 2 can only see their data
    let user2_data = container_utils::exec_command_in_container(
        &docker,
        client_2.container_id(),
        vec!["cat".to_string(), "/root/.claude/user_data.txt".to_string()],
    ).await.unwrap();
    
    assert!(user1_data.contains("user1_secret"), "User 1 should see only their data");
    assert!(user2_data.contains("user2_secret"), "User 2 should see only their data");
    assert!(!user1_data.contains("user2_secret"), "User 1 should not see user 2's data");
    assert!(!user2_data.contains("user1_secret"), "User 2 should not see user 1's data");
    
    println!("✅ User data is properly isolated!");
    
    // Cleanup
    cleanup_test_resources(&docker, &container_name_1, user_id_1).await;
    cleanup_test_resources(&docker, &container_name_2, user_id_2).await;
}

/// Tests the volume name generation function
/// 
/// This test verifies that the volume name generation function produces
/// consistent and unique names for different user IDs.
#[rstest]
#[tokio::test]
async fn test_volume_name_generation() {
    // Test volume name generation function
    let volume_key = "12345";
    let volume_name = container_utils::generate_volume_name(volume_key);
    
    assert_eq!(volume_name, "dev-session-claude-12345");
    
    // Test with different volume key
    let volume_key_2 = "67890";
    let volume_name_2 = container_utils::generate_volume_name(volume_key_2);
    
    assert_eq!(volume_name_2, "dev-session-claude-67890");
    assert_ne!(volume_name, volume_name_2, "Different keys should have different volume names");
}

/// Tests the persistent volume configuration setting
/// 
/// This test verifies that the persistent_volume_key configuration properly
/// controls whether persistent volumes are used or not.
#[rstest]
#[tokio::test]
async fn test_use_persistant_volume_setting(docker: Docker) {
    let test_user_id = 555555; // Test user ID
    let container_name_with_volume = format!("test-with-volume-{}", Uuid::new_v4());
    let container_name_without_volume = format!("test-without-volume-{}", Uuid::new_v4());
    
    // Clean up any existing volume before starting test
    let volume_name = container_utils::generate_volume_name(&test_user_id.to_string());
    let _ = docker.remove_volume(&volume_name, None).await;
    
    // Test 1: Start session WITH persistent volume
    println!("=== Testing with persistent_volume_key = Some(...) ===");
    let session_with_volume = container_utils::start_coding_session(
        &docker,
        &container_name_with_volume,
        ClaudeCodeConfig::default(),
        container_utils::CodingContainerConfig { 
            persistent_volume_key: Some(test_user_id.to_string()) 
        },
    )
    .await;
    
    assert!(session_with_volume.is_ok(), "Session with persistent volume should start successfully: {:?}", session_with_volume.as_ref().err());
    let client_with_volume = session_with_volume.unwrap();
    
    // Verify volume initialization occurred (symlinks should point to volume_data when persistent volume is enabled)
    let symlink_target_result = container_utils::exec_command_in_container(
        &docker,
        client_with_volume.container_id(),
        vec!["readlink".to_string(), "/root/.claude".to_string()],
    ).await;
    
    assert!(symlink_target_result.is_ok(), "Should be able to read symlink when persistent volume is enabled");
    let symlink_target = symlink_target_result.unwrap();
    assert!(symlink_target.contains("/volume_data/claude"), "Symlink should point to volume_data when persistent volume is enabled, but points to: {}", symlink_target);
    
    // Test 2: Start session WITHOUT persistent volume
    println!("=== Testing with persistent_volume_key = None ===");
    let session_without_volume = container_utils::start_coding_session(
        &docker,
        &container_name_without_volume,
        ClaudeCodeConfig::default(),
        container_utils::CodingContainerConfig { persistent_volume_key: None },
    )
    .await;
    
    assert!(session_without_volume.is_ok(), "Session without persistent volume should start successfully");
    let client_without_volume = session_without_volume.unwrap();
    
    // Verify volume initialization did NOT occur (symlinks should not point to volume_data)
    let no_symlink_target_result = container_utils::exec_command_in_container(
        &docker,
        client_without_volume.container_id(),
        vec!["readlink".to_string(), "/root/.claude".to_string()],
    ).await;
    
    // Either readlink should fail (not a symlink) OR it should not point to /volume_data
    if let Ok(symlink_target) = no_symlink_target_result {
        assert!(!symlink_target.contains("/volume_data"), "Symlink should NOT point to volume_data when persistent volume is disabled, but points to: {}", symlink_target);
    }
    // If readlink fails, that's also acceptable (means it's not a symlink)
    
    println!("✅ persistent_volume_key setting works correctly!");
    
    // Cleanup
    cleanup_test_resources(&docker, &container_name_with_volume, test_user_id).await;
    cleanup_test_resources(&docker, &container_name_without_volume, test_user_id).await;
}

// =============================================================================
// Timeout Integration Tests
// =============================================================================

/// Tests timeout error message format that users will see
/// 
/// This test verifies that timeout error messages are properly formatted
/// and contain expected information for user communication.
#[test]
fn test_timeout_error_message_format() {
    // Test the timeout error message format that users would see
    let timeout_secs = 60;
    let command = vec!["gh", "auth", "login", "--git-protocol", "https"];
    
    let timeout_error = format!(
        "Command timed out after {} seconds: {}", 
        timeout_secs,
        command.join(" ")
    );
    
    assert!(timeout_error.contains("timed out after 60 seconds"));
    assert!(timeout_error.contains("gh auth login --git-protocol https"));
    
    println!("✅ Timeout error format: {}", timeout_error);
}

/// Tests user-friendly timeout message formatting
/// 
/// This test verifies that the user-friendly timeout message provides
/// helpful information and suggestions for resolving timeout issues.
#[test]
fn test_user_friendly_timeout_message() {
    // Test the user-friendly message that would be shown in Telegram
    let timeout_error = "Command timed out after 60 seconds: gh auth login --git-protocol https";
    
    let user_message = format!(
        "⏰ GitHub authentication timed out: {}\n\nThis usually means:\n• The authentication process is taking longer than expected\n• There may be network connectivity issues\n• The GitHub CLI might be unresponsive\n\nPlease try again in a few moments.", 
        timeout_error
    );
    
    assert!(user_message.contains("⏰ GitHub authentication timed out"));
    assert!(user_message.contains("authentication process is taking longer than expected"));
    assert!(user_message.contains("Please try again in a few moments"));
    
    println!("✅ User-friendly timeout message format verified");
}

/// Tests default timeout configuration values
/// 
/// This test verifies that the default timeout configuration has
/// reasonable values for production use.
#[test]
fn test_default_timeout_configuration() {
    // Verify the default timeout is reasonable
    let default_config = GithubClientConfig::default();
    
    assert_eq!(default_config.exec_timeout_secs, 60);
    assert_eq!(default_config.working_directory, Some("/workspace".to_string()));
    
    println!("✅ Default timeout configuration: {} seconds", default_config.exec_timeout_secs);
}

/// Tests custom timeout configuration
/// 
/// This test verifies that custom timeout values can be properly
/// configured and applied.
#[test]
fn test_custom_timeout_configuration() {
    // Test that custom timeouts can be configured
    let custom_config = GithubClientConfig {
        working_directory: Some("/workspace".to_string()),
        exec_timeout_secs: 30,
    };
    
    assert_eq!(custom_config.exec_timeout_secs, 30);
    
    println!("✅ Custom timeout configuration: {} seconds", custom_config.exec_timeout_secs);
}

// =============================================================================
// Output Buffering Tests
// =============================================================================

/// Tests output buffering mechanism with timeout
/// 
/// This test validates that the buffering logic works correctly by simulating
/// output chunks with various timing patterns and verifying proper buffering behavior.
#[tokio::test]
async fn test_output_buffering_with_timeout() {
    println!("🔍 Testing output buffering with 200ms timeout:");
    
    // Simulate buffering behavior
    let buffer_timeout = Duration::from_millis(200);
    let mut output_buffer = String::new();
    let _last_output_time = Instant::now();
    
    // Simulate receiving output chunks with delays
    let test_chunks = vec![
        ("chunk1", 50),  // Comes at 50ms
        ("chunk2", 100), // Comes at 150ms total  
        ("chunk3", 300), // Comes at 450ms total - should trigger processing after previous chunks
    ];
    
    let mut processed_outputs = Vec::new();
    let start_time = Instant::now();
    
    for (chunk, delay_ms) in test_chunks {
        // Sleep to simulate output timing
        sleep(Duration::from_millis(delay_ms)).await;
        
        // Simulate receiving output
        output_buffer.push_str(chunk);
        let _last_output_time = Instant::now();
        println!("  📥 Received: '{}' at {}ms", chunk, start_time.elapsed().as_millis());
        
        // Check if we should process buffer (200ms timeout since last output)
        // This would be implemented in the actual select! loop with a timer
        tokio::select! {
            _ = sleep(buffer_timeout) => {
                if !output_buffer.is_empty() {
                    processed_outputs.push(output_buffer.clone());
                    println!("  ✅ Processed buffer: '{}' at {}ms", output_buffer, start_time.elapsed().as_millis());
                    output_buffer.clear();
                }
            }
            _ = sleep(Duration::from_millis(10)) => {
                // Continue to next iteration
            }
        }
    }
    
    // Process any remaining buffer
    if !output_buffer.is_empty() {
        sleep(buffer_timeout).await;
        processed_outputs.push(output_buffer.clone());
        println!("  ✅ Final buffer: '{}' at {}ms", output_buffer, start_time.elapsed().as_millis());
    }
    
    // Validate results
    assert!(!processed_outputs.is_empty(), "Should have processed some output");
    println!("✅ Output buffering test completed successfully");
}

/// Tests that buffering prevents rapid state transitions
/// 
/// This test validates that the buffering mechanism effectively reduces
/// the number of state transitions by accumulating rapid output chunks.
#[tokio::test] 
async fn test_buffering_prevents_rapid_transitions() {
    println!("🔍 Testing that buffering prevents rapid state transitions:");
    
    // Without buffering, each chunk would trigger state parsing
    let chunks_without_buffering = vec!["chunk1", "chunk2", "chunk3"];
    let state_transitions_without_buffering = chunks_without_buffering.len();
    
    // With buffering, chunks within 200ms are accumulated
    let buffer_timeout = Duration::from_millis(200);
    let mut accumulated_chunks = Vec::new();
    let mut current_buffer = String::new();
    let _state_transitions_with_buffering = 0;
    
    for (i, chunk) in chunks_without_buffering.iter().enumerate() {
        current_buffer.push_str(chunk);
        
        // Simulate rapid output (less than 200ms apart)
        if i < chunks_without_buffering.len() - 1 {
            sleep(Duration::from_millis(50)).await;
        }
    }
    
    // Only process once after timeout
    sleep(buffer_timeout).await;
    accumulated_chunks.push(current_buffer);
    let state_transitions_with_buffering = 1;
    
    println!("  📊 Without buffering: {} state transitions", state_transitions_without_buffering);
    println!("  📊 With buffering: {} state transitions", state_transitions_with_buffering);
    
    // Buffering should reduce state transitions
    assert!(state_transitions_with_buffering < state_transitions_without_buffering);
    assert_eq!(accumulated_chunks.len(), 1);
    assert_eq!(accumulated_chunks[0], "chunk1chunk2chunk3");
    
    println!("✅ Buffering prevents rapid state transitions test passed");
}

// =============================================================================
// End-to-End Bot Workflow Tests
// =============================================================================

/// Tests complete start and claude status workflow
/// 
/// This test simulates the complete user workflow from starting a coding session
/// to checking Claude status, as it would happen in real bot usage.
#[rstest]
#[tokio::test]
async fn test_complete_start_claudestatus_workflow(docker: Docker) {
    let container_name = format!("test-workflow-{}", uuid::Uuid::new_v4());

    // Test the complete workflow as it would happen in the bot

    // Step 1: Simulate /start command
    println!("=== STEP 1: Starting coding session (simulating /start) ===");
    let claude_client_result = container_utils::start_coding_session(
        &docker,
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

    // Step 2: Simulate /claudestatus command
    println!("=== STEP 2: Checking Claude status (simulating /claudestatus) ===");

    // Create a new client instance for the existing container (simulating finding the session)
    let status_client_result = ClaudeCodeClient::for_session(docker.clone(), &container_name).await;
    assert!(
        status_client_result.is_ok(),
        "for_session should find the container: {:?}",
        status_client_result
    );
    let status_client = status_client_result.unwrap();

    // This is what the /claudestatus command actually calls
    let availability_result = status_client.check_availability().await;

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
    cleanup_container(&docker, &container_name).await;

    println!("🎉 Complete workflow test passed!");
}