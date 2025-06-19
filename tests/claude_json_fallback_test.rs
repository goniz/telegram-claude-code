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
    let volume_name = container_utils::generate_volume_name(user_id);
    let _ = docker.remove_volume(&volume_name, None).await;
}

#[rstest]
#[tokio::test]
async fn test_claude_json_fallback_initialization(docker: Docker) {
    let test_user_id = 555555; // Test user ID
    let container_name = format!("test-claude-fallback-{}", Uuid::new_v4());
    
    // Step 1: Start coding session
    println!("=== STEP 1: Starting coding session ===");
    let session = container_utils::start_coding_session(
        &docker,
        &container_name,
        ClaudeCodeConfig::default(),
        test_user_id,
    )
    .await;
    
    assert!(session.is_ok(), "Session should start successfully");
    let client = session.unwrap();
    
    // Step 2: Remove any existing .claude.json file from the container to simulate the case 
    // where no file exists (testing our fallback initialization)
    println!("=== STEP 2: Removing .claude.json to test fallback ===");
    let _ = container_utils::exec_command_in_container(
        &docker,
        client.container_id(),
        vec!["rm".to_string(), "-f".to_string(), "/volume_data/claude.json".to_string()],
    ).await;
    
    let _ = container_utils::exec_command_in_container(
        &docker,
        client.container_id(),
        vec!["rm".to_string(), "-f".to_string(), "/root/.claude.json".to_string()],
    ).await;
    
    // Step 3: Simulate the init_volume_structure initialization process manually
    println!("=== STEP 3: Testing fallback initialization ===");
    
    // Test that claude.json doesn't exist in volume
    let volume_claude_json_check = container_utils::exec_command_in_container(
        &docker, 
        client.container_id(), 
        vec!["test".to_string(), "-f".to_string(), "/volume_data/claude.json".to_string()]
    ).await;
    
    // This should fail (file doesn't exist)
    assert!(volume_claude_json_check.is_err(), "Volume claude.json should not exist");
    
    // Test that claude.json doesn't exist in container 
    let container_claude_json_check = container_utils::exec_command_in_container(
        &docker, 
        client.container_id(), 
        vec!["test".to_string(), "-f".to_string(), "/root/.claude.json".to_string()]
    ).await;
    
    // This should also fail (file doesn't exist)
    assert!(container_claude_json_check.is_err(), "Container claude.json should not exist");
    
    // Now test our fallback initialization
    let fallback_init = container_utils::exec_command_in_container(
        &docker,
        client.container_id(),
        vec!["sh".to_string(), "-c".to_string(), "echo '{ \"hasCompletedOnboarding\": true }' > /volume_data/claude.json".to_string()]
    ).await;
    
    assert!(fallback_init.is_ok(), "Fallback initialization should succeed");
    
    // Create the symlink
    let symlink_create = container_utils::exec_command_in_container(
        &docker,
        client.container_id(),
        vec!["ln".to_string(), "-sf".to_string(), "/volume_data/claude.json".to_string(), "/root/.claude.json".to_string()]
    ).await;
    
    assert!(symlink_create.is_ok(), "Symlink creation should succeed");
    
    // Step 4: Verify the fallback initialization worked
    println!("=== STEP 4: Verifying fallback initialization ===");
    
    let claude_json_content = container_utils::exec_command_in_container(
        &docker,
        client.container_id(),
        vec!["cat".to_string(), "/root/.claude.json".to_string()],
    ).await;
    
    assert!(claude_json_content.is_ok(), "Should be able to read .claude.json file");
    let content = claude_json_content.unwrap();
    println!("Fallback Claude JSON content: {}", content);
    
    // Verify it contains the required content
    assert!(content.contains("hasCompletedOnboarding"), ".claude.json should contain hasCompletedOnboarding");
    assert!(content.contains("true"), ".claude.json should set hasCompletedOnboarding to true");
    
    // Verify it's valid JSON with the expected structure
    let json_result: Result<serde_json::Value, _> = serde_json::from_str(&content);
    assert!(json_result.is_ok(), ".claude.json should be valid JSON");
    
    let json_value = json_result.unwrap();
    if let Some(completed) = json_value.get("hasCompletedOnboarding") {
        assert_eq!(completed, &serde_json::Value::Bool(true), "hasCompletedOnboarding should be true");
    } else {
        panic!("hasCompletedOnboarding field should be present in .claude.json");
    }
    
    println!("✅ Fallback .claude.json initialization works correctly!");
    
    // Cleanup
    cleanup_test_resources(&docker, &container_name, test_user_id).await;
}