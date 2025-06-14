# Docker Deployment Guide

This guide covers how to build and run the Telegram bot using Docker.

## Files Overview

- `Dockerfile` - Multi-stage build configuration for the main bot
- `Dockerfile.runtime` - Pre-built runtime image for development session containers
- `.dockerignore` - Files to exclude from Docker build context
- `docker-compose.yml` - Docker Compose configuration
- `DOCKER.md` - This documentation

## Building and Running

### Option 1: Docker Compose (Recommended)

1. **Set up environment variables:**
   ```bash
   # Create .env file with your bot token
   echo "TELOXIDE_TOKEN=your_bot_token_here" > .env
   echo "RUST_LOG=info" >> .env
   ```

2. **Run with Docker Compose:**
   ```bash
   docker-compose up -d
   ```

3. **View logs:**
   ```bash
   docker-compose logs -f telegram-bot
   ```

4. **Stop the bot:**
   ```bash
   docker-compose down
   ```

### Option 2: Manual Docker Commands

1. **Build the image:**
   ```bash
   docker build -t telegram-bot .
   ```

2. **Run the container:**
   ```bash
   docker run -d \
     --name telegram-bot \
     --restart unless-stopped \
     -e TELOXIDE_TOKEN=your_bot_token_here \
     -e RUST_LOG=info \
     telegram-bot
   ```

## Runtime Container Image

The repository includes a `Dockerfile.runtime` for building pre-configured development session containers.

### Building the Runtime Image

```bash
docker build -f Dockerfile.runtime -t telegram-claude-runtime:latest .
```

### Runtime Image Features

- **Base Image**: Uses `ghcr.io/goniz/claude-code-runtime:latest` for multi-language development with Claude Code pre-installed
- **Pre-installed Tools**:
  - Claude Code CLI (`@anthropic-ai/claude-code`) via npm
  - GitHub CLI (`gh`) for repository management
- **Optimized for Development**: Reduces container startup time by pre-installing common tools
- **Working Directory**: Set to `/workspace` for development sessions

### Usage

The runtime image is designed for use with the bot's development session containers, providing:
- Faster container startup (tools already installed)
- Consistent development environment
- GitHub integration capabilities

3. **View logs:**
   ```bash
   docker logs -f telegram-bot
   ```

## Dockerfile Features

### Multi-stage Build
- **Builder stage**: Uses `rust:1.87-slim` to compile the application
- **Runtime stage**: Uses minimal `debian:bookworm-slim` for smaller final image

### Security Features
- Runs as non-root user (`telegram-bot`)
- Minimal runtime dependencies
- Resource limits in docker-compose

### Optimization
- Dependency caching: Dependencies are built separately from source code
- Minimal runtime image: Only includes necessary libraries
- Health checks: Monitors bot process status

## Image Size Optimization

The multi-stage build significantly reduces the final image size:
- Builder stage: ~1.5GB (includes Rust toolchain)
- Final runtime image: ~100MB (only runtime dependencies)

## Environment Variables

- `TELOXIDE_TOKEN` - Your Telegram bot token (required)
- `RUST_LOG` - Log level (default: info)

## Resource Limits

The docker-compose configuration includes resource limits:
- Memory: 128MB limit, 64MB reservation
- CPU: 0.5 CPU limit, 0.1 CPU reservation

## Monitoring

### Health Checks
The container includes health checks that verify the bot process is running:
- Interval: 30 seconds
- Timeout: 10 seconds
- Retries: 3

### Viewing Status
```bash
# Check container status
docker-compose ps

# View resource usage
docker stats telegram-bot

# Check health status
docker inspect telegram-bot | grep -A 5 Health
```

## Troubleshooting

### Common Issues

1. **Bot token not set:**
   ```
   Error: TELOXIDE_TOKEN not set
   ```
   Solution: Ensure your `.env` file contains the correct token.

2. **Permission denied:**
   ```
   Error: Permission denied
   ```
   Solution: Check that the bot user has necessary permissions.

3. **Build failures:**
   - Clear Docker cache: `docker system prune -a`
   - Check internet connection for dependency downloads

### Debugging

```bash
# Enter running container
docker exec -it telegram-bot /bin/bash

# Check logs with timestamps
docker-compose logs -f -t telegram-bot

# Restart container
docker-compose restart telegram-bot
```

## CI/CD Pipeline

The repository includes a GitHub Actions workflow that automatically builds and publishes Docker images to GitHub Container Registry (ghcr.io) when code is pushed to the main branch.

### Automated Docker Image Publishing

- **Trigger**: Push to main branch
- **Registry**: GitHub Container Registry (ghcr.io)
- **Image Tags**: 
  - `ghcr.io/goniz/telegram-claude-code:main` (latest main branch)
  - `ghcr.io/goniz/telegram-claude-code:main-<commit-sha>` (specific commit)
- **Authentication**: Uses `GITHUB_TOKEN` (automatically provided by GitHub Actions)

### Using Published Images

```bash
# Pull the latest main branch image
docker pull ghcr.io/goniz/telegram-claude-code:main

# Pull a specific commit version
docker pull ghcr.io/goniz/telegram-claude-code:main-abc1234

# Run using the published image
docker run -d \
  --name telegram-bot \
  --restart unless-stopped \
  -e TELOXIDE_TOKEN=your_bot_token_here \
  -e RUST_LOG=info \
  ghcr.io/goniz/telegram-claude-code:main
```

## Production Considerations

1. **Environment Variables**: Use Docker secrets or external configuration management
2. **Logging**: Configure log rotation and centralized logging
3. **Monitoring**: Add application metrics and monitoring
4. **Updates**: Use published Docker images for automated deployments
5. **Backup**: Consider backing up any persistent data
