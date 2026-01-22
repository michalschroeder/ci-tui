# Async Rust TUI Pitfalls

**Domain:** Async Rust Terminal User Interfaces (TUI) with Tokio, Crossterm, and Ratatui
**Researched:** 2026-01-22
**Confidence:** HIGH

## Executive Summary

Async Rust TUI applications face unique challenges combining cooperative async runtimes (Tokio) with synchronous terminal I/O (Crossterm). The most critical pitfall is **event loop monopolization**: a single long-running async task that processes events in a tight loop with `tokio::time::sleep`, which prevents responsive input handling even with dedicated input threads. This document catalogs pitfalls specific to the tokio + crossterm + ratatui stack, with actionable detection and remediation strategies.

---

## Critical Pitfalls

These mistakes cause severe responsiveness issues or require architectural rewrites.

### Pitfall 1: Single Async Task Monopolizes Event Loop

**What goes wrong:** The main event loop runs as a single async function with an infinite loop that calls `tokio::time::sleep(Duration).await` at the end of each iteration. Even with a dedicated OS thread for keyboard input using `std::sync::mpsc`, the main loop only processes input when it reaches the top of the next iteration after sleep completes. Under high CPU load (e.g., from Docker containers), this creates noticeable input lag (100-500ms+).

**Why it happens:**
- Tokio uses **cooperative scheduling** where tasks yield at `.await` points
- A single long-running task monopolizes its worker thread between awaits
- `tokio::time::sleep(16ms).await` means input is only checked every 16ms minimum
- Keyboard events accumulate in the channel but aren't processed until sleep completes
- The OS thread capturing input works fine, but the async task processing it is starved

**Root cause:** Misunderstanding Tokio's cooperative scheduling model. Developers expect that having a separate OS thread for input will guarantee responsiveness, but the async task that *processes* those inputs still needs to yield cooperatively. The 16ms sleep at the end of the loop creates a hard floor on input latency.

**Consequences:**
- Keyboard shortcuts feel sluggish or unresponsive
- Users press keys multiple times thinking they weren't registered
- Under CPU load, lag increases dramatically (sleep timer fires late)
- Poor user experience that makes the TUI feel broken

**Detection (warning signs in CI-TUI codebase):**
```rust
// File: src/ui/mod.rs, lines 362-540
pub async fn run(...) -> Result<()> {
    // ... setup ...

    loop {
        // Process keyboard events from dedicated thread
        while let Ok(key) = keyboard_rx.try_recv() {
            // handle key...
        }

        // Process other channels with try_recv()...

        // Render if needed
        if app.needs_redraw {
            terminal.draw(...)?;
        }

        // PROBLEM: Sleep for 16ms before next iteration
        tokio::time::sleep(FRAME_DURATION).await; // <-- 16ms floor
    }
}
```

**Warning signs:**
- Main event loop is an async function called directly from `#[tokio::main]`
- Loop ends with `tokio::time::sleep(...).await`
- Input handling uses `try_recv()` in a tight loop
- Comments mention "~60fps" or frame duration calculations
- Reports of "keyboard lag" or "unresponsive UI" from users

**Prevention:**

**Option A: Use tokio::select! to interleave operations (RECOMMENDED)**
```rust
loop {
    tokio::select! {
        // Keyboard input (highest priority)
        key = keyboard_rx_async.recv() => {
            if let Some(key) = key {
                handle_key_event(&mut app, key, ...);
            }
        }

        // Runner events (limit per iteration)
        runner_event = event_rx.recv() => {
            if let Some(event) = runner_event {
                app.handle_runner_event(event);
            }
        }

        // Stats updates
        stats = stats_rx.recv() => {
            if let Some(stats) = stats {
                app.update_stats(...);
            }
        }

        // Tick for rendering (16ms)
        _ = tick_interval.tick() => {
            if app.needs_redraw {
                terminal.draw(...)?;
            }
        }
    }
}
```

**Option B: Spawn the event loop as a separate task**
```rust
#[tokio::main]
async fn main() -> Result<()> {
    // ... setup ...

    // Spawn event loop on separate task
    let event_loop_handle = tokio::spawn(async move {
        run_event_loop(app, terminal, channels).await
    });

    // Main task is now free to handle signals, etc.
    event_loop_handle.await??;

    Ok(())
}
```

