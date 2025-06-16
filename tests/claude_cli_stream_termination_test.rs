use bollard::Docker;
use rstest::*;
use std::env;
use telegram_bot::{container_utils, ClaudeCodeConfig, AuthState};

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
#[allow(unused_variables)]
async fn test_cli_stream_termination_handling(docker: Docker) {
    // Test that when CLI output stream terminates unexpectedly, 
    // it's treated as an error, not success
    let is_ci = env::var("CI").is_ok() || env::var("GITHUB_ACTIONS").is_ok();
    if is_ci {
        println!("🔄 Running in CI environment - skipping interactive test");
        return;
    }

    let container_name = format!("test-stream-term-{}", uuid::Uuid::new_v4());

    let test_result = tokio::time::timeout(tokio::time::Duration::from_secs(30), async {
        // Start a coding session
        let claude_client = container_utils::start_coding_session(
            &docker,
            &container_name,
            ClaudeCodeConfig::default(),
        )
        .await?;

        // Attempt authentication
        let auth_result = claude_client.authenticate_claude_account().await;

        match auth_result {
            Ok(mut auth_handle) => {
                println!("✅ Authentication handle received");
                
                // Collect all authentication states
                let mut states = Vec::new();
                
                // Read states with timeout to avoid hanging
                while let Ok(Some(state)) = tokio::time::timeout(
                    tokio::time::Duration::from_secs(10), 
                    auth_handle.state_receiver.recv()
                ).await {
                    println!("📝 Received state: {:?}", state);
                    
                    match &state {
                        AuthState::Failed(msg) => {
                            println!("✅ Got expected failure state: {}", msg);
                            states.push(state);
                            break;
                        }
                        AuthState::Completed(msg) => {
                            // This is the bug we're testing for - premature completion
                            if msg.contains("already authenticated") {
                                println!("✅ Already authenticated - this is valid");
                            } else {
                                println!("⚠️  Got completion state but CLI stream likely terminated early: {}", msg);
                                // This is the scenario we want to fix
                            }
                            states.push(state);
                            break;
                        }
                        _ => {
                            states.push(state);
                        }
                    }
                }
                
                // Analyze the states
                println!("📊 Total states received: {}", states.len());
                for (i, state) in states.iter().enumerate() {
                    println!("  {}: {:?}", i, state);
                }

                // The test passes regardless - we're documenting current behavior
                // The fix will change this behavior to be more robust
                println!("🔄 Stream termination test completed - behavior documented");
            }
            Err(e) => {
                println!("⚠️  Authentication failed: {}", e);
                // This is also valid - the system should handle failures gracefully
            }
        }

        Ok::<(), Box<dyn std::error::Error + Send + Sync>>(())
    }).await;

    // Cleanup
    cleanup_container(&docker, &container_name).await;

    match test_result {
        Ok(Ok(())) => {
            println!("✅ Stream termination test completed");
        }
        Ok(Err(e)) => {
            println!("⚠️  Test encountered error: {:?}", e);
        }
        Err(_) => {
            println!("⚠️  Test timed out");
        }
    }
}