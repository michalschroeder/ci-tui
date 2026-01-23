# CLAUDE.md

This file provides guidance to Claude Code (claude.ai/code) when working with code in this repository.

## Project Overview

CI-TUI is a terminal UI application for running CI checks on changed files. It detects git changes, determines which checks to run based on file patterns, and executes them in Docker containers with real-time output streaming.

## Code Validation (REQUIRED)

**IMPORTANT:** After writing or modifying code, ALWAYS run this command to validate:

```bash
docker run -it --rm -v /var/run/docker.sock:/var/run/docker.sock -v "$(pwd)":/app -e HOST_PWD="$(pwd)" -w /app ghcr.io/michalschroeder/ci-tui:latest --config ./ci-tui.yaml --simple
```

This runs ci-tui in simple mode (self-hosting) which executes:
- `cargo fmt` - Code formatting
- `cargo clippy` - Linter checks
- `cargo nextest run` - Test suite

This is the ONLY command Claude should use to validate code changes. Do not use `make test`, `cargo test`, or other commands directly.

## Other Development Commands

```bash
# Build Docker image locally
make build

# Run with local cargo (requires Rust installed)
cargo run -- --config <path-to-config.yaml>
```

## Architecture

### Core Flow
1. **main.rs** - CLI entry point using clap. Loads config, detects git changes, determines checks, launches UI or simple mode
2. **config.rs** - YAML config parsing with `CiConfig` as the root type. Uses `IndexMap` to preserve YAML key ordering for group execution order
3. **git.rs** - Git change detection comparing against base branch (tries `origin/{base}`, `{base}`, fallback in order)
4. **checks.rs** - `determine_checks()` matches changed files against file patterns and test discovery rules to build `CheckToRun` list
5. **runner.rs** - `CheckRunner` executes checks via `docker compose exec` with event streaming through mpsc channels
6. **ui/mod.rs** - Main TUI event loop using ratatui. Keyboard input runs on dedicated OS thread for responsiveness under high CPU load

### Key Design Patterns

**Check Execution Groups**: Groups defined in config execute sequentially. Within a group, `parallel: true` runs checks concurrently. Groups can have `pre_commands` (e.g., DB init) that run before checks.

**Test Discovery**: Two strategies for finding related tests when source files change:
- `path_mapping`: Maps source paths to test paths (e.g., `src/{path}.php` → `tests/{path}Test.php`)
- `grep_search`: Searches test directories for content patterns with placeholders (`{basename}`, `{filename}`, etc.)

**On-Demand Checks**: Checks marked `on_demand: true` don't run automatically - user triggers with 't' key. Used for expensive tests when no specific test files are found.

**Event-Driven UI**: Runner sends `RunnerEvent`s through channel. UI processes events non-blocking with `try_recv()`, limiting events per frame to prevent starvation.

### Module Responsibilities

- **config.rs**: All config structs (`CiConfig`, `GroupConfig`, `CheckDefinition`, `TestDiscoveryConfig`), YAML deserialization, pattern compilation caching
- **checks.rs**: `CheckToRun` struct, `determine_checks()` logic, `group_checks()` for execution grouping
- **runner.rs**: `CheckRunner`, `CheckResult`, `CheckStatus` enum, Docker command execution, `RunnerEvent` variants
- **test_discovery.rs**: `find_related_tests()`, path mapping, grep search, placeholder expansion
- **ui/app.rs**: `App` state struct with all UI state (selected check, results, filters, etc.)
- **ui/dashboard.rs**: Rendering logic using ratatui widgets
- **simple.rs**: Non-TUI console output mode for CI pipelines

## Config File Structure

The tool expects a YAML config with:
- `docker.project_dir`: Path for `docker compose --project-directory`
- `docker.service`: Default container service name
- `git.base_branch` / `git.fallback_branch`: For change detection
- `file_patterns`: Named regex patterns with optional colors
- `checks`: Groups containing check definitions with triggers
- `ignore_patterns`: Regex patterns for files to exclude from change detection