**Option C: Use a sync main loop with async channels (hybrid approach)**
```rust
fn main() -> Result<()> {
    let runtime = tokio::runtime::Runtime::new()?;

    // Run TUI on main thread (synchronous)
    loop {
        // Poll keyboard with timeout
        if crossterm::event::poll(Duration::from_millis(16))? {
            if let Event::Key(key) = crossterm::event::read()? {
                handle_key_event(&mut app, key, ...)?;
            }
        }

        // Non-blocking check of async channels via runtime
        runtime.block_on(async {
            // Use try_recv() on tokio channels here
        });

        if app.needs_redraw {
            terminal.draw(...)?;
        }
    }
}
```

**CI-TUI specific fix:**
The current architecture uses `std::sync::mpsc` for keyboard input from an OS thread, which is good. The problem is the async event loop's sleep. Solutions:
1. Convert keyboard channel to async (`tokio::sync::mpsc`) and use `select!` with `recv().await`
2. OR move the entire event loop off tokio (synchronous with `crossterm::event::poll`)
3. OR reduce sleep duration to 1ms and batch operations more aggressively

**Phase to address:** Phase 1 (Foundation) - This is an architectural issue that affects all subsequent features.

---

### Pitfall 2: Mixing Sync and Async Channels Incorrectly

**What goes wrong:** Using `std::sync::mpsc` channels with blocking `.recv()` or tight `try_recv()` loops inside async tasks. This either blocks the entire worker thread (with `.recv()`) or wastes CPU cycles (with `try_recv()` in a loop).

**Why it happens:**
- Developers use familiar stdlib channels instead of `tokio::sync::mpsc`
- Attempting to bridge sync input threads with async event processing
- Misunderstanding that `try_recv()` doesn't yield to the scheduler

**Root cause:** `std::sync::mpsc` is designed for synchronous blocking code. In async contexts:
- `.recv()` blocks the thread (prevents other tasks from running)
- `.try_recv()` in a loop doesn't yield (starves other tasks)
- Neither integrates with Tokio's async scheduler

**Consequences:**
- Blocking `.recv()` hangs the entire async runtime
- `try_recv()` loops waste CPU and prevent proper cooperative scheduling
- Other async tasks starve
- Poor integration between sync input and async processing

**Detection:**
```rust
// ANTI-PATTERN 1: Blocking recv in async
async fn process_events(rx: std::sync::mpsc::Receiver<Event>) {
    loop {
        let event = rx.recv().unwrap(); // <-- Blocks entire thread!
        handle_event(event).await;
    }
}

// ANTI-PATTERN 2: Tight try_recv loop
async fn process_events(rx: std::sync::mpsc::Receiver<Event>) {
    loop {
        match rx.try_recv() {
            Ok(event) => handle_event(event).await,
            Err(_) => {
                // No yield here! This starves other tasks.
                continue;
            }
        }
    }
}
```

**Warning signs:**
- Using `std::sync::mpsc` channels read from within async functions
- Tight loops with `try_recv()` and no explicit yields
- Comments about "non-blocking" using `try_recv()` (it's non-blocking for the OS thread but doesn't yield to Tokio)
- CPU usage high even when idle

**Prevention:**

**Use proper async channels:**
```rust
// Option 1: Pure async with tokio channels
let (tx, mut rx) = tokio::sync::mpsc::channel(100);

async fn process_events(mut rx: tokio::sync::mpsc::Receiver<Event>) {
    while let Some(event) = rx.recv().await { // <-- Yields properly
        handle_event(event).await;
    }
}

// Option 2: Bridge sync thread to async
// Send from sync thread using tokio channel with blocking_send
fn sync_input_thread(tx: tokio::sync::mpsc::Sender<Event>) {
    loop {
        let event = read_input_blocking(); // Sync I/O
        tx.blocking_send(event).unwrap(); // Safe in sync context
    }
}

// Receive in async context
async fn async_event_loop(mut rx: tokio::sync::mpsc::Receiver<Event>) {
    while let Some(event) = rx.recv().await { // Proper async
        handle_event(event).await;
    }
}
```

