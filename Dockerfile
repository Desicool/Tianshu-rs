# Multi-stage Dockerfile for Agent Workspace Rust
# Stage 1: Builder
FROM rust:1.94.0-alpine3.22 AS builder

# Install build dependencies
RUN apk add --no-cache musl-dev openssl-dev openssl-libs-static pkgconfig g++ make

WORKDIR /usr/src/app

# Copy the Rust workspace.
COPY agent_workspace_rust/ .

# Build the entrypoint crate in release mode
RUN cargo build --release -p entrypoint

# Stage 2: Runtime
FROM alpine:latest

# Install runtime dependencies
RUN apk add --no-cache openssl libgcc ca-certificates

# Create a non-root user
RUN adduser -D -u 1000 appuser
WORKDIR /app
RUN mkdir -p /app/config && chown -R appuser:appuser /app

# Copy the binary from builder
COPY --from=builder --chown=appuser:appuser /usr/src/app/target/release/entrypoint /app/agent-workspace-rust
COPY --chown=appuser:appuser common/conf/config_files/data_config.yml /app/config/data_config.yml

# Use the non-root user
USER appuser

# Set environment variables
ENV RUST_LOG=info \
    DATA_CONFIG_PATH=/app/config/data_config.yml

# Entry point
ENTRYPOINT ["/app/agent-workspace-rust"]
