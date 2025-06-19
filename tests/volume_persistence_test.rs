use bollard::Docker;
use rstest::*;
use telegram_bot::{container_utils, ClaudeCodeConfig};
use uuid::Uuid;

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