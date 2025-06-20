use bollard::Docker;
use rstest::*;
use std::sync::Once;
use telegram_bot::{container_utils, AuthState, ClaudeCodeClient, ClaudeCodeConfig};
use uuid;

static INIT: Once = Once::new();

fn init_logger() {
    INIT.call_once(|| {
        pretty_env_logger::init();
    });
}

/// Test fixture that provides a Docker client
#[fixture]
pub fn docker() -> Docker {
    Docker::connect_with_local_defaults().expect("Failed to connect to Docker")
}

/// Test fixture that creates a coding session container outside timeout blocks
/// This ensures Docker image pulling doesn't interfere with timing accuracy
#[fixture]
pub async fn claude_url_session() -> (Docker, ClaudeCodeClient, String) {
    let docker = Docker::connect_with_local_defaults().expect("Failed to connect to Docker");
    let container_name = format!("test-auth-url-{}", uuid::Uuid::new_v4());
    
    // Start coding session outside of timeout - this may pull Docker images
    let claude_client = container_utils::start_coding_session(
        &docker,
        &container_name,
        ClaudeCodeConfig::default(),
        container_utils::CodingContainerConfig::default(),
    )
    .await
    .expect("Failed to start coding session for URL test");
    
    (docker, claude_client, container_name)
}

/// Cleanup fixture that ensures test containers are removed
pub async fn cleanup_container(docker: &Docker, container_name: &str) {
    let _ = container_utils::clear_coding_session(docker, container_name).await;
}

#[rstest]
#[tokio::test]
async fn test_claude_auth_url_generation_like_bot(
    #[future] claude_url_session: (Docker, ClaudeCodeClient, String)
) {
    init_logger();
    let (docker, claude_client, container_name) = claude_url_session.await;

    // Use a reasonable timeout for the time-sensitive test logic
    // Note: Container creation now happens outside this timeout block for timing accuracy
    let test_timeout = tokio::time::Duration::from_secs(60); // 1 minutes

    let test_result = tokio::time::timeout(test_timeout, async {
        println!("=== STEP 1: Coding session already started ===");
        println!(
            "✅ Coding session started with container: {}",
            claude_client.container_id()
        );

        println!("=== STEP 2: Initiating Claude authentication (same API as bot) ===");

        // Step 2: Authenticate using the same API the bot uses
        let auth_handle = match claude_client.authenticate_claude_account().await {
            Ok(handle) => {
                println!("✅ Authentication handle created successfully");
                handle
            }
            Err(e) => {
                println!("❌ Failed to create authentication handle: {}", e);
                return Err(e);
            }
        };

        println!("=== STEP 3: Monitoring authentication states ===");

        // Step 3: Monitor authentication states (same as the bot does)
        let mut state_receiver = auth_handle.state_receiver;
        let mut url_received = false;
        let mut auth_started = false;

        // Track states we've seen
        while let Some(state) = state_receiver.recv().await {
            println!("📡 Received auth state: {:?}", state);

            match state {
                AuthState::Starting => {
                    println!("✅ Authentication process started");
                    auth_started = true;
                }
                AuthState::UrlReady(url) => {
                    println!("🔗 URL received: {}", url);
                    url_received = true;
                    break;
                }
                AuthState::WaitingForCode => {
                    println!("🔑 Authentication is waiting for user code");
                    // This is expected after URL, but our test goal is achieved
                    if url_received {
                        println!("✅ Code waiting state reached after URL - test objective met");
                        break;
                    }
                }
                AuthState::Completed(message) => {
                    println!("✅ Authentication completed: {}", message);
                    break;
                }
                AuthState::Failed(error) => {
                    println!("❌ Authentication failed: {}", error);
                    // Depending on the error, this might be expected in a test environment
                    if error.contains("timed out") || error.contains("not authenticated") {
                        println!("⚠️  Timeout/auth failure expected in test environment");
                        break;
                    } else {
                        return Err(format!("Authentication failed unexpectedly: {}", error).into());
                    }
                }
            }
        }

        let _ = auth_handle.cancel_sender.send(());
        drop(auth_handle.code_sender);

        // Verify test objectives
        if !auth_started {
            return Err("Authentication never started".into());
        }

        if !url_received {
            return Err("No URL received during authentication".into());
        } else {
            println!(
                "🎯 FULL SUCCESS: Authentication process worked up to URL generation as required"
            );
        }

        Ok::<(), Box<dyn std::error::Error + Send + Sync>>(())
    })
    .await;

    // Cleanup
    println!("=== CLEANUP: Removing test container ===");
    cleanup_container(&docker, &container_name).await;

    // Evaluate test results
    match test_result {
        Ok(Ok(())) => {
            println!("✅ Test completed successfully");
        }
        Ok(Err(e)) => {
            println!("❌ Test failed: {}", e);
            panic!("Test failed: {}", e);
        }
        Err(_) => {
            println!("⏰ Test timed out");
            panic!("Test timed out after the specified duration");
        }
    }
}

