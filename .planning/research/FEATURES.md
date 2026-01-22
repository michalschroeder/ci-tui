# Code Quality Patterns for Rust TUI Applications

**Domain:** Rust TUI application maintainability and code quality
**Researched:** 2026-01-22
**Focus:** What patterns make a Rust TUI codebase maintainable, testable, and idiomatic

## Table Stakes

Code quality features that well-maintained Rust TUI applications must have. Missing these = technical debt accumulates fast.

| Feature | Why Expected | Complexity | Notes |
|---------|--------------|------------|-------|
| **Clear separation of concerns (Model/View/Update)** | TEA or MVC pattern prevents UI entanglement with business logic | Medium | Ratatui docs explicitly recommend this. View should be pure function, state isolated in Model |
| **Proper async event handling with tokio::select!** | Prevents UI responsiveness issues during I/O | Medium | Use spawn_blocking for input polling to avoid blocking main loop. Critical for keyboard responsiveness |
| **Terminal cleanup on panic** | Corrupted terminal state on crash = horrible UX | Low | Install panic hook that restores terminal before displaying error |
| **Linting with Clippy at deny level** | Catches common mistakes, enforces idioms | Low | Run `cargo clippy -- -D warnings` in CI. Addresses unused code, style issues |
| **Rustfmt for consistent formatting** | Readability and merge conflicts | Low | Standard 4-space indentation, snake_case naming enforced automatically |
| **Unit tests for business logic** | Core logic (checks, config parsing, discovery) must be testable in isolation | Medium | Use TestBackend for widget tests, prefer unit tests over integration tests |
| **Error handling with Result types** | ? operator propagation, no unwrap() in production paths | Low | Use thiserror for library code, anyhow for application code |
| **No dead code or unused imports** | Sign of incomplete refactoring or AI-generated cruft | Low | Enable dead_code lint, remove during cleanup phase |
| **Documented non-obvious code paths** | Future maintainers (including yourself) need context | Low | Doc comments with /// for public items, // for complex internal logic |
| **Idiomatic Rust patterns** | Use if let, while let, iterators over manual loops | Medium | Follow Rust API Guidelines, avoid clone() abuse |

## Differentiators

Patterns that distinguish high-quality TUI codebases from adequate ones. Not required but highly valued.

| Feature | Value Proposition | Complexity | Notes |
|---------|-------------------|------------|-------|
| **Component-based architecture** | Modular UI with isolated components in components/ directory | High | Once set up, most work stays in components/ folder. Scales to complex UIs |
| **Action/Command mapping system** | Decouple key bindings from business logic | Medium | Define Action enum, map keys to actions, enables configurable keybindings |
| **File-based logging (tracing/log4rs)** | Debug without interfering with TUI rendering | Medium | XDG-compliant directories for user-shareable debug logs |
| **State sharing with Arc<Mutex<T>>** | Clean multi-threaded state management | Medium | Wrap shared state, provide accessor methods that handle locking internally |
| **Immediate-mode rendering at fixed FPS** | Simplifies render logic, ensures consistent responsiveness | Low | Separate tick rate (state updates) from render rate (30-60 FPS) |
| **Defensive programming with #[must_use]** | Prevents accidentally ignoring important return values | Low | Mark functions where ignoring result is likely a mistake |
| **Integration tests with ratatui-testlib** | End-to-end TUI testing with real PTY | High | Complement unit tests for full user interaction testing |
| **XDG-compliant configuration** | Config in ~/.config/app/, follows platform conventions | Low | Use directories or dirs crate, support TOML/YAML with config-rs |
| **Comprehensive module responsibilities** | Each module has clear, documented scope | Medium | Like CLAUDE.md "Module Responsibilities" section |
| **TestBackend for widget tests** | Render widgets to buffer, assert output without real terminal | Medium | Preferred over integration tests for widget-level testing |

## Anti-Features

Patterns to explicitly avoid or remove. Common in AI-generated code or premature optimization.

| Anti-Feature | Why Avoid | What to Do Instead |
|--------------|-----------|-------------------|
| **Blocking I/O on main thread** | Kills keyboard responsiveness, makes UI feel frozen | Use tokio::spawn_blocking for any blocking operations including input polling |
| **Over-abstraction layers** | AI loves unnecessary trait hierarchies and generic wrappers | Keep it simple. Two levels of abstraction max for most code |
| **Using .clone() everywhere** | Top Rust anti-pattern 2025, performance killer | Use references, Arc for shared ownership, Rc for single-threaded |
| **Unwrap() or expect() in production paths** | Panics = terminal corruption = terrible UX | Return Result, install panic hook as safety net only |
| **Preludes for application code** | Makes imports unclear, only justified for frameworks | Re-export common types at crate root with pub use instead |
| **Feature modules with pub(crate) everything** | Hides actual public API surface | Use pub selectively, document intended public interface |
| **Integration tests in tests/ folder** | Slower, harder to maintain than unit tests | Prefer unit tests and doc tests directly in source files |
| **Awaiting long operations on main thread** | UI won't receive updates or key events until complete | Spawn separate task/thread, send results via channel |
| **Manual loops instead of iterators** | Unidiomatic, harder to read, potentially slower | Use .iter(), .filter(), .map(), etc. |
| **Ignoring clippy::restriction lints** | Some are valuable (e.g., unwrap_used) for production code | Review restriction lints case-by-case, enable selectively |
| **Global mutable state without synchronization** | Race conditions, undefined behavior | Use Arc<Mutex<T>> or channels for shared state |
| **Tightly coupled modules** | Can't test in isolation, changes ripple through codebase | Dependency injection via traits or constructor parameters |

