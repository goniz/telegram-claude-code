/// Test to verify that volume data persists between container runs
/// This ensures that data stored in persistent volumes is preserved when containers are recreated
#[cfg(test)]
mod tests {
    use bollard::Docker;
    use rstest::*;
    use telegram_bot::{container_utils, ClaudeCodeConfig};
    use uuid::Uuid;

    /// Test fixture that provides a Docker client
    #[fixture]
    pub fn docker() -> Docker {
        Docker::connect_with_socket_defaults().expect("Failed to connect to Docker")
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
    async fn test_volume_data_persistence_between_sessions(docker: Docker) {
        let test_user_id = 888888; // Test user ID for config persistence
        let container_name_1 = format!("test-config-persistence-1-{}", Uuid::new_v4());
        let container_name_2 = format!("test-config-persistence-2-{}", Uuid::new_v4());
        
        // Clean up any existing volume before starting test
        let volume_name = container_utils::generate_volume_name(&test_user_id.to_string());
        let _ = docker.remove_volume(&volume_name, None).await;
        
        // Step 1: Start first coding session with persistent volume
        println!("=== STEP 1: Starting first coding session ===");
        let first_session = container_utils::start_coding_session(
            &docker,
            &container_name_1,
            ClaudeCodeConfig::default(),
            container_utils::CodingContainerConfig { 
                persistent_volume_key: Some(test_user_id.to_string()) 
            },
        )
        .await;
        
        assert!(first_session.is_ok(), "First session should start successfully");
        let first_client = first_session.unwrap();
        
        // Step 2: Create a test file in the persistent volume to verify persistence
        println!("=== STEP 2: Creating test file for persistence verification ===");
        let test_file_content = "persistence-test-content-12345";
        
        // Create a test file in the volume directory
        let create_file_result = first_client.exec_basic_command(vec![
            "sh".to_string(),
            "-c".to_string(),
            format!("echo '{}' > /volume_data/test-persistence-file.txt", test_file_content),
        ]).await;
        
        assert!(create_file_result.is_ok(), "Creating test file should succeed: {:?}", create_file_result);
        println!("Test file created successfully");
        
        // Step 3: Verify the file was created
        println!("=== STEP 3: Verifying test file was created ===");
        let read_file_result = first_client.exec_basic_command(vec![
            "sh".to_string(),
            "-c".to_string(),
            "cat /volume_data/test-persistence-file.txt".to_string(),
        ]).await;
        
        assert!(read_file_result.is_ok(), "Reading test file should succeed: {:?}", read_file_result);
        let file_content = read_file_result.unwrap();
        assert!(
            file_content.contains(test_file_content), 
            "File should contain test content. Expected: {}, Got: {}", 
            test_file_content, file_content
        );
        
        // Step 4: Stop the first session
        println!("=== STEP 4: Stopping first session ===");
        container_utils::clear_coding_session(&docker, &container_name_1).await
            .expect("Should clear session successfully");
        
        // Step 5: Start second coding session with same user ID (should reuse volume)
        println!("=== STEP 5: Starting second coding session with same user ===");
        let second_session = container_utils::start_coding_session(
            &docker,
            &container_name_2,
            ClaudeCodeConfig::default(),
            container_utils::CodingContainerConfig { 
                persistent_volume_key: Some(test_user_id.to_string()) 
            },
        )
        .await;
        
        assert!(second_session.is_ok(), "Second session should start successfully");
        let second_client = second_session.unwrap();
        
        // Step 6: Verify the test file persisted in the new session
        println!("=== STEP 6: Verifying test file persisted in new session ===");
        let read_file_result_2 = second_client.exec_basic_command(vec![
            "sh".to_string(),
            "-c".to_string(),
            "cat /volume_data/test-persistence-file.txt".to_string(),
        ]).await;
        
        // Cleanup
        cleanup_test_resources(&docker, &container_name_2, test_user_id).await;
        
        assert!(read_file_result_2.is_ok(), "Reading test file in second session should succeed: {:?}", read_file_result_2);
        let file_content_2 = read_file_result_2.unwrap();
        assert!(
            file_content_2.contains(test_file_content), 
            "File content should persist between sessions. Expected: {}, Got: {}", 
            test_file_content, 
            file_content_2
        );
        
        println!("✅ Volume data successfully persisted between sessions!");
    }

    #[rstest]
    #[tokio::test]
    async fn test_volume_data_isolation_between_users(docker: Docker) {
        let test_user_id_1 = 777777;
        let test_user_id_2 = 777778;
        let container_name_1 = format!("test-config-isolation-1-{}", Uuid::new_v4());
        let container_name_2 = format!("test-config-isolation-2-{}", Uuid::new_v4());
        
        // Clean up any existing volumes
        let volume_name_1 = container_utils::generate_volume_name(&test_user_id_1.to_string());
        let volume_name_2 = container_utils::generate_volume_name(&test_user_id_2.to_string());
        let _ = docker.remove_volume(&volume_name_1, None).await;
        let _ = docker.remove_volume(&volume_name_2, None).await;
        
        // Step 1: Start session for user 1
        println!("=== STEP 1: Starting session for user 1 ===");
        let session_1 = container_utils::start_coding_session(
            &docker,
            &container_name_1,
            ClaudeCodeConfig::default(),
            container_utils::CodingContainerConfig { 
                persistent_volume_key: Some(test_user_id_1.to_string()) 
            },
        )
        .await;
        
        assert!(session_1.is_ok(), "User 1 session should start successfully");
        let client_1 = session_1.unwrap();
        
        // Step 2: Start session for user 2
        println!("=== STEP 2: Starting session for user 2 ===");
        let session_2 = container_utils::start_coding_session(
            &docker,
            &container_name_2,
            ClaudeCodeConfig::default(),
            container_utils::CodingContainerConfig { 
                persistent_volume_key: Some(test_user_id_2.to_string()) 
            },
        )
        .await;
        
        assert!(session_2.is_ok(), "User 2 session should start successfully");
        let client_2 = session_2.unwrap();
        
        // Step 3: Create different test files for each user
        println!("=== STEP 3: Creating different test files for each user ===");
        let test_content_1 = "user1-test-content-67890";
        let test_content_2 = "user2-test-content-54321";
        
        let create_file_1 = client_1.exec_basic_command(vec![
            "sh".to_string(),
            "-c".to_string(),
            format!("echo '{}' > /volume_data/user-test-file.txt", test_content_1),
        ]).await;
        
        let create_file_2 = client_2.exec_basic_command(vec![
            "sh".to_string(),
            "-c".to_string(),
            format!("echo '{}' > /volume_data/user-test-file.txt", test_content_2),
        ]).await;
        
        assert!(create_file_1.is_ok(), "Creating test file for user 1 should succeed");
        assert!(create_file_2.is_ok(), "Creating test file for user 2 should succeed");
        
        // Step 4: Verify each user has their own isolated file content
        println!("=== STEP 4: Verifying file isolation ===");
        let read_file_1 = client_1.exec_basic_command(vec![
            "sh".to_string(),
            "-c".to_string(),
            "cat /volume_data/user-test-file.txt".to_string(),
        ]).await;
        
        let read_file_2 = client_2.exec_basic_command(vec![
            "sh".to_string(),
            "-c".to_string(),
            "cat /volume_data/user-test-file.txt".to_string(),
        ]).await;
        
        // Cleanup
        cleanup_test_resources(&docker, &container_name_1, test_user_id_1).await;
        cleanup_test_resources(&docker, &container_name_2, test_user_id_2).await;
        
        assert!(read_file_1.is_ok(), "Reading file for user 1 should succeed");
        assert!(read_file_2.is_ok(), "Reading file for user 2 should succeed");
        
        let content_1 = read_file_1.unwrap();
        let content_2 = read_file_2.unwrap();
        
        assert!(
            content_1.contains(test_content_1),
            "User 1 should have their own file content. Expected: {}, Got: {}", 
            test_content_1, content_1
        );
        assert!(
            content_2.contains(test_content_2),
            "User 2 should have their own file content. Expected: {}, Got: {}", 
            test_content_2, content_2
        );
        
        println!("✅ Volume data properly isolated between users!");
    }
}