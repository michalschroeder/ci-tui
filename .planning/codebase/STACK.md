# Technology Stack

**Analysis Date:** 2026-01-22

## Languages

**Primary:**
- Rust (2021 edition) - All core application code and TUI implementation

## Runtime

**Environment:**
- Rust compiler (rustc) - For compilation
- Alpine Linux 3.21 - Runtime container environment

**Package Manager:**
- Cargo - Rust dependency and build management
- Lockfile: `Cargo.lock` (present and committed)

## Frameworks & Core Libraries

**TUI & Terminal UI:**
- ratatui 0.30 - Terminal user interface framework with rendering widgets
- crossterm 0.28 - Cross-platform terminal event handling with feature `event-stream`
- ansi-to-tui 8 - ANSI color sequence parsing for output display

**Async Runtime:**
- tokio 1.x - Multi-threaded async runtime with features:
  - `rt-multi-thread` - Multi-threaded executor
  - `macros` - Async macro support
  - `process` - Child process management for Docker execution
  - `sync` - MPSC channels for event streaming
  - `time` - Timer utilities
  - `io-util` - I/O utilities

**CLI Framework:**
- clap 4.x - Command-line argument parsing with derive macros

**Configuration:**
- serde 1.x - Serialization framework with derive support
- serde_yaml 0.9 - YAML deserialization for config files
- indexmap 2.x - Ordered map that preserves YAML key ordering (critical for check group execution order)

**Error Handling:**
- thiserror 2.x - Error type derivation macros
- anyhow 1.x - Error context and wrapping
- color-eyre 0.6 - Panic and error formatting with color output

**Utilities:**
- regex 1.x - Regular expression matching for file patterns and grep search
- chrono 0.4 - DateTime handling for check timing
- base64 0.22 - Base64 encoding (for future use)
- sysinfo 0.32 - System information and monitoring

## Build Configuration

**Compiler Optimization (Release Profile):**
- opt-level: 3 - Full optimization
- lto: true - Link-time optimization
- strip: true - Strip debug symbols from binary
- codegen-units: 1 - Single codegen unit for better optimization
- panic: abort - Panic immediately without unwinding

## Docker Build

**Multi-stage Dockerfile:** `Dockerfile`
- Build stage: `rust:alpine` - Alpine Linux with Rust toolchain for static musl binary
- Runtime stage: `alpine:3.21` - Minimal Alpine Linux container
- Build caching: Cargo registry and git source caches mounted
- Static linking: Produces fully static musl binary for Alpine compatibility

**Runtime Dependencies in Container:**
- git - Change detection via git CLI
- docker-cli - Docker command access
- docker-cli-compose - Docker Compose plugin for container management

## Configuration Files

**YAML Configuration:**
- Location: Default config path `./tools/ci/ci-config.yaml` (configurable via CLI)
- Format: YAML with structured schema for:
  - Docker settings (project directory, service name, environment variables)
  - Git settings (base branch, fallback branch)
  - File patterns (regex with optional UI colors)
  - Check groups (execution order preserved by IndexMap)
  - Check definitions (command, triggers, test discovery strategies)
  - Ignore patterns (regex to exclude files from analysis)

**CLI Arguments:**
- `--config <PATH>` - Path to configuration file (required)
- `--simple` - Run in simple console mode instead of TUI

## Platform Requirements

**Development:**
- Rust 1.70+ (2021 edition required)
- Cargo
- Docker with BuildKit support (for build process)
- Git (for repository operations)

**Production:**
- Docker runtime - Application runs inside containers
- Docker Compose - For executing checks within services
- Git - For change detection within container
- Mounted project root - Application requires access to git repository
- Mounted Docker socket - Allows Docker commands inside container (`/var/run/docker.sock`)

## Build Process

**Local Build:**
```bash
make build              # Builds Docker image with tag ci-tui:local
DOCKER_BUILDKIT=1 docker build -t ci-tui:local .
```

**Docker Build Features:**
- BuildKit enabled for faster, more reliable builds
- Two-stage build reduces final image size significantly
- Dependency layer caching (only invalidates on Cargo.toml/lock changes)
- Source code layer cached separately (cache invalidated on source changes)

## Entry Points

**Binary:**
- `src/main.rs` - CLI entry point using clap parser
  - Initializes color-eyre error handling
  - Parses CLI arguments
  - Auto-detects terminal for TUI vs. simple mode
  - Orchestrates config loading, git analysis, and check determination

**Library:**
- `src/lib.rs` - Public API exports key modules and types for programmatic use

---

*Stack analysis: 2026-01-22*
