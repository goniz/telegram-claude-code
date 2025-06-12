#!/bin/bash

# Test script for docker-compose.yml validation
# This script validates the docker-compose configuration and tests basic functionality
# Usage: ./test-docker-compose.sh [--use-published-image]

set -e

USE_PUBLISHED_IMAGE=false
COMPOSE_FILE="docker-compose.yml"

# Parse command line arguments
while [[ $# -gt 0 ]]; do
  case $1 in
    --use-published-image)
      USE_PUBLISHED_IMAGE=true
      COMPOSE_FILE="docker-compose.test.yml"
      shift
      ;;
    *)
      echo "Unknown option $1"
      echo "Usage: $0 [--use-published-image]"
      exit 1
      ;;
  esac
done

if [ "$USE_PUBLISHED_IMAGE" = true ]; then
    echo "🔍 Testing docker-compose.test.yml configuration with published image..."
else
    echo "🔍 Testing docker-compose.yml configuration with source build..."
fi

# Create test environment if it doesn't exist
if [ ! -f .env ]; then
    echo "📝 Creating test environment file..."
    echo "TELOXIDE_TOKEN=test_token_for_validation" > .env
    echo "RUST_LOG=debug" >> .env
fi

# Test 1: Validate syntax
echo "✅ Test 1: Validating Docker Compose syntax..."
docker compose -f "$COMPOSE_FILE" config --quiet
echo "✅ Syntax validation passed"

# Test 2: Check configuration structure
echo "✅ Test 2: Checking configuration structure..."
CONFIG=$(docker compose -f "$COMPOSE_FILE" config)

# Check required sections exist
echo "$CONFIG" | grep -q "telegram-bot" || { echo "❌ Service 'telegram-bot' not found"; exit 1; }
echo "$CONFIG" | grep -q "volumes" || { echo "❌ Volumes section not found"; exit 1; }
echo "$CONFIG" | grep -q "healthcheck" || { echo "❌ Healthcheck not found"; exit 1; }
echo "$CONFIG" | grep -q "environment" || { echo "❌ Environment section not found"; exit 1; }
echo "$CONFIG" | grep -q "deploy" || { echo "❌ Deploy/resources section not found"; exit 1; }

echo "✅ Configuration structure validation passed"

# Test 3: Check environment variable handling
echo "✅ Test 3: Testing environment variable handling..."
# Test required variable validation by creating a minimal env file without the token
TMP_ENV_FILE=$(mktemp)
echo "RUST_LOG=debug" > "$TMP_ENV_FILE"
if docker compose -f "$COMPOSE_FILE" --env-file "$TMP_ENV_FILE" config 2>&1 | grep -q "required variable TELOXIDE_TOKEN is missing"; then
    echo "✅ Required environment variable validation working"
else
    echo "❌ Required environment variable validation failed"
    rm "$TMP_ENV_FILE"
    exit 1
fi
rm "$TMP_ENV_FILE"

# Test environment variable substitution
export TELOXIDE_TOKEN="test_validation_token"
if docker compose -f "$COMPOSE_FILE" config | grep -q "test_validation_token"; then
    echo "✅ Environment variable substitution working"
else
    echo "❌ Environment variable substitution failed"
    exit 1
fi
unset TELOXIDE_TOKEN

# Test 4: Validate resource limits
echo "✅ Test 4: Validating resource limits..."
if echo "$CONFIG" | grep -q "memory.*268435456"; then
    echo "✅ Memory limit (256M) correctly set"
else
    echo "❌ Memory limit not properly configured"
    exit 1
fi

if echo "$CONFIG" | grep -q "cpus.*0.5"; then
    echo "✅ CPU limit (0.5) correctly set"
else
    echo "❌ CPU limit not properly configured"
    exit 1
fi

# Test 5: Check volumes configuration
echo "✅ Test 5: Checking volumes configuration..."
if echo "$CONFIG" | grep -q "/var/run/docker.sock"; then
    echo "✅ Docker socket mount configured"
else
    echo "❌ Docker socket mount missing"
    exit 1
fi

if echo "$CONFIG" | grep -q "/app/logs"; then
    echo "✅ Logs volume mount configured"
else
    echo "❌ Logs volume mount missing"
    exit 1
fi

# Test 6: Check healthcheck configuration
echo "✅ Test 6: Validating healthcheck configuration..."
if echo "$CONFIG" | grep -q "interval.*30s"; then
    echo "✅ Healthcheck interval correctly set"
else
    echo "❌ Healthcheck interval not properly configured"
    exit 1
fi

if echo "$CONFIG" | grep -q "start_period.*15s"; then
    echo "✅ Healthcheck start period correctly set"
else
    echo "❌ Healthcheck start period not properly configured"
    exit 1
fi

echo ""
echo "🎉 All docker-compose.yml validation tests passed!"
echo ""
echo "📋 Configuration Summary:"
echo "  - File tested: $COMPOSE_FILE"
if [ "$USE_PUBLISHED_IMAGE" = true ]; then
    echo "  - Image source: Published GitHub image (ghcr.io/goniz/telegram-claude-code:main)"
else
    echo "  - Image source: Built from source"
fi
echo "  - Syntax: Valid"
echo "  - Environment variables: Properly handled with defaults"
echo "  - Resource limits: 256M memory, 0.5 CPU"
echo "  - Volumes: Logs and Docker socket mounted"
echo "  - Healthcheck: Enhanced with better monitoring"
echo "  - Best practices: Version field removed, improved structure"