# Phase 1: Foundation - Research

**Researched:** 2026-01-22
**Domain:** Async Rust event loops with tokio + Testing infrastructure
**Confidence:** HIGH

## Summary

Phase 1 focuses on two parallel tracks: refactoring the event loop to use tokio::select! for sub-millisecond keyboard responsiveness, and establishing comprehensive testing infrastructure to enable confident future development.

The current codebase uses a sleep-based polling loop (16ms frames) with std::sync::mpsc channels for keyboard input handled by a dedicated OS thread. The refactor to tokio::select! will eliminate sleep-based polling, convert channels to tokio::sync::mpsc, and introduce an explicit Message enum following the Elm Architecture pattern for predictable state transitions. This enables the event loop to wake immediately when events occur rather than waiting for the next frame tick.

For testing infrastructure, the standard stack is cargo-nextest (3x faster test execution), mockall (trait-based mocking), rstest (fixtures and parameterized tests), pretty_assertions (visual diffs), and cargo-llvm-cov (code coverage). These tools integrate seamlessly and are the de facto standard in the Rust ecosystem as of 2026.

**Primary recommendation:** Use tokio::select! with explicit branch ordering (biased mode) to guarantee keyboard events are processed first, convert all channels to tokio::sync::mpsc, introduce a Message enum for state transitions, and configure cargo-nextest with profiles for local vs CI execution.

## Standard Stack

The established libraries/tools for async Rust event loops and testing:

### Core (Event Loop)
| Library | Version | Purpose | Why Standard |
|---------|---------|---------|--------------|
| tokio | 1.x (latest) | Async runtime with select! macro | Industry standard async runtime, cooperative scheduler, excellent documentation |
| tokio::sync::mpsc | 1.x | Async multi-producer single-consumer channels | Async-aware channels that integrate with select!, proper backpressure |

### Core (Testing)
| Library | Version | Purpose | Why Standard |
|---------|---------|---------|--------------|
| cargo-nextest | Latest | Next-generation test runner | 3x faster via process isolation, JUnit XML output, official nextest-rs project |
| mockall | 0.13+ | Mock object generation | Most feature-complete mocking library, supports async, automock attribute |
| rstest | 0.23+ | Fixtures and parameterized tests | Declarative test setup, reduces boilerplate, async support built-in |
| pretty_assertions | 1.4+ | Better assertion diffs | Colorful diffs, drop-in replacement, minimal overhead (dev-dependencies only) |
| cargo-llvm-cov | 0.6+ | Code coverage via LLVM | Official Rust coverage tool, supports nextest, multiple output formats |

### Supporting
| Library | Version | Purpose | When to Use |
|---------|---------|---------|-------------|
| tokio::time | 1.x | Sleep and interval timers | Replace std::thread::sleep in async code |
| tokio::task | 1.x | Spawn background tasks | Run operations concurrently without select! |

### Alternatives Considered
| Instead of | Could Use | Tradeoff |
|------------|-----------|----------|
| tokio::select! | async-std | Tokio has better ecosystem support, more mature tooling |
| cargo-nextest | cargo test | nextest 3x faster but doesn't support doctests |
| mockall | mockers | mockall has more features and better async support |
| rstest | test-case | rstest has fixture composition, more flexible |

**Installation:**
```bash
# Testing tools (install globally)
cargo install cargo-nextest --locked
cargo install cargo-llvm-cov

# Test dependencies (add to Cargo.toml [dev-dependencies])
cargo add --dev mockall rstest pretty_assertions

# Tokio is already installed, but ensure features are enabled
cargo add tokio --features rt-multi-thread,macros,process,sync,time,io-util
```

## Architecture Patterns

### Recommended Project Structure
Current structure is adequate - no changes needed:
```
src/
├── ui/              # TUI rendering and app state
├── runner/          # Check execution logic
├── config/          # Configuration parsing
└── main.rs          # CLI entry point
```

