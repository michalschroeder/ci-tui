# CLAUDE.md

This file provides guidance to Claude Code (claude.ai/code) when working with code in this repository.

## Project Overview

CI-TUI is a terminal UI application for running CI checks on changed files. It detects git changes, determines which checks to run based on file patterns, and executes them in Docker containers with real-time output streaming.

## Code Validation (REQUIRED)

**IMPORTANT:** After writing or modifying code, ALWAYS run these commands to validate.

Requires the dev image — build it once with `make build-dev`.

### Step 1: Auto-fix (before commits)

```bash
make fmt
```

This runs `cargo fmt` to fix formatting issues automatically.

### Step 2: Validate

```bash
make ci
```

This runs all CI checks in Docker (using the dev image):
- `cargo fmt --check` - Code formatting check
- `cargo clippy -- -D warnings` - Linter checks
- `cargo nextest run` - Test suite

**Workflow:** Run `make fmt` first, then `make ci` to validate. Both must pass before any commit or push.

These are the ONLY commands Claude should use to validate code changes. Do not use `cargo test`, `cargo clippy`, or other commands directly.

### Failure Handling

If validation commands fail or do not pass after **3 attempts**, STOP and ask the user:

1. **Continue with 3 more retries?** - Keep trying to fix the issue
2. **Need more context?** - User provides additional information about the problem
3. **Stop here?** - Abandon the current approach and discuss alternatives

Do not keep retrying indefinitely. After 3 failed attempts, always pause and ask for guidance.

## Other Development Commands

```bash
# Build dev image (required for make ci/fmt/test/clippy)
make build-dev

# Build production Docker image locally
make build

# Run individual checks
make test        # Run tests only
make fmt-check   # Check formatting without fixing
make clippy      # Run clippy only

# Run with local cargo (requires Rust installed)
cargo run -- --config <path-to-config.yaml>
```

## Versioning (Release Please)

This project uses [Release Please](https://github.com/googleapis/release-please) for automated versioning and changelog generation.

**How it works:**
1. Push commits to `master` with [Conventional Commits](https://www.conventionalcommits.org/) format
2. CI runs → Release Please creates/updates a release PR
3. Merging the release PR triggers version bump, CHANGELOG.md update, and Docker image build

**Version bumps based on commit types:**
- `feat:` → Minor version bump (0.1.x → 0.2.0)
- `fix:` → Patch version bump (0.1.8 → 0.1.9)
- `feat!:` or `BREAKING CHANGE:` → Major version bump (0.x → 1.0.0)

**Do NOT:**
- Manually create git tags (Release Please manages them)
- Edit `Cargo.toml` version field directly
- Edit `CHANGELOG.md` directly

**Config files:**
- `release-please-config.json` - Release Please configuration
- `.release-please-manifest.json` - Current version tracking

## Test Fixtures

Test configs are centralized in `tests/common/configs.rs`:

- **Fixtures**: Use `minimal_config()`, `php_project_config()`, `rust_project_config()` for common scenarios
- **Builder**: Use `ConfigBuilder::new().with_*().build()` for custom configs
- **Inline**: Keep truly unique edge-case configs inline with `// Edge case: ...` comment

New tests should require <10 lines of config setup.

### Test Architecture Note

**Integration tests** (`tests/*.rs`): Import shared fixtures from `tests/common/configs.rs`

**Library tests** (`src/*.rs #[cfg(test)]`): Use inline structured fixtures. Cannot import from `tests/common/` due to cargo fmt limitations in Docker - the `#[path = "..."]` directive breaks when cargo fmt runs inside containers. This is a known Rust tooling limitation.

The inline fixtures in `src/config.rs` and `src/checks.rs` mirror the structure of `tests/common/configs.rs` but are defined locally. When adding new library tests, follow the existing inline fixture pattern in that module.

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
