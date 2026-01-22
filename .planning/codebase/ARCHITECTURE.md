# Architecture

**Analysis Date:** 2026-01-22

## Pattern Overview

**Overall:** Event-Driven Layered Architecture with TUI and Console Outputs

**Key Characteristics:**
- Event-driven communication between execution layer (Docker) and UI layer
- Two rendering modes (TUI via ratatui, simple console) sharing core logic
- Git-based file change detection triggers selective check execution
- Grouped check execution with optional parallelization
- Test discovery strategies for mapping source changes to test files
- Dedicated OS thread for keyboard input to prevent starvation under high CPU load

## Layers

**Configuration Layer:**
- Purpose: Parse and cache YAML config, compile regex patterns
- Location: `src/config.rs`
- Contains: `CiConfig`, `GroupConfig`, `CheckDefinition`, `TestDiscoveryStrategy` types
- Depends on: serde_yaml, regex, indexmap
- Used by: main.rs, checks.rs, runner.rs

**Git & File Detection Layer:**
- Purpose: Detect changed files, apply ignore patterns, determine file extensions
- Location: `src/git.rs`
- Contains: `ChangedFiles` struct with filtering methods
- Depends on: git command execution via std::process::Command
- Used by: main.rs, checks.rs

**Check Determination Layer:**
- Purpose: Match changed files against triggers, discover related tests, resolve commands
- Location: `src/checks.rs`
- Contains: `CheckToRun` struct, `determine_checks()` function, file pattern matching logic
- Depends on: git.rs, test_discovery.rs, config.rs
- Used by: main.rs, ui/mod.rs, simple.rs, runner.rs

**Test Discovery Layer:**
- Purpose: Find test files related to source changes using path mapping or grep search
- Location: `src/test_discovery.rs`
- Contains: `find_related_tests()`, path extraction logic, grep search implementation
- Depends on: std::process::Command for grep execution
- Used by: checks.rs

**Execution/Runner Layer:**
- Purpose: Execute checks in Docker containers, stream output, manage check lifecycle
- Location: `src/runner.rs`
- Contains: `CheckRunner`, `CheckResult`, `CheckStatus`, `RunnerEvent` enum
- Depends on: tokio, std::process::Command for docker compose exec
- Used by: ui/mod.rs, simple.rs

**TUI Layer:**
- Purpose: Interactive terminal UI with real-time updates and keyboard control
- Location: `src/ui/mod.rs`, `src/ui/app.rs`, `src/ui/dashboard.rs`
- Contains: Event loop, app state management, ratatui rendering
- Depends on: runner.rs, crossterm, ratatui, tokio
- Used by: main.rs

**Simple Console Layer:**
- Purpose: Non-interactive console output for CI pipelines and headless execution
- Location: `src/simple.rs`
- Contains: Sequential/parallel check execution with console output
- Depends on: runner.rs, checks.rs
- Used by: main.rs

## Data Flow

**Initialization Flow:**
1. `main.rs` - Parse CLI args, load config from YAML
2. `config.rs` - Deserialize YAML, cache regex patterns in `OnceLock`
3. `git.rs` - Detect changed files against base branch (tries multiple refs)
4. `checks.rs` - Match files against file patterns and test discovery rules
5. Branch: Simple mode → `simple.rs` | TUI mode → `ui/mod.rs`

**Check Execution Flow (TUI):**
1. `ui/mod.rs` - Spawn `CheckRunner` in tokio task with event channel
2. `runner.rs` - Group checks by execution group, run pre-commands
3. `runner.rs` - For each group: run checks sequentially or in parallel
4. `runner.rs` - Emit `RunnerEvent` (Started, Output, Finished) via mpsc channel
5. `ui/mod.rs` - Non-blocking `try_recv()` on events, limit to 50/frame to prevent starvation
6. `ui/app.rs` - Update app state with results
7. `ui/dashboard.rs` - Render updated state with ratatui

**Check Execution Flow (Simple):**
1. `simple.rs` - Group checks from `checks::group_checks()`
2. For each group: run checks sequentially or in parallel via `tokio::task::spawn()`
3. Print results to stdout with color codes
4. Print summary and exit

