# Architecture Research: Module Organization & Test Fixtures

**Project:** CI-TUI
**Focus:** Rust module organization, test fixture consolidation, refactoring strategy
**Confidence:** HIGH

## Executive Summary

CI-TUI has a clean modular architecture with 12 modules and no circular dependencies. The primary refactoring opportunity is consolidating ~37 inline YAML test configs into shared fixtures using the `tests/common/mod.rs` pattern. Module splitting is NOT recommended - while several files exceed 1000 lines, they maintain strong cohesion and splitting would reduce maintainability. The codebase follows Rust best practices and should focus on test organization rather than structural changes.

## Test Fixture Architecture

### Current State

**Inline YAML configs scattered across modules:**
- `src/config.rs`: 27 occurrences of `version: 2` (YAML configs)
- `src/checks.rs`: 9 occurrences
- `src/ui/app.rs`: 1 occurrence
- `tests/common/mod.rs`: 1 shared config (widget tests only)

**Problem:** High duplication, each test module recreates similar configs with minor variations.

**Current shared utilities location:** `/home/ms/projects/ci-tui/tests/common/mod.rs` (223 lines)
- Already contains widget test utilities (`make_test_app()`, `parse_widget_config()`)
- Uses proper Rust convention (subdirectory pattern to avoid test output clutter)

### Recommended Structure

**Location:** Expand `tests/common/` into a fixture library:

```
tests/
├── common/
│   ├── mod.rs              # Re-export all fixtures
│   ├── configs.rs          # Shared YAML configurations
│   ├── builders.rs         # Test data builders (CheckToRun, App state)
│   └── mocks.rs            # Mock executors (already has some)
├── git_tests.rs
├── runner_tests.rs
├── widget_tests.rs
└── panic_hook_tests.rs
```

