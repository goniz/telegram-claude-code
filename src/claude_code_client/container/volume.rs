//! Docker volume management functionality for authentication persistence
//!
//! This module handles the creation and management of Docker volumes used for
//! persisting authentication data across container sessions. It provides functions
//! to create named volumes, set up mount configurations, and manage the volume
//! structure for Claude and GitHub authentication data.

use bollard::models::{Mount, MountTypeEnum};
use bollard::volume::CreateVolumeOptions;
use bollard::Docker;
use std::collections::HashMap;

/// Generate a standardized volume name for a user's persistent authentication data
///
/// # Arguments
/// * `volume_key` - Unique identifier for the user's volume (typically chat_id or user_id)
///
/// # Returns
/// A formatted volume name following the pattern: `dev-session-claude-{volume_key}`
///
/// # Examples
/// ```
/// let volume_name = generate_volume_name("12345");
/// assert_eq!(volume_name, "dev-session-claude-12345");
/// ```
pub fn generate_volume_name(volume_key: &str) -> String {
    format!("dev-session-claude-{}", volume_key)
}

/// Create or get existing volume for user authentication persistence
///
/// This function is idempotent - it will not fail if the volume already exists.
/// The volume is created with labels for identification and management purposes.
///
/// # Arguments
/// * `docker` - Docker client instance
/// * `volume_key` - Unique identifier for the user's volume
///
/// # Returns
/// * `Ok(String)` - The volume name that was created or already existed
/// * `Err(Box<dyn std::error::Error + Send + Sync>)` - Error if volume creation fails
///
/// # Volume Labels
/// The created volume includes the following labels:
/// - `created_by`: "telegram-claude-code"
/// - `volume_key`: The provided volume key
/// - `purpose`: "authentication_persistence"
///
/// # Examples
/// ```no_run
/// use bollard::Docker;
///
/// async fn example() -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
///     let docker = Docker::connect_with_socket_defaults()?;
///     let volume_name = ensure_user_volume(&docker, "user123").await?;
///     println!("Volume created: {}", volume_name);
///     Ok(())
/// }
/// ```
pub async fn ensure_user_volume(
    docker: &Docker,
    volume_key: &str,
) -> Result<String, Box<dyn std::error::Error + Send + Sync>> {
    let volume_name = generate_volume_name(volume_key);

    // Create the volume - Docker will return an error if it already exists
    let create_options = CreateVolumeOptions {
        name: volume_name.clone(),
        driver: "local".to_string(),
        driver_opts: HashMap::new(),
        labels: {
            let mut labels = HashMap::new();
            labels.insert("created_by".to_string(), "telegram-claude-code".to_string());
            labels.insert("volume_key".to_string(), volume_key.to_string());
            labels.insert(
                "purpose".to_string(),
                "authentication_persistence".to_string(),
            );
            labels
        },
    };

    match docker.create_volume(create_options).await {
        Ok(_) => {
            log::info!(
                "Created new volume '{}' for key {}",
                volume_name,
                volume_key
            );
            Ok(volume_name)
        }
        Err(e) => {
            // Check if the error is because the volume already exists
            if e.to_string().contains("already exists") || e.to_string().contains("Conflict") {
                log::info!("Volume '{}' already exists, reusing", volume_name);
                Ok(volume_name)
            } else {
                Err(format!("Failed to create volume '{}': {}", volume_name, e).into())
            }
        }
    }
}

