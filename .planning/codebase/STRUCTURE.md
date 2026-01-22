# Codebase Structure

**Analysis Date:** 2026-01-22

## Directory Layout

```
ci-tui/
├── src/                    # Rust source code
│   ├── main.rs            # CLI entry point using clap
│   ├── lib.rs             # Library root, module exports
│   ├── config.rs          # YAML config parsing and types
│   ├── git.rs             # Git change detection
│   ├── checks.rs          # Check determination and file pattern matching
│   ├── runner.rs          # Docker execution and event streaming
│   ├── test_discovery.rs  # Test file discovery strategies
│   ├── simple.rs          # Non-TUI console output mode
│   └── ui/                # Terminal UI implementation
│       ├── mod.rs         # Main TUI event loop and keyboard handling
│       ├── app.rs         # Application state struct
│       └── dashboard.rs   # Rendering logic using ratatui
├── Cargo.toml             # Rust package manifest
├── Cargo.lock             # Dependency lock file
├── Dockerfile             # Docker image for build/test environment
├── Makefile               # Build commands (test, clippy, fmt, check)
├── run.sh                 # Docker run wrapper script
├── CLAUDE.md              # Project instructions for Claude
└── .planning/
    └── codebase/          # Analysis documents (this directory)
```

## Directory Purposes

**src/:**
- Purpose: All Rust source code for library and binary
- Contains: Modules for config, git ops, check logic, execution, UI
- Key files: `main.rs` (entry), `lib.rs` (exports)

**src/ui/:**
- Purpose: Terminal UI implementation using ratatui and crossterm
- Contains: Event loop, app state, rendering logic
- Key files: `mod.rs` (event loop), `app.rs` (state), `dashboard.rs` (rendering)

**.planning/codebase/:**
- Purpose: Architecture and structure analysis documents
- Contains: ARCHITECTURE.md, STRUCTURE.md, CONVENTIONS.md, TESTING.md, CONCERNS.md (as created)
- Generated: Yes (created by GSD mapping)
- Committed: Yes

## Key File Locations

**Entry Points:**
- `src/main.rs`: Binary entry point, CLI parsing, orchestration of config load → change detection → check determination → mode selection
- `src/ui/mod.rs::run()`: TUI mode function, initializes terminal and event loop
- `src/simple.rs::run()`: Simple console mode function, sequential/parallel execution with text output

**Configuration:**
- `src/config.rs`: All config structs (CiConfig, GroupConfig, CheckDefinition, TestDiscoveryStrategy, PathMappingRule)
- Cargo.toml: Runtime dependencies and build profile settings

**Core Logic:**
- `src/git.rs`: ChangedFiles detection and filtering
- `src/checks.rs`: CheckToRun determination, command resolution, check grouping
- `src/test_discovery.rs`: Path mapping and grep search for test files
- `src/runner.rs`: CheckRunner, Docker execution, event emission

**Testing:**
- `src/runner.rs`: Contains run_checks(), run_parallel(), run_sequential() functions
- `src/checks.rs`: determine_checks(), group_checks() functions
- Search for `.rs` files with `#[test]` or integration test patterns (if they exist)

## Naming Conventions

**Files:**
- Module files: `module_name.rs` (e.g., `config.rs`, `git.rs`)
- Submodule directory: `ui/mod.rs` contains submodule code, child files are `app.rs`, `dashboard.rs`
- Main binary: `main.rs`
- Library root: `lib.rs`

**Directories:**
- Lowercase with underscores: `src/ui/` (no underscores since single word)
- Modules that are single files use `module_name.rs` pattern
- Modules with multiple files use `module_name/mod.rs` pattern (src/ui/)

**Functions:**
- snake_case: `determine_checks()`, `detect_changes()`, `run_checks()`, `apply_ignore_patterns()`
- Async functions have no special prefix: `run()`, `run_checks()` (use `async fn`)

**Structs:**
- PascalCase: `CiConfig`, `CheckToRun`, `CheckRunner`, `ChangedFiles`, `CheckResult`, `App`
- Config structs: `CiConfig`, `DockerConfig`, `GitConfig`, `GroupConfig`, `CheckDefinition`
- Enum types: `CheckStatus`, `CheckTriggers`, `TestDiscoveryStrategy`, `RunnerEvent`, `StatusFilter`

