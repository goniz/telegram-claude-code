//! Container management module
//!
//! This module provides functionality for managing Docker containers, volumes,
//! and file operations used by the telegram-claude-code application.

pub mod file_ops;
pub mod lifecycle;
pub mod volume;

// Re-export commonly used functions for convenience
pub use file_ops::{container_get_file, container_put_file};
pub use lifecycle::{
    clear_all_session_containers, clear_coding_session, create_test_container,
    exec_command_in_container, start_coding_session, wait_for_container_ready,
    CodingContainerConfig, MAIN_CONTAINER_IMAGE,
};
pub use volume::{
    create_auth_mounts, ensure_user_volume, generate_volume_name, validate_volume_key,
};