/// Create Docker mount configurations for authentication persistence
///
/// This function creates mount configurations that map a Docker volume to container
/// paths for persistent authentication data storage. The volume is mounted to
/// `/volume_data` in the container, where symbolic links are later created to
/// connect authentication directories.
///
/// # Arguments
/// * `volume_name` - Name of the Docker volume to mount
///
/// # Returns
/// A vector containing a single `Mount` configuration for the authentication volume
///
/// # Mount Configuration
/// - **Source**: The specified Docker volume
/// - **Target**: `/volume_data` directory in the container
/// - **Type**: Volume mount (not bind mount)
/// - **Read-only**: False (allows write access)
/// - **Consistency**: "default"
///
/// # Container Directory Structure
/// After mounting, the container will have the following structure setup via symbolic links:
/// - `/root/.claude` → `/volume_data/claude` (Claude authentication data)
/// - `/root/.config/gh` → `/volume_data/gh` (GitHub CLI authentication)
/// - `/root/.claude.json` → `/volume_data/claude.json` (Claude configuration)
///
/// # Examples
/// ```
/// let volume_name = "dev-session-claude-12345";
/// let mounts = create_auth_mounts(&volume_name);
/// assert_eq!(mounts.len(), 1);
/// assert_eq!(mounts[0].target, Some("/volume_data".to_string()));
/// assert_eq!(mounts[0].source, Some(volume_name.to_string()));
/// ```
pub fn create_auth_mounts(volume_name: &str) -> Vec<Mount> {
    vec![
        // Mount volume to a base directory, then use symbolic links or init commands
        // to set up the proper structure
        Mount {
            target: Some("/volume_data".to_string()),
            source: Some(volume_name.to_string()),
            typ: Some(MountTypeEnum::VOLUME),
            read_only: Some(false),
            consistency: Some("default".to_string()),
            ..Default::default()
        },
    ]
}

/// Helper function to validate volume key format
///
/// Ensures the volume key contains only safe characters for Docker volume naming.
/// Docker volume names must follow specific naming conventions.
///
/// # Arguments
/// * `volume_key` - The volume key to validate
///
/// # Returns
/// * `Ok(())` - If the volume key is valid
/// * `Err(String)` - If the volume key contains invalid characters
///
/// # Valid Characters
/// Volume keys should contain only:
/// - Alphanumeric characters (a-z, A-Z, 0-9)
/// - Hyphens (-)
/// - Underscores (_)
/// - Periods (.)
///
/// # Examples
/// ```
/// assert!(validate_volume_key("user123").is_ok());
/// assert!(validate_volume_key("user-123_test.1").is_ok());
/// assert!(validate_volume_key("user@123").is_err()); // @ is not allowed
/// ```
pub fn validate_volume_key(volume_key: &str) -> Result<(), String> {
    if volume_key.is_empty() {
        return Err("Volume key cannot be empty".to_string());
    }

    // Check for invalid characters
    if !volume_key
        .chars()
        .all(|c| c.is_alphanumeric() || c == '-' || c == '_' || c == '.')
    {
        return Err("Volume key contains invalid characters. Only alphanumeric, hyphens, underscores, and periods are allowed".to_string());
    }

    // Check length constraints (Docker has limits)
    if volume_key.len() > 200 {
        return Err("Volume key is too long. Maximum length is 200 characters".to_string());
    }

    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_generate_volume_name() {
        let volume_name = generate_volume_name("12345");
        assert_eq!(volume_name, "dev-session-claude-12345");

        let volume_name = generate_volume_name("user-test_123.1");
        assert_eq!(volume_name, "dev-session-claude-user-test_123.1");
    }

    #[test]
    fn test_create_auth_mounts() {
        let volume_name = "test-volume";
        let mounts = create_auth_mounts(volume_name);

        assert_eq!(mounts.len(), 1);

        let mount = &mounts[0];
        assert_eq!(mount.target, Some("/volume_data".to_string()));
        assert_eq!(mount.source, Some(volume_name.to_string()));
        assert_eq!(mount.typ, Some(MountTypeEnum::VOLUME));
        assert_eq!(mount.read_only, Some(false));
        assert_eq!(mount.consistency, Some("default".to_string()));
    }

    #[test]
    fn test_validate_volume_key() {
        // Valid keys
        assert!(validate_volume_key("user123").is_ok());
        assert!(validate_volume_key("user-123").is_ok());
        assert!(validate_volume_key("user_123").is_ok());
        assert!(validate_volume_key("user.123").is_ok());
        assert!(validate_volume_key("user-test_123.1").is_ok());

        // Invalid keys
        assert!(validate_volume_key("").is_err());
        assert!(validate_volume_key("user@123").is_err());
        assert!(validate_volume_key("user#123").is_err());
        assert!(validate_volume_key("user 123").is_err());
        assert!(validate_volume_key("user/123").is_err());

        // Test length limit
        let long_key = "a".repeat(201);
        assert!(validate_volume_key(&long_key).is_err());

        let max_key = "a".repeat(200);
        assert!(validate_volume_key(&max_key).is_ok());
    }
}
