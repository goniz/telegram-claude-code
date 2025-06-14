use bollard::Docker;
use rstest::*;
use telegram_bot::{ClaudeCodeClient, ClaudeCodeConfig, container_utils};
use std::env;

/// Test fixture that provides a Docker client
#[fixture]
pub fn docker() -> Docker {
    Docker::connect_with_local_defaults().expect("Failed to connect to Docker")
}

/// Cleanup fixture that ensures test containers are removed
pub async fn cleanup_container(docker: &Docker, container_name: &str) {
    let _ = container_utils::clear_coding_session(docker, container_name).await;
}

#[rstest]
#[tokio::test]
async fn test_container_health_validation_during_auth(docker: Docker) {
    let is_ci = env::var("CI").is_ok() || env::var("GITHUB_ACTIONS").is_ok();
    if is_ci {
        // Skip this test in CI due to container image pull issues
        println!("🔄 Skipping container health validation test in CI environment");
        return;
    }

    let container_name = format!("test-health-{}", uuid::Uuid::new_v4());
    
    // Test the container health validation workflow
    let test_timeout = tokio::time::Duration::from_secs(120);
    
    let test_result = tokio::time::timeout(test_timeout, async {
        // Step 1: Start a coding session
        println!("=== STEP 1: Starting coding session ===");
        let claude_client = container_utils::start_coding_session(&docker, &container_name, ClaudeCodeConfig::default()).await?;
        
        // Step 2: Validate container health
        println!("=== STEP 2: Validating container health ===");
        claude_client.validate_container_health().await?;
        println!("✅ Container health validation passed");
        
        // Step 3: Test that for_session properly validates health
        println!("=== STEP 3: Testing for_session health validation ===");
        let session_client = ClaudeCodeClient::for_session(docker.clone(), &container_name).await?;
        println!("✅ for_session with health validation passed");
        
        // Step 4: Test container connectivity
        println!("=== STEP 4: Testing container connectivity ===");
        let result = session_client.exec_basic_command(vec!["echo".to_string(), "test".to_string()]).await?;
        assert_eq!(result.trim(), "test");
        println!("✅ Container connectivity test passed");

        Ok::<(), Box<dyn std::error::Error + Send + Sync>>(())
    }).await;
    
    // Cleanup regardless of test outcome
    cleanup_container(&docker, &container_name).await;
    
    match test_result {
        Ok(Ok(())) => {
            println!("✅ Container health validation test completed successfully");
        }
        Ok(Err(e)) => {
            panic!("Container health validation test failed: {:?}", e);
        }
        Err(_) => {
            panic!("Container health validation test timed out");
        }
    }
}

#[rstest]
#[tokio::test]
async fn test_container_error_detection(docker: Docker) {
    let _is_ci = env::var("CI").is_ok() || env::var("GITHUB_ACTIONS").is_ok();
    
    let container_name = format!("test-error-detection-{}", uuid::Uuid::new_v4());
    
    // Test error detection for non-existent container
    println!("=== Testing error detection for non-existent container ===");
    
    let result = ClaudeCodeClient::for_session(docker.clone(), &container_name).await;
    assert!(result.is_err(), "for_session should fail for non-existent container");
    
    let error = result.unwrap_err();
    let error_msg = error.to_string();
    assert!(
        error_msg.contains("Container not found") || error_msg.contains("not found"),
        "Error should indicate container not found: {}", error_msg
    );
    
    println!("✅ Container error detection test passed");
}

#[rstest]
#[tokio::test]
async fn test_retry_logic_simulation() {
    // Test that retry logic handles various error conditions appropriately
    println!("=== Testing retry logic error pattern recognition ===");
    
    // Test error patterns that should trigger retries
    let retryable_errors = vec![
        "409 Conflict",
        "timeout occurred",
        "network error",
        "temporary failure",
    ];
    
    let non_retryable_errors = vec![
        "image not found",
        "permission denied",
        "invalid parameter",
    ];
    
    for error in retryable_errors {
        let is_retryable = error.to_lowercase().contains("409") ||
                          error.to_lowercase().contains("timeout") ||
                          error.to_lowercase().contains("network") ||
                          error.to_lowercase().contains("temporary");
        assert!(is_retryable, "Error '{}' should be retryable", error);
    }
    
    for error in non_retryable_errors {
        let is_retryable = error.to_lowercase().contains("409") ||
                          error.to_lowercase().contains("timeout") ||
                          error.to_lowercase().contains("network") ||
                          error.to_lowercase().contains("temporary");
        assert!(!is_retryable, "Error '{}' should not be retryable", error);
    }
    
    println!("✅ Retry logic pattern recognition test passed");
}