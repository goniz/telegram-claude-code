# Docker Deployment Guide

This guide covers how to build and run the Telegram Claude Code bot using Docker with comprehensive Docker integration and security features.

## 🔧 Files Overview

- `Dockerfile` - Multi-stage build configuration with security hardening
- `.dockerignore` - Files to exclude from Docker build context
- `docker-compose.yml` - Production-ready Docker Compose configuration
- `DOCKER.md` - This comprehensive documentation

## 🚀 Building and Running

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

3. **With logging infrastructure:**
   ```bash
   # Enable centralized logging
   docker-compose --profile logging up -d
   ```

4. **View logs:**
   ```bash
   docker-compose logs -f telegram-bot
   ```

5. **Stop the bot:**
   ```bash
   docker-compose down
   ```

### Option 2: Manual Docker Commands

1. **Build the image:**
   ```bash
   docker build -t telegram-claude-bot .
   ```

2. **Run the container with Docker socket access:**
   ```bash
   docker run -d \
     --name telegram-claude-bot \
     --restart unless-stopped \
     -e TELOXIDE_TOKEN=your_bot_token_here \
     -e RUST_LOG=info \
     -v /var/run/docker.sock:/var/run/docker.sock:ro \
     -v ./logs:/app/logs:rw \
     --group-add docker \
     telegram-claude-bot
   ```

3. **View logs:**
   ```bash
   docker logs -f telegram-claude-bot
   ```

## 🏗️ Dockerfile Features

### Multi-stage Build
- **Builder stage**: Uses `rust:1.87-slim` to compile the application
- **Runtime stage**: Uses minimal `debian:bookworm-slim` for smaller final image
- **Dependency caching**: Optimized layer caching for faster rebuilds

### Security Features
- **Non-root execution**: Runs as dedicated `telegram-bot` user
- **Docker group access**: Secure Docker socket access
- **Minimal runtime dependencies**: Only essential libraries included
- **Resource constraints**: Memory and CPU limits enforced

### Optimization
- **Dependency caching**: Dependencies built separately from source code
- **Minimal runtime image**: ~100MB final image size
- **Health checks**: Comprehensive process monitoring

## 📊 Docker Compose Configuration

### Services
- **telegram-bot**: Main bot service with Docker integration
- **log-aggregator**: Optional centralized logging with Fluentd

### Features
- **Docker socket mounting**: Secure read-only access to Docker daemon
- **Resource limits**: Production-ready resource constraints
- **Health monitoring**: Automated health checks and restarts
- **Network isolation**: Dedicated bridge network
- **Volume management**: Persistent logs and temporary files

### Security Enhancements
- **User mapping**: Runs as non-root user with Docker group access
- **Read-only Docker socket**: Prevents unauthorized Docker modifications
- **Network segmentation**: Isolated network for bot services
- **Resource quotas**: Prevents resource exhaustion attacks

## 🔐 Security Considerations

### Docker Socket Access
The bot requires Docker socket access for container management:
```yaml
volumes:
  - /var/run/docker.sock:/var/run/docker.sock:ro
```

**Security measures:**
- Read-only socket access
- Non-root user execution
- Docker group membership for controlled access
- Resource limits to prevent abuse

### Container Isolation
- Each coding session runs in an isolated container
- Proper cleanup of temporary containers
- Network isolation between sessions
- Resource limits per session

## 📈 Resource Management

### Current Limits
```yaml
deploy:
  resources:
    limits:
      memory: 256M
      cpus: '1.0'
    reservations:
      memory: 128M
      cpus: '0.2'
```

### Scaling Recommendations
- **Development**: 128MB memory, 0.2 CPU
- **Production**: 512MB memory, 1.0 CPU
- **High-traffic**: 1GB memory, 2.0 CPU

## 📊 Monitoring and Logging

### Health Checks
Comprehensive health monitoring:
- **Process check**: Verifies bot process is running
- **Interval**: 30 seconds
- **Timeout**: 10 seconds
- **Retries**: 3 attempts
- **Start period**: 10 seconds grace period