### Pattern 1: tokio::select! Event Loop with Message Enum
**What:** Replace sleep-based polling with tokio::select! that wakes on any event, using explicit Message enum for state transitions

**When to use:** Any async event loop requiring responsive input handling

**Example:**
```rust
// Source: https://tokio.rs/tokio/tutorial/select + https://ratatui.rs/concepts/application-patterns/the-elm-architecture/

#[derive(Debug)]
enum Message {
    KeyPress(KeyEvent),
    RunnerEvent(RunnerEvent),
    SystemStats(SystemStats),
    Tick,
}

async fn run_event_loop() -> Result<()> {
    let (key_tx, mut key_rx) = mpsc::unbounded_channel();
    let (runner_tx, mut runner_rx) = mpsc::channel(100);
    let (stats_tx, mut stats_rx) = mpsc::channel(4);

    // Spawn keyboard handler
    tokio::spawn(async move {
        // Read keyboard events and send via key_tx
    });

    // Main event loop
    loop {
        let msg = tokio::select! {
            // biased; ensures keyboard is checked first
            biased;

            Some(key) = key_rx.recv() => Message::KeyPress(key),
            Some(event) = runner_rx.recv() => Message::RunnerEvent(event),
            Some(stats) = stats_rx.recv() => Message::SystemStats(stats),
        };

        // Handle message and update state
        if handle_message(&mut app, msg)? == Action::Quit {
            break;
        }

        // Render only if state changed
        if app.needs_redraw {
            terminal.draw(|f| render(&app, f))?;
            app.needs_redraw = false;
        }
    }

    Ok(())
}
```

### Pattern 2: Channel Conversion from std to tokio
**What:** Replace std::sync::mpsc with tokio::sync::mpsc for async-aware message passing

**When to use:** Any channel used in select! branches or async contexts

**Example:**
```rust
// Source: https://tokio.rs/tokio/tutorial/channels

// OLD (std::sync::mpsc - blocks thread)
let (tx, rx) = std::sync::mpsc::channel();
while let Ok(msg) = rx.try_recv() { /* ... */ }

// NEW (tokio::sync::mpsc - async)
let (tx, mut rx) = tokio::sync::mpsc::unbounded_channel();
// In select! branch:
Some(msg) = rx.recv() => { /* ... */ }

// For bounded channels with backpressure:
let (tx, mut rx) = tokio::sync::mpsc::channel(100);
```

**Key differences:**
- tokio mpsc has async `.recv()` that integrates with select!
- std mpsc blocks the thread, not allowed in async code
- tokio unbounded for sync-to-async, bounded for async-to-async
- Use `blocking_send` when sending from sync code to async

### Pattern 3: Biased select! for Priority Ordering
**What:** Use `biased;` parameter to ensure keyboard events are processed before other events

**When to use:** When certain branches must have priority over others

**Example:**
```rust
// Source: https://docs.rs/tokio/latest/tokio/macro.select.html

tokio::select! {
    biased;  // Disables random branch selection

    // Keyboard checked first (highest priority)
    Some(key) = key_rx.recv() => handle_keyboard(key),

    // Runner events second
    Some(event) = runner_rx.recv() => handle_runner(event),

    // System stats last (lowest priority)
    Some(stats) = stats_rx.recv() => handle_stats(stats),
}
```

**Tradeoff:** Manual fairness management required - high-volume channels positioned first could starve lower branches.

### Pattern 4: Cancellation-Safe Channel Operations
**What:** Only use cancellation-safe operations in select! branches to avoid data loss

**When to use:** Always verify operations used in select! are cancellation-safe

**Example:**
```rust
// Source: https://docs.rs/tokio/latest/tokio/macro.select.html

// SAFE - mpsc::Receiver::recv is cancellation-safe
Some(msg) = rx.recv() => { /* ... */ }

// UNSAFE - Mutex::lock loses your place in the queue if cancelled
result = mutex.lock() => { /* ... */ }

// UNSAFE - read_exact loses partial reads if cancelled
n = reader.read_exact(&mut buf) => { /* ... */ }
```