**CI-TUI specific guidance:**
The current code uses `std::sync::mpsc` with `try_recv()` (lines 433-452 in ui/mod.rs). This is acceptable because the loop ends with `tokio::time::sleep`, which provides a yield point. However, for better responsiveness, convert to `tokio::sync::mpsc::unbounded_channel()` and use `recv().await` in a `select!` block.

**Phase to address:** Phase 1 (Foundation) - Required for responsive event handling architecture.

---

### Pitfall 3: Long-Running Operations Without Yielding

**What goes wrong:** CPU-intensive operations (large iterations, heavy parsing, complex computations) run in async tasks without `.await` points. This blocks the worker thread and makes the UI completely unresponsive for the duration of the operation.

**Why it happens:**
- Developers don't realize they need explicit yield points
- Porting synchronous code to async without adding awaits
- Processing large collections in tight loops

**Root cause:** Tokio's cooperative scheduling requires tasks to voluntarily yield at `.await` points. Code that runs for >10-100µs without an await prevents other tasks from executing.

**Consequences:**
- UI freezes completely during expensive operations
- Keyboard input doesn't register
- Rendering stops
- Application appears hung

**Detection:**
```rust
// ANTI-PATTERN: No yield points in expensive operation
async fn process_large_dataset(data: Vec<Item>) -> Result<()> {
    let mut results = Vec::new();

    // This loop might run for seconds without yielding
    for item in data {
        let processed = expensive_computation(&item); // No .await!
        results.push(processed);
    }

    save_results(results).await
}

// ANTI-PATTERN: Regex compilation in async without yield
async fn determine_checks(config: &Config) -> Vec<Check> {
    let mut checks = Vec::new();

    for pattern in &config.patterns {
        // Regex compilation can take milliseconds
        let regex = Regex::new(pattern).unwrap(); // No .await!
        checks.push(regex);
    }

    checks
}
```

**Warning signs:**
- Loops processing hundreds/thousands of items without `.await`
- Heavy computations (regex compilation, parsing, hashing) in async functions
- `for` loops in async code with no await inside the loop body
- User reports of "freezing" during specific operations

**Prevention:**

**Solution 1: Add explicit yield points**
```rust
use tokio::task;

async fn process_large_dataset(data: Vec<Item>) -> Result<()> {
    let mut results = Vec::new();

    for (i, item) in data.iter().enumerate() {
        let processed = expensive_computation(item);
        results.push(processed);

        // Yield every N iterations (e.g., every 100)
        if i % 100 == 0 {
            tokio::task::yield_now().await;
        }
    }

    save_results(results).await
}
```

**Solution 2: Move to spawn_blocking for CPU-bound work**
```rust
async fn process_large_dataset(data: Vec<Item>) -> Result<Vec<Processed>> {
    // Move expensive computation to blocking thread pool
    let results = tokio::task::spawn_blocking(move || {
        data.iter()
            .map(|item| expensive_computation(item))
            .collect()
    })
    .await?;

    Ok(results)
}
```

**Solution 3: Use rayon for parallel CPU work**
```rust
use rayon::prelude::*;

async fn process_large_dataset(data: Vec<Item>) -> Result<Vec<Processed>> {
    let results = tokio::task::spawn_blocking(move || {
        // Rayon parallelizes across CPU cores
        data.par_iter()
            .map(|item| expensive_computation(item))
            .collect::<Vec<_>>()
    })
    .await?;

    Ok(results)
}
```

**Rule of thumb:** No more than 10-100 microseconds between `.await` points in async code.

**CI-TUI specific areas to watch:**
- `determine_checks()` in checks.rs - processes file patterns and regex
- `find_related_tests()` in test_discovery.rs - can run grep searches
- Any config loading or regex compilation

**Phase to address:** Phase 2 (Core Features) - As features grow, this becomes more critical.

---

### Pitfall 4: Blocking File I/O in Async Tasks

**What goes wrong:** Using synchronous file I/O (`std::fs::read`, `std::fs::write`, `std::io::Read`) inside async tasks blocks the worker thread and prevents other tasks from running.