### Logging Infrastructure
Optional centralized logging with Fluentd:
```bash
# Enable logging profile
docker-compose --profile logging up -d
```

### Monitoring Commands
```bash
# Check container status
docker-compose ps

# View resource usage
docker stats telegram-bot

# Check health status
docker inspect telegram-bot | grep -A 10 Health

# Monitor logs in real-time
docker-compose logs -f --tail=100 telegram-bot
```

## 🔧 Environment Variables

### Required
- `TELOXIDE_TOKEN` - Your Telegram bot token

### Optional
- `RUST_LOG` - Log level (debug, info, warn, error)
- `DOCKER_HOST` - Docker daemon address (defaults to socket)

### Production Environment
```bash
# Production .env example
TELOXIDE_TOKEN=your_bot_token_here
RUST_LOG=info
# Add any additional production configuration
```

## 🚨 Troubleshooting

### Common Issues

1. **Bot token not set:**
   ```
   Error: TELOXIDE_TOKEN not set
   ```
   **Solution**: Verify your `.env` file contains the correct token.

2. **Docker socket permission denied:**
   ```
   Error: Permission denied accessing Docker socket
   ```
   **Solution**: Ensure user is in docker group and socket is accessible.

3. **Container creation failures:**
   ```
   Error: Failed to create coding session container
   ```
   **Solution**: Check Docker daemon status and available resources.

4. **Resource exhaustion:**
   ```
   Error: Cannot allocate memory
   ```
   **Solution**: Increase resource limits or clean up unused containers.

### Debugging Commands

```bash
# Enter running container for debugging
docker exec -it telegram-bot /bin/bash

# Check Docker socket connectivity from inside container
docker exec telegram-bot docker ps

# View detailed logs with timestamps
docker-compose logs -f -t telegram-bot

# Restart specific service
docker-compose restart telegram-bot

# Check container resource usage
docker stats --no-stream telegram-bot

# Inspect container configuration
docker inspect telegram-bot
```

### Log Analysis
```bash
# Search for specific errors
docker-compose logs telegram-bot | grep -i error

# Monitor Docker API calls
docker-compose logs telegram-bot | grep -i docker

# Check session management
docker-compose logs telegram-bot | grep -i session
```

## 🏭 Production Deployment

### Pre-deployment Checklist
- [ ] Bot token configured securely
- [ ] Docker daemon running and accessible
- [ ] Resource limits appropriate for expected load
- [ ] Monitoring and alerting configured
- [ ] Log rotation configured
- [ ] Backup strategy for persistent data

### Production Best Practices
1. **Secrets Management**: Use Docker secrets or external secret management
2. **Log Management**: Configure log rotation and centralized collection
3. **Monitoring**: Implement application metrics and health monitoring
4. **Updates**: Automated builds and blue-green deployments
5. **Security**: Regular security updates and vulnerability scanning

### Scaling Strategy
```yaml
# Example scaling configuration
deploy:
  replicas: 3
  update_config:
    parallelism: 1
    order: start-first
  resources:
    limits:
      memory: 512M
      cpus: '2.0'
```

## 🔄 Maintenance

### Regular Tasks
```bash
# Update the bot image
docker-compose pull
docker-compose up -d

# Clean up unused containers and images
docker system prune -f

# Backup logs
tar -czf logs-backup-$(date +%Y%m%d).tar.gz logs/

# Check for security updates
docker scan telegram-claude-bot
```

### Performance Optimization
- Monitor container resource usage
- Optimize Docker image layers
- Implement container orchestration for high availability
- Configure resource quotas and limits

## 📝 Development Workflow

### Local Development
```bash
# Build and test locally
docker build -t telegram-claude-bot:dev .
docker run --rm -it telegram-claude-bot:dev

# Development with auto-reload
docker-compose -f docker-compose.dev.yml up
```

### CI/CD Integration
- Automated testing in containers
- Multi-architecture builds
- Security scanning in pipeline
- Automated deployment to staging/production