**Cancellation-safe tokio operations:**
- `mpsc::Receiver::recv`
- `broadcast::Receiver::recv`
- `watch::Receiver::changed`
- `TcpListener::accept`
- `AsyncReadExt::read` (NOT read_exact)

### Pattern 5: Test Fixtures with rstest
**What:** Use #[fixture] for reusable test setup, reducing boilerplate

**When to use:** When multiple tests need the same setup (config, temp files, mock objects)

**Example:**
```rust
// Source: https://docs.rs/rstest/latest/rstest/

#[fixture]
fn test_config() -> CiConfig {
    CiConfig {
        docker: DockerConfig { /* ... */ },
        checks: vec![/* ... */],
    }
}

#[fixture]
fn mock_runner() -> MockCheckRunner {
    let mut runner = MockCheckRunner::new();
    runner.expect_run_checks()
        .returning(|_| Ok(()));
    runner
}

#[rstest]
fn test_with_fixtures(test_config: CiConfig, mock_runner: MockCheckRunner) {
    // Test uses fixtures automatically
}

// Parameterized tests
#[rstest]
#[case(0, 0)]
#[case(1, 1)]
#[case(5, 120)]
fn test_factorial(#[case] input: u32, #[case] expected: u32) {
    assert_eq!(factorial(input), expected);
}
```

### Pattern 6: Trait Mocking with mockall
**What:** Use #[automock] to generate mock implementations of traits

**When to use:** Testing code that depends on traits (runners, file I/O, network)

**Example:**
```rust
// Source: https://docs.rs/mockall/latest/mockall/

use mockall::automock;

#[automock]
trait CheckRunner {
    async fn run_checks(&self, checks: Vec<CheckToRun>) -> Result<()>;
    fn cancel(&self);
}

#[tokio::test]
async fn test_runner() {
    let mut mock = MockCheckRunner::new();

    mock.expect_run_checks()
        .times(1)
        .returning(|_| Ok(()));

    mock.expect_cancel()
        .never();

    // Use mock in test
    let result = mock.run_checks(vec![]).await;
    assert!(result.is_ok());
}
```

### Pattern 7: nextest Configuration Profiles
**What:** Use .config/nextest.toml with profiles for different test scenarios

**When to use:** Always - separates local dev from CI configuration

**Example:**
```toml
# Source: https://nexte.st/docs/configuration/

# .config/nextest.toml
[profile.default]
retries = 0
test-threads = "num-cpus"

[profile.ci]
fail-fast = false
retries = 2
test-threads = "num-cpus"

[profile.ci.junit]
path = "target/nextest/ci/junit.xml"
```

**Usage:**
```bash
# Local development
cargo nextest run

# CI pipeline
cargo nextest run --profile ci
```

### Pattern 8: Code Coverage with cargo-llvm-cov
**What:** Generate coverage reports for tests and CI integration

**When to use:** CI pipelines and pre-commit checks

**Example:**
```bash
# Source: https://github.com/taiki-e/cargo-llvm-cov

# Local HTML report
cargo llvm-cov nextest --open

# CI - generate LCOV for Codecov/Coveralls
cargo llvm-cov nextest --lcov --output-path lcov.info

# Merge coverage from multiple runs
cargo llvm-cov clean --workspace
cargo llvm-cov nextest --no-report --features feature-a
cargo llvm-cov nextest --no-report --features feature-b
cargo llvm-cov report --lcov
```

### Anti-Patterns to Avoid

- **Mixing std and tokio mpsc**: Use tokio::sync::mpsc everywhere in async code, only use std::sync::mpsc in pure sync contexts
- **Blocking operations in async**: Never use std::thread::sleep, std::fs, or blocking I/O in async functions - use tokio equivalents
- **try_recv in select!**: Don't use try_recv() in select! branches - let the async .recv() handle waiting
- **Unbounded select! loops**: Always have a way to exit select! loops (quit flag, channel closed, timeout)
- **Non-cancellation-safe operations**: Avoid Mutex::lock, read_exact, write_all in select! branches - they lose state when cancelled
- **Forgetting biased keyword**: If priority matters, explicitly use `biased;` rather than relying on implicit behavior

