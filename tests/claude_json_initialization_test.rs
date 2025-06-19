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
async fn test_claude_json_initialization_from_runtime(docker: Docker) {
    let test_user_id = 888888; // Test user ID
    let container_name = format!("test-claude-init-{}", Uuid::new_v4());
    
    // Step 1: Start coding session (this should initialize .claude.json)
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
    
    // Step 2: Verify that .claude.json was created with the correct content
    println!("=== STEP 2: Verifying .claude.json initialization ===");
    
    // Check the .claude.json file exists and has the correct content
    let claude_json_content = container_utils::exec_command_in_container(
        &docker,
        client.container_id(),
        vec!["cat".to_string(), "/root/.claude.json".to_string()],
    ).await;
    
    assert!(claude_json_content.is_ok(), "Should be able to read .claude.json file");
    let content = claude_json_content.unwrap();
    println!("Claude JSON content: {}", content);
    
    // Verify it contains the hasCompletedOnboarding flag
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
    
    println!("✅ .claude.json initialized correctly with required content!");
    
    // Cleanup
    cleanup_test_resources(&docker, &container_name, test_user_id).await;
}

#[rstest]
#[tokio::test]
async fn test_claude_json_persistence_across_sessions(docker: Docker) {
    let test_user_id = 777777; // Test user ID
    let container_name_1 = format!("test-claude-persist-1-{}", Uuid::new_v4());
    let container_name_2 = format!("test-claude-persist-2-{}", Uuid::new_v4());
    
    // Step 1: Start first coding session
    println!("=== STEP 1: Starting first coding session ===");
    let session_1 = container_utils::start_coding_session(
        &docker,
        &container_name_1,
        ClaudeCodeConfig::default(),
        test_user_id,
    )
    .await;
    
    assert!(session_1.is_ok(), "First session should start successfully");
    let client_1 = session_1.unwrap();
    
    // Step 2: Verify .claude.json was initialized correctly
    let claude_json_content_1 = container_utils::exec_command_in_container(
        &docker,
        client_1.container_id(),
        vec!["cat".to_string(), "/root/.claude.json".to_string()],
    ).await;
    
    assert!(claude_json_content_1.is_ok(), "Should be able to read .claude.json in first session");
    let content_1 = claude_json_content_1.unwrap();
    assert!(content_1.contains("hasCompletedOnboarding"), "First session should have correct .claude.json");
    
    // Step 3: Stop first session
    println!("=== STEP 3: Stopping first session ===");
    container_utils::clear_coding_session(&docker, &container_name_1).await
        .expect("Should clear first session successfully");
    
    // Step 4: Start second session with same user ID
    println!("=== STEP 4: Starting second session with same user ===");
    let session_2 = container_utils::start_coding_session(
        &docker,
        &container_name_2,
        ClaudeCodeConfig::default(),
        test_user_id, // Same user ID
    )
    .await;
    
    assert!(session_2.is_ok(), "Second session should start successfully");
    let client_2 = session_2.unwrap();
    
    // Step 5: Verify .claude.json persisted from first session
    println!("=== STEP 5: Verifying .claude.json persistence ===");
    let claude_json_content_2 = container_utils::exec_command_in_container(
        &docker,
        client_2.container_id(),
        vec!["cat".to_string(), "/root/.claude.json".to_string()],
    ).await;
    
    assert!(claude_json_content_2.is_ok(), "Should be able to read .claude.json in second session");
    let content_2 = claude_json_content_2.unwrap();
    assert!(content_2.contains("hasCompletedOnboarding"), "Second session should have persisted .claude.json");
    
    // Verify the content is the same (or at least has the required fields)
    assert_eq!(content_1.trim(), content_2.trim(), ".claude.json content should persist between sessions");
    
    println!("✅ .claude.json successfully persisted between sessions!");
    
    // Cleanup
    cleanup_test_resources(&docker, &container_name_2, test_user_id).await;
}