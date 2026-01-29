# Features Research: Rust Code Quality Patterns

**Domain:** Rust TUI application code quality, test organization, and module boundaries
**Researched:** 2026-01-29 (Updated)
**Overall confidence:** HIGH
**Focus:** Test fixture patterns, shared utilities, and module organization for v2.0 cleanup

## Executive Summary

Well-maintained Rust projects follow consistent patterns for test organization and code modularity. The official Rust documentation establishes clear conventions: unit tests live alongside code with `#[cfg(test)]`, integration tests use `tests/` directory with `tests/common/mod.rs` (NOT `tests/common.rs`) for shared utilities, and fixture frameworks like `rstest` eliminate test data duplication through parametrized testing. Large modules split along responsibility boundaries when they mix unrelated concerns. Modern tooling like Clippy's `too_many_lines` and `excessive_nesting` lints provide objective complexity gates (while `cognitive_complexity` is deprecated as flawed). For TUI applications specifically, the Ratatui community recommends The Elm Architecture (TEA) with clear separation between state (app), rendering (ui), and event handling.

**Key findings for CI-TUI v2.0:**
- 40 duplicate YAML configs → Extract to `tests/common/fixtures.rs` or use `rstest` fixtures
- 3 copies of `format_duration()` → Move to `src/utils/formatting.rs`
- 155-line functions → Enable `too_many_lines` lint with threshold of ~100 lines
- 3,495 LOC UI module → Split by responsibility: `ui/app.rs` (state), `ui/dashboard.rs` (rendering), `ui/events.rs` (input)

## Table Stakes (Must Have)