#[rstest]
#[tokio::test]
async fn test_claude_auth_invalid_code_handling(
    #[future] claude_url_session: (Docker, ClaudeCodeClient, String)
) {
    init_logger();
    let (docker, claude_client, container_name) = claude_url_session.await;

    // Use a reasonable timeout for the test
    let test_timeout = tokio::time::Duration::from_secs(120); // 2 minutes

    let test_result = tokio::time::timeout(test_timeout, async {
        println!("=== STEP 1: Coding session already started ===");
        println!(
            "✅ Coding session started with container: {}",
            claude_client.container_id()
        );

        println!("=== STEP 2: Initiating Claude authentication ===");

        // Step 2: Authenticate using the same API the bot uses
        let auth_handle = match claude_client.authenticate_claude_account().await {
            Ok(handle) => {
                println!("✅ Authentication handle created successfully");
                handle
            }
            Err(e) => {
                println!("❌ Failed to create authentication handle: {}", e);
                return Err(e);
            }
        };

        println!("=== STEP 3: Monitoring authentication states and sending invalid code ===");

        // Step 3: Monitor authentication states
        let mut state_receiver = auth_handle.state_receiver;
        let code_sender = auth_handle.code_sender;
        let mut url_received = false;
        let mut code_waiting_received = false;
        let mut invalid_code_sent = false;
        let mut auth_failed_received = false;

        // Invalid authentication code to test with (matches the format from problem statement)
        let invalid_auth_code = "ormGKQUdto7UkPsUSjVTD26QJHXU1QLpUhM8NkqEFc1TW1j5#qywGB2tquY5Buvd_Mz6iy8dSCQwg0FUSkqXqT12RrXY";

        // Track states we've seen
        while let Some(state) = state_receiver.recv().await {
            println!("📡 Received auth state: {:?}", state);

            match state {
                AuthState::Starting => {
                    println!("✅ Authentication process started");
                }
                AuthState::UrlReady(url) => {
                    println!("🔗 URL received: {}", url);
                    url_received = true;
                    // Continue to wait for WaitingForCode state
                }
                AuthState::WaitingForCode => {
                    println!("🔑 Authentication is waiting for user code");
                    code_waiting_received = true;
                    
                    if url_received && !invalid_code_sent {
                        println!("📤 Sending invalid authentication code: {}", invalid_auth_code);
                        match code_sender.send(invalid_auth_code.to_string()) {
                            Ok(_) => {
                                println!("✅ Invalid code sent successfully");
                                invalid_code_sent = true;
                            }
                            Err(e) => {
                                return Err(format!("Failed to send authentication code: {}", e).into());
                            }
                        }
                    }
                }
                AuthState::Failed(error) => {
                    println!("❌ Authentication failed: {}", error);
                    auth_failed_received = true;
                    
                    // Check if this is a timeout error - this should fail the test
                    let error_lower = error.to_lowercase();
                    if error_lower.contains("timed out") || error_lower.contains("timeout") {
                        return Err(format!("Test failed: Authentication timed out, indicating the invalid code never reached the CLI. Error: {}", error).into());
                    }
                    
                    // Verify that the error indicates the CLI processed and rejected the invalid code
                    if error_lower.contains("invalid") || 
                       error_lower.contains("unauthorized") || 
                       error_lower.contains("authentication") ||
                       error_lower.contains("code") ||
                       error_lower.contains("failed") {
                        println!("✅ Error message indicates the CLI processed and rejected the invalid authentication code as expected");
                        break;
                    } else {
                        return Err(format!("Unexpected error message - does not indicate the CLI processed the invalid code: {}", error).into());
                    }
                }
                AuthState::Completed(message) => {
                    println!("❌ Unexpected: Authentication completed when it should have failed: {}", message);
                    return Err("Authentication completed unexpectedly with invalid code".into());
                }
            }

            // Exit if we've achieved our test objectives
            if auth_failed_received {
                break;
            }
        }

        // Clean up
        let _ = auth_handle.cancel_sender.send(());

        // Verify test objectives
        if !url_received {
            return Err("No URL received during authentication".into());
        }

        if !code_waiting_received {
            return Err("Never received WaitingForCode state".into());
        }

        if !invalid_code_sent {
            return Err("Failed to send invalid authentication code".into());
        }

        if !auth_failed_received {
            return Err("Expected authentication to fail with invalid code, but no failure received".into());
        }

        println!("🎯 FULL SUCCESS: Invalid authentication code was properly handled and rejected");
        Ok::<(), Box<dyn std::error::Error + Send + Sync>>(())
    })
    .await;

    // Cleanup
    println!("=== CLEANUP: Removing test container ===");
    cleanup_container(&docker, &container_name).await;

    // Evaluate test results
    match test_result {
        Ok(Ok(())) => {
            println!("✅ Test completed successfully");
        }
        Ok(Err(e)) => {
            println!("❌ Test failed: {}", e);
            panic!("Test failed: {}", e);
        }
        Err(_) => {
            println!("⏰ Test timed out");
            panic!("Test timed out after the specified duration");
        }
    }
}
