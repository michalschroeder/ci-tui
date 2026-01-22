# Architecture Patterns for Ratatui Applications

**Domain:** Terminal User Interface (TUI) for CI/CD tooling
**Researched:** 2026-01-22
**Confidence:** HIGH (based on official Ratatui documentation and community best practices)

## Overview

Ratatui is an immediate-mode rendering library for terminal UIs. Unlike retained-mode UIs, every frame must be fully rendered from scratch. This fundamental characteristic shapes all architectural decisions. Well-structured ratatui applications follow established patterns from GUI development (Elm Architecture, MVC, Component-based) adapted for terminal constraints.

## Recommended Architecture: Modified Elm Architecture with Async Extensions

For CI/CD TUI applications like CI-TUI, the **Elm Architecture (TEA) with async extensions** provides the best balance of:
- Predictable state flow
- Testability through pure functions
- Non-blocking async operations
- Maintainability as complexity grows

### Why TEA for CI/CD TUIs

**CI/CD tooling has specific requirements:**
- Long-running background tasks (check execution, Docker operations)
- Real-time output streaming
- User interaction during async operations
- Complex state transitions (pending → running → passed/failed)

TEA's unidirectional data flow and message-driven updates naturally model these requirements as a finite state machine, while async extensions prevent blocking.

---

## Core Architectural Patterns

### Pattern 1: The Elm Architecture (TEA)

**Structure:**
```
┌─────────────────────────────────────────────┐
│              User Input / Events            │
└─────────────────┬───────────────────────────┘
                  ↓
         ┌────────────────┐
         │    Message     │ (enum of all possible actions)
         └────────┬───────┘
                  ↓
         ┌────────────────┐
         │  Update Fn     │ (Model + Message → New Model)
         └────────┬───────┘
                  ↓
         ┌────────────────┐
         │     Model      │ (application state)
         └────────┬───────┘
                  ↓
         ┌────────────────┐
         │   View Fn      │ (Model → UI)
         └────────┬───────┘
                  ↓
         ┌────────────────┐
         │    Render      │
         └────────────────┘
```

**Components:**

1. **Model** - Single source of truth for application state
   ```rust
   struct App {
       checks: Vec<CheckToRun>,
       results: HashMap<String, CheckResult>,
       selected_check: usize,
       status_filter: StatusFilter,
       // ... all state lives here
   }
   ```

2. **Message** - Enum representing all state transitions
   ```rust
   enum Message {
       KeyPress(KeyEvent),
       CheckStarted { check_id: String },
       CheckOutput { check_id: String, line: String },
       CheckFinished { result: CheckResult },
       AllFinished,
   }
   ```

3. **Update** - Pure function transforming state
   ```rust
   fn update(app: &mut App, message: Message) -> Option<Command> {
       match message {
           Message::CheckFinished { result } => {
               app.results.insert(result.check_id.clone(), result);
               app.needs_redraw = true;
               None
           }
           // ... handle all message types
       }
   }
   ```

4. **View** - Pure rendering function
   ```rust
   fn view(app: &App, frame: &mut Frame) {
       // Render based solely on app state
       // No side effects, no mutation
   }
   ```

**Benefits:**
- **Predictability**: Same state always renders identically
- **Testability**: Update function is pure, easily tested
- **Debugging**: All state changes happen through messages
- **Finite State Machine**: Natural representation of check lifecycle

**Trade-offs:**
- Requires discipline to avoid direct state mutation
- More boilerplate than ad-hoc state management
- Learning curve for developers unfamiliar with functional patterns

**When to use:**
- Applications with complex state transitions
- Need for time-travel debugging
- Multiple concurrent operations
- CI/CD tools, monitoring dashboards, process managers

---

### Pattern 2: Component Architecture

**Structure:**
```rust
trait Component {
    fn init(&mut self) -> Result<()>;
    fn handle_events(&mut self, event: Event) -> Result<Option<Action>>;
    fn update(&mut self, action: Action) -> Result<Option<Action>>;
    fn render(&mut self, f: &mut Frame, area: Rect);
}
```

**Characteristics:**
- Each component owns its state
- Localized event handling
- Object-oriented organization
- Composable widget hierarchy

**Benefits:**
- Lower ceremony than TEA
- Natural for reusable widgets
- Familiar OOP patterns
- Co-location of concerns

**Trade-offs:**
- State scattered across components
- Harder to debug interactions
- Less predictable data flow