## Code Organization Patterns

### Recommended File Structure (Component-Based)

```
src/
├── main.rs           # Entry point, initialization
├── tui.rs            # Terminal setup/teardown, panic hooks
├── app.rs            # Main application state and event loop
├── action.rs         # Action/Command enum for key mapping
├── cli.rs            # CLI argument parsing (clap)
├── config.rs         # Configuration loading/parsing
├── errors.rs         # Error types (thiserror)
├── logging.rs        # Logging setup (tracing)
├── components/       # Modular UI components
│   ├── home.rs
│   ├── fps.rs
│   └── ...
└── components.rs     # Component trait definitions

tests/
└── (minimal integration tests only)
```

**Key principle:** "Once you have set up the project, you shouldn't need to change the contents of anything outside the `components` folder."

### Module Organization Best Practices

- **One file per module** when possible (utils.rs, config.rs)
- **Directory with mod.rs** only for grouping related submodules
- **pub(crate)** to expose items only within crate
- **pub use at root** to re-export common types cleanly
- **Module docstrings** explaining responsibility boundaries

## Testing Patterns

### Widget/View Testing
```rust
use ratatui::backend::TestBackend;
use ratatui::Terminal;

let mut backend = TestBackend::new(80, 24);
let mut terminal = Terminal::new(backend)?;
terminal.draw(|frame| {
    // Render widget
})?;

// Assert buffer contents
let buffer = terminal.backend().buffer();
assert_eq!(buffer.get(0, 0).symbol(), "expected");
```

### State Logic Testing
```rust
// Business logic functions should take state as parameter
fn update(model: &mut Model, msg: Message) -> Result<()> {
    match msg {
        Message::Tick => model.tick(),
        // ...
    }
}

#[cfg(test)]
mod tests {
    #[test]
    fn test_update_logic() {
        let mut model = Model::default();
        update(&mut model, Message::Tick).unwrap();
        assert_eq!(model.state, expected);
    }
}
```

### Integration Testing Strategy
- **Unit tests:** Business logic, configuration parsing, data transformations
- **TestBackend:** Widget rendering, layout calculations
- **ratatui-testlib:** Full application flow with PTY (expensive, use sparingly)

## Async Responsiveness Patterns

### The Responsiveness Contract

**Critical rule:** Main thread must never block. Rendering and input handling live or die on main thread responsiveness.

### Event Loop Pattern (tokio::select!)

```rust
loop {
    tokio::select! {
        // Fixed interval for state updates
        _ = tick_interval.tick() => {
            tx.send(Message::Tick).await?;
        }

        // Fixed interval for rendering
        _ = render_interval.tick() => {
            tx.send(Message::Render).await?;
        }

        // Non-blocking message processing
        Some(msg) = event_rx.recv() => {
            handle_message(msg).await?;
        }

        // Blocking input on separate task
        _ = tokio::task::spawn_blocking(|| {
            crossterm::event::poll(Duration::from_millis(100))
        }) => {
            if let Ok(event) = crossterm::event::read() {
                handle_input(event).await?;
            }
        }
    }
}
```

**Key insight:** Input polling uses spawn_blocking so it doesn't freeze async loop. All branches progress concurrently.

## Error Handling Patterns

### Library vs Application Code

| Context | Use | Pattern |
|---------|-----|---------|
| **Application code** (main.rs, app.rs) | anyhow::Result<T> | Simple error propagation with context |
| **Library/reusable modules** | thiserror custom errors | Structured errors caller can match on |
| **Widget rendering** | Don't panic, return Result or skip | UI should degrade gracefully |
| **Critical paths** | #[must_use] on Result | Prevent accidentally ignoring errors |

### Panic Hook Pattern

```rust
// In tui.rs setup
let original_hook = std::panic::take_hook();
std::panic::set_hook(Box::new(move |panic_info| {
    crossterm::terminal::disable_raw_mode().ok();
    crossterm::execute!(io::stdout(), LeaveAlternateScreen).ok();
    original_hook(panic_info);
}));
```

## Linting Configuration

### Recommended Clippy Settings

```toml
# In Cargo.toml or .cargo/config.toml
[lints.rust]
unsafe_code = "forbid"
dead_code = "deny"
unused_imports = "deny"

[lints.clippy]
# Core lints
all = "deny"
correctness = "deny"
style = "warn"      # Opinionated, some teams disable
complexity = "warn"

# Selective restriction lints
unwrap_used = "deny"         # For production code
expect_used = "warn"         # expect() better than unwrap()
panic = "deny"               # No explicit panics
todo = "warn"                # Track unfinished work
```

### CI Integration

