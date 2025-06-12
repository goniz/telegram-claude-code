# Telegram Claude Code Bot

A powerful Telegram bot built with Rust that integrates Docker container management with Claude Code capabilities for automated coding assistance.

## 🚀 MVP Features

### Core Features
- **Docker Integration**: Full Docker container management via Docker API
  - List and manage containers
  - Display Docker system information
  - Robust error handling for Docker API failures
- **Telegram Bot Commands**: Essential bot commands with rich formatting
  - `/start` - Welcome message
  - `/help` - Display supported commands
  - `/listcontainers` - List running Docker containers
  - `/dockerinfo` - Show Docker system stats
  - `/startsession` - Create a new coding session container
  - `/clearsession` - Stop and remove the coding session container
  - `/reviewcode <file_path>` - Review code using Claude Code
  - `/generatedocs <file_path>` - Generate documentation for code
  - `/echo <text>` - Echo the provided text

### Security Enhancements
- **Non-root User**: Bot runs as a non-root user in Docker for better security
- **Environment Variables**: Uses environment variables for sensitive information
- **Access Controls**: Limited Docker socket access with appropriate permissions

### Code Review and Documentation
- **Claude Code Integration**: Built-in code review and documentation generation
- **Session Management**: Per-user coding sessions with isolated containers
- **Automated Analysis**: Intelligent code analysis and improvement suggestions

## 📦 Deployment

### Using Docker Compose (Recommended)

1. **Clone the repository:**
   ```bash
   git clone <repository-url>
   cd telegram-claude-code
   ```

2. **Create environment file:**
   ```bash
   cp .env.example .env
   # Edit .env and add your TELOXIDE_TOKEN
   ```

3. **Deploy with Docker Compose:**
   ```bash
   docker-compose up -d
   ```

4. **With logging (optional):**
   ```bash
   docker-compose --profile logging up -d
   ```

### Manual Setup

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
   cargo build --release
   cargo run
   ```

## 🏗️ Architecture

### Project Structure
```
src/
├── main.rs                 # Main bot logic and command handlers
├── claude_code_client.rs   # Claude Code integration client
Dockerfile                  # Multi-stage Docker build
docker-compose.yml         # Deployment configuration
DOCKER.md                  # Docker-specific documentation
```

### Dependencies
- `teloxide` - Telegram bot framework for Rust
- `tokio` - Async runtime
- `bollard` - Docker daemon API client for Rust
- `serde` & `serde_json` - JSON serialization
- `uuid` - UUID generation for sessions
- `log` & `pretty_env_logger` - Structured logging

## 🔧 Extending the Bot

### Adding New Docker Commands
1. Add a new variant to the `Command` enum with appropriate description
2. Handle the new command in the `answer` function
3. Use the `docker` client to interact with Docker API
4. Add comprehensive error handling and logging

### Claude Code Integration
The bot integrates with Claude Code for:
- **Code Review**: Automated code analysis and suggestions
- **Documentation Generation**: Comprehensive code documentation
- **Code Fixing**: Automated issue resolution
- **Authentication**: Support for Claude account authentication

## 📊 Monitoring and Troubleshooting

### Logging
- Structured logging with different levels (DEBUG, INFO, WARN, ERROR)
- Container lifecycle events are logged
- Docker API failures are captured and logged
- Optional centralized logging with Fluentd

### Health Checks
- Built-in health checks for the bot container
- Process monitoring with automatic restarts
- Resource usage monitoring

### Troubleshooting Guide
- Check Docker daemon connectivity: `docker ps`
- Verify bot token: Test with a simple message
- Check container logs: `docker logs telegram-bot`
- Monitor resource usage: `docker stats telegram-bot`

## ⚠️ Security Considerations

- **Docker Socket Access**: The bot requires Docker socket access - use appropriate ACLs
- **Container Isolation**: Coding sessions run in isolated containers
- **Resource Limits**: Memory and CPU limits prevent resource exhaustion
- **Non-root Execution**: All containers run with non-root users
- **Environment Variables**: Sensitive data managed through environment variables

## 🧪 Testing

### Unit Tests
```bash
cargo test
```

### Integration Tests
```bash
# Test Docker connectivity
docker info

# Test bot commands manually through Telegram
# Or use the Telegram Bot API for automated testing
```

## 📋 Environment Variables

- `TELOXIDE_TOKEN` - Your Telegram bot token (required)
- `RUST_LOG` - Log level (optional, default: info)
- `DOCKER_HOST` - Docker daemon address (optional, uses socket by default)

## 🚢 Production Deployment

### Resource Requirements
- **Memory**: 256MB minimum, 512MB recommended
- **CPU**: 0.2 cores minimum, 1 core recommended
- **Storage**: 1GB for logs and temporary files
- **Network**: Internet access for Telegram API

### Scaling Considerations
- Horizontal scaling supported with session management
- Database integration for session persistence (future enhancement)
- Load balancing for high-traffic deployments

## 📝 License

[Include your license information here]

## 🤝 Contributing

[Include contribution guidelines here]