**When to use:**
- Applications with independent UI sections
- Need for reusable components
- Team prefers OOP style
- Simpler applications without complex state

---

### Pattern 3: Model-View-Controller (MVC)

**Structure:**
- **Model**: Data structures (checks, results, config)
- **View**: Rendering logic (dashboard.rs)
- **Controller**: Event handling and orchestration (mod.rs)

**CI-TUI's Current Approach:**
CI-TUI uses a hybrid of MVC and TEA:
- `app.rs` = Model (state container)
- `dashboard.rs` = View (pure rendering)
- `mod.rs` = Controller (event loop + state updates)
- `runner.rs` = Async execution layer

This works but has **responsiveness issues** due to:
1. Tight coupling between controller and async tasks
2. Event handling mixed with state management
3. No clear message queue abstraction

---

## Async Integration Patterns

### The Critical Challenge

Ratatui's immediate-mode rendering requires the main loop to call `terminal.draw()` regularly (ideally 60fps). Long-running operations that block the main thread freeze the UI.

**Anti-pattern (blocking):**
```rust
loop {
    let event = events.recv()?; // BLOCKS until event arrives
    handle_event(event);
    terminal.draw()?;
}
```

**Problem:** If no events arrive, UI doesn't redraw. Animations stop, timers freeze, output doesn't stream.

### Solution 1: tokio::select! with Non-Blocking Channels

**Pattern:**
```rust
loop {
    tokio::select! {
        Some(key_event) = keyboard_rx.recv() => {
            handle_key(key_event);
        }
        Some(runner_event) = runner_rx.recv() => {
            handle_runner_event(runner_event);
        }
        Some(stats) = stats_rx.recv() => {
            update_stats(stats);
        }
        _ = tick_interval.tick() => {
            // Force periodic redraws
        }
    }

    if needs_redraw {
        terminal.draw(|f| render(app, f))?;
    }
}
```

**Benefits:**
- UI remains responsive
- Multiple async sources
- Periodic redraws guaranteed

**CI-TUI's Approach:**
CI-TUI uses a **variant** of this pattern:
```rust
loop {
    // Phase 1: Drain keyboard events (std::sync::mpsc, non-blocking)
    while let Ok(key) = keyboard_rx.try_recv() {
        handle_key(key);
    }

    // Phase 2: Process runner events (tokio::mpsc, limited per frame)
    for _ in 0..MAX_EVENTS_PER_FRAME {
        match runner_rx.try_recv() {
            Ok(event) => handle_event(event),
            Err(_) => break,
        }
    }

    // Phase 3: Render if needed
    if app.needs_redraw {
        terminal.draw()?;
    }

    // Phase 4: Sleep to yield CPU
    tokio::time::sleep(FRAME_DURATION).await;
}
```

**Why this works:**
- `try_recv()` never blocks
- `MAX_EVENTS_PER_FRAME` prevents UI starvation from event floods
- Fixed sleep ensures consistent frame timing
- Dedicated OS thread for keyboard guarantees input responsiveness

**Trade-off:** More complex than `tokio::select!` but necessary for guaranteeing keyboard responsiveness under extreme CPU load from Docker containers.

### Solution 2: Message Passing via Channels

**Pattern for spawning async work:**
```rust
// In event handler
(KeyCode::Char('r'), _) => {
    let tx = self.result_tx.clone();
    let check = self.selected_check.clone();

    tokio::spawn(async move {
        let result = run_check(&check).await;
        let _ = tx.send(Message::CheckFinished { result }).await;
    });
}

// In main loop
loop {
    tokio::select! {
        Some(msg) = message_rx.recv() => {
            update(&mut app, msg);
        }
        // ... other branches
    }
}
```

**Benefits:**
- Tasks don't block UI
- Clean separation of concerns
- Works with TEA message pattern

**CI-TUI Implementation:**
```rust
// Key handler spawns task
tokio::spawn(async move {
    let result = run_single_check(&check, ...).await;
    let _ = retry_tx.send(result).await;
});

// Main loop consumes results
if let Ok(result) = retry_rx.try_recv() {
    app.results.insert(result.check_id.clone(), result);
}
```

### Solution 3: Background Workers

