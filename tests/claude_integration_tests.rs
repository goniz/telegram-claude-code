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
    
    // Test that claude binary is present and reachable via PATH
    let which_result = client.exec_basic_command(vec!["which".to_string(), "claude".to_string()]).await;
    assert!(which_result.is_ok(), "claude binary not found in PATH: {:?}", which_result);
    let claude_path = which_result.unwrap();
    assert!(!claude_path.is_empty(), "claude binary path should not be empty");
    assert!(claude_path.contains("claude"), "Path should contain 'claude': {}", claude_path);
    
    // Test that the claude binary is executable
    let test_exec_result = client.exec_basic_command(vec!["test".to_string(), "-x".to_string(), claude_path.trim().to_string()]).await;
    assert!(test_exec_result.is_ok(), "claude binary is not executable: {:?}", test_exec_result);
    
    // Test basic Claude CLI invocation (help command)
    let help_result = client.exec_basic_command(vec!["claude".to_string(), "--help".to_string()]).await;
    
    // Cleanup
    cleanup_container(&docker, &container_name).await;
    
    assert!(help_result.is_ok(), "Claude CLI basic invocation failed: {:?}", help_result);
    let help_output = help_result.unwrap();
    println!("Claude help output: '{}'", help_output);
    
    // Help output should not be empty
    assert!(!help_output.is_empty(), "Help output should not be empty");
    
    // The output should either show help information OR indicate that Claude is available but needs auth
    // This validates that the CLI is properly installed and accessible
    let output_lower = help_output.to_lowercase();
    let is_valid_output = output_lower.contains("usage") || 
                         output_lower.contains("command") || 
                         output_lower.contains("help") ||
                         output_lower.contains("option") ||
                         output_lower.contains("claude") ||
                         output_lower.contains("auth") ||
                         output_lower.contains("login") ||
                         output_lower.contains("anthropic");
    
    assert!(is_valid_output, "Help output should contain CLI-related keywords or indicate Claude is available, got: '{}'", help_output);
}