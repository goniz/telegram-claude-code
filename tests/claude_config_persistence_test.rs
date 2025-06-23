/// Test to verify that Claude configuration persists between container runs
/// This ensures that config changes made by users are preserved when containers are recreated
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
    async fn test_claude_config_persistence_between_sessions(docker: Docker) {
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
        
        // Step 2: First check the initial value, then set a custom configuration value
        println!("=== STEP 2: Checking initial config and setting custom value ===");
        let config_key = "hasCompletedProjectOnboarding";
        
        // Check initial value
        let initial_config_result = first_client.exec_basic_command(vec![
            "sh".to_string(),
            "-c".to_string(),
            format!("/opt/entrypoint.sh -c \"nvm use default && claude config get {}\"", config_key),
        ]).await;
        
        assert!(initial_config_result.is_ok(), "Getting initial config should succeed: {:?}", initial_config_result);
        let initial_value = initial_config_result.unwrap();
        println!("Initial config value: {}", initial_value);
        
        // Set opposite value - if it's false, set true; if it's true, set false
        let config_value = if initial_value.trim().to_lowercase().contains("false") { "true" } else { "false" };
        println!("Setting config to: {}", config_value);
        
        let set_config_result = first_client.exec_basic_command(vec![
            "sh".to_string(),
            "-c".to_string(),
            format!("/opt/entrypoint.sh -c \"nvm use default && claude config set {} {}\"", config_key, config_value),
        ]).await;
        
        assert!(set_config_result.is_ok(), "Setting config should succeed: {:?}", set_config_result);
        println!("Config set result: {:?}", set_config_result.unwrap());
        
        // Step 3: Verify the configuration was set
        println!("=== STEP 3: Verifying configuration was set ===");
        let get_config_result = first_client.exec_basic_command(vec![
            "sh".to_string(),
            "-c".to_string(),
            format!("/opt/entrypoint.sh -c \"nvm use default && claude config get {}\"", config_key),
        ]).await;
        
        assert!(get_config_result.is_ok(), "Getting config should succeed: {:?}", get_config_result);
        let config_output = get_config_result.unwrap();
        assert!(
            config_output.contains(config_value), 
            "Config should contain the set value. Output: {}", 
            config_output
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
        
        // Step 6: Verify the configuration persisted
        println!("=== STEP 6: Verifying configuration persisted in new session ===");
        let get_config_result_2 = second_client.exec_basic_command(vec![
            "sh".to_string(),
            "-c".to_string(),
            format!("/opt/entrypoint.sh -c \"nvm use default && claude config get {}\"", config_key),
        ]).await;
        
        // Cleanup
        cleanup_test_resources(&docker, &container_name_2, test_user_id).await;
        
        assert!(get_config_result_2.is_ok(), "Getting config in second session should succeed: {:?}", get_config_result_2);
        let config_output_2 = get_config_result_2.unwrap();
        assert!(
            config_output_2.contains(config_value), 
            "Config should persist between sessions. Expected: {}, Got: {}", 
            config_value, 
            config_output_2
        );
        
        println!("✅ Configuration successfully persisted between sessions!");
    }

    #[rstest]
    #[tokio::test]
    async fn test_claude_config_isolation_between_users(docker: Docker) {
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
        
        // Step 3: Set different config for each user
        println!("=== STEP 3: Setting different configs for each user ===");
        let config_key = "hasCompletedProjectOnboarding";
        let config_value_1 = "true";
        let config_value_2 = "false";
        
        let set_config_1 = client_1.exec_basic_command(vec![
            "sh".to_string(),
            "-c".to_string(),
            format!("/opt/entrypoint.sh -c \"nvm use default && claude config set {} {}\"", config_key, config_value_1),
        ]).await;
        
        let set_config_2 = client_2.exec_basic_command(vec![
            "sh".to_string(),
            "-c".to_string(),
            format!("/opt/entrypoint.sh -c \"nvm use default && claude config set {} {}\"", config_key, config_value_2),
        ]).await;
        
        assert!(set_config_1.is_ok(), "Setting config for user 1 should succeed");
        assert!(set_config_2.is_ok(), "Setting config for user 2 should succeed");
        
        // Step 4: Verify each user has their own config
        println!("=== STEP 4: Verifying config isolation ===");
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
        
        // Cleanup
        cleanup_test_resources(&docker, &container_name_1, test_user_id_1).await;
        cleanup_test_resources(&docker, &container_name_2, test_user_id_2).await;
        
        assert!(get_config_1.is_ok(), "Getting config for user 1 should succeed");
        assert!(get_config_2.is_ok(), "Getting config for user 2 should succeed");
        
        let output_1 = get_config_1.unwrap();
        let output_2 = get_config_2.unwrap();
        
        assert!(
            output_1.contains(config_value_1),
            "User 1 should have their own config value. Got: {}", 
            output_1
        );
        assert!(
            output_2.contains(config_value_2),
            "User 2 should have their own config value. Got: {}", 
            output_2
        );
        
        println!("✅ Configuration properly isolated between users!");
    }
}