**Keyboard Input Handling (TUI):**
1. Dedicated OS thread spawned in `ui/mod.rs::spawn_keyboard_thread()`
2. Thread polls `crossterm::event::poll()` at 50ms intervals
3. Sends events via unbounded `std::sync::mpsc::channel()` (never drops events)
4. Main event loop receives keyboard events and dispatches actions

**State Management:**
- App state centralized in `ui/app.rs::App` struct
- Results stored in `HashMap<check_id, CheckResult>`
- Pre-command output in `Vec<PreCommandState>`
- UI state (selection, scroll, filters) also in App
- Event-driven updates: runner emits event → app processes → dashboard renders

## Key Abstractions

**CheckToRun:**
- Purpose: Resolved check with all placeholders expanded
- Examples: `src/checks.rs` lines 7-25
- Pattern: Contains original definition + computed fields (resolved_command, service, files)
- Created by: `determine_checks()` after pattern matching

**CheckRunner:**
- Purpose: Encapsulates Docker execution of checks
- Examples: `src/runner.rs` lines 125-140
- Pattern: Async struct with Arc-wrapped config and project path
- Interface: `run_checks()` returns Result, emits events via channel

**RunnerEvent Enum:**
- Purpose: Communication channel between runner and UI
- Examples: `src/runner.rs` lines 100-117
- Pattern: Discriminated union of event types (CheckStarted, CheckOutput, CheckFinished, etc.)
- Used by: TUI receives events, simple mode can ignore streaming output

**App State:**
- Purpose: Complete UI application state
- Examples: `src/ui/app.rs` lines 50-85
- Pattern: Single-source-of-truth for all UI data, checkbox results, filters, selections
- Updated by: Event handler processes `RunnerEvent` and updates App fields

**ChangedFiles:**
- Purpose: Encapsulates detected changes with filtering methods
- Examples: `src/git.rs` lines 9-68
- Pattern: List of files + base_ref, with filter_by_pattern(), extensions(), apply_ignore_patterns()

**CheckStatus Enum:**
- Purpose: Lifecycle states during execution
- Examples: `src/runner.rs` lines 23-36
- Pattern: Six states (Pending, Running, Passed, Failed, Skipped, OnDemand)
- Used by: CheckResult.status field, UI rendering colors/icons

## Entry Points

**CLI Entry:**
- Location: `src/main.rs`
- Triggers: User runs `ci-tui --config path/to/config.yaml`
- Responsibilities: Parse args, load config, detect changes, determine checks, dispatch to simple or TUI mode

**TUI Entry:**
- Location: `src/ui/mod.rs::run()`
- Triggers: stdout is terminal, or `--simple` flag not set
- Responsibilities: Initialize terminal, spawn keyboard thread, spawn runner task, event loop

**Simple Mode Entry:**
- Location: `src/simple.rs::run()`
- Triggers: stdout not terminal, or `--simple` flag set
- Responsibilities: Execute checks sequentially/in parallel, print output, exit with status

**Config Loading:**
- Location: `src/config.rs::load_config()`
- Triggers: Called from main before any execution
- Responsibilities: Read YAML file, deserialize, validate structure

## Error Handling

**Strategy:** Result-based error propagation with anyhow for context

**Patterns:**
- `anyhow::Result<T>` used throughout public APIs
- `.context()` added at layer boundaries for diagnostic messages
- Git operations fallback gracefully (tries multiple refs, returns empty if all fail)
- Regex compilation errors caught and ignored (invalid patterns don't crash)
- Docker command errors caught, status set to Failed, output captured
- Channel send errors silently ignored (receiver dropped during shutdown)

## Cross-Cutting Concerns

**Logging:** No structured logging framework. Console output in simple.rs, debug prints in runner. Future: tracing crate recommended.

**Validation:** Regex patterns compiled with error handling. File paths validated by filesystem checks during test discovery. Docker service names not validated against actual services.

**Authentication:** None required. Docker access assumes local daemon available and user has permission.

**Concurrency:** Tokio multi-threaded runtime for I/O-heavy operations (Docker exec). OS thread for keyboard input to prevent starvation. Channels use Arc for shared state. No mutex locking needed due to channel-based communication.

---

*Architecture analysis: 2026-01-22*