**Pattern for continuous tasks:**
```rust
fn spawn_stats_worker(tx: mpsc::Sender<SystemStats>) -> JoinHandle<()> {
    tokio::spawn(async move {
        let mut interval = tokio::time::interval(Duration::from_millis(500));
        loop {
            interval.tick().await;
            let stats = collect_stats();
            let _ = tx.try_send(stats); // Non-blocking
        }
    })
}
```

**CI-TUI Usage:**
- System stats (CPU/memory) collected in background
- Main loop consumes with `try_recv()` (non-blocking)
- No UI impact from slow syscalls

---

## State Management Patterns

### Single State Tree (Recommended)

**Pattern:**
```rust
pub struct App {
    // Configuration (immutable)
    config: CiConfig,

    // Core data
    checks: Vec<CheckToRun>,
    results: HashMap<String, CheckResult>,

    // UI state
    selected_check: usize,
    output_scroll: usize,
    status_filter: StatusFilter,

    // Async operation state
    fix_running: bool,
    fix_result: Option<CheckResult>,

    // System monitoring
    cpu_history: VecDeque<f32>,
    mem_history: VecDeque<f32>,

    // Render optimization
    needs_redraw: bool,
}
```

**Benefits:**
- Single source of truth
- Easy to serialize/deserialize
- Straightforward undo/redo
- Predictable state transitions

**Anti-pattern (scattered state):**
```rust
// State split across multiple locations - hard to reason about
let mut checks = Vec::new();
let results = Arc::new(Mutex::new(HashMap::new()));
thread_local! { static SCROLL: Cell<usize> = Cell::new(0); }
```

### Dirty Flag Pattern

**Purpose:** Avoid unnecessary redraws in immediate-mode rendering.

**Implementation:**
```rust
pub struct App {
    needs_redraw: bool,
    // ... other fields
}

impl App {
    fn handle_event(&mut self, event: Event) {
        self.needs_redraw = true; // Mark dirty
        // ... update state
    }
}

// Main loop
if app.needs_redraw {
    terminal.draw(|f| render(&app, f))?;
    app.needs_redraw = false; // Clear flag
}
```

**Benefits:**
- Reduces CPU usage
- Improves battery life
- Still responsive (redraws on state change)

### State Validation

**Pattern:**
```rust
impl App {
    fn next_check(&mut self) {
        let max = self.filtered_checks().len().saturating_sub(1);
        self.selected_check = self.selected_check.saturating_add(1).min(max);
        self.needs_redraw = true;
    }

    fn previous_check(&mut self) {
        self.selected_check = self.selected_check.saturating_sub(1);
        self.needs_redraw = true;
    }
}
```

**Benefits:**
- Bounds checking in methods
- Prevents invalid states
- Centralized validation logic

---

## Component Boundaries

### Recommended Module Structure

```
src/
├── main.rs           # Entry point, CLI arg parsing
├── config.rs         # Configuration types and parsing
├── ui/
│   ├── mod.rs        # Main event loop (controller)
│   ├── app.rs        # State container (model)
│   ├── dashboard.rs  # Rendering (view)
│   └── widgets/      # Reusable UI components
│       ├── check_list.rs
│       ├── output_panel.rs
│       └── stats_panel.rs
├── domain/
│   ├── checks.rs     # Check determination logic
│   ├── git.rs        # Git operations
│   └── runner.rs     # Check execution
└── utils/
    └── terminal.rs   # Terminal setup/cleanup
```

### Responsibility Boundaries

| Module | Responsibility | Owns | Does NOT |
|--------|---------------|------|----------|
| `ui/mod.rs` | Event loop orchestration | Terminal, event sources | Business logic, domain operations |
| `ui/app.rs` | State container | All UI state, results | Rendering, event handling details |
| `ui/dashboard.rs` | Rendering | Widget layout | State mutation, I/O |
| `runner.rs` | Async execution | Check execution, Docker | UI concerns, state management |
| `checks.rs` | Check determination | Pattern matching logic | Execution, UI |
| `config.rs` | Configuration | YAML parsing, validation | Runtime state |

### Data Flow

```
User Input → Event Loop → State Update → Render
                ↑                            ↓
                └────── Background Tasks ────┘
```

**Key Principles:**
1. **One-way data flow**: Events → Updates → State → View
2. **View is read-only**: Rendering never mutates state
3. **Updates are centralized**: All mutations through update functions
4. **Async work is isolated**: Background tasks communicate via messages

---

## Handling Async Operations Without Blocking

### Principle: Separate Compute from Coordination

