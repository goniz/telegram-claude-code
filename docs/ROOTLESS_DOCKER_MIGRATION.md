# Rootless Docker-in-Docker Migration Guide

## Overview

This document describes the migration from the OpenAI Codex Universal base image to an official rootless Docker-in-Docker base image for enhanced security and standardization.

## Changes Made

### Base Image Migration

**Before:**
```dockerfile
FROM ghcr.io/openai/codex-universal:latest
```

**After:**
```dockerfile
FROM docker:25.0-dind-rootless
```

### Simplified Architecture

The new approach focuses on simplicity and security:

1. **Official Base Image**: Using Docker's official rootless DinD image
2. **Alpine Package Manager**: Using apk for clean, minimal installations
3. **No Custom Entrypoint**: Leverages the base image's existing functionality
4. **System Package Versions**: Using Alpine's package versions instead of custom installs

### Development Environment

The refactored image provides essential development tools:

- **Node.js**: Alpine's nodejs and npm packages
- **Python**: Python 3 with pip and virtualenv
- **Rust**: Installed via rustup (configurable version via `RUST_VERSION` build arg, default: 1.87.0)
- **Go**: Installed from official source (configurable version via `GO_VERSION` build arg, default: 1.23.4)
- **GitHub CLI**: For repository operations
- **Claude Code**: Pre-installed globally via npm
- **Build Tools**: gcc, g++, make, cmake, musl-dev, linux-headers

### Security Improvements

1. **Rootless Operation**: Runs as UID 1000 (non-root) by default
2. **Official Base Image**: Docker's maintained and supported image
3. **Minimal Attack Surface**: Only essential packages installed
4. **No Custom Scripts**: Reduces complexity and potential vulnerabilities

### Compatibility

The image maintains essential compatibility:

- **Working Directory**: `/workspace`
- **Claude Code**: Available globally
- **Environment Variables**: Claude-specific telemetry/update controls

## Testing

### Build the Image

**Default versions:**
```bash
docker build -f Dockerfile.runtime -t telegram-claude-code-runtime:rootless .
```

**Custom versions:**
```bash
docker build -f Dockerfile.runtime \
  --build-arg RUST_VERSION=1.76.0 \
  --build-arg GO_VERSION=1.22.0 \
  -t telegram-claude-code-runtime:rootless .
```

**Available build arguments:**
- `RUST_VERSION`: Rust toolchain version (default: 1.87.0)
- `GO_VERSION`: Go version (default: 1.23.4)

### Quick Test

```bash
docker run --rm -it telegram-claude-code-runtime:rootless bash
```

### Verify Tools

```bash
# Test development tools
node --version
python3 --version
go version
rustc --version
gh --version
claude --version
```

## Container Requirements

### Privileged Mode

Docker-in-Docker functionality requires `--privileged`:

```bash
docker run --privileged -d telegram-claude-code-runtime:rootless
```

### User Context

The container runs as UID 1000 (rootless user) by default.

## Migration Benefits

1. **Enhanced Security**: Rootless operation by default
2. **Simplified Maintenance**: Standard Alpine package management
3. **Reduced Complexity**: No custom entrypoint or environment scripts
4. **Official Support**: Docker-maintained base image
5. **Smaller Attack Surface**: Minimal package installation

## Breaking Changes

### Removed Features

- **NVM and Multiple Node Versions**: Now uses single Alpine Node.js version
- **Custom Environment Setup**: No complex PATH or version management
- **Custom Entrypoint Script**: Uses base image defaults
- **Specific Tool Versions**: Uses Alpine package versions

### Impact Assessment

Most applications should work without changes, but consider:

- **Node.js Version**: May differ from previous NVM-managed versions
- **Tool Versions**: Alpine package versions vs. custom installations
- **Environment Variables**: Simplified environment setup

## Rollback Plan

If issues occur:

1. Revert `Dockerfile.runtime` to original content
2. Rebuild and redeploy the runtime image
3. Test with existing containers

## Next Steps

1. **Build Testing**: Verify the image builds successfully
2. **Integration Testing**: Test with the Telegram bot application
3. **Performance Validation**: Compare container startup and runtime performance
4. **Security Review**: Validate rootless operation and security posture