## Don't Hand-Roll

Problems that look simple but have existing solutions:

| Problem | Don't Build | Use Instead | Why |
|---------|-------------|-------------|-----|
| Test fixtures | Manual setup functions | rstest #[fixture] | Handles cleanup, composition, async, reduces boilerplate |
| Mocking traits | Manual stub implementations | mockall #[automock] | Generates mocks automatically, expectations API, thread-safe |
| Test parallelization | Custom test harness | cargo-nextest | Process isolation prevents state leaks, 3x faster, JUnit output |
| Code coverage | Manual instrumentation | cargo-llvm-cov | Official tool, integrates with nextest, multiple formats |
| Assertion diffs | Custom formatters | pretty_assertions | Drop-in replacement, colorful diffs, maintained |
| Event loop polling | sleep/interval loops | tokio::select! | Wakes on events, no wasted CPU cycles, composable |
| Channel backpressure | Manual queuing logic | tokio::sync::mpsc bounded | Built-in flow control, async-aware, prevents OOM |
| Parameterized tests | Loop-based tests | rstest #[case] | Separate test runs, better failure reporting, IDE integration |

**Key insight:** The Rust async ecosystem is mature - building custom solutions for these problems means reimplementing complex edge case handling (cancellation safety, fairness, cleanup, thread safety) that took years to get right in these libraries.

## Common Pitfalls

### Pitfall 1: Forgetting Cancellation Safety in select!
**What goes wrong:** Using non-cancellation-safe operations in select! branches causes data loss when a different branch completes first. Example: Mutex::lock() in one branch gets cancelled when another branch completes, losing your place in the lock queue.

**Why it happens:** The select! macro drops all unfinished futures when one completes. Operations with intermediate state (partially read data, queue position) lose that state.

**How to avoid:**
1. Only use operations explicitly documented as cancellation-safe in select! branches
2. For unsafe operations, spawn them in a separate task: `tokio::spawn(async move { /* unsafe operation */ })`
3. Review the cancellation safety section at https://docs.rs/tokio/latest/tokio/macro.select.html

**Warning signs:**
- Tests flake when run with high parallelism
- Messages appear to be dropped from channels
- Operations restart from the beginning unexpectedly

### Pitfall 2: Blocking the Tokio Runtime
**What goes wrong:** Using std::thread::sleep, std::fs::File, or other blocking operations in async code prevents the runtime from scheduling other tasks, causing the entire application to freeze.

**Why it happens:** Tokio uses cooperative scheduling - tasks must voluntarily yield via .await. Blocking operations hold the thread indefinitely without yielding.

**How to avoid:**
1. Use tokio::time::sleep instead of std::thread::sleep
2. Use tokio::fs instead of std::fs
3. Use tokio::process instead of std::process
4. For unavoidable blocking code, use `tokio::task::spawn_blocking`

**Warning signs:**
- Application becomes unresponsive during operations that should be non-blocking
- Keyboard input doesn't work during file I/O
- Tasks scheduled but never execute

### Pitfall 3: Unbounded Channel Memory Exhaustion
**What goes wrong:** Using unbounded channels (std::sync::mpsc::channel or tokio::sync::mpsc::unbounded_channel) with slow consumers causes memory to grow until OOM.

**Why it happens:** Unbounded channels never apply backpressure - senders can enqueue unlimited messages while the receiver falls behind.

**How to avoid:**
1. Use bounded channels with appropriate capacity: `tokio::sync::mpsc::channel(100)`
2. The sender will block when capacity is reached, providing natural flow control
3. Only use unbounded for sync-to-async communication where the sender can't block (like OS thread to async task)

**Warning signs:**
- Memory usage grows linearly over time
- OOM errors during high-load scenarios
- Channel sizes grow to millions of messages