**Compute (slow, blocking):**
- Docker command execution
- Git operations
- File I/O
- System stats collection

**Coordination (fast, non-blocking):**
- Event dispatching
- State updates
- Rendering
- Input handling

**Implementation:**
```rust
// Coordination layer (main thread, fast)
async fn run(app: App) {
    loop {
        // Non-blocking event consumption
        while let Ok(event) = rx.try_recv() {
            handle_event(&mut app, event);
        }

        // Fast render
        if app.needs_redraw {
            terminal.draw(|f| render(&app, f))?;
        }

        // Yield to prevent busy-loop
        tokio::time::sleep(Duration::from_millis(16)).await;
    }
}

// Compute layer (spawned tasks, slow)
tokio::spawn(async move {
    let output = expensive_operation().await; // Can take seconds
    tx.send(Message::OperationComplete { output }).await?;
});
```

### Pattern: Event Rate Limiting

**Problem:** Async tasks can flood the UI with events (e.g., output streaming).

**Solution:**
```rust
// Limit events processed per frame
const MAX_EVENTS_PER_FRAME: usize = 50;

for _ in 0..MAX_EVENTS_PER_FRAME {
    match event_rx.try_recv() {
        Ok(event) => handle_event(event),
        Err(_) => break, // No more events
    }
}
```

**Benefits:**
- UI stays responsive during output bursts
- Rendering not starved by event processing
- Prevents frame drops

### Pattern: Dedicated Input Thread

**Problem:** Under extreme CPU load, Tokio tasks may be starved, making the UI feel frozen.

**Solution:**
```rust
fn spawn_keyboard_thread() -> (Receiver<KeyEvent>, JoinHandle<()>) {
    let (tx, rx) = std::sync::mpsc::channel(); // Unbounded

    let handle = std::thread::spawn(move || {
        loop {
            if event::poll(Duration::from_millis(50))? {
                if let Event::Key(key) = event::read()? {
                    tx.send(key)?;
                }
            }
        }
    });

    (rx, handle)
}
```

**Why std::thread not tokio::spawn:**
- OS scheduler guarantees execution
- Independent of Tokio runtime
- Critical for user experience

**CI-TUI Implementation:**
This is CI-TUI's key innovation for responsiveness. The dedicated keyboard thread ensures 'q' to quit always works, even when Docker containers max out CPU.

---

## Testing Patterns

### Unit Testing State Transitions

**Pattern:**
```rust
#[test]
fn test_check_navigation() {
    let mut app = make_test_app();
    assert_eq!(app.selected_check, 0);

    app.next_check();
    assert_eq!(app.selected_check, 1);

    app.previous_check();
    assert_eq!(app.selected_check, 0);

    // Can't go below 0
    app.previous_check();
    assert_eq!(app.selected_check, 0);
}
```

**Benefits:**
- Fast (no I/O)
- Deterministic
- Tests business logic

### Testing Rendering with TestBackend

**Pattern:**
```rust
use ratatui::backend::TestBackend;
use ratatui::Terminal;

#[test]
fn test_render_check_list() {
    let backend = TestBackend::new(80, 24);
    let mut terminal = Terminal::new(backend)?;
    let app = make_test_app();

    terminal.draw(|f| render(&app, f))?;

    let buffer = terminal.backend().buffer();
    assert!(buffer.content.contains("✓ PHP Lint"));
}
```

**Benefits:**
- Tests actual rendering
- No real terminal needed
- Can verify layout

**CI-TUI Implementation:**
CI-TUI has extensive unit tests for `App` state transitions (1000+ lines of tests in `app.rs`). No rendering tests yet (opportunity for improvement).

### Integration Testing with ratatui-testlib

**For testing terminal interactions:**
- PTY-based harness
- Real terminal escape sequences
- User interaction flows

**Not currently used by CI-TUI** but recommended for complex interaction testing.

---

## Anti-Patterns to Avoid

### Anti-Pattern 1: Blocking the Main Loop

**Bad:**
```rust
loop {
    let event = events.recv()?; // BLOCKS - UI freezes
    handle_event(event);
    terminal.draw()?;
}
```

**Good:**
```rust
loop {
    tokio::select! {
        event = events.recv() => handle_event(event),
        _ = tick.tick() => {}, // Periodic wakeup
    }
    terminal.draw()?;
}
```

### Anti-Pattern 2: State Mutation in Render