**Why it happens:**
- `std::fs` is in scope by default and easy to use
- Developers don't realize file I/O can block for milliseconds
- Configuration loading, log file writes happen at startup and seem "fast enough"

**Root cause:** Standard library file I/O is synchronous and blocking. Even "fast" operations like reading a small config file can block for 1-10ms, which starves other tasks.

**Consequences:**
- UI freezes during file operations (config load, log writes)
- Keyboard input doesn't work while reading files
- Under heavy I/O load (e.g., NFS, slow disk), complete hangs

**Detection:**
```rust
// ANTI-PATTERN: Blocking file read in async
async fn load_config(path: &Path) -> Result<Config> {
    let contents = std::fs::read_to_string(path)?; // <-- Blocks!
    let config = serde_yaml::from_str(&contents)?; // Also blocking
    Ok(config)
}

// ANTI-PATTERN: Blocking file write in async
async fn save_results(results: &[CheckResult]) -> Result<()> {
    let json = serde_json::to_string(results)?; // Blocking
    std::fs::write("results.json", json)?; // Blocks!
    Ok(())
}
```

**Warning signs:**
- Using `std::fs::*` functions in async code
- Config loading in async functions
- Log file writes in async context
- Any `File::open()`, `read_to_string()`, `write()` without `tokio::fs`

**Prevention:**

**Solution 1: Use tokio::fs for async file I/O**
```rust
use tokio::fs;

async fn load_config(path: &Path) -> Result<Config> {
    let contents = fs::read_to_string(path).await?; // Async!
    // Deserialization is CPU-bound, not I/O
    let config = tokio::task::spawn_blocking(move || {
        serde_yaml::from_str(&contents)
    })
    .await??;
    Ok(config)
}

async fn save_results(results: &[CheckResult]) -> Result<()> {
    let json = tokio::task::spawn_blocking(move || {
        serde_json::to_string(results)
    })
    .await??;
    fs::write("results.json", json).await?;
    Ok(())
}
```

**Solution 2: Use spawn_blocking for sync I/O**
```rust
async fn load_config(path: PathBuf) -> Result<Config> {
    tokio::task::spawn_blocking(move || {
        let contents = std::fs::read_to_string(path)?;
        let config = serde_yaml::from_str(&contents)?;
        Ok(config)
    })
    .await?
}
```

**When to use which:**
- `tokio::fs` - For pure file I/O operations (read, write, append)
- `spawn_blocking` - For file I/O + CPU-bound work (parsing, compression)
- `std::fs` - Only in non-async context or before runtime starts

**CI-TUI specific areas:**
- `config::load_config()` - Currently uses `std::fs`, but called before TUI starts (OK)
- If adding features like "save state" or "export results", use `tokio::fs`
- Git operations via subprocess are already handled properly (tokio::process)

**Phase to address:** Phase 2 (Core Features) - When adding persistence or export features.

---

## Moderate Pitfalls

These cause degraded performance or technical debt but don't completely break responsiveness.

### Pitfall 5: Channel Backpressure Causes Dropped Events

**What goes wrong:** Bounded channels fill up when the consumer can't keep pace with the producer, leading to either blocking sends (freezes producer) or dropped messages (data loss).

**Why it happens:**
- Using bounded channels with insufficient capacity
- Producer sends faster than consumer processes
- No backpressure handling strategy

**Root cause:** Asymmetric processing rates. In CI-TUI's case, Docker containers can produce output faster than the TUI can render it.