### Pitfall 4: Incorrect std/tokio Channel Mixing
**What goes wrong:** Using std::sync::mpsc in select! branches fails to compile, or using try_recv() in a loop wastes CPU.

**Why it happens:** std channels don't implement async traits needed by select!, and try_recv() requires polling loops.

**How to avoid:**
1. Use tokio::sync::mpsc for all channels in async code
2. Use `rx.recv().await` in select! branches, not try_recv()
3. Only use std::sync::mpsc when both sender and receiver are in sync (non-async) code

**Warning signs:**
- Compiler error: "std::sync::mpsc::Receiver doesn't implement Future"
- High CPU usage from polling loops
- Latency spikes when messages arrive between polls

### Pitfall 5: cargo-nextest Doctest Confusion
**What goes wrong:** Running `cargo nextest run` reports success but doctests are silently skipped, leading to false confidence in test coverage.

**Why it happens:** cargo-nextest doesn't support doctests due to Rust compiler limitations. Developers coming from cargo test assume all tests run.

**How to avoid:**
1. Always run doctests separately: `cargo test --doc`
2. Document this in CI configuration
3. Add a check in CI that fails if only nextest is run

**Warning signs:**
- Coverage drops when switching from cargo test to nextest
- Doctests that fail with cargo test pass with nextest
- Fewer tests reported than expected

### Pitfall 6: Forgetting to Set biased for Priority
**What goes wrong:** Without `biased;`, tokio::select! randomly picks which branch to check first, causing keyboard input to be delayed by other high-volume channels.

**Why it happens:** Default select! behavior prioritizes fairness over priority. With random selection, a high-throughput runner event channel might be checked before keyboard 50% of the time.

**How to avoid:**
1. Use `biased;` at the top of select! when priority matters
2. Order branches from highest to lowest priority
3. Monitor branch execution order in testing

**Warning signs:**
- Inconsistent input latency
- Input feels "laggy" during high runner output
- Profiling shows keyboard handler called less frequently than expected

### Pitfall 7: mockall Expectation Order Violations
**What goes wrong:** Tests fail with "unexpected method call" even though the method was mocked. This happens when expectations are set in one order but calls happen in a different order.

**Why it happens:** mockall expectations are checked in FIFO order. Without using Sequence, there's no ordering guarantee between different method calls.

**How to avoid:**
1. Use `.in_sequence(&mut seq)` when call order matters
2. Set `.times(1..=N)` for flexible call counts
3. Use `.times(mockall::predicate::always())` for "any number of calls"

**Warning signs:**
- Intermittent test failures
- Failures only occur when tests run in parallel
- Error message: "No matching expectation found"

### Pitfall 8: rstest Fixture Lifetime Issues
**What goes wrong:** Fixtures that return references fail to compile with "borrowed value does not live long enough" errors.

**Why it happens:** rstest fixtures are called at the start of each test function. References can't outlive the fixture function scope.

**How to avoid:**
1. Have fixtures return owned types (Vec, String, Box) not references
2. Use Arc for shared data across fixtures
3. For expensive setup, use `#[fixture(once)]` to initialize once

**Warning signs:**
- Compiler errors about temporary values dropped
- Tests work when fixture is inlined but fail when extracted
- Lifetime parameter soup in fixture signatures

## Code Examples

Verified patterns from official sources:

