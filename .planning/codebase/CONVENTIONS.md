# Coding Conventions

**Analysis Date:** 2026-01-22

## Naming Patterns

**Files:**
- Lowercase with underscores: `config.rs`, `runner.rs`, `test_discovery.rs`
- Module grouping uses subdirectories: `ui/mod.rs`, `ui/app.rs`, `ui/dashboard.rs`
- Binary entry point: `main.rs`
- Library entry point: `lib.rs`

**Functions:**
- Lowercase with underscores: `detect_changes()`, `determine_checks()`, `filter_by_pattern()`
- Utility functions often have descriptive prefixes indicating action: `get_*()`, `is_*()`, `apply_*()`
- Private helpers use leading underscore naming convention by default (implicit)
- Example from `config.rs`: `pub fn get_file_pattern()`, `pub fn should_ignore_file()`, `pub fn get_file_color()`

**Variables:**
- Lowercase with underscores: `changed_files`, `base_ref`, `project_root`, `check_id`
- Boolean flags use `is_*` or `has_*` prefix: `is_empty()`, `has_fix()`, `is_on_demand()`
- Configuration/state objects are verbose: `group_config`, `check_definition`, `docker_project_dir`
- Iterator variables use short names when context is clear: `rules`, `files`, `checks`

**Types:**
- PascalCase for structs: `CiConfig`, `CheckToRun`, `ChangedFiles`, `CheckResult`, `CheckRunner`
- PascalCase for enums: `CheckStatus`, `RunnerEvent`, `StatusFilter`, `TestDiscoveryStrategy`
- Single-word or meaningful phrases: `PreCommand`, `FilePattern`, `GroupConfig`, `DockerConfig`

## Code Style

**Formatting:**
- Rust standard formatting via `cargo fmt` (configured in Makefile target `fmt`)
- 2021 edition (see `Cargo.toml` line 4: `edition = "2021"`)
- Line length follows default rustfmt conventions (~100 chars practical)

**Linting:**
- Clippy enabled via Makefile target `clippy`
- Command: `cargo clippy` in Docker (`cargo clippy` run via `rust:latest` image)
- Warnings are treated seriously - linting is part of CI pipeline

**Derive Macros:**
- Debug is near-universal: `#[derive(Debug, ...)]`
- Serde for configuration structures: `#[derive(Debug, Deserialize)]` (see `config.rs` structs)
- Clone for data passed through channels: `#[derive(Debug, Clone)]` (see `CheckResult`, `RunnerEvent`)
- PartialEq, Eq for status enums: `#[derive(Debug, Clone, PartialEq, Eq)]` (see `CheckStatus`)
- Common pattern: `#[derive(Debug, Clone, Deserialize)]` for config types

## Import Organization

**Order:**
1. Internal crate imports with paths: `use crate::checks`, `use crate::config`, `use crate::runner`
2. External crate imports (alphabetical): `use anyhow`, `use chrono`, `use indexmap`, `use regex`, `use serde`, `use tokio`
3. Standard library: `use std::collections`, `use std::path`, `use std::process`, `use std::sync`

**Example from `simple.rs`:**
```rust
use crate::checks::{group_checks, CheckToRun};
use crate::config::CiConfig;
use crate::git::ChangedFiles;
use crate::runner::{CheckResult, CheckStatus};
use anyhow::Result;
use std::path::PathBuf;
use std::time::Instant;
use tokio::process::Command;
```

**Path Aliases:**
- Not used - explicit module paths preferred (crate::module_name)
- Re-exports in `lib.rs` for public API convenience (`pub use checks::CheckToRun`)

## Error Handling

**Patterns:**
- Return type is `Result<T>` throughout (aliased from `anyhow::Result`)
- Used in main.rs: `async fn main() -> Result<()>`
- Used in async functions: `pub async fn run_checks(...) -> Result<()>`
- Context-aware error messages via `.context()` and `.with_context()`

**Example from `git.rs`:**
```rust
pub fn detect_changes(project_root: &Path, git_config: &GitConfig) -> Result<ChangedFiles> {
    // Try different base refs in order
    let base_refs = [...];

    for base_ref in &base_refs {
        match get_changed_files(project_root, base_ref) {
            Ok(changed_files) => return Ok(changed_files),
            Err(_) => continue,  // Silently try next ref
        }
    }

    // Return empty on all failures - graceful degradation
    Ok(ChangedFiles { files: vec![], base_ref: "HEAD".to_string() })
}
```