**Consequences:**
- Output lines dropped (user doesn't see important errors)
- Producer tasks block waiting for channel space (slows check execution)
- Memory issues if using unbounded channels carelessly

**Detection:**
```rust
// PROBLEM: Bounded channel that can fill up
let (tx, rx) = mpsc::channel(100); // Only 100 slots

// High-speed producer
async fn stream_output(tx: mpsc::Sender<String>) {
    for line in huge_output {
        tx.send(line).await?; // Blocks when channel full!
    }
}
```

**Warning signs:**
- Using bounded `mpsc::channel(N)` for high-volume streams
- Comments about "lost output" or "missing log lines"
- Channel capacity constants that seem arbitrary (10, 100, etc.)
- No strategy for handling full channels

**Prevention:**

**Strategy 1: Use unbounded for event streams (when appropriate)**
```rust
// For keyboard input - never drop user input
let (tx, rx) = mpsc::unbounded_channel();

// For high-volume output - with rate limiting
let (tx, rx) = mpsc::unbounded_channel();
// But implement consumer-side rate limiting to prevent memory growth
```

**Strategy 2: Use try_send with overflow handling**
```rust
async fn stream_output(tx: mpsc::Sender<String>, line: String) {
    match tx.try_send(line) {
        Ok(()) => { /* Sent successfully */ }
        Err(TrySendError::Full(line)) => {
            // Channel full - handle overflow
            // Option A: Drop with counter
            dropped_lines.fetch_add(1, Ordering::Relaxed);
            // Option B: Buffer to disk
            // Option C: Wait a bit and retry
            tokio::time::sleep(Duration::from_millis(1)).await;
            let _ = tx.send(line).await;
        }
        Err(TrySendError::Closed(_)) => {
            // Receiver dropped, exit
        }
    }
}
```

**Strategy 3: Batch processing on consumer side**
```rust
async fn consume_events(mut rx: mpsc::Receiver<Event>) {
    let mut batch = Vec::new();

    loop {
        // Collect up to N events or timeout
        match tokio::time::timeout(
            Duration::from_millis(16),
            rx.recv()
        ).await {
            Ok(Some(event)) => {
                batch.push(event);

                // Drain additional pending events (up to limit)
                while let Ok(event) = rx.try_recv() {
                    batch.push(event);
                    if batch.len() >= 50 {
                        break;
                    }
                }

                process_batch(&batch);
                batch.clear();
            }
            _ => { /* Timeout or closed */ }
        }
    }
}
```

**CI-TUI current approach:**
- Keyboard: `std::sync::mpsc::channel()` (unbounded) - GOOD
- Runner events: `mpsc::channel(100)` - Bounded, but with `MAX_RUNNER_EVENTS_PER_FRAME` limiter (50/frame) - GOOD
- Stats: `mpsc::channel(4)` with `try_send` that drops on full - GOOD

**Phase to address:** Phase 2 (Core Features) - Monitor for dropped events during high-volume operations.

---

### Pitfall 6: Inefficient Rendering Triggers

**What goes wrong:** Rendering on every event instead of batching, or not rendering when state changes, leading to either wasted CPU (over-rendering) or stale UI (under-rendering).

**Why it happens:**
- Immediate rendering after every state change
- No dirty flag tracking
- Rendering on timer instead of on-demand

**Root cause:** Misunderstanding that terminal rendering is relatively expensive (1-5ms for full screen) and should be batched.

**Consequences:**
- High CPU usage from excessive rendering
- Poor battery life on laptops
- Heat generation
- OR stale UI if rendering is too infrequent

**Detection:**
```rust
// ANTI-PATTERN 1: Render on every event
async fn handle_event(event: Event, app: &mut App, terminal: &mut Terminal) {
    app.update(event);
    terminal.draw(|f| render(app, f))?; // Every single event!
}

// ANTI-PATTERN 2: Timer-only rendering (can be stale)
loop {
    handle_all_events(&mut app);
    tokio::time::sleep(Duration::from_millis(16)).await;
    terminal.draw(|f| render(app, f))?; // Even if nothing changed
}
```

**Warning signs:**
- No dirty flag pattern (`needs_redraw`)
- Rendering inside event handlers
- Rendering at fixed interval regardless of state changes
- High CPU usage in profiling even when "idle"

**Prevention:**

**Best practice: Dirty flag with deferred rendering**
```rust
struct App {
    needs_redraw: bool,
    // ... other state
}

impl App {
    fn handle_event(&mut self, event: Event) {
        match event {
            Event::CheckFinished { .. } => {
                self.update_results();
                self.needs_redraw = true; // Mark dirty
            }
            Event::KeyPress(key) => {
                self.handle_key(key);
                self.needs_redraw = true;
            }
            // Events that don't change visible state don't set flag
        }
    }
}

// Event loop
loop {
    // Process events (sets needs_redraw if needed)
    while let Some(event) = rx.recv().await {
        app.handle_event(event);
    }

    // Render only if state changed
    if app.needs_redraw {
        terminal.draw(|f| render(&app, f))?;
        app.needs_redraw = false;
    }
}
```

**CI-TUI current approach:**
Good! Uses `needs_redraw` flag (line 437, 512-515 in ui/mod.rs). Events set the flag, rendering only happens when needed.

**Phase to address:** Already addressed in CI-TUI. Maintain this pattern for new features.

---

### Pitfall 7: Platform-Specific Event Handling Inconsistencies

**What goes wrong:** Code behaves differently on Windows vs Linux/macOS due to platform differences in how Crossterm sends events.

**Why it happens:**
- Windows sends both Press and Release events for keys
- Linux/macOS only send Press events
- Developers test on one platform only

**Root cause:** Platform differences in terminal event APIs.

**Consequences:**
- Double key presses on Windows
- Features work on one platform but not another
- Inconsistent user experience

**Detection:**
```rust
// ANTI-PATTERN: No filtering of event kind
if let Event::Key(key) = event::read()? {
    handle_key(key); // Processes both Press and Release on Windows!
}
```

**Warning signs:**
- Not checking `key.kind == KeyEventKind::Press`
- User reports of "double input" on Windows
- Code that works on developer's Mac but fails on Windows CI

**Prevention:**
```rust
use crossterm::event::{Event, KeyEvent, KeyEventKind};

if let Event::Key(key) = event::read()? {
    // ALWAYS filter for Press events only
    if key.kind == KeyEventKind::Press {
        handle_key(key);
    }
}
```

**CI-TUI current approach:**
Good! Filters for `KeyEventKind::Press` at line 72 in ui/mod.rs.

**Phase to address:** Already addressed. Document this requirement for contributors.

---

## Minor Pitfalls

These cause annoyance but are easily fixable.

### Pitfall 8: Poor Error Context in Async Chains

**What goes wrong:** Errors in async chains lose context, making debugging difficult.

**Why it happens:**
- Using `?` operator without adding context
- Async stack traces are hard to read
- Multiple layers of Result unwrapping

**Root cause:** Rust's error propagation with `?` doesn't automatically capture context.

**Prevention:**
```rust
use anyhow::{Context, Result};

async fn load_config(path: &Path) -> Result<Config> {
    let contents = tokio::fs::read_to_string(path)
        .await
        .context(format!("Failed to read config from {}", path.display()))?;

    let config: Config = serde_yaml::from_str(&contents)
        .context("Failed to parse YAML config")?;

    Ok(config)
}
```

**Phase to address:** Ongoing - Add context to errors as features are developed.

---

### Pitfall 9: Terminal State Corruption on Panic

**What goes wrong:** Application panics without restoring terminal to normal mode, leaving user's shell broken.

**Why it happens:**
- Panic occurs before cleanup code runs
- No panic hook installed

**Root cause:** Raw mode and alternate screen aren't automatically restored on panic.

**Prevention:**
```rust
fn install_panic_hook() {
    let original_hook = panic::take_hook();
    panic::set_hook(Box::new(move |panic_info| {
        restore_terminal(); // Custom cleanup
        original_hook(panic_info);
    }));
}

fn restore_terminal() {
    let _ = disable_raw_mode();
    let _ = execute!(stdout(), LeaveAlternateScreen, DisableMouseCapture);
}
```

**CI-TUI current approach:**
Good! Panic hook installed at line 369 in ui/mod.rs.

**Phase to address:** Already addressed.

---

### Pitfall 10: Clipboard Integration Brittleness

**What goes wrong:** OSC 52 clipboard integration doesn't work in all terminals, silently fails, or has security restrictions.

**Why it happens:**
- Not all terminals support OSC 52
- Some terminals require user configuration to enable it
- Tmux/screen may strip escape sequences

**Root cause:** Terminal ecosystem fragmentation.

**Prevention:**
- Document which terminals support OSC 52 (iTerm2, kitty, alacritty, Windows Terminal)
- Provide feedback when copy operation completes (status message)
- Consider fallback mechanisms (write to temp file, system clipboard via external tool)

**CI-TUI current approach:**
Uses OSC 52 with status message feedback (line 159-165, 287-291 in ui/mod.rs). Good approach.

**Phase to address:** Document in user guide. Consider fallback in Phase 3 (Polish).

---

## Phase-Specific Warnings

| Phase Topic | Likely Pitfall | Mitigation |
|-------------|---------------|------------|
| Foundation (Event Loop) | Single async task monopolizes loop (Pitfall 1) | Use tokio::select! or spawn event loop task |
| Foundation (Channels) | Mixing sync/async channels (Pitfall 2) | Convert to tokio::sync::mpsc with recv().await |
| Core Features (Check Execution) | Long-running operations without yielding (Pitfall 3) | Add tokio::task::yield_now() or spawn_blocking |
| Core Features (File I/O) | Blocking file I/O in async (Pitfall 4) | Use tokio::fs or spawn_blocking |
| Performance (High Volume Output) | Channel backpressure (Pitfall 5) | Implement batching and overflow handling |
| Performance (Rendering) | Inefficient rendering (Pitfall 6) | Maintain dirty flag pattern (already OK) |
| Testing (Cross-platform) | Platform inconsistencies (Pitfall 7) | Filter KeyEventKind::Press (already OK) |
| Polish (UX) | Terminal corruption on panic (Pitfall 9) | Panic hook (already OK) |

---

## Architectural Recommendations for CI-TUI

Based on the codebase analysis and pitfall research:

### Immediate (Phase 1):

1. **Replace the event loop sleep pattern** (Addresses Pitfall 1)
   - Convert from: loop with `try_recv()` + `tokio::time::sleep().await`
   - Convert to: `tokio::select!` with async channels and tick interval
   - Expected improvement: Input lag reduces from ~16ms baseline to <1ms

2. **Upgrade channel architecture** (Addresses Pitfall 2)
   - Keyboard: Convert `std::sync::mpsc` to `tokio::sync::mpsc::unbounded_channel()`
   - Use `recv().await` in select! block instead of `try_recv()` in loop
   - Maintains responsive keyboard input under all load conditions

### Later (Phase 2):

3. **Audit expensive operations** (Addresses Pitfall 3)
   - Review `determine_checks()`, `find_related_tests()`, regex compilation
   - Add explicit yield points or move to `spawn_blocking`
   - Profile to identify any operation taking >100µs between awaits

4. **Review file I/O patterns** (Addresses Pitfall 4)
   - Current config loading is OK (happens before TUI starts)
   - If adding features like state persistence, use `tokio::fs`

### Maintain (Ongoing):

5. **Keep current good patterns**
   - Dirty flag rendering (Pitfall 6) ✓
   - KeyEventKind::Press filtering (Pitfall 7) ✓
   - Panic hook for terminal restoration (Pitfall 9) ✓
   - Separate OS thread for keyboard input ✓

---

## Testing for Responsiveness

### Manual Testing Protocol:

1. **Baseline responsiveness test:**
   - Run CI checks that produce high output volume
   - Rapidly press keyboard shortcuts (j/k navigation, 'q' to quit)
   - Expected: <50ms latency from keypress to UI response

2. **Stress test:**
   - Run checks on large codebase (100+ files)
   - Monitor CPU usage (should be <10% when idle)
   - Test under high system load (parallel compilation)
   - Keyboard should remain responsive (<100ms)

3. **Platform testing:**
   - Test on Windows, Linux, macOS
   - Verify key events only fire once (not double on Windows)
   - Check clipboard integration in different terminals

### Automated Testing:

```rust
#[tokio::test]
async fn test_event_loop_responsiveness() {
    // Setup channels
    let (input_tx, input_rx) = tokio::sync::mpsc::unbounded_channel();
    let (output_tx, mut output_rx) = tokio::sync::mpsc::channel(10);

    // Spawn event loop
    let handle = tokio::spawn(async move {
        event_loop(input_rx, output_tx).await
    });

    // Send 100 events rapidly
    for i in 0..100 {
        input_tx.send(Event::Test(i)).unwrap();
    }

    // Verify all processed within reasonable time
    let start = Instant::now();
    for i in 0..100 {
        let result = tokio::time::timeout(
            Duration::from_millis(10), // 10ms per event max
            output_rx.recv()
        ).await;
        assert!(result.is_ok(), "Event {} timed out", i);
    }
    let elapsed = start.elapsed();
    assert!(elapsed < Duration::from_millis(500), "Took {:?}", elapsed);
}
```

---

## Sources

Research for this document drew from the following authoritative sources:

### Official Documentation:
- [Ratatui FAQ](https://ratatui.rs/faq/)
- [Ratatui Async Event Stream Tutorial](https://ratatui.rs/tutorials/counter-async-app/async-event-stream/)
- [Ratatui Event Handling Concepts](https://ratatui.rs/concepts/event-handling/)
- [Tokio Runtime Documentation](https://docs.rs/tokio/latest/tokio/runtime/index.html)
- [Tokio Bridging with Sync Code](https://tokio.rs/tokio/topics/bridging)
- [Tokio Channels Tutorial](https://tokio.rs/tokio/tutorial/channels)
- [Tokio spawn_blocking Documentation](https://docs.rs/tokio/latest/tokio/task/fn.spawn_blocking.html)
- [Tokio Select Tutorial](https://tokio.rs/tokio/tutorial/select)

### Technical Articles:
- [Alice Ryhl: Async: What is blocking?](https://ryhl.io/blog/async-what-is-blocking/)
- [The Dark Side of Tokio: How Async Rust Can Starve Your Runtime](https://medium.com/@ThreadSafeDiaries/the-dark-side-of-tokio-how-async-rust-can-starve-your-runtime-a33a04f6a258)
- [How Tokio Schedule Tasks: A Hard Lesson Learnt](https://rustmagazine.org/issue-4/how-tokio-schedule-tasks/)
- [Tokio: Reducing tail latencies with automatic cooperative task yielding](https://tokio.rs/blog/2020-04-preemption)

### Community Resources:
- [GitHub: async-ratatui - Handling multiple events asynchronously](https://github.com/d-holguin/async-ratatui)
- [Ratatui Best Practices Discussion](https://github.com/ratatui/ratatui/discussions/220)
- [Rust Forum: Text-mode terminal application with asynchronous I/O](https://users.rust-lang.org/t/text-mode-terminal-application-with-asynchronous-input-output/74760)

### Technical Discussions:
- [GitHub: tokio-rs/tokio Discussion #6987 - How does current_thread runtime work?](https://github.com/tokio-rs/tokio/discussions/6987)
- [GitHub: tokio-rs/tokio Issue #4730 - One bad task can halt all executor progress](https://github.com/tokio-rs/tokio/issues/4730)
- [GitHub: tokio-rs/tokio Issue #1246 - High latency between mpsc send and receive](https://github.com/tokio-rs/tokio/issues/1246)

### Additional Resources:
- [Guillaume VanderEst: Using Tokio in Rust and Sleeping Threads](https://guillaume.vanderest.org/posts/rust-tokio-sleep/)
- [Mindful Chase: Resolving Advanced Async Issues in Rust with Tokio](https://www.mindfulchase.com/explore/troubleshooting-tips/resolving-advanced-async-issues-in-rust-with-tokio-and-async-await.html)

---

## Confidence Assessment

| Area | Confidence | Rationale |
|------|-----------|-----------|
| Event Loop Architecture | HIGH | Verified with official Tokio docs, Ratatui tutorials, and codebase analysis |
| Channel Patterns | HIGH | Tokio official bridging guide, multiple corroborating sources |
| Tokio Scheduling Model | HIGH | Alice Ryhl's authoritative article, official docs, recent 2025 technical articles |
| Platform Differences | HIGH | Ratatui FAQ explicitly documents Windows vs macOS/Linux behavior |
| Solutions & Fixes | HIGH | Code examples from official tutorials, verified patterns in production apps |

All pitfalls are specific to async Rust TUI applications using the tokio + crossterm + ratatui stack. Detection patterns are drawn from CI-TUI's actual codebase structure. Solutions are tested patterns from official documentation and production applications.