**Bad:**
```rust
fn render(app: &mut App, f: &mut Frame) {
    app.last_render_time = Instant::now(); // MUTATION
    // ... render logic
}
```

**Good:**
```rust
fn render(app: &App, f: &mut Frame) {
    // Pure function, read-only
}

// Track last render separately
app.last_render_time = Instant::now();
terminal.draw(|f| render(&app, f))?;
```

### Anti-Pattern 3: Shared Mutable State Without Sync

**Bad:**
```rust
let mut results = HashMap::new();
tokio::spawn(async move {
    results.insert(...); // Data race!
});
```

**Good:**
```rust
let (tx, rx) = mpsc::channel();
tokio::spawn(async move {
    let result = compute().await;
    tx.send(result).await?;
});

// Main thread owns state
if let Ok(result) = rx.try_recv() {
    app.results.insert(result);
}
```

### Anti-Pattern 4: Unbounded Event Processing

**Bad:**
```rust
loop {
    // Process ALL events before rendering
    while let Ok(event) = rx.try_recv() {
        handle_event(event); // Could process 1000s
    }
    terminal.draw()?; // UI starved
}
```

**Good:**
```rust
loop {
    // Limit events per frame
    for _ in 0..MAX_EVENTS {
        match rx.try_recv() {
            Ok(e) => handle_event(e),
            Err(_) => break,
        }
    }
    terminal.draw()?; // Regular redraws
}
```

---

## CI-TUI Architecture Analysis

### Current Structure

CI-TUI uses a **hybrid MVC/TEA architecture** with async extensions:

**Strengths:**
1. ✅ Dedicated keyboard thread for guaranteed responsiveness
2. ✅ Comprehensive state container (`App`)
3. ✅ Separation of concerns (rendering, state, execution)
4. ✅ Event rate limiting prevents UI starvation
5. ✅ Extensive unit tests for state transitions
6. ✅ Dirty flag optimization (`needs_redraw`)

**Weaknesses:**
1. ❌ No explicit Message enum (implicit event handling)
2. ❌ Event handling scattered across `handle_key_event()` branches
3. ❌ Tight coupling between event loop and async task spawning
4. ❌ No time-travel debugging capability
5. ❌ Hard to test event sequences

### Recommended Refactoring

**Phase 1: Introduce Message Enum** (Low Risk)
```rust
pub enum Message {
    // Input
    KeyPress(KeyEvent),
    Tick,

    // Check execution
    CheckStarted { check_id: String },
    CheckOutput { check_id: String, line: String },
    CheckFinished { result: CheckResult },

    // Fix operations
    FixStarted,
    FixFinished { result: CheckResult },

    // System
    StatsUpdate { cpu: f32, mem_used: u64, mem_total: u64 },
    AllFinished,
}
```

**Benefits:**
- Explicit state transitions
- Easier to add logging/debugging
- Foundation for future improvements

**Phase 2: Centralize Update Logic** (Medium Risk)
```rust
fn update(app: &mut App, message: Message) -> Option<Command> {
    app.needs_redraw = true;

    match message {
        Message::KeyPress(key) => handle_key(app, key),
        Message::CheckFinished { result } => {
            app.results.insert(result.check_id.clone(), result);
            None
        }
        // ... all message types
    }
}
```

**Benefits:**
- Single place for state mutations
- Easier to reason about state changes
- Testable update logic

**Phase 3: Command Pattern for Async** (Medium Risk)
```rust
pub enum Command {
    RunCheck { check: CheckToRun },
    RunFix { check_id: String, command: String },
    RefreshGit,
    None,
}

fn update(app: &mut App, message: Message) -> Command {
    match message {
        Message::KeyPress(KeyCode::Char('r')) => {
            if let Some(check) = app.selected_check() {
                Command::RunCheck { check: check.clone() }
            } else {
                Command::None
            }
        }
        // ...
    }
}

// Event loop
loop {
    let message = next_message().await;
    let command = update(&mut app, message);

    // Execute commands (spawn tasks)
    match command {
        Command::RunCheck { check } => {
            spawn_check_runner(check, tx.clone());
        }
        // ...
    }
}
```

**Benefits:**
- Decouples update logic from async execution
- Testable without spawning tasks
- Clear separation of concerns

---

## Scalability Considerations

### At 10 Checks

**Current architecture is sufficient:**
- Simple state management
- Direct event handling works fine

### At 100 Checks

