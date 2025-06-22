use bollard::Docker;
use rstest::*;
use telegram_bot::{container_utils, GithubClient, GithubClientConfig};
use uuid;

/// Test fixture that provides a Docker client
#[fixture]
pub fn docker() -> Docker {
    Docker::connect_with_socket_defaults().expect("Failed to connect to Docker")
}

/// Test fixture that creates a test container and cleans it up
#[fixture]
pub async fn test_container(docker: Docker) -> (Docker, String, String) {
    let container_name = format!("test-exec-allow-failure-{}", uuid::Uuid::new_v4());
    let container_id = container_utils::create_test_container(&docker, &container_name)
        .await
        .expect("Failed to create test container");

    (docker, container_id, container_name)
}

/// Cleanup fixture that ensures test containers are removed
pub async fn cleanup_container(docker: &Docker, container_name: &str) {
    let _ = container_utils::clear_coding_session(docker, container_name).await;
}

// =============================================================================
// DIRECT TESTS FOR exec_command_allow_failure METHOD
// =============================================================================

#[rstest]
#[tokio::test]
async fn test_exec_command_allow_failure_success(#[future] test_container: (Docker, String, String)) {
    let (docker, container_id, container_name) = test_container.await;

    let client = GithubClient::new(docker.clone(), container_id, GithubClientConfig::default());

    // Test successful command execution
    let result = client.exec_command_allow_failure(vec!["echo".to_string(), "hello".to_string()]).await;

    // Cleanup
    cleanup_container(&docker, &container_name).await;

    assert!(result.is_ok(), "Command should execute successfully: {:?}", result);
    let (output, success) = result.unwrap();
    
    assert!(success, "Command should be marked as successful");
    assert_eq!(output.trim(), "hello", "Output should match expected value");
}

#[rstest]
#[tokio::test]
async fn test_exec_command_allow_failure_command_failure(#[future] test_container: (Docker, String, String)) {
    let (docker, container_id, container_name) = test_container.await;

    let client = GithubClient::new(docker.clone(), container_id, GithubClientConfig::default());

    // Test failing command execution (exit code != 0)
    let result = client.exec_command_allow_failure(vec!["sh".to_string(), "-c".to_string(), "exit 1".to_string()]).await;

    // Cleanup
    cleanup_container(&docker, &container_name).await;

    assert!(result.is_ok(), "Method should return Ok even for failing commands: {:?}", result);
    let (output, success) = result.unwrap();
    
    assert!(!success, "Command should be marked as failed");
    // Output might be empty for simple exit commands
    println!("Failed command output: '{}'", output);
}

#[rstest]
#[tokio::test]
async fn test_exec_command_allow_failure_stderr_capture(#[future] test_container: (Docker, String, String)) {
    let (docker, container_id, container_name) = test_container.await;

    let client = GithubClient::new(docker.clone(), container_id, GithubClientConfig::default());

    // Test command that outputs to stderr
    let result = client.exec_command_allow_failure(vec![
        "sh".to_string(), 
        "-c".to_string(), 
        "echo 'error output' >&2".to_string()
    ]).await;

    // Cleanup
    cleanup_container(&docker, &container_name).await;

    assert!(result.is_ok(), "Command should execute successfully: {:?}", result);
    let (output, success) = result.unwrap();
    
    assert!(success, "Command should be marked as successful");
    assert!(output.contains("error output"), "Output should contain stderr content: '{}'", output);
}

#[rstest]
#[tokio::test]
async fn test_exec_command_allow_failure_mixed_output(#[future] test_container: (Docker, String, String)) {
    let (docker, container_id, container_name) = test_container.await;

    let client = GithubClient::new(docker.clone(), container_id, GithubClientConfig::default());

    // Test command that outputs to both stdout and stderr
    let result = client.exec_command_allow_failure(vec![
        "sh".to_string(), 
        "-c".to_string(), 
        "echo 'stdout line'; echo 'stderr line' >&2".to_string()
    ]).await;

    // Cleanup
    cleanup_container(&docker, &container_name).await;

    assert!(result.is_ok(), "Command should execute successfully: {:?}", result);
    let (output, success) = result.unwrap();
    
    assert!(success, "Command should be marked as successful");
    assert!(output.contains("stdout line"), "Output should contain stdout: '{}'", output);
    assert!(output.contains("stderr line"), "Output should contain stderr: '{}'", output);
}