### 1. Official Test Organization Pattern
- **What:** Unit tests in `#[cfg(test)] mod tests` alongside code, integration tests in `tests/` directory, shared test utilities in `tests/common/mod.rs` (NOT `tests/common.rs`)
- **Why table stakes:** This is the official Rust convention from The Rust Book. Files in subdirectories of `tests/` (like `tests/common/mod.rs`) are NOT compiled as separate crates, avoiding empty test section pollution in output.
- **Complexity:** Low
- **Current status:** ✅ CI-TUI already follows this pattern
- **Example:**
```rust
// tests/common/mod.rs - shared utilities
pub fn minimal_config() -> CiConfig {
    serde_yaml::from_str(include_str!("fixtures/minimal.yaml")).unwrap()
}

// tests/integration_test.rs
mod common;

#[test]
fn test_with_fixture() {
    let config = common::minimal_config();
    assert_eq!(config.checks.len(), 1);
}
```
- **Source:** [Official Rust Book - Test Organization](https://doc.rust-lang.org/book/ch11-03-test-organization.html)

### 2. DRY Test Fixtures - Shared Helper Functions
- **What:** Extract repeated test setup into shared fixture functions in `tests/common/fixtures.rs` or `#[cfg(test)] mod tests` for unit tests
- **Why table stakes:** 40 inline YAML test configs that are "nearly identical" violates basic DRY principles. Every mature Rust project consolidates test data. Changes to config structure should require updating ONE fixture, not 40 test strings.
- **Complexity:** Low to Medium (refactoring existing tests)
- **Current status:** ❌ CI-TUI has extensive duplication
- **Example:**
```rust
// tests/common/fixtures.rs
pub fn minimal_config() -> &'static str {
    include_str!("fixtures/minimal.yaml")
}

pub fn multi_group_config() -> &'static str {
    include_str!("fixtures/multi_group.yaml")
}

// Or as actual configs
pub fn config_with_checks(n: usize) -> CiConfig {
    let mut config = minimal_config_obj();
    config.checks = vec![check_definition(); n];
    config
}
```
- **Source:** [Official Rust Book - Test Organization](https://doc.rust-lang.org/book/ch11-03-test-organization.html)

### 3. Clippy Complexity Lints
- **What:** Enable `too_many_lines` and `excessive_nesting` lints to enforce objective complexity limits. **Avoid `cognitive_complexity`** (flawed, moved to restriction category)
- **Why table stakes:** Clippy maintainers explicitly state: "The true Cognitive Complexity of a method is not calculable using modern technology." They recommend `too_many_lines` and `excessive_nesting` instead. 155-line functions should trigger warnings.
- **Complexity:** Low (just configuration)
- **Current status:** ⚠️ CI-TUI has 155-line functions without lint enforcement
- **Configuration:**
```toml
# In Cargo.toml or clippy.toml
[lints.clippy]
too_many_lines = "warn"  # Default threshold: 100 lines
excessive_nesting = "warn"
```
- **Source:** [Clippy Lints Documentation](https://rust-lang.github.io/rust-clippy/master/index.html)

### 4. Module Boundary by Responsibility
- **What:** Split modules when they mix unrelated responsibilities, not when they hit arbitrary line counts. Extract cross-module utilities to dedicated modules like `src/utils/`.
- **Why table stakes:** Official Rust guidance: "When modules get large, move definitions to separate file to make code easier to navigate." But split by responsibility, not just size. A 3,495 LOC module that handles state management, rendering, AND event handling violates Single Responsibility Principle.
- **Complexity:** Medium (requires understanding responsibility boundaries)
- **Current status:** ⚠️ UI module is 3,495 LOC (70% of codebase), `format_duration()` duplicated 3x
- **Example:**
```
Before:                       After:
src/                         src/
├── ui/                      ├── ui/
│   └── mod.rs (3,495 LOC)   │   ├── mod.rs (re-exports)
                             │   ├── app.rs (state)
                             │   ├── dashboard.rs (rendering)
                             │   ├── events.rs (input handling)
                             │   └── components/
                             └── utils/
                                 └── formatting.rs (format_duration)
```
- **Source:** [Rust Book - Separating Modules](https://doc.rust-lang.org/book/ch07-05-separating-modules-into-different-files.html), [Rust Book - Refactoring](https://doc.rust-lang.org/book/ch12-03-improving-error-handling-and-modularity.html)

### 5. Private by Default
- **What:** Keep functions and structs private unless they're part of public API. Use `pub(crate)` for internal visibility across modules within crate.
- **Why table stakes:** "Keeping functions and structs private prevents implementation details from leaking out of modules, making future refactoring easier."
- **Complexity:** Low (Rust's default behavior, just be deliberate about `pub`)
- **Current status:** Unknown (needs audit)
- **Source:** [Long-term Rust Maintenance](https://corrode.dev/blog/long-term-rust-maintenance/)

### 6. Clear separation of concerns (TEA/MVC)
- **What:** Separate state management (Model/App), rendering logic (View/Dashboard), and event handling (Update/Events) - following The Elm Architecture or similar pattern
- **Why table stakes:** Ratatui docs explicitly recommend this. Prevents UI entanglement with business logic, makes testing easier (can test state transitions without rendering).
- **Complexity:** Medium
- **Current status:** ✅ CI-TUI has `app.rs` and separation (per CLAUDE.md), but UI module size suggests possible entanglement
- **Source:** [Ratatui - The Elm Architecture](https://ratatui.rs/concepts/application-patterns/the-elm-architecture/)

### 7. Terminal cleanup on panic
- **What:** Install panic hook that restores terminal state before displaying error
- **Why table stakes:** Corrupted terminal state on crash = horrible UX. User's terminal becomes unusable.
- **Complexity:** Low
- **Pattern:**
```rust
let original_hook = std::panic::take_hook();
std::panic::set_hook(Box::new(move |panic_info| {
    crossterm::terminal::disable_raw_mode().ok();
    crossterm::execute!(io::stdout(), LeaveAlternateScreen).ok();
    original_hook(panic_info);
}));
```
- **Source:** Community best practice (multiple TUI examples)

### 8. Linting with Clippy at deny level
- **What:** Run `cargo clippy -- -D warnings` in CI. Catches unused code, style issues, common mistakes.
- **Why table stakes:** Prevents accumulation of dead code, enforces Rust idioms
- **Complexity:** Low
- **Current status:** ✅ CI-TUI runs clippy in CI (per CLAUDE.md validation commands)
- **Source:** [Clippy Documentation](https://doc.rust-lang.org/clippy/usage.html)

### 9. Rustfmt for consistent formatting
- **What:** Run `cargo fmt` automatically, enforce with `cargo fmt --check` in CI
- **Why table stakes:** Standard 4-space indentation, snake_case naming. Prevents formatting bikeshedding.
- **Complexity:** Low
- **Current status:** ✅ CI-TUI runs `cargo fmt --check` in CI (per CLAUDE.md)
- **Source:** [Rustfmt Documentation](https://github.com/rust-lang/rustfmt)

### 10. Unit tests for business logic
- **What:** Core logic (checks, config parsing, discovery) must be testable in isolation from UI
- **Why table stakes:** Can't validate state transitions, edge cases, error handling without unit tests
- **Complexity:** Medium
- **Current status:** ✅ 250 tests, 65% coverage (per milestone context)
- **Source:** [Official Rust Book - Testing](https://doc.rust-lang.org/book/ch11-01-writing-tests.html)

## Differentiators (Excellence)

### 11. Fixture-Based Testing with rstest
- **What:** Use `rstest` crate for parametrized tests and fixture injection instead of manual setup code or duplicated test functions
- **Why differentiator:** "The core idea is that you can inject your test dependencies by passing them as test arguments." Eliminates test boilerplate while maintaining clarity. Enables table-driven testing with `#[case]` attributes. One test function can verify 40 different scenarios.
- **Complexity:** Low (add dependency, refactor tests)
- **When to use:** When you have multiple similar test cases (like 40 YAML configs) or expensive setup operations
- **Example:**
```rust
use rstest::rstest;

#[rstest]
#[case("minimal.yaml", 1)]
#[case("multi_group.yaml", 3)]
#[case("with_discovery.yaml", 2)]
fn test_config_parsing(#[case] fixture_name: &str, #[case] expected_checks: usize) {
    let yaml = std::fs::read_to_string(format!("tests/fixtures/{}", fixture_name)).unwrap();
    let config: CiConfig = serde_yaml::from_str(&yaml).unwrap();
    assert_eq!(config.checks.len(), expected_checks);
}

// Instead of 40 separate test functions with inline YAML!
```
- **Source:** [rstest GitHub](https://github.com/la10736/rstest)

### 12. Utility Module Hierarchy
- **What:** Create `src/utils/` module with submodules (`formatting.rs`, `git.rs`, etc.) for cross-cutting concerns that multiple modules need
- **Why differentiator:** Community best practice: "Separate utility functions by creating new files like src/utils.rs for cleaner code organization"
- **Complexity:** Low (create directory, move functions)
- **Current status:** ⚠️ `format_duration()` duplicated 3x across modules
- **Example:**
```rust
// src/utils/mod.rs
pub mod formatting;
pub mod git;

// src/utils/formatting.rs
pub fn format_duration(duration: Duration) -> String {
    // Single implementation
}

// Other modules
use crate::utils::formatting::format_duration;
```
- **Source:** [Rust Project Structure Best Practices](https://www.djamware.com/post/68b2c7c451ce620c6f5efc56/rust-project-structure-and-best-practices-for-clean-scalable-code)

### 13. Component-Based TUI Architecture
- **What:** Organize UI into `components/` directory with isolated, reusable components. Each component has clear rendering responsibility.
- **Why differentiator:** Ratatui template pattern. "Once set up, most work stays in components/ folder." Scales to complex UIs, makes testing individual widgets easier.
- **Complexity:** High (architectural refactor)
- **When to use:** Any non-trivial TUI with multiple distinct UI regions
- **Structure:**
```
src/
├── app.rs            # Application state
├── tui.rs            # Terminal management
├── action.rs         # Action enum for key mapping
├── components/       # Modular UI components
│   ├── home.rs
│   ├── fps.rs
│   └── status.rs
└── components.rs     # Component trait
```
- **Source:** [Ratatui Project Structure](https://ratatui.rs/templates/component/project-structure/)

### 14. Action/Command mapping system
- **What:** Define `Action` enum, map keys to actions, separate from business logic
- **Why differentiator:** Decouples key bindings from implementation. Enables configurable keybindings, testable event handling.
- **Complexity:** Medium
- **Pattern:**
```rust
pub enum Action {
    Quit,
    ToggleCheck(usize),
    ScrollUp,
    // ...
}

// In event handler
fn handle_key(&mut self, key: KeyEvent) -> Action {
    match key.code {
        KeyCode::Char('q') => Action::Quit,
        KeyCode::Up => Action::ScrollUp,
        // ...
    }
}

// In update/business logic
fn update(&mut self, action: Action) {
    match action {
        Action::Quit => self.should_quit = true,
        Action::ScrollUp => self.scroll_offset -= 1,
        // ...
    }
}
```
- **Source:** [Ratatui component template](https://ratatui.rs/templates/component/project-structure/)

### 15. File-based logging (tracing/log4rs)
- **What:** Log to file (e.g., `~/.local/share/app/app.log`) instead of stdout (which interferes with TUI)
- **Why differentiator:** Essential for debugging TUI apps. XDG-compliant directories.
- **Complexity:** Medium
- **Source:** Community best practice

### 16. Testlib for Integration Testing
- **What:** Use `ratatui-testlib` for snapshot testing TUI output in a real PTY instead of just TestBackend
- **Why differentiator:** "Runs your TUI in a real pseudo-terminal (PTY), captures output using a terminal emulator, and provides ergonomic API for assertions and snapshot testing." Bridges gap between unit tests and real-world behavior.
- **Complexity:** Medium (learning new testing paradigm)
- **When to use:** When existing test coverage is high (like CI-TUI's 65%) but you want to verify actual rendered output
- **Source:** [ratatui-testlib](https://lib.rs/crates/ratatui-testlib)

### 17. Clippy Item Ordering Lint
- **What:** Enable `arbitrary_source_item_ordering` lint for consistent ordering of structs, impls, functions within modules
- **Why differentiator:** Reduces bikeshedding about file organization. Configurable groupings.
- **Complexity:** Low (enable lint, auto-fix or manual reorder once)
- **When to use:** When team debates ordering conventions or new contributors unsure where to add items
- **Source:** [Clippy Lints](https://rust-lang.github.io/rust-clippy/master/index.html)

### 18. Async responsiveness with tokio::select!
- **What:** Use `tokio::select!` for event loop with separate tick rate (state updates) and render rate (FPS). Use `spawn_blocking` for input polling.
- **Why differentiator:** Prevents UI responsiveness issues during I/O. Critical for keyboard responsiveness under high CPU load.
- **Complexity:** Medium
- **Current status:** ✅ CI-TUI already does this (per CLAUDE.md Architecture: "Keyboard input runs on dedicated OS thread")
- **Source:** [async-ratatui example](https://github.com/d-holguin/async-ratatui)

### 19. State sharing with Arc<Mutex<T>>
- **What:** Wrap shared state in Arc<Mutex<T>>, provide accessor methods that handle locking
- **Why differentiator:** Clean multi-threaded state management
- **Complexity:** Medium
- **Source:** Community pattern

### 20. TestBackend for widget tests
- **What:** Use ratatui's TestBackend to render widgets to buffer and assert output without real terminal
- **Why differentiator:** Preferred over full integration tests for widget-level testing. Fast, deterministic.
- **Complexity:** Medium
- **Example:**
```rust
use ratatui::backend::TestBackend;
use ratatui::Terminal;

let mut backend = TestBackend::new(80, 24);
let mut terminal = Terminal::new(backend)?;
terminal.draw(|frame| {
    render_dashboard(frame, &app_state);
})?;

let buffer = terminal.backend().buffer();
assert_eq!(buffer.get(0, 0).symbol(), "Expected");
```
- **Source:** [Ratatui TestBackend docs](https://docs.rs/ratatui/latest/ratatui/backend/struct.TestBackend.html)

## Anti-Features (Avoid)

### Anti-Feature 1: tests/common.rs Pattern
- **What:** Creating `tests/common.rs` for shared integration test utilities
- **Why avoid:** Cargo treats it as a separate test crate, creating empty test section in output: "Running tests/common.rs ... running 0 tests". Pollutes test output unnecessarily.
- **What to do instead:** Use `tests/common/mod.rs` - "Files in subdirectories of tests/ are not compiled as separate crates and don't appear in test output"
- **Source:** [Official Rust Book - Test Organization](https://doc.rust-lang.org/book/ch11-03-test-organization.html)

### Anti-Feature 2: cognitive_complexity Lint
- **What:** Relying on Clippy's `cognitive_complexity` lint for complexity measurement
- **Why avoid:** Clippy maintainers explicitly state it's flawed: "The true Cognitive Complexity of a method is not calculable using modern technology." Has known bugs with macro expansion. Moved to restriction category. Discussion about deprecating it.
- **What to do instead:** Use `too_many_lines` and `excessive_nesting` lints which measure objective properties
- **Source:** [Clippy cognitive_complexity](https://rust-lang.github.io/rust-clippy/master/index.html), [GitHub Issue #14417](https://github.com/rust-lang/rust-clippy/issues/14417)

### Anti-Feature 3: Over-modularization by Line Count
- **What:** Splitting modules purely because they hit arbitrary line counts (e.g., "every file over 500 lines must be split")
- **Why avoid:** "Don't over-engineer your project structure, but be ready to refactor as the codebase grows." Split by responsibility boundaries, not size. A well-organized 800-line module with single responsibility is better than 4 poorly-organized 200-line modules.
- **What to do instead:** Split when modules mix unrelated concerns or when "you have a hard time writing tests" (sign code is too complex). CI-TUI's 3,495 LOC UI module should split because it likely mixes state, rendering, AND event handling - that's 3 responsibilities.
- **Source:** [Best Way to Structure Rust Web Services](https://blog.logrocket.com/best-way-structure-rust-web-services/), [Long-term Rust Maintenance](https://corrode.dev/blog/long-term-rust-maintenance/)

### Anti-Feature 4: Inline Test Data Duplication
- **What:** Copy-pasting YAML configs, JSON fixtures, or other test data across test functions
- **Why avoid:** Every change requires updating N locations. Creates maintenance burden and drift (tests start with identical fixtures, then diverge slightly, making it unclear what's intentional vs copy-paste error). CI-TUI's 40 "nearly identical" inline YAML configs are textbook example.
- **What to do instead:**
  - **For unit tests:** Helper functions in `#[cfg(test)] mod tests`
  - **For integration tests:** Fixtures in `tests/common/fixtures/` or `tests/fixtures.yaml` loaded via `include_str!`
  - **For parametrized tests:** Use `rstest` with `#[case]` attributes
- **Example:**
```rust
// BAD: 40 test functions with inline YAML
#[test]
fn test_minimal_config_1() {
    let yaml = r#"
docker:
  project_dir: "."
  service: "app"
checks:
  - name: "test1"
    # ...
"#;
    let config: CiConfig = serde_yaml::from_str(yaml).unwrap();
    // test logic
}

#[test]
fn test_minimal_config_2() {
    let yaml = r#"  // Nearly identical...
// ... 38 more times

// GOOD: Extract fixture
fn minimal_config() -> &'static str {
    include_str!("fixtures/minimal.yaml")
}

#[rstest]
#[case("minimal", 1)]
#[case("multi", 3)]
fn test_config(#[case] fixture: &str, #[case] expected: usize) {
    let yaml = include_str!(concat!("fixtures/", fixture, ".yaml"));
    let config: CiConfig = serde_yaml::from_str(yaml).unwrap();
    assert_eq!(config.checks.len(), expected);
}
```
- **Source:** [rstest](https://github.com/la10736/rstest), [Official Rust Book - Test Organization](https://doc.rust-lang.org/book/ch11-03-test-organization.html)

### Anti-Feature 5: God Modules
- **What:** Single modules that handle multiple unrelated responsibilities (e.g., a `ui` module that contains state management, event handling, rendering, AND utility functions)
- **Why avoid:** "Poor separation of concerns, making it hard to test or refactor code." Makes it difficult to understand module boundaries and hard to reason about changes. A 3,495 LOC module almost certainly mixes concerns.
- **What to do instead:** Apply Single Responsibility Principle at module level. Split by responsibility:
```
Before:                       After:
src/ui/mod.rs (3,495 LOC)    src/ui/
                              ├── mod.rs (re-exports)
                              ├── app.rs (state)
                              ├── dashboard.rs (rendering)
                              ├── events.rs (input handling)
                              └── components/ (widgets)
```
- **Source:** [Rust Project Structure](https://www.djamware.com/post/68b2c7c451ce620c6f5efc56/rust-project-structure-and-best-practices-for-clean-scalable-code), [Rust Book - Refactoring](https://doc.rust-lang.org/book/ch12-03-improving-error-handling-and-modularity.html)

### Anti-Feature 6: Using .clone() Everywhere
- **What:** Cloning data instead of using references or smart pointers (Arc/Rc)
- **Why avoid:** Top Rust anti-pattern 2025, performance killer. Often sign of fighting borrow checker instead of understanding ownership.
- **What to do instead:** Use references, Arc for shared ownership, Rc for single-threaded
- **Source:** [7 Rust Anti-Patterns (2025)](https://medium.com/solo-devs/the-7-rust-anti-patterns-that-are-secretly-killing-your-performance-and-how-to-fix-them-in-2025-dcebfdef7b54)

### Anti-Feature 7: Unwrap() in Production Paths
- **What:** Using `.unwrap()` or `.expect()` where errors can occur during normal operation
- **Why avoid:** Panics = terminal corruption in TUI = terrible UX
- **What to do instead:** Return `Result`, propagate with `?`, install panic hook as safety net only
- **Source:** Community best practice

### Anti-Feature 8: Blocking I/O on Main Thread
- **What:** Calling blocking operations (file I/O, network, etc.) on the async runtime's main thread
- **Why avoid:** Kills keyboard responsiveness, makes UI feel frozen
- **What to do instead:** Use `tokio::spawn_blocking` for any blocking operations
- **Current status:** ✅ CI-TUI already handles this correctly (per CLAUDE.md: "Keyboard input runs on dedicated OS thread")
- **Source:** [async-ratatui](https://github.com/d-holguin/async-ratatui)

## Feature Dependencies

```
Foundation (do first):
├─ #2 DRY test fixtures (must exist before rstest refactor)
├─ #3 Clippy lints (enables objective gates)
└─ #4 Module boundaries (enables further organization)

Build on foundation:
├─ #11 rstest (requires #2 - fixtures extracted)
├─ #12 Utils module (requires #4 - boundaries defined)
├─ #13 Component architecture (requires #4 - module splitting)
└─ #17 Item ordering (cosmetic, can be done anytime)

Optional/Future:
├─ #16 Testlib (nice-to-have, new paradigm)
└─ #14 Action mapping (if keybindings need to be configurable)
```

## MVP Recommendation for v2.0 Cleanup

Given CI-TUI's specific issues (40 duplicate configs, 3x `format_duration()`, 155-line functions, 3,495 LOC UI module), prioritize:

### Phase 1: Test Foundation (High Impact, Low Complexity)
1. **#2 DRY Test Fixtures** - Extract 40 duplicate YAML configs to `tests/common/fixtures.rs` or helper functions. Create `minimal_config()`, `multi_group_config()`, etc.
2. **#3 Clippy Lints** - Enable `too_many_lines` (threshold ~100) and `excessive_nesting`, fix violations in 155-line functions

**Rationale:** These are table stakes (every mature Rust project does this) and directly address known issues. Low complexity, high value.

### Phase 2: Module Organization (Medium Impact, Medium Complexity)
3. **#4 Module Splitting** - Split 3,495 LOC UI module by responsibility:
   - `ui/app.rs` - Application state
   - `ui/dashboard.rs` - Rendering logic
   - `ui/events.rs` - Event handling
   - Keep `ui/mod.rs` for re-exports
4. **#12 Utility Module** - Extract `format_duration()` and other duplicated utilities to `src/utils/formatting.rs`

**Rationale:** Addresses "70% of codebase in one module" issue. Improves navigability and testability.

### Phase 3: Test Excellence (Optional, Nice-to-Have)
5. **#11 rstest** - Refactor parametrized tests to use fixtures. One test function can replace many duplicate test functions.

**Rationale:** Further reduces duplication, but requires new dependency. Can defer if timeline tight.

**Defer to post-v2.0:**
- **#16 Testlib** - Snapshot testing is valuable but requires learning new paradigm. Already have 65% coverage.
- **#13 Component architecture** - Useful but may be overkill if current UI structure works well
- **#14 Action mapping** - Only if keybindings need to be configurable

## Confidence Assessment

| Area | Confidence | Rationale |
|------|------------|-----------|
| Test organization (common/mod.rs) | HIGH | Official Rust Book, verified rust-lang.org |
| Test fixtures (DRY principle) | HIGH | Official docs + rstest widely adopted |
| Clippy lints | HIGH | Direct from Clippy docs, maintainer recommendations |
| Module boundaries | MEDIUM | Community consensus but subjective (when to split) |
| rstest framework | HIGH | Mature library, well-documented |
| TUI architecture (TEA) | MEDIUM | Ratatui community pattern, not official "standard" |
| Utility module pattern | HIGH | Standard Rust practice, multiple sources |

## Sources

### Official Documentation (HIGH confidence)
- [Test Organization - The Rust Programming Language](https://doc.rust-lang.org/book/ch11-03-test-organization.html)
- [Integration Testing - Rust By Example](https://doc.rust-lang.org/rust-by-example/testing/integration_testing.html)
- [Separating Modules into Different Files - The Rust Programming Language](https://doc.rust-lang.org/book/ch07-05-separating-modules-into-different-files.html)
- [Refactoring to Improve Modularity - The Rust Programming Language](https://doc.rust-lang.org/book/ch12-03-improving-error-handling-and-modularity.html)
- [Clippy Lints](https://rust-lang.github.io/rust-clippy/master/index.html)

### Authoritative Libraries/Frameworks (HIGH confidence)
- [rstest - Fixture-based test framework](https://github.com/la10736/rstest)
- [ratatui - TUI framework](https://github.com/ratatui/ratatui)
- [Ratatui Project Structure](https://ratatui.rs/templates/component/project-structure/)
- [Ratatui - The Elm Architecture](https://ratatui.rs/concepts/application-patterns/the-elm-architecture/)
- [ratatui-testlib](https://lib.rs/crates/ratatui-testlib)

### Best Practices Guides (MEDIUM confidence, verified with official docs)
- [Long-term Rust Project Maintenance](https://corrode.dev/blog/long-term-rust-maintenance/)
- [Best Way to Structure Rust Web Services - LogRocket](https://blog.logrocket.com/best-way-structure-rust-web-services/)
- [Rust Project Structure and Best Practices](https://www.djamware.com/post/68b2c7c451ce620c6f5efc56/rust-project-structure-and-best-practices-for-clean-scalable-code)
- [Step-by-Step Guide: Refactoring a Large Rust Codebase](https://codenotary.com/blog/step-by-step-guide-refactoring-a-large-rust-codebase-with-aiderdev-and-custom-llms)

### Additional Community Resources (MEDIUM confidence)
- [Integration testing TUI applications in Rust](https://quantonganh.com/2024/01/21/integration-testing-tui-app-in-rust.md)
- [7 Rust Idioms for Clean, High-Performance Code (Dec 2025)](https://medium.com/@jamesmiller22871/7-rust-idioms-for-clean-high-performance-code-6d7433e66d65)
- [7 Rust Anti-Patterns (2025)](https://medium.com/solo-devs/the-7-rust-anti-patterns-that-are-secretly-killing-your-performance-and-how-to-fix-them-in-2025-dcebfdef7b54)
- [async-ratatui example](https://github.com/d-holguin/async-ratatui)
- [How To Structure Unit Tests in Rust](https://betterprogramming.pub/how-to-structure-unit-tests-in-rust-cc4945536a32)

### Clippy GitHub Issues (MEDIUM confidence, directly from maintainers)
- [Clippy cognitive_complexity discussion](https://github.com/rust-lang/rust-clippy/issues/4470)
- [cognitive_complexity includes macro code bug](https://github.com/rust-lang/rust-clippy/issues/14417)
- [Correct cognitive_complexity docs PR](https://github.com/rust-lang/rust-clippy/pull/14915)
