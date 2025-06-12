# Telegram Bot in Rust

A Telegram bot built with Rust using the `teloxide` crate with Docker integration via `bollard`.

## Setup

1. **Create a bot with BotFather:**
   - Start a chat with [@BotFather](https://t.me/botfather) on Telegram
   - Send `/newbot` and follow the instructions
   - Copy the bot token you receive

2. **Ensure Docker is running:**
   - Make sure Docker daemon is running on your system
   - The bot connects to Docker via Unix socket (`/var/run/docker.sock`)

3. **Configure the environment:**
   - Create a `.env` file in the project root
   - Add your bot token: `TELOXIDE_TOKEN=your_bot_token_here`

4. **Build and run:**
   ```bash
   cargo build
   cargo run
   ```

## Features

The bot currently supports these commands:
- `/start` - Welcome message
- `/help` - Show available commands
- `/echo <text>` - Echo back the provided text
- `/startsession` - Starts a new coding session with Claude Code (Starts a new dev container)
- `/endsession` - Ends the current active session (Removes dev container)

## Project Structure

- `src/main.rs` - Main bot logic and command handlers with Docker integration
- `Cargo.toml` - Dependencies and project configuration
- `.env` - Environment variables (bot token)

## Dependencies

- `teloxide` - Telegram bot framework for Rust
- `tokio` - Async runtime
- `bollard` - Docker daemon API client for Rust
- `log` & `pretty_env_logger` - Logging functionality

## Docker Integration

The bot uses the `bollard` crate to interact with the Docker daemon:
- **Container Management**: List running containers with names, images, and status
- **System Information**: Display Docker system stats including version, container count, images, memory, and CPU info
- **Error Handling**: Graceful error handling for Docker API failures

### Docker Requirements

- Docker daemon must be running
- Bot needs access to Docker socket (usually `/var/run/docker.sock`)
- For production deployment, consider Docker socket permissions and security

## Extending the Bot

To add new Docker commands:
1. Add a new variant to the `Command` enum with appropriate description
2. Handle the new command in the `answer` function
3. Use the `docker` client to interact with Docker API
4. Add appropriate error handling

### Available Docker Operations

The `bollard` crate provides access to the full Docker API:
- Container operations (create, start, stop, remove)
- Image management (pull, build, push, list)
- Network management
- Volume management
- System monitoring and events

## Environment Variables

- `TELOXIDE_TOKEN` - Your Telegram bot token (required)
- `RUST_LOG` - Log level (optional, default: info)
- `DOCKER_HOST` - Docker daemon address (optional, uses socket by default)

## Security Considerations

- The bot has access to your Docker daemon - use appropriate access controls
- Consider running the bot in a container with limited Docker permissions
- In production, use Docker secrets or environment variable injection for tokens
- Monitor Docker API access and implement rate limiting if needed