**Optimizations needed:**
- Virtual scrolling for check list
- Incremental rendering
- Output buffering to limit memory

**Pattern:**
```rust
struct App {
    check_list_viewport: Range<usize>, // Only render visible
    output_buffer: VecDeque<String>,   // Ring buffer, max 10K lines
}
```

### At 1000 Checks

**Architectural changes required:**
- Paging/filtering at data layer
- Lazy loading of results
- Streaming output to disk

**Consider:**
- Database for results persistence
- Worker pool for check execution
- Separate indexing layer

---

## Comparison: CI-TUI vs. Recommended Patterns

| Aspect | CI-TUI Current | Recommended TEA | Gap |
|--------|---------------|-----------------|-----|
| State container | ✅ Single `App` struct | ✅ Single source of truth | None |
| Message enum | ❌ Implicit via events | ✅ Explicit `Message` enum | Add Message enum |
| Update logic | ⚠️ Scattered in handlers | ✅ Centralized `update()` | Refactor Phase 2 |
| View purity | ✅ Read-only rendering | ✅ Pure function | None |
| Async handling | ✅ Task spawning via channels | ✅ Command pattern | Optional improvement |
| Event rate limiting | ✅ `MAX_EVENTS_PER_FRAME` | ✅ Bounded processing | None |
| Keyboard responsiveness | ✅ Dedicated OS thread | ⚠️ Usually `tokio::select!` | Better than recommended! |
| Testing | ✅ Extensive unit tests | ✅ Testable update | None |

**Overall:** CI-TUI's architecture is **80% aligned** with TEA best practices. The main gap is lack of explicit Message enum and centralized update function. The dedicated keyboard thread is a **novel improvement** over standard TEA.

---

## Suggested Refactoring Order

### Priority 1: No Breaking Changes (Safe)
1. Add `Message` enum (parallel to current event handling)
2. Add unit tests for new `update()` function
3. Gradually migrate event handlers to use `update()`

### Priority 2: Centralize Update Logic (Medium Risk)
1. Create `update(app: &mut App, msg: Message)` function
2. Move all state mutations into `update()`
3. Keep event loop as message dispatcher

### Priority 3: Command Pattern (Optional)
1. Add `Command` enum for async operations
2. Return `Command` from `update()`
3. Execute commands in event loop

### Priority 4: Advanced Improvements (Low Priority)
1. Add undo/redo via message history
2. Time-travel debugging
3. State serialization for crash recovery

---

## Resources & References

### Official Documentation
- [The Elm Architecture | Ratatui](https://ratatui.rs/concepts/application-patterns/the-elm-architecture/)
- [Component Architecture | Ratatui](https://ratatui.rs/concepts/application-patterns/component-architecture/)
- [Async Counter App Tutorial](https://ratatui.rs/tutorials/counter-async-app/)
- [Best Practices Discussion](https://github.com/ratatui/ratatui/discussions/220)

### Async Patterns
- [Async Event Stream](https://ratatui.rs/tutorials/counter-async-app/async-event-stream/)
- [Forum: Running Async Tasks](https://forum.ratatui.rs/t/how-do-i-run-an-async-task-and-update-ui-when-finished/129)

### Testing
- [TestBackend Documentation](https://docs.rs/ratatui/latest/ratatui/backend/struct.TestBackend.html)
- [ratatui-testlib](https://lib.rs/crates/ratatui-testlib)

### Community Examples
- [async-ratatui Example](https://github.com/d-holguin/async-ratatui)
- [Async Template](https://ratatui.github.io/async-template/)
- [Component Template](https://github.com/ratatui/templates/tree/main/component)

---

## Summary

**Recommended Architecture:** Modified Elm Architecture with Async Extensions

**Key Principles:**
1. **Unidirectional data flow**: Events → Messages → Update → State → View
2. **Single state tree**: All state in one place (`App`)
3. **Non-blocking async**: `tokio::select!` or `try_recv()` patterns
4. **Event rate limiting**: Bounded processing per frame
5. **Dirty flag optimization**: Only redraw when state changes

**CI-TUI Status:**
- Already follows 80% of best practices
- Main improvement: Add explicit Message enum
- Keyboard thread pattern is better than typical TEA
- Solid foundation for future enhancements

**Next Steps:**
1. Add `Message` enum (Phase 1)
2. Centralize update logic (Phase 2)
3. Consider Command pattern for testability (Phase 3)
