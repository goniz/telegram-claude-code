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
async fn test_claude_json_debug(docker: Docker) {
    let test_user_id = 666666; // Test user ID
    let container_name = format!("test-claude-debug-{}", Uuid::new_v4());
    
    // Step 0: Clean up any existing volume first
    println!("=== STEP 0: Cleaning up any existing volume ===");
    cleanup_test_resources(&docker, &container_name, test_user_id).await;
    
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
    
    // Step 2: Debug the file system state
    println!("=== STEP 2: Debugging file system state ===");
    
    // Check what exists in /volume_data
    let volume_ls = container_utils::exec_command_in_container(
        &docker,
        client.container_id(),
        vec!["ls".to_string(), "-la".to_string(), "/volume_data/".to_string()],
    ).await;
    println!("Volume data directory: {:?}", volume_ls);
    
    // Check what exists in /root
    let root_ls = container_utils::exec_command_in_container(
        &docker,
        client.container_id(),
        vec!["ls".to_string(), "-la".to_string(), "/root/".to_string()],
    ).await;
    println!("Root directory: {:?}", root_ls);
    
    // Check if .claude.json exists in volume data
    let volume_claude_check = container_utils::exec_command_in_container(
        &docker,
        client.container_id(),
        vec!["test".to_string(), "-f".to_string(), "/volume_data/claude.json".to_string()],
    ).await;
    println!("Volume claude.json test result: {:?}", volume_claude_check);
    println!("Volume claude.json exists: {:?}", volume_claude_check.is_ok());
    
    // If it exists, show its content
    if volume_claude_check.is_ok() {
        let content = container_utils::exec_command_in_container(
            &docker,
            client.container_id(),
            vec!["cat".to_string(), "/volume_data/claude.json".to_string()],
        ).await;
        println!("Volume claude.json content: {:?}", content);
    }
    
    // Check if symlink exists and where it points
    let symlink_check = container_utils::exec_command_in_container(
        &docker,
        client.container_id(),
        vec!["ls".to_string(), "-la".to_string(), "/root/.claude.json".to_string()],
    ).await;
    println!("Symlink status: {:?}", symlink_check);
    
    // Try to manually create the file and see what happens
    let manual_create = container_utils::exec_command_in_container(
        &docker,
        client.container_id(),
        vec!["sh".to_string(), "-c".to_string(), "echo '{ \"test\": true }' > /volume_data/test.json".to_string()],
    ).await;
    println!("Manual file creation: {:?}", manual_create);
    
    // Check if manual file was created
    let manual_check = container_utils::exec_command_in_container(
        &docker,
        client.container_id(),
        vec!["cat".to_string(), "/volume_data/test.json".to_string()],
    ).await;
    println!("Manual file content: {:?}", manual_check);
    
    // Cleanup
    cleanup_test_resources(&docker, &container_name, test_user_id).await;
}