**Constants:**
- SCREAMING_SNAKE_CASE: `MAX_HISTORY_SAMPLES`, `STATS_UPDATE_INTERVAL`, `KEYBOARD_POLL_TIMEOUT`, `MAX_RUNNER_EVENTS_PER_FRAME`, `FRAME_DURATION`

**Type Parameters:**
- Single letter or short name: None in current codebase
- Lifetimes: `'a` in SelectableItem<'a>

## Where to Add New Code

**New Feature - Check Execution Enhancement:**
- Modify check logic: `src/checks.rs`
- Modify runner: `src/runner.rs`
- Modify events: Update `RunnerEvent` enum in `src/runner.rs`
- Update UI: `src/ui/app.rs` (App state), `src/ui/dashboard.rs` (rendering)

**New Component - Test Discovery Strategy:**
- Add new variant to `TestDiscoveryStrategy` enum in `src/config.rs`
- Implement strategy in `src/test_discovery.rs` (add match arm in `find_related_tests()`)
- Add test cases if test file exists

**New Check Trigger Type:**
- Add variant to `CheckTriggers` struct in `src/config.rs`
- Handle in `checks::determine_checks()` matching logic
- Update config deserialization if needed

**Utilities - Shared Helpers:**
- Small functions: Add to existing module (e.g., helpers in `src/checks.rs`)
- Reusable types: Consider new file or add to appropriate module
- String formatting: `src/runner.rs::filter_docker_warnings()` pattern

**UI Component - New Dashboard Panel:**
- State: Add field to `src/ui/app.rs::App` struct
- Rendering: Add rendering function to `src/ui/dashboard.rs`
- Event handling: Add keyboard handler in `src/ui/mod.rs` event loop

**Configuration - New Config Option:**
- Add field to struct in `src/config.rs`
- Add default value if optional
- Update CLAUDE.md doc with example
- Load and pass through to consumer (checks.rs, runner.rs, etc.)

## Special Directories

**target/:**
- Purpose: Cargo build artifacts and dependencies
- Generated: Yes (run `cargo build`)
- Committed: No (.gitignore)

**.git/:**
- Purpose: Git repository metadata
- Generated: Yes (git init)
- Committed: N/A

**.planning/codebase/:**
- Purpose: Analysis documents for code maintenance
- Generated: Yes (by GSD mapping commands)
- Committed: Yes (intended for version control)

**.idea/:**
- Purpose: IntelliJ IDEA IDE settings
- Generated: Yes
- Committed: No (.gitignore)

## Module Dependencies

**Dependency Graph (simplified):**

```
main.rs
  ├─> config.rs (load config)
  ├─> git.rs (detect changes)
  ├─> checks.rs (determine checks)
  │    ├─> test_discovery.rs (find tests)
  │    └─> config.rs (check definitions)
  ├─> simple.rs (non-TUI mode)
  │    ├─> checks.rs (group_checks)
  │    ├─> runner.rs (execute)
  │    └─> git.rs (for re-detection)
  └─> ui/mod.rs (TUI mode)
      ├─> runner.rs (CheckRunner)
      ├─> checks.rs (re-determine on refresh)
      ├─> git.rs (refresh changes)
      ├─> ui/app.rs (App state)
      └─> ui/dashboard.rs (rendering)

ui/app.rs
  └─> runner.rs (CheckResult, CheckStatus)

ui/dashboard.rs
  └─> ui/app.rs (App struct for state)
```

**Module exports (from lib.rs):**
- `pub mod checks;` - For determine_checks, CheckToRun
- `pub mod config;` - For load_config, CiConfig
- `pub mod git;` - For detect_changes, ChangedFiles
- `pub mod runner;` - For CheckRunner, CheckResult, CheckStatus
- `pub mod test_discovery;` - Internal use by checks.rs
- `pub mod ui;` - For run() TUI entry
- `pub mod simple;` - For run() simple mode entry

---

*Structure analysis: 2026-01-22*
