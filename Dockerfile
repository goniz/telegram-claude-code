# Multi-stage Dockerfile for Telegram Bot in Rust

# Stage 1: Build stage
FROM rust:1.87-slim AS builder

# Install build dependencies
RUN apt-get update && apt-get install -y \
    pkg-config \
    libssl-dev \
    ca-certificates \
    && update-ca-certificates \
    && rm -rf /var/lib/apt/lists/*

# Create app directory
WORKDIR /app

# Copy dependency files and source code
COPY Cargo.toml Cargo.lock ./
COPY src ./src

# Build the application
ENV CARGO_NET_GIT_FETCH_WITH_CLI=true
ENV CARGO_HTTP_CHECK_REVOKE=false
RUN cargo build --release

# Stage 2: Runtime stage
FROM debian:bookworm-slim

# Install runtime dependencies
RUN apt-get update && apt-get install -y \
    ca-certificates \
    libssl3 \
    && rm -rf /var/lib/apt/lists/*

# Create a non-root user
RUN useradd -r -s /bin/false -m -d /app telegram-bot

# Set working directory
WORKDIR /app

# Copy the binary from builder stage
COPY --from=builder /app/target/release/telegram-bot /app/telegram-bot

# Change ownership to the telegram-bot user
RUN chown -R telegram-bot:telegram-bot /app

# Switch to non-root user
USER telegram-bot

# Set environment variables
ENV RUST_LOG=info

# Expose port (if needed for webhooks in the future)
EXPOSE 8080

# Health check
HEALTHCHECK --interval=30s --timeout=10s --start-period=5s --retries=3 \
    CMD pgrep -x telegram-bot || exit 1

# Run the bot
CMD ["./telegram-bot"]
