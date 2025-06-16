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
   - Command handlers for `/start`, `/help`, `/clearsession`, etc.
   - Docker integration via `bollard` crate
   - Authentication session management with global state tracking

2. **Claude Code Client** (`src/claude_code_client/mod.rs`)
   - Interactive Claude authentication with OAuth flow
   - State machine for login process: DarkMode → SelectLoginMethod → ProvideUrl → WaitingForCode → Completed
   - TTY-enabled exec for interactive CLI sessions
   - JSON result parsing for Claude Code responses

3. **GitHub Client** (`src/claude_code_client/github_client.rs`)
   - OAuth device flow authentication
   - Repository cloning via `gh` CLI
   - Authentication status checking

4. **Container Management** (`src/claude_code_client/container_utils.rs`)
   - Docker container lifecycle management
   - Uses runtime image: `ghcr.io/goniz/telegram-claude-code-runtime:main`
   - Container naming pattern: `coding-session-{chat_id}`

### Key Patterns

- **Session Management**: Each Telegram chat gets its own Docker container for isolation
- **Authentication State**: Complex state machine for handling Claude CLI interactive authentication
- **Early Returns**: Authentication processes return URLs/instructions immediately while continuing background processes
- **Docker Integration**: All coding happens inside containers with pre-installed development tools

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