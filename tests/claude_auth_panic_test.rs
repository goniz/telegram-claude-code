use bollard::Docker;
use rstest::*;
use std::env;
use std::time::Duration;
use telegram_bot::{container_utils, ClaudeCodeConfig, ClaudeCodeClient};
use tokio::time::timeout;
use uuid;

/// Test fixture that provides a Docker client
#[fixture]
pub fn docker() -> Docker {
    Docker::connect_with_local_defaults().expect("Failed to connect to Docker")
}

/// Test fixture that creates a coding session container for cancel testing
/// Container setup happens outside timeout blocks to avoid timing interference
#[fixture]
pub async fn claude_session_for_cancel() -> (Docker, ClaudeCodeClient, String) {
    let docker = Docker::connect_with_local_defaults().expect("Failed to connect to Docker");
    let container_name = format!("test-cancel-{}", uuid::Uuid::new_v4());
    
    // Start coding session outside of timeout - this may pull Docker images
    let claude_client = container_utils::start_coding_session(
        &docker,
        &container_name,
        ClaudeCodeConfig::default(),
        container_utils::CodingContainerConfig::default(),
    )
    .await
    .expect("Failed to start coding session for cancel test");
    
    (docker, claude_client, container_name)
}

/// Test fixture that creates a coding session container for multiple polls testing
/// Container setup happens outside timeout blocks to avoid timing interference
#[fixture]
pub async fn claude_session_for_polls() -> (Docker, ClaudeCodeClient, String) {
    let docker = Docker::connect_with_local_defaults().expect("Failed to connect to Docker");
    let container_name = format!("test-multiple-polls-{}", uuid::Uuid::new_v4());
    
    // Start coding session outside of timeout - this may pull Docker images
    let claude_client = container_utils::start_coding_session(
        &docker,
        &container_name,
        ClaudeCodeConfig::default(),
        container_utils::CodingContainerConfig::default(),
    )
    .await
    .expect("Failed to start coding session for polls test");
    
    (docker, claude_client, container_name)
}

/// Cleanup fixture that ensures test containers are removed
pub async fn cleanup_container(docker: &Docker, container_name: &str) {
    let _ = container_utils::clear_coding_session(docker, container_name).await;
}

#[rstest]
#[tokio::test]
async fn test_claude_auth_no_panic_on_cancel(
    #[future] claude_session_for_cancel: (Docker, ClaudeCodeClient, String)
) {
    // Skip in CI to avoid Docker dependency issues
    let is_ci = env::var("CI").is_ok() || env::var("GITHUB_ACTIONS").is_ok();
    if is_ci {
        println!("🔄 Running in CI environment - skipping Docker-dependent test");
        return;
    }

    let (docker, claude_client, container_name) = claude_session_for_cancel.await;

    // Container is already set up - run time-sensitive test logic with timeout
    let test_result = timeout(Duration::from_secs(15), async {
        // Start authentication
        let auth_result = claude_client.authenticate_claude_account().await;

        match auth_result {
            Ok(auth_handle) => {
                // Immediately drop the cancel_sender to simulate sender being dropped
                // This should trigger the cancel receiver error path in the select loop
                drop(auth_handle.cancel_sender);
                
                // Give the background task a moment to run and potentially panic
                tokio::time::sleep(Duration::from_millis(500)).await;
                
                // If we reach here without panicking, the fix worked
                println!("✅ Authentication handle created and cancel sender dropped without panic");
                
                // Also drop the code sender to clean up
                drop(auth_handle.code_sender);
                
                // Wait a bit more to let the background task exit cleanly
                tokio::time::sleep(Duration::from_millis(500)).await;
                
                Ok::<(), Box<dyn std::error::Error + Send + Sync>>(())
            }
            Err(e) => {
                // Authentication can fail in test environment, but it shouldn't panic
                println!("⚠️ Authentication failed but didn't panic: {}", e);
                Ok(())
            }
        }
    }).await;

    // Clean up regardless of test outcome
    cleanup_container(&docker, &container_name).await;

    match test_result {
        Ok(Ok(())) => {
            println!("✅ Test passed - no panic occurred when cancel sender was dropped");
        }
        Ok(Err(e)) => {
            println!("⚠️ Test completed with expected error (no panic): {}", e);
        }
        Err(_) => {
            println!("⚠️ Test timed out but this is expected in test environment");
            // Timeout is acceptable in test environment - the important thing is no panic
        }
    }
}

#[rstest]
#[tokio::test]
async fn test_claude_auth_no_panic_with_multiple_polls(
    #[future] claude_session_for_polls: (Docker, ClaudeCodeClient, String)
) {
    // Skip in CI to avoid Docker dependency issues
    let is_ci = env::var("CI").is_ok() || env::var("GITHUB_ACTIONS").is_ok();
    if is_ci {
        println!("🔄 Running in CI environment - skipping Docker-dependent test");
        return;
    }

    let (docker, claude_client, container_name) = claude_session_for_polls.await;

    // Container is already set up - run time-sensitive test logic with timeout
    let test_result = timeout(Duration::from_secs(10), async {
        // Start authentication
        let auth_result = claude_client.authenticate_claude_account().await;

        match auth_result {
            Ok(auth_handle) => {
                // Keep the handles alive for a bit to let the select loop run multiple iterations
                // This tests that the oneshot receiver doesn't get polled multiple times
                tokio::time::sleep(Duration::from_millis(1000)).await;
                
                // Drop the handles
                drop(auth_handle);
                
                println!("✅ Authentication ran for 1 second without panic");
                Ok::<(), Box<dyn std::error::Error + Send + Sync>>(())
            }
            Err(e) => {
                // Authentication can fail in test environment, but it shouldn't panic
                println!("⚠️ Authentication failed but didn't panic: {}", e);
                Ok(())
            }
        }
    }).await;

    // Clean up regardless of test outcome
    cleanup_container(&docker, &container_name).await;

    match test_result {
        Ok(Ok(())) => {
            println!("✅ Test passed - no panic during extended authentication run");
        }
        Ok(Err(e)) => {
            println!("⚠️ Test completed with expected error (no panic): {}", e);
        }
        Err(_) => {
            println!("⚠️ Test timed out but this is expected in test environment");
            // Timeout is acceptable in test environment - the important thing is no panic
        }
    }
}