### Complete Event Loop Refactor
```rust
// Source: https://tokio.rs/tokio/tutorial/select + https://ratatui.rs/concepts/application-patterns/the-elm-architecture/

use tokio::sync::mpsc;
use crossterm::event::{KeyEvent, KeyEventKind};

#[derive(Debug)]
enum Message {
    KeyPress(KeyEvent),
    RunnerEvent(RunnerEvent),
    SystemStats(SystemStats),
}

pub async fn run(
    config: CiConfig,
    changed_files: ChangedFiles,
    checks: Vec<CheckToRun>,
    project_root: PathBuf,
) -> Result<()> {
    // Setup terminal
    enable_raw_mode()?;
    let mut stdout = io::stdout();
    execute!(stdout, EnterAlternateScreen, EnableMouseCapture)?;
    let backend = CrosstermBackend::new(stdout);
    let mut terminal = Terminal::new(backend)?;

    // Create app state
    let mut app = App::new(config.clone(), changed_files, checks.clone());

    // Create channels
    let (key_tx, mut key_rx) = mpsc::unbounded_channel();
    let (runner_tx, mut runner_rx) = mpsc::channel(100);
    let (stats_tx, mut stats_rx) = mpsc::channel(4);

    // Spawn keyboard handler (uses tokio::spawn, NOT std::thread)
    tokio::spawn(async move {
        loop {
            if event::poll(Duration::from_millis(50)).unwrap_or(false) {
                if let Ok(Event::Key(key)) = event::read() {
                    if key.kind == KeyEventKind::Press {
                        if key_tx.send(key).is_err() {
                            break; // Receiver dropped
                        }
                    }
                }
            }
        }
    });

    // Spawn runner
    tokio::spawn(async move {
        let runner = CheckRunner::new(config, &project_root);
        runner.run_checks(checks, runner_tx).await
    });

    // Spawn stats worker
    tokio::spawn(async move {
        let mut system = System::new();
        let mut interval = tokio::time::interval(Duration::from_millis(500));
        loop {
            interval.tick().await;
            system.refresh_cpu_usage();
            system.refresh_memory();
            let stats = SystemStats { /* ... */ };
            let _ = stats_tx.try_send(stats);
        }
    });

    // Main event loop
    loop {
        let msg = tokio::select! {
            biased;  // Keyboard always checked first

            Some(key) = key_rx.recv() => Message::KeyPress(key),
            Some(event) = runner_rx.recv() => Message::RunnerEvent(event),
            Some(stats) = stats_rx.recv() => Message::SystemStats(stats),
        };

        // Handle message
        let action = handle_message(&mut app, msg)?;

        if action == Action::Quit {
            break;
        }

        // Render if needed
        if app.needs_redraw {
            terminal.draw(|f| dashboard::render(&app, f))?;
            app.needs_redraw = false;
        }
    }

    restore_terminal()?;
    Ok(())
}

fn handle_message(app: &mut App, msg: Message) -> Result<Action> {
    match msg {
        Message::KeyPress(key) => {
            app.needs_redraw = true;
            match key.code {
                KeyCode::Char('q') => Ok(Action::Quit),
                KeyCode::Char('j') => {
                    app.next_check();
                    Ok(Action::Continue)
                }
                KeyCode::Char('k') => {
                    app.previous_check();
                    Ok(Action::Continue)
                }
                _ => Ok(Action::Continue),
            }
        }
        Message::RunnerEvent(event) => {
            app.needs_redraw = true;
            app.handle_runner_event(event);
            Ok(Action::Continue)
        }
        Message::SystemStats(stats) => {
            app.update_stats(stats.cpu_usage, stats.mem_used, stats.mem_total);
            app.needs_redraw = true;
            Ok(Action::Continue)
        }
    }
}
```