**Rationale:**
1. **Subdirectory pattern**: Rust official guidance is to use `tests/common/mod.rs` instead of `tests/common.rs` to avoid treating common utilities as a test crate
2. **Multiple files under common/**: As common utilities grow, split by purpose (configs, builders, mocks) for maintainability
3. **No fixtures/ directory**: Keep everything under `common/` to maintain simplicity

### Fixture Categories

#### 1. Config Fixtures (`tests/common/configs.rs`)

Create canonical YAML configs for common scenarios:

```rust
/// Minimal valid config (for edge case tests)
pub fn minimal_config_yaml() -> &'static str { ... }

/// PHP project config (current test_config_yaml from checks.rs)
pub fn php_project_config_yaml() -> &'static str { ... }

/// Rust project config (current widget_test_config_yaml from common/mod.rs)
pub fn rust_project_config_yaml() -> &'static str { ... }

/// Multi-language config (for complex trigger tests)
pub fn multi_language_config_yaml() -> &'static str { ... }

/// Config with test discovery (for test_discovery tests)
pub fn config_with_test_discovery_yaml() -> &'static str { ... }
```

**Naming convention:** `{domain}_{purpose}_yaml()` returns `&'static str`

**Integration approach:**
1. Create `configs.rs` with consolidated YAMLs
2. Add `pub mod configs;` to `tests/common/mod.rs`
3. Replace inline configs with `use common::configs::*;`
4. Use rstest for parameterized tests if multiple configs needed

#### 2. Builder Fixtures (Already Present)

**Keep existing builders in `tests/common/mod.rs`:**
- `make_widget_check()` - Creates CheckToRun for testing
- `make_test_app()` - Creates App with known state
- `make_test_app_all_passed()` - Success state variant
- `make_test_app_running()` - Progress state variant

**Why not move to builders.rs?** These are simple, widget-specific. Only split if common/ exceeds ~500 lines.

#### 3. Mock Fixtures (Already Present)

**Keep existing mocks in `tests/common/mod.rs`:**
- `mock_git_with_output()` - MockGitExecutor returning output
- `mock_git_error()` - MockGitExecutor returning error
- `mock_executor_success()` - MockCommandExecutor success
- `mock_executor_failure()` - MockCommandExecutor failure

### Integration Points

**From inline tests to shared fixtures:**

```rust
// BEFORE (in src/checks.rs)
#[test]
fn test_something() {
    let config_yaml = r#"
version: 2
docker:
  project_dir: ./infrastructure
  ...
"#;
    let config: CiConfig = serde_yaml::from_str(config_yaml).unwrap();
    // test logic
}

// AFTER
use common::configs::php_project_config_yaml;

#[test]
fn test_something() {
    let config: CiConfig = serde_yaml::from_str(php_project_config_yaml()).unwrap();
    // test logic
}

// OR with helper function
use common::configs::parse_php_config;

#[test]
fn test_something() {
    let config = parse_php_config();
    // test logic
}
```

**Dev dependencies already in place:**
- `rstest = "0.26"` for fixtures and parameterized tests
- `tempfile = "3"` for filesystem tests
- `mockall = "0.14"` for trait mocking
- `pretty_assertions = "1.4"` for test output

## Module Splitting Guidelines

### When to Split

Rust community has **no official line count standard**. Projects commonly have 2,000-5,000 line files before splitting. The decision should be based on **cohesion and complexity**, not arbitrary limits.

**Split when:**
1. **Multiple unrelated concerns in one file** - Different domains mixed together
2. **Poor separation makes testing difficult** - Can't isolate functionality
3. **High cognitive load from context switching** - Reading requires understanding disparate concepts
4. **Organic growth indicators** - Clear subdomains emerge naturally

**Signals to split:**
- "This function doesn't belong here" thoughts while coding
- Test organization becomes awkward (testing concerns from multiple domains)
- git blame shows different authors/timeframes for different sections
- Difficulty naming the module (trying to capture too many purposes)

### When NOT to Split

**DO NOT split when:**
1. **Strong cohesion exists** - All code serves a unified purpose
2. **Tests are well-organized** - Clear test structure despite file size
3. **Low coupling to other modules** - File is self-contained
4. **Single responsibility maintained** - Large but focused

**Why line count is misleading:**
- `src/checks.rs`: 1261 lines (289 code + 972 tests) - **DON'T SPLIT**
  - Single responsibility: determine which checks to run
  - Tests are organized into clear submodules
  - High cohesion around check determination logic

- `src/config.rs`: 1227 lines (404 code + 823 tests) - **DON'T SPLIT**
  - Single responsibility: config parsing and validation
  - Tests are well-structured by concern (triggers, patterns, etc.)
  - Splitting would scatter related config logic

- `src/ui/app.rs`: 1647 lines (874 code + 773 tests) - **DON'T SPLIT**
  - Single responsibility: application state management
  - Tests are organized into functional groups
  - App state is inherently large - splitting creates artificial boundaries

- `src/ui/dashboard.rs`: 993 lines (993 code, 0 tests) - **DON'T SPLIT**
  - Single responsibility: rendering logic
  - No inline tests (tested via widget_tests.rs)
  - Rendering is naturally large - widget composition requires context

### Analysis of Current Modules

**Module sizes and cohesion assessment:**

| Module | Lines | Code | Tests | Assessment | Action |
|--------|-------|------|-------|------------|--------|
| checks.rs | 1261 | 289 | 972 | High cohesion | Keep as-is |
| config.rs | 1227 | 404 | 823 | High cohesion | Keep as-is |
| ui/app.rs | 1647 | 874 | 773 | High cohesion | Keep as-is |
| ui/dashboard.rs | 993 | 993 | 0 | High cohesion | Keep as-is |
| ui/mod.rs | 855 | 855 | 0 | Event loop logic | Keep as-is |
| runner.rs | 730 | 730 | 0 | Check execution | Keep as-is |
| test_discovery.rs | 603 | 603 | 0 | Test finding logic | Keep as-is |
| git.rs | 404 | 404 | 0 | Git operations | Keep as-is |
| simple.rs | 382 | 382 | 0 | Non-TUI output | Keep as-is |
| fix.rs | 281 | 281 | 0 | Fix command logic | Keep as-is |

**Verdict:** No modules need splitting. All maintain strong cohesion and single responsibility.

### UI Module Structure

**Current: 3 files, 3,495 total lines**
- `ui/mod.rs`: Event loop (855 lines)
- `ui/app.rs`: State management (1647 lines)
- `ui/dashboard.rs`: Rendering (993 lines)

**Should we add more submodules?** NO.

**Rationale:**
1. **Clear separation of concerns**: Event loop / State / Rendering
2. **Low coupling**: Each file has distinct responsibility
3. **Easy navigation**: Three-file structure is simple
4. **No stuttering**: Names don't duplicate module name (not `ui::ui_app`)

**Alternative considered and rejected:**
- Splitting dashboard.rs into widget files (header.rs, status.rs, etc.)
  - **Why rejected:** Creates fragmentation, rendering needs context across widgets
  - **When to reconsider:** If adding completely new views (settings screen, help screen)

## Recommended Refactoring Order

### Phase 1: Test Fixture Consolidation (Priority: HIGH)

**Goal:** Eliminate YAML duplication, establish shared fixture pattern

1. **Create `tests/common/configs.rs`** (1-2 hours)
   - Extract canonical configs from inline tests
   - Start with most-duplicated: `php_project_config_yaml()`, `minimal_config_yaml()`
   - Add helper parsers: `parse_php_config()`, `parse_minimal_config()`

2. **Update `tests/common/mod.rs`** (15 minutes)
   - Add `pub mod configs;`
   - Re-export key functions: `pub use configs::*;`

3. **Migrate `src/config.rs` tests** (2-3 hours)
   - Replace 27 inline configs with shared fixtures
   - Use rstest `#[rstest]` for parameterized tests where needed
   - Run tests after each migration to ensure correctness

4. **Migrate `src/checks.rs` tests** (1-2 hours)
   - Replace 9 inline configs
   - Consolidate similar test scenarios

5. **Migrate `src/ui/app.rs` tests** (30 minutes)
   - Single config to migrate
   - May already use `common::widget_test_config_yaml()`

**Success criteria:**
- Zero inline YAML configs in src/ modules
- All configs in `tests/common/configs.rs`
- Test suite passes with no behavior changes
- Reduced LOC in test modules

**Risk mitigation:**
- Migrate one module at a time
- Run `--simple` validation after each module
- Keep git commits small (one module per commit)

### Phase 2: Test Organization Review (Priority: MEDIUM)

**Goal:** Improve test discoverability and organization

1. **Add module documentation** (1 hour)
   - Document each fixture in `tests/common/`
   - Add examples showing typical usage
   - Update top-level comments

2. **Review integration tests** (1 hour)
   - Ensure `tests/git_tests.rs`, `tests/runner_tests.rs` use shared fixtures
   - Check for additional duplication opportunities

3. **Consider rstest adoption** (2-3 hours)
   - Identify parameterized test candidates
   - Refactor repetitive test patterns to use `#[rstest]` with `#[case]`
   - Example: testing multiple file patterns with same logic

**Success criteria:**
- Clear documentation for all fixtures
- Reduced test code duplication
- Easier to add new test cases

### Phase 3: Documentation and Conventions (Priority: LOW)

**Goal:** Formalize patterns for future development

1. **Document testing conventions** (1 hour)
   - Add section to CLAUDE.md about using shared fixtures
   - Explain when to add new fixtures vs. inline config
   - Show rstest patterns for the codebase

2. **Add fixture naming conventions** (30 minutes)
   - Pattern: `{domain}_{purpose}_yaml()` for YAML strings
   - Pattern: `parse_{domain}_config()` for parsed configs
   - Pattern: `make_{domain}_{variant}()` for builders

**Success criteria:**
- CLAUDE.md has testing section
- Clear guidance for contributors
- Consistent patterns established

## File/Module Conventions

### Naming Conventions (Rust API Guidelines)

Following [RFC 430](https://rust-lang.github.io/api-guidelines/naming.html):

| Item | Convention | Example |
|------|------------|---------|
| Modules | snake_case | `test_discovery`, `ui` |
| Types/Traits | UpperCamelCase | `CheckToRun`, `CiConfig` |
| Functions/Methods | snake_case | `determine_checks()`, `run()` |
| Constants/Statics | SCREAMING_SNAKE_CASE | `MAX_HISTORY_SAMPLES` |
| Crate names | snake_case | `ci_tui` (not `ci-tui-rs`) |

**Special rules:**
- Acronyms in CamelCase: `Uuid` not `UUID`, `Stdin` not `StdIn`
- Acronyms in snake_case: `is_xid_start` (all lowercase)
- Single letters in snake_case: Only at end (`btree_map` not `b_tree_map`)

### Module Organization Patterns

**src/ layout (current - maintain this):**
```
src/
├── lib.rs           # Public API, re-exports
├── main.rs          # CLI entry point (thin)
├── config.rs        # Configuration types
├── checks.rs        # Check determination
├── git.rs           # Git operations
├── test_discovery.rs # Test file discovery
├── runner.rs        # Check execution
├── fix.rs           # Fix command logic
├── simple.rs        # Non-TUI output
└── ui/              # TUI module
    ├── mod.rs       # Event loop
    ├── app.rs       # State management
    └── dashboard.rs # Rendering
```

**tests/ layout (recommended):**
```
tests/
├── common/
│   ├── mod.rs       # Re-exports
│   ├── configs.rs   # Shared YAML configs (NEW)
│   ├── builders.rs  # Test data builders (FUTURE - if needed)
│   └── mocks.rs     # Mock utilities (FUTURE - if needed)
├── git_tests.rs
├── runner_tests.rs
├── widget_tests.rs
└── panic_hook_tests.rs
```

### Integration Test Organization

**Pattern:** `tests/{module}_tests.rs` tests integration of `src/{module}.rs`

**Current integration tests:**
- `tests/git_tests.rs` (255 lines) - Tests git operations
- `tests/runner_tests.rs` (473 lines) - Tests check runner
- `tests/widget_tests.rs` (504 lines) - Tests UI rendering
- `tests/panic_hook_tests.rs` (85 lines) - Tests panic handling

**Pattern to maintain:**
- Integration tests in separate `tests/` directory
- Unit tests as `#[cfg(test)] mod tests` in source files
- Shared fixtures in `tests/common/`

### When to Add New Modules

**Add a new module when:**
1. **New domain emerges** - Distinct responsibility not covered by existing modules
2. **Clear API boundary** - Can define public interface separate from implementation
3. **Multiple files needed** - Complex enough to warrant subdirectory

**Don't add a module for:**
- Line count reduction alone
- Organizational aesthetics
- Following arbitrary rules (like "files must be under 500 lines")

**Example of good module addition (if needed in future):**
- `src/reporting/` - If CI-TUI adds JUnit XML output, coverage reports, etc.
  - Clear domain: generating reports
  - Multiple output formats: JUnit, TAP, JSON
  - Separate from existing concerns

**Example of bad module addition:**
- `src/checks/determination.rs` + `src/checks/resolution.rs` from current `checks.rs`
  - Artificial split: both are part of check determination
  - Increases coupling: both need same config types
  - Reduces cohesion: related logic separated

## Sources

### Official Rust Documentation
- [Test Organization - The Rust Programming Language](https://doc.rust-lang.org/book/ch11-03-test-organization.html)
- [Refactoring to Improve Modularity - The Rust Programming Language](https://doc.rust-lang.org/book/ch12-03-improving-error-handling-and-modularity.html)
- [Packages, Crates, and Modules - The Rust Programming Language](https://doc.rust-lang.org/book/ch07-00-managing-growing-projects-with-packages-crates-and-modules.html)
- [Rust API Guidelines - Naming Conventions](https://rust-lang.github.io/api-guidelines/naming.html)
- [Rust API Guidelines - Checklist](https://rust-lang.github.io/api-guidelines/checklist.html)

### Testing and Fixtures
- [rstest - Fixture-based test framework](https://github.com/la10736/rstest)
- [rstest Documentation](https://docs.rs/rstest)
- [Testing With Fixtures in Rust](https://dawchihliou.github.io/articles/testing-with-fixtures-in-rust)
- [Everything you need to know about testing in Rust | Shuttle](https://www.shuttle.dev/blog/2024/03/21/testing-in-rust)

### Module Organization
- [Best Practices for Structuring Large-Scale Rust Applications](https://www.slingacademy.com/article/best-practices-for-structuring-large-scale-rust-applications-with-modules/)
- [Using submodules to Split Large Rust Files](https://www.slingacademy.com/article/using-submodules-to-split-large-rust-files-into-manageable-components/)
- [Rust Project Structure and Best Practices](https://www.djamware.com/post/68b2c7c451ce620c6f5efc56/rust-project-structure-and-best-practices-for-clean-scalable-code)
- [Do we have standard for LOC per file? - Rust Forum](https://users.rust-lang.org/t/do-we-have-standard-for-loc-per-file/63509)

### Community Resources
- [Rust for C-Programmers - Test Organization](https://rust-for-c-programmers.com/ch24/24_3_test_organization.html)
- [Organizing Tests in Rust - KodeKloud](https://notes.kodekloud.com/docs/Rust-Programming/Testing-Continuous-Integration/Organizing-Tests-in-Rust)
- [Rust Project Structure Example - DEV Community](https://dev.to/ghost/rust-project-structure-example-step-by-step-3ee)
