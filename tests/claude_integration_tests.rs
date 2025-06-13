use bollard::Docker;
use rstest::*;
use telegram_bot::{ClaudeCodeClient, ClaudeCodeConfig, container_utils};

/// Test fixture that provides a Docker client
#[fixture]
pub fn docker() -> Docker {
    Docker::connect_with_local_defaults().expect("Failed to connect to Docker")
}

/// Test fixture that creates a test container and cleans it up
#[fixture]
pub async fn test_container(docker: Docker) -> (Docker, String, String) {
    let container_name = format!("test-claude-integration-{}", uuid::Uuid::new_v4());
    let container_id = container_utils::create_test_container(&docker, &container_name)
        .await
        .expect("Failed to create test container");
    
    (docker, container_id, container_name)
}

/// Cleanup fixture that ensures test containers are removed
pub async fn cleanup_container(docker: &Docker, container_name: &str) {
    let _ = container_utils::clear_coding_session(docker, container_name).await;
}

#[rstest]
#[tokio::test]
async fn test_container_launch_and_connectivity(
    #[future] test_container: (Docker, String, String)
) {
    let (docker, container_id, container_name) = test_container.await;
    
    // Test basic connectivity
    let client = ClaudeCodeClient::new(docker.clone(), container_id, ClaudeCodeConfig::default());
    
    // Try to execute a simple command to verify container is working
    let result = client.exec_basic_command(vec!["echo".to_string(), "Hello World".to_string()]).await;
    
    // Cleanup
    cleanup_container(&docker, &container_name).await;
    
    assert!(result.is_ok(), "Container connectivity test failed: {:?}", result);
    assert_eq!(result.unwrap().trim(), "Hello World");
}

#[rstest]
#[tokio::test]
async fn test_claude_code_installation(
    #[future] test_container: (Docker, String, String)
) {
    let (docker, container_id, container_name) = test_container.await;
    
    let client = ClaudeCodeClient::new(docker.clone(), container_id, ClaudeCodeConfig::default());
    
    // Test Claude Code installation
    let install_result = client.install().await;
    
    // Cleanup
    cleanup_container(&docker, &container_name).await;
    
    assert!(install_result.is_ok(), "Claude Code installation failed: {:?}", install_result);
}

#[rstest]
#[tokio::test]
async fn test_claude_availability_check(
    #[future] test_container: (Docker, String, String)
) {
    let (docker, container_id, container_name) = test_container.await;
    
    let client = ClaudeCodeClient::new(docker.clone(), container_id, ClaudeCodeConfig::default());
    
    // Install Claude Code first
    client.install().await.expect("Failed to install Claude Code");
    
    // Test Claude availability check
    let availability_result = client.check_availability().await;
    
    // Cleanup
    cleanup_container(&docker, &container_name).await;
    
    assert!(availability_result.is_ok(), "Claude availability check failed: {:?}", availability_result);
    let version_output = availability_result.unwrap();
    // Should contain version information or help text
    assert!(!version_output.is_empty(), "Version output should not be empty");
}

#[rstest]
#[tokio::test]
async fn test_claude_cli_basic_invocation_and_binary_presence(
    #[future] test_container: (Docker, String, String)
) {
    let (docker, container_id, container_name) = test_container.await;
    
    let client = ClaudeCodeClient::new(docker.clone(), container_id, ClaudeCodeConfig::default());
    
    // Install Claude Code first
    client.install().await.expect("Failed to install Claude Code");
    
    // Debug: Check what's in the PATH and npm global bin
    let npm_bin_result = client.exec_basic_command(vec!["npm".to_string(), "bin".to_string(), "-g".to_string()]).await;
    println!("npm global bin directory: {:?}", npm_bin_result);
    
    let path_result = client.exec_basic_command(vec!["echo".to_string(), "$PATH".to_string()]).await;
    println!("Current PATH: {:?}", path_result);
    
    // Test that claude binary is present and reachable via PATH
    let which_result = client.exec_basic_command(vec!["which".to_string(), "claude".to_string()]).await;
    
    // If which fails, try to find claude in common npm locations
    let claude_path = if which_result.is_ok() && !which_result.as_ref().unwrap().is_empty() {
        which_result.unwrap()
    } else {
        println!("which claude failed: {:?}", which_result);
        // Try common npm global bin locations
        let npm_global_result = client.exec_basic_command(vec!["ls".to_string(), "-la".to_string(), "/usr/local/bin/claude".to_string()]).await;
        if npm_global_result.is_ok() {
            "/usr/local/bin/claude".to_string()
        } else {
            // Try node_modules location with a simpler approach
            let node_modules_result = client.exec_basic_command(vec!["sh".to_string(), "-c".to_string(), "find / -name claude -type f -executable 2>/dev/null | head -1".to_string()]).await;
            if node_modules_result.is_ok() && !node_modules_result.as_ref().unwrap().is_empty() {
                node_modules_result.unwrap()
            } else {
                println!("Could not find claude binary anywhere");
                // Check if the npm package was actually installed
                let npm_list_result = client.exec_basic_command(vec!["npm".to_string(), "list".to_string(), "-g".to_string(), "@anthropic-ai/claude-code".to_string()]).await;
                println!("npm list result: {:?}", npm_list_result);
                
                String::new() // Return empty string to trigger proper failure message
            }
        }
    };
    
    assert!(!claude_path.is_empty(), "claude binary path should not be empty");
    assert!(claude_path.contains("claude"), "Path should contain 'claude': {}", claude_path);
    
    // Test that the claude binary is executable (only if we found a path)
    if !claude_path.is_empty() {
        let test_exec_result = client.exec_basic_command(vec!["test".to_string(), "-x".to_string(), claude_path.trim().to_string()]).await;
        assert!(test_exec_result.is_ok(), "claude binary is not executable: {:?}", test_exec_result);
        
        // Test basic Claude CLI invocation (help command)
        let help_result = client.exec_basic_command(vec!["claude".to_string(), "--help".to_string()]).await;
        println!("Claude help output: '{}'", help_result.as_ref().unwrap_or(&"failed".to_string()));
        
        // The help command might fail if Claude requires authentication, so we just verify it produces some output
        // or fails with an expected authentication error
        assert!(help_result.is_ok() || 
                help_result.as_ref().err().unwrap().to_string().contains("auth") ||
                help_result.as_ref().err().unwrap().to_string().contains("login"),
                "Claude CLI basic invocation failed unexpectedly: {:?}", help_result);
    }
    
    // Cleanup
    cleanup_container(&docker, &container_name).await;
}