**Error Context:**
- `anyhow::Context` trait used for wrapping with context: `.context("Failed to run git diff")?`
- `anyhow::bail!()` for explicit failures with formatted messages
- Errors propagate up with `?` operator - no unwrap() or expect() in main code paths

## Logging

**Framework:** No explicit logging framework used
- Uses ANSI escape codes for colored terminal output in simple.rs: `"\x1b[1mCI Checks\x1b[0m"`
- Uses ANSI codes in UI output as well for structured display
- println!() and eprintln!() avoided in favor of structured output through TUI
- No log/warn/info! macros - outputs are user-facing console output only

**Patterns:**
- Simple mode (non-TUI) outputs to println! for CI pipelines
- TUI mode sends all output through event channels to dashboard rendering
- Pre-command execution shows status and output through UI events

## Comments

**When to Comment:**
- Comments are sparse but strategic
- Used for filtering logic: `// Filter out common Docker Compose warnings that are noise` in `runner.rs`
- Used to explain non-obvious behavior or configuration reasoning
- Especially present in test helper functions to document test intent

**JSDoc/TSDoc:**
- Rust doc comments (`///`) used for public APIs and important types
- Module-level doc comments (`//!`) in lib.rs explain overall architecture
- Field doc comments explain intent and optional behaviors
- Example from `config.rs`:
```rust
/// Check if a file should be ignored based on ignore_patterns
pub fn should_ignore_file(&self, path: &str) -> bool {

/// Get color name for a file based on file_patterns
pub fn get_file_color(&self, path: &str) -> &str {

/// Get the default Docker service (from docker config)
pub fn default_service(&self) -> &str {
```

**Multi-line doc comments:**
```rust
/// Determine which checks should run based on changed files
/// Returns ALL checks from config - those that match are set to run,
/// those that don't match are marked as skipped (can be run on-demand)
pub fn determine_checks(config: &CiConfig, changed_files: &ChangedFiles, project_root: &Path) -> Vec<CheckToRun> {
```

## Function Design

**Size:** Functions are generally small and focused
- Average 10-50 lines for business logic
- Longer functions (100+ lines) are in main loops with clear phases
- Example: `determine_checks()` in `checks.rs` is ~120 lines with clear sections for different check types

**Parameters:**
- Using references for owned data that isn't modified: `&Path`, `&CiConfig`, `&ChangedFiles`
- Using owned values for small copyable types and return values
- Using `&str` for string slices, `String` for owned strings to pass
- Example pattern: `fn apply_ignore_patterns(&mut self, ignore_patterns: &[String])`

**Return Values:**
- Simple values returned directly: `pub fn is_empty(&self) -> bool`
- Complex data returned as owned: `pub fn extensions(&self) -> HashSet<&str>`
- Errors propagated with Result<T>: `pub fn detect_changes(...) -> Result<ChangedFiles>`
- Iterator return types for lazy evaluation: `pub fn groups(&self) -> impl Iterator<Item = (&str, &GroupConfig)>`

## Module Design

**Exports:**
- Public types and functions explicitly marked `pub`
- Re-exports in `lib.rs` for commonly used public API
- Internal modules (ui/dashboard.rs) keep implementation details private
- Only essential types exposed at crate level

**Example from lib.rs:**
```rust
pub use checks::{determine_checks, CheckToRun};
pub use config::{load_config, CiConfig};
pub use git::{detect_changes, ChangedFiles};
pub use runner::{CheckResult, CheckRunner, CheckStatus};
```

**Barrel Files:**
- `lib.rs` serves as the public API barrel
- `ui/mod.rs` acts as barrel for UI submodule
- `src/main.rs` imports from `ci_tui::` crate by name

**Module Structure:**
- Tests live inline with `#[cfg(test)]` blocks at module end
- Test helper functions defined within test module
- Test fixtures (YAML strings) defined as `&'static str` functions

---

*Convention analysis: 2026-01-22*