### Testing with Fixtures and Mocks
```rust
// Source: https://docs.rs/rstest/latest/rstest/ + https://docs.rs/mockall/latest/mockall/

use rstest::*;
use mockall::automock;
use pretty_assertions::assert_eq;

#[fixture]
fn test_config() -> CiConfig {
    CiConfig {
        docker: DockerConfig {
            project_dir: "/app".to_string(),
            service: "app".to_string(),
            env: HashMap::new(),
        },
        checks: vec![
            CheckDefinition {
                name: "phpunit".to_string(),
                command: "vendor/bin/phpunit".to_string(),
                file_patterns: vec!["**/*.php".to_string()],
                on_demand: false,
            },
        ],
        ignore_patterns: vec![],
    }
}

#[fixture]
fn changed_files() -> ChangedFiles {
    ChangedFiles {
        files: vec![
            "src/app/Models/User.php".to_string(),
            "tests/Unit/UserTest.php".to_string(),
        ],
        base_ref: "origin/main".to_string(),
    }
}

#[automock]
trait CheckRunner {
    async fn run_checks(&self, checks: Vec<CheckToRun>, tx: mpsc::Sender<RunnerEvent>) -> Result<()>;
}

#[rstest]
#[case("src/app/Models/User.php", true)]
#[case("src/assets/image.png", false)]
#[case("tests/Unit/UserTest.php", true)]
fn test_file_patterns(
    test_config: CiConfig,
    #[case] file_path: &str,
    #[case] should_match: bool,
) {
    let result = matches_any_pattern(&test_config.checks[0].file_patterns, file_path);
    assert_eq!(result, should_match, "File: {}", file_path);
}

#[rstest]
#[tokio::test]
async fn test_runner_integration(
    test_config: CiConfig,
    changed_files: ChangedFiles,
) {
    let checks = determine_checks(&test_config, &changed_files, &PathBuf::from("/app"));

    let mut mock_runner = MockCheckRunner::new();
    mock_runner.expect_run_checks()
        .times(1)
        .returning(|_, _| Ok(()));

    let (tx, mut rx) = mpsc::channel(10);
    let result = mock_runner.run_checks(checks, tx).await;

    assert!(result.is_ok());
}
```

### cargo-nextest Configuration
```toml
# Source: https://nexte.st/docs/configuration/

# .config/nextest.toml
[profile.default]
retries = 0
test-threads = "num-cpus"
fail-fast = true

[profile.ci]
retries = 2
test-threads = "num-cpus"
fail-fast = false

[profile.ci.junit]
path = "target/nextest/junit.xml"

[[profile.default.overrides]]
filter = 'test(integration_)'
retries = 1
threads-required = 1
```

### GitHub Actions CI Pipeline
```yaml
# Source: https://github.com/taiki-e/cargo-llvm-cov + https://nexte.st/

name: CI

on: [push, pull_request]

jobs:
  test:
    runs-on: ubuntu-latest
    steps:
      - uses: actions/checkout@v4

      - name: Install Rust
        uses: dtolnay/rust-toolchain@stable

      - name: Install cargo-nextest
        uses: taiki-e/install-action@nextest

      - name: Install cargo-llvm-cov
        uses: taiki-e/install-action@cargo-llvm-cov

      - name: Run tests with coverage
        run: cargo llvm-cov nextest --lcov --output-path lcov.info --profile ci

      - name: Run doctests
        run: cargo test --doc

      - name: Upload coverage to Codecov
        uses: codecov/codecov-action@v4
        with:
          files: lcov.info
          fail_ci_if_error: true

  lint:
    runs-on: ubuntu-latest
    steps:
      - uses: actions/checkout@v4

      - name: Install Rust
        uses: dtolnay/rust-toolchain@stable
        with:
          components: clippy, rustfmt

      - name: Check formatting
        run: cargo fmt -- --check

      - name: Run clippy
        run: cargo clippy -- -D warnings
```

## State of the Art

| Old Approach | Current Approach | When Changed | Impact |
|--------------|------------------|--------------|--------|
| sleep-based event loops | tokio::select! | Tokio 0.2+ (2019) | Eliminates wasted CPU cycles, immediate wake on events |
| std::sync::mpsc in async | tokio::sync::mpsc | Tokio 1.0 (2021) | Native async support, integrates with select! |
| cargo test | cargo-nextest | 2021-2022 | 3x speed improvement via process isolation |
| Manual mocking | mockall with #[automock] | 2020+ | Reduces boilerplate, compile-time safety |
| Loop-based parameterized tests | rstest #[case] | 2019+ | Better failure reporting, IDE integration |
| tarpaulin (Linux-only) | cargo-llvm-cov | 2022+ | Cross-platform, official Rust coverage tool |
| Random select! polling | biased select! for priority | Tokio 1.0+ | Explicit control over branch checking order |