```bash
# In CI pipeline
cargo fmt -- --check
cargo clippy -- -D warnings
cargo test
```

## Idiomatic Rust Checklist

- [ ] **Naming:** snake_case (vars/fns), PascalCase (types), SCREAMING_SNAKE_CASE (consts)
- [ ] **Pattern matching:** Use if let and while let instead of match with single arm
- [ ] **Error propagation:** ? operator instead of manual match/unwrap
- [ ] **Iterators:** .iter().filter().map() instead of manual for loops
- [ ] **Borrowing:** References by default, clone only when ownership needed
- [ ] **Traits:** Implement standard traits (Debug, Clone, Default, Display)
- [ ] **Documentation:** /// doc comments for public API, examples in docs
- [ ] **Edition:** Use Rust 2021 idioms (no extern crate, use for macros)

## MVP Recommendations for Brownfield Cleanup

For cleaning up AI-generated TUI code, prioritize in order:

### Phase 1: Foundation (Must Fix)
1. Fix async event handling for keyboard responsiveness (blocking I/O to spawn_blocking)
2. Add panic hook for terminal cleanup
3. Enable clippy at deny level, fix all warnings
4. Remove dead code and unused imports
5. Apply rustfmt consistently

### Phase 2: Structure (High Value)
1. Separate Model/View/Update concerns clearly
2. Move business logic out of UI rendering
3. Document module responsibilities
4. Add unit tests for core logic (config, checks, discovery)

### Phase 3: Quality (Nice to Have)
1. Implement Action/Command mapping for extensibility
2. Add file-based logging for debugging
3. TestBackend tests for widgets
4. Refactor clone() abuse to proper borrowing

## Confidence Assessment

| Pattern Category | Confidence | Source |
|-----------------|------------|--------|
| TEA/MVC architecture | HIGH | Official Ratatui docs, multiple articles |
| Async responsiveness patterns | HIGH | async-ratatui example, community discussions |
| Testing with TestBackend | HIGH | Ratatui API docs, contributing guide |
| Clippy/rustfmt standards | HIGH | Official Rust tooling docs |
| Error handling (anyhow/thiserror) | HIGH | Multiple 2025 guides, widespread adoption |
| Component-based structure | MEDIUM | Ratatui templates (newer pattern) |
| Anti-patterns (clone abuse, blocking I/O) | HIGH | 2025 Rust anti-pattern articles, performance guides |
| Prelude avoidance | MEDIUM | Community debate, Tokio removed theirs |

## Sources

### Official Documentation
- [Best practices for ratatui apps - GitHub Discussion #220](https://github.com/ratatui/ratatui/discussions/220)
- [The Elm Architecture (TEA) | Ratatui](https://ratatui.rs/concepts/application-patterns/the-elm-architecture/)
- [Project Structure | Ratatui](https://ratatui.rs/templates/component/project-structure/)
- [TestBackend in ratatui::backend - Rust Docs](https://docs.rs/ratatui/latest/ratatui/backend/struct.TestBackend.html)
- [Clippy Documentation](https://doc.rust-lang.org/clippy/usage.html)

### Architecture & Patterns
- [async-ratatui - Handling multiple events asynchronously](https://github.com/d-holguin/async-ratatui)
- [Integration testing TUI applications in Rust](https://quantonganh.com/2024/01/21/integration-testing-tui-app-in-rust.md)
- [7 Rust Idioms for Clean, High-Performance Code (Dec 2025)](https://medium.com/@jamesmiller22871/7-rust-idioms-for-clean-high-performance-code-6d7433e66d65)
- [Rust Design Patterns - Anti-patterns](https://rust-unofficial.github.io/patterns/anti_patterns/)

### Error Handling
- [Rust Error Handling Guide 2025: New Techniques and Best Practices](https://markaicode.com/rust-error-handling-2025-guide/)
- [Rust Error Handling: thiserror, anyhow, and When to Use Each](https://momori.dev/posts/rust-error-handling-thiserror-anyhow/)

### Testing
- [ratatui-testlib - Integration testing library](https://lib.rs/crates/ratatui-testlib)
- [Testing - Command Line Applications in Rust](https://rust-cli.github.io/book/tutorial/testing.html)

### Code Quality
- [The 7 Rust Anti-Patterns Secretly Killing Your Performance (2025)](https://medium.com/solo-devs/the-7-rust-anti-patterns-that-are-secretly-killing-your-performance-and-how-to-fix-them-in-2025-dcebfdef7b54)
- [Patterns for Defensive Programming in Rust](https://corrode.dev/blog/defensive-programming/)
- [Don't Use Preludes And Globs](https://corrode.dev/blog/dont-use-preludes-and-globs/)
- [Rust Project Structure and Best Practices for Clean, Scalable Code](https://www.djamware.com/post/68b2c7c451ce620c6f5efc56/rust-project-structure-and-best-practices-for-clean-scalable-code)

### Responsiveness
- [Text-mode terminal application with asynchronous input/output](https://users.rust-lang.org/t/text-mode-terminal-application-with-asynchronous-input-output/74760)
- [r3bl_tui - Async TUI framework](https://docs.rs/r3bl_tui/latest/r3bl_tui/)
