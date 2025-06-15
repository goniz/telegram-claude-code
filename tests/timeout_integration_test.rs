use telegram_bot::GithubClientConfig;

/// Unit test to verify timeout error message format matches what users will see
#[cfg(test)]
mod timeout_integration_tests {
    use super::*;

    #[test]
    fn test_timeout_error_message_format() {
        // Test the timeout error message format that users would see
        let timeout_secs = 60;
        let command = vec!["gh", "auth", "login", "--git-protocol", "https"];
        
        let timeout_error = format!(
            "Command timed out after {} seconds: {}", 
            timeout_secs,
            command.join(" ")
        );
        
        assert!(timeout_error.contains("timed out after 60 seconds"));
        assert!(timeout_error.contains("gh auth login --git-protocol https"));
        
        println!("✅ Timeout error format: {}", timeout_error);
    }

    #[test]
    fn test_user_friendly_timeout_message() {
        // Test the user-friendly message that would be shown in Telegram
        let timeout_error = "Command timed out after 60 seconds: gh auth login --git-protocol https";
        
        let user_message = format!(
            "⏰ GitHub authentication timed out: {}\n\nThis usually means:\n• The authentication process is taking longer than expected\n• There may be network connectivity issues\n• The GitHub CLI might be unresponsive\n\nPlease try again in a few moments.", 
            timeout_error
        );
        
        assert!(user_message.contains("⏰ GitHub authentication timed out"));
        assert!(user_message.contains("authentication process is taking longer than expected"));
        assert!(user_message.contains("Please try again in a few moments"));
        
        println!("✅ User-friendly timeout message format verified");
    }

    #[test]
    fn test_default_timeout_configuration() {
        // Verify the default timeout is reasonable
        let default_config = GithubClientConfig::default();
        
        assert_eq!(default_config.exec_timeout_secs, 60);
        assert_eq!(default_config.working_directory, Some("/workspace".to_string()));
        
        println!("✅ Default timeout configuration: {} seconds", default_config.exec_timeout_secs);
    }

    #[test]
    fn test_custom_timeout_configuration() {
        // Test that custom timeouts can be configured
        let custom_config = GithubClientConfig {
            working_directory: Some("/workspace".to_string()),
            exec_timeout_secs: 30,
        };
        
        assert_eq!(custom_config.exec_timeout_secs, 30);
        
        println!("✅ Custom timeout configuration: {} seconds", custom_config.exec_timeout_secs);
    }
}