**Deprecated/outdated:**
- `tokio-core`: Replaced by tokio 1.0+ unified runtime
- `futures::mpsc`: Use tokio::sync::mpsc for tokio applications
- `cargo-tarpaulin`: Still works but cargo-llvm-cov is officially recommended
- `test-case`: rstest has superseded it with more features

## Open Questions

Things that couldn't be fully resolved:

1. **Keyboard Thread vs Async Task**
   - What we know: Current code uses std::thread for keyboard input to ensure OS-level scheduling
   - What's unclear: Whether tokio::spawn would provide the same guarantees under high CPU load
   - Recommendation: Test both approaches with CPU stress testing. The std::thread approach may still be necessary if tokio tasks get starved, but initial refactor should try tokio::spawn with biased select! first.

2. **Message Enum Granularity**
   - What we know: Elm Architecture recommends fine-grained messages (Increment, Decrement) vs coarse messages (KeyPress(KeyEvent))
   - What's unclear: Optimal balance between granular messages (easier testing) and practical key handling (less boilerplate)
   - Recommendation: Start with KeyPress(KeyEvent) message, extract high-level actions (RetryCheck, ToggleFilter) as distinct messages. Refine based on testing experience.

3. **Target Response Time Measurement**
   - What we know: Requirement is <1ms keyboard response time
   - What's unclear: How to instrument and measure this accurately in tests
   - Recommendation: Use tokio::time::Instant before/after message handling, add assertion helper that fails if >1ms. May need to be integration test rather than unit test.

## Sources

### Primary (HIGH confidence)
- [tokio::select! tutorial](https://tokio.rs/tokio/tutorial/select) - Official tokio documentation on select! macro usage, cancellation safety, patterns
- [tokio::select! API docs](https://docs.rs/tokio/latest/tokio/macro.select.html) - Complete API reference including biased parameter and cancellation safety list
- [tokio channels tutorial](https://tokio.rs/tokio/tutorial/channels) - Official guide on tokio::sync::mpsc vs std::sync::mpsc
- [cargo-nextest documentation](https://nexte.st/) - Official nextest docs including configuration, performance claims, limitations
- [cargo-nextest configuration](https://nexte.st/docs/configuration/) - Profile setup, overrides, CI configuration
- [mockall documentation](https://docs.rs/mockall/latest/mockall/) - Official API docs for #[automock], expectations, async support
- [rstest documentation](https://docs.rs/rstest/latest/rstest/) - Official API docs for fixtures, parameterized tests, async
- [cargo-llvm-cov README](https://github.com/taiki-e/cargo-llvm-cov) - Official installation, usage, nextest integration
- [Ratatui Elm Architecture guide](https://ratatui.rs/concepts/application-patterns/the-elm-architecture/) - Message enum pattern for TUI apps

### Secondary (MEDIUM confidence)
- [Tokio cancellation safety discussion](https://developerlife.com/2024/07/10/rust-async-cancellation-safety-tokio/) - Detailed article on cancellation safety pitfalls (2024)
- [Oxide RFD 400: Cancel Safety](https://rfd.shared.oxide.computer/rfd/400) - Engineering discussion of cancel safety patterns
- [Ratatui async event stream tutorial](https://ratatui.rs/tutorials/counter-async-app/async-event-stream/) - Pattern for keyboard input in async TUI

### Tertiary (LOW confidence)
- WebSearch results for "rust async event loop patterns" - General patterns, not specific recommendations

## Metadata

**Confidence breakdown:**
- Standard stack: HIGH - All tools are official or de facto standards with extensive documentation
- Architecture: HIGH - Patterns sourced from official tokio and ratatui documentation
- Pitfalls: HIGH - Based on official cancellation safety docs and community experience articles
- Response time measurement: LOW - No authoritative source found for <1ms testing approach

**Research date:** 2026-01-22
**Valid until:** 2026-02-22 (30 days - tokio and testing tools are stable)
