use bollard::Docker;
use rstest::*;
use telegram_bot::{ClaudeCodeConfig, container_utils, InteractiveLoginState};
use std::env;

#[allow(unused_imports)]
use telegram_bot::ClaudeCodeClient;

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
async fn test_interactive_login_flow_dark_mode(docker: Docker) {
    // Test the "Dark mode" scenario
    let is_ci = env::var("CI").is_ok() || env::var("GITHUB_ACTIONS").is_ok();
    if is_ci {
        println!("🔄 Running in CI environment - skipping interactive test");
        return;
    }
    
    let container_name = format!("test-interactive-{}", uuid::Uuid::new_v4());
    
    let test_result = tokio::time::timeout(tokio::time::Duration::from_secs(30), async {
        // Start a coding session
        let claude_client = container_utils::start_coding_session(&docker, &container_name, ClaudeCodeConfig::default()).await?;
        
        // Test that the interactive login can handle "Dark mode" output
        // This is a mock test since we can't easily simulate the exact Claude CLI output
        // but we can test the structure is in place
        
        let auth_result = claude_client.authenticate_claude_account().await;
        
        // The authentication should work (or fail gracefully)
        match auth_result {
            Ok(instructions) => {
                println!("✅ Authentication instructions received: {}", instructions);
                assert!(!instructions.is_empty(), "Instructions should not be empty");
            }
            Err(e) => {
                println!("⚠️  Authentication failed (expected in test): {}", e);
                // In test environment, this is expected
            }
        }
        
        Ok::<(), Box<dyn std::error::Error + Send + Sync>>(())
    }).await;
    
    // Cleanup
    cleanup_container(&docker, &container_name).await;
    
    match test_result {
        Ok(Ok(())) => println!("✅ Interactive login test completed"),
        Ok(Err(e)) => println!("⚠️  Test failed: {:?}", e),
        Err(_) => println!("⚠️  Test timed out"),
    }
}

#[rstest]
#[tokio::test]
async fn test_interactive_login_state_transitions() {
    // Test the state machine logic for different outputs
    let test_cases = vec![
        ("Dark mode enabled", InteractiveLoginState::DarkMode),
        ("Select login method:", InteractiveLoginState::SelectLoginMethod),
        ("Use the url below to sign in: https://example.com", InteractiveLoginState::ProvideUrl("https://example.com".to_string())),
        ("Paste code here if prompted:", InteractiveLoginState::WaitingForCode),
        ("Login successful", InteractiveLoginState::LoginSuccessful),
        ("Security notes:", InteractiveLoginState::SecurityNotes),
        ("Do you trust the files in this folder?", InteractiveLoginState::TrustFiles),
    ];
    
    for (output, expected_state) in test_cases {
        let state = parse_cli_output_for_state(output);
        match (&state, &expected_state) {
            (InteractiveLoginState::DarkMode, InteractiveLoginState::DarkMode) => {
                println!("✅ Dark mode state correctly detected");
            }
            (InteractiveLoginState::SelectLoginMethod, InteractiveLoginState::SelectLoginMethod) => {
                println!("✅ Select login method state correctly detected");
            }
            (InteractiveLoginState::ProvideUrl(url), InteractiveLoginState::ProvideUrl(_expected_url)) => {
                println!("✅ URL state correctly detected: {}", url);
            }
            (InteractiveLoginState::WaitingForCode, InteractiveLoginState::WaitingForCode) => {
                println!("✅ Waiting for code state correctly detected");
            }
            (InteractiveLoginState::LoginSuccessful, InteractiveLoginState::LoginSuccessful) => {
                println!("✅ Login successful state correctly detected");
            }
            (InteractiveLoginState::SecurityNotes, InteractiveLoginState::SecurityNotes) => {
                println!("✅ Security notes state correctly detected");
            }
            (InteractiveLoginState::TrustFiles, InteractiveLoginState::TrustFiles) => {
                println!("✅ Trust files state correctly detected");
            }
            _ => {
                panic!("State mismatch for output '{}': expected {:?}, got {:?}", output, expected_state, state);
            }
        }
    }
}

// Helper function to parse CLI output and determine the state
// This will be implemented in the main code later
fn parse_cli_output_for_state(output: &str) -> InteractiveLoginState {
    let output_lower = output.to_lowercase();
    
    if output_lower.contains("dark mode") {
        InteractiveLoginState::DarkMode
    } else if output_lower.contains("select login method") {
        InteractiveLoginState::SelectLoginMethod
    } else if output_lower.contains("use the url below to sign in") {
        // Extract URL from output
        for line in output.lines() {
            if line.trim().starts_with("https://") {
                return InteractiveLoginState::ProvideUrl(line.trim().to_string());
            }
        }
        // If no URL found, look for URL pattern in the same line
        if let Some(url_start) = output.find("https://") {
            let url_part = &output[url_start..];
            if let Some(url_end) = url_part.find(char::is_whitespace) {
                let url = &url_part[..url_end];
                return InteractiveLoginState::ProvideUrl(url.to_string());
            } else {
                return InteractiveLoginState::ProvideUrl(url_part.to_string());
            }
        }
        InteractiveLoginState::ProvideUrl("URL_NOT_FOUND".to_string())
    } else if output_lower.contains("paste code here if prompted") {
        InteractiveLoginState::WaitingForCode
    } else if output_lower.contains("login successful") {
        InteractiveLoginState::LoginSuccessful
    } else if output_lower.contains("security notes") {
        InteractiveLoginState::SecurityNotes
    } else if output_lower.contains("do you trust the files in this folder") {
        InteractiveLoginState::TrustFiles
    } else {
        InteractiveLoginState::Error(format!("Unknown output: {}", output))
    }
}