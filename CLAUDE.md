# CLAUDE.md

This file provides guidance to Claude Code (claude.ai/code) when working with code in this repository.

## Development Commands

### Build and Test
- **Build**: `cargo build`
- **Run**: `cargo run`
- **Test**: `cargo test`
- **Check**: `cargo check`

### Environment Setup
- Create `.env` file with `TELOXIDE_TOKEN=your_bot_token_here`
- Ensure Docker daemon is running (required for container operations)

## High-Level Architecture

This is a Telegram bot built with Rust that provides coding sessions through Docker containers with Claude Code pre-installed.

### Core Components

1. **Main Bot Logic** (`src/main.rs`)
   - Telegram bot using `teloxide` framework
   - Coordinates command handlers and global state
   - Docker integration via `bollard` crate

2. **Bot Module** (`src/bot/`)
   - `handlers.rs` - Core command routing and message handling
   - `auth_session.rs` - Authentication session management with global state tracking
   - `state.rs` - Bot state management structures
   - `markdown.rs` - Telegram markdown formatting utilities

3. **Commands Module** (`src/commands/`)
   - Modular command handlers for all bot commands:
   - `start.rs`, `help.rs`, `clear_session.rs` - Basic bot commands
   - `authenticate_claude.rs`, `claude_status.rs`, `update_claude.rs` - Claude authentication
   - `github_auth.rs`, `github_status.rs`, `github_clone.rs`, `github_repo_list.rs` - GitHub operations
   - `debug_claude_login.rs` - Debug utilities

4. **Claude Code Client** (`src/claude_code_client/`)
   - `auth.rs` - Interactive Claude authentication with OAuth flow
   - `config.rs` - Claude Code configuration management
   - `executor.rs` - TTY-enabled exec for interactive CLI sessions
   - `container_cred_storage.rs` - Container-based credential storage

5. **OAuth Module** (`src/oauth/`)
   - `flow.rs` - OAuth 2.0 flow with PKCE implementation
   - `config.rs` - OAuth configuration structures
   - `credentials.rs` - OAuth credential management
   - `storage.rs` - File-based credential storage with `CredStorageOps` trait
   - `errors.rs` - OAuth-specific error types

6. **GitHub Client** (`src/claude_code_client/github_client/github/`)
   - `auth.rs` - OAuth device flow authentication (405 lines)
   - `operations.rs` - Repository cloning and listing operations (255 lines)
   - `types.rs` - Shared GitHub data structures
   - `mod.rs` - Main GitHub client interface

7. **Container Management** (`src/claude_code_client/container/`)
   - `lifecycle.rs` - Container creation, destruction, and management (520 lines)
   - `volume.rs` - Docker volume and persistence management (250 lines)
   - `file_ops.rs` - Bidirectional file operations between host and container (139 lines)
   - Uses runtime image: `ghcr.io/goniz/telegram-claude-code-runtime:main`
   - Container naming pattern: `coding-session-{chat_id}`

### Architecture Benefits

- **Modular Design**: Large files split into focused modules (200-300 lines each) for better maintainability
- **Clear Separation of Concerns**: Each module has single responsibility (authentication, container management, GitHub operations)
- **Backward Compatibility**: All functionality preserved via re-exports
- **Enhanced Testability**: Individual modules testable in isolation
- **Clean Dependencies**: Minimal interdependencies between modules

### Key Patterns

- **Session Management**: Each Telegram chat gets its own Docker container for isolation
- **Authentication State**: Complex state machine for handling Claude CLI interactive authentication
- **Early Returns**: Authentication processes return URLs/instructions immediately while continuing background processes
- **Docker Integration**: All coding happens inside containers with pre-installed development tools
- **Command Pattern**: Bot commands implemented as separate modules with consistent interfaces

### Container Environment

- Working directory: `/workspace`
- Node.js via NVM with multiple versions (18.20.8, 20.19.2, 22.16.0)
- Claude Code CLI pre-installed
- GitHub CLI (`gh`) available for Git operations

### Error Handling

- Graceful Docker API failure handling
- Command exit code checking with detailed error messages
- Timeout protection for long-running operations (60s for auth, configurable elsewhere)
- Fallback instructions when interactive processes fail
