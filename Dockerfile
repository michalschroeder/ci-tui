# syntax=docker/dockerfile:1.4
# Version arguments (global scope)
ARG ALPINE_VERSION=3.21
ARG CI_TUI_GIT_HASH=unknown
ARG CI_TUI_BUILD_DATE=unknown

# Build stage - use Alpine for static musl build
FROM rust:alpine AS builder

# Re-declare ARGs after FROM (Docker scope rules)
ARG CI_TUI_GIT_HASH
ARG CI_TUI_BUILD_DATE

WORKDIR /build

# Install build dependencies for Alpine
RUN apk add --no-cache musl-dev git

# Copy manifest, lockfile, and build script
COPY Cargo.toml ./
COPY Cargo.lock* ./
COPY build.rs ./

# Create dummy src to build dependencies (will be replaced)
RUN mkdir src && echo "fn main() {}" > src/main.rs

# Build dependencies only (cached unless Cargo.toml changes)
RUN --mount=type=cache,target=/root/.cargo/registry \
    --mount=type=cache,target=/root/.cargo/git \
    cargo build --release 2>/dev/null || true

# Now copy actual source (this invalidates cache when src changes)
COPY src ./src

# Touch main.rs to ensure rebuild (source files are newer than cached artifacts)
RUN touch src/main.rs src/lib.rs 2>/dev/null || touch src/main.rs

# Build the actual application
# NOTE: NOT using cache mount for /build/target to ensure source changes are detected
# Pass version info as env vars for build.rs to use when git is unavailable
RUN --mount=type=cache,target=/root/.cargo/registry \
    --mount=type=cache,target=/root/.cargo/git \
    CI_TUI_GIT_HASH=${CI_TUI_GIT_HASH} CI_TUI_BUILD_DATE=${CI_TUI_BUILD_DATE} \
    cargo build --release && \
    cp /build/target/release/ci-tui /tmp/ci-tui

# Runtime stage - Alpine is small and has musl
ARG ALPINE_VERSION
FROM alpine:${ALPINE_VERSION}

# OCI labels
LABEL org.opencontainers.image.title="CI TUI"
LABEL org.opencontainers.image.description="TUI for running CI checks"
LABEL org.opencontainers.image.source="https://github.com/lendable/us-loans-backend"

# Install runtime dependencies (git for change detection, docker CLI with compose plugin)
RUN apk add --no-cache git docker-cli docker-cli-compose

# Mark any directory as safe for git (needed for mounted volumes)
RUN git config --global --add safe.directory '*'

WORKDIR /app

# Copy the binary
COPY --from=builder /tmp/ci-tui /usr/local/bin/ci-tui

# Default command
ENTRYPOINT ["ci-tui"]