#[rstest]
#[tokio::test]
async fn test_exec_command_allow_failure_nonexistent_command(#[future] test_container: (Docker, String, String)) {
    let (docker, container_id, container_name) = test_container.await;

    let client = GithubClient::new(docker.clone(), container_id, GithubClientConfig::default());

    // Test execution of nonexistent command
    let result = client.exec_command_allow_failure(vec!["nonexistent-command-12345".to_string()]).await;

    // Cleanup
    cleanup_container(&docker, &container_name).await;

    assert!(result.is_ok(), "Method should return Ok even for nonexistent commands: {:?}", result);
    let (output, success) = result.unwrap();
    
    assert!(!success, "Nonexistent command should be marked as failed");
    assert!(!output.is_empty(), "Output should contain error message about nonexistent command");
    println!("Nonexistent command output: '{}'", output);
}

#[rstest]
#[tokio::test]
async fn test_exec_command_allow_failure_working_directory(#[future] test_container: (Docker, String, String)) {
    let (docker, container_id, container_name) = test_container.await;

    // Test with custom working directory
    let mut config = GithubClientConfig::default();
    config.working_directory = Some("/tmp".to_string());
    let client = GithubClient::new(docker.clone(), container_id, config);

    // Test command that shows current working directory
    let result = client.exec_command_allow_failure(vec!["pwd".to_string()]).await;

    // Cleanup
    cleanup_container(&docker, &container_name).await;

    assert!(result.is_ok(), "Command should execute successfully: {:?}", result);
    let (output, success) = result.unwrap();
    
    assert!(success, "Command should be marked as successful");
    assert!(output.contains("/tmp"), "Command should run in specified working directory: '{}'", output);
}

#[rstest]
#[tokio::test]
async fn test_exec_command_allow_failure_timeout_behavior(#[future] test_container: (Docker, String, String)) {
    let (docker, container_id, container_name) = test_container.await;

    // Test with very short timeout to trigger timeout behavior
    let mut config = GithubClientConfig::default();
    config.exec_timeout_secs = 1; // 1 second timeout
    let client = GithubClient::new(docker.clone(), container_id, config);

    // Test command that takes longer than timeout
    let result = client.exec_command_allow_failure(vec![
        "sleep".to_string(), 
        "2".to_string()
    ]).await;

    // Cleanup
    cleanup_container(&docker, &container_name).await;

    // This should return an error due to timeout
    assert!(result.is_err(), "Long-running command should timeout and return error");
    let error_msg = result.unwrap_err().to_string();
    assert!(error_msg.contains("timed out"), "Error should mention timeout: '{}'", error_msg);
    assert!(error_msg.contains("sleep 2"), "Error should mention the command: '{}'", error_msg);
}

#[rstest]
#[tokio::test]
async fn test_exec_command_allow_failure_environment_variables(#[future] test_container: (Docker, String, String)) {
    let (docker, container_id, container_name) = test_container.await;

    let client = GithubClient::new(docker.clone(), container_id, GithubClientConfig::default());

    // Test that environment variables are set correctly (HOME and PATH)
    let result = client.exec_command_allow_failure(vec![
        "sh".to_string(), 
        "-c".to_string(), 
        "echo \"HOME=$HOME\" && echo \"PATH=$PATH\"".to_string()
    ]).await;

    // Cleanup
    cleanup_container(&docker, &container_name).await;

    assert!(result.is_ok(), "Command should execute successfully: {:?}", result);
    let (output, success) = result.unwrap();
    
    assert!(success, "Command should be marked as successful");
    assert!(output.contains("HOME=/root"), "HOME should be set to /root: '{}'", output);
    assert!(output.contains("PATH=") && output.contains("/usr/local/bin"), "PATH should contain standard paths: '{}'", output);
}

#[rstest]
#[tokio::test]
async fn test_exec_command_allow_failure_empty_command(#[future] test_container: (Docker, String, String)) {
    let (docker, container_id, container_name) = test_container.await;

    let client = GithubClient::new(docker.clone(), container_id, GithubClientConfig::default());

    // Test execution with empty command vector
    let result = client.exec_command_allow_failure(vec![]).await;

    // Cleanup
    cleanup_container(&docker, &container_name).await;

    // This should likely fail at the Docker API level
    // But we still want to ensure it returns a Result rather than panicking
    println!("Empty command result: {:?}", result);
    // We don't assert success/failure here since the behavior may vary
    // The important thing is that it doesn't panic
}