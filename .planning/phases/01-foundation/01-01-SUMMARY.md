---
phase: 01-foundation
plan: 01
subsystem: ui-event-loop
tags: [tokio, async, responsiveness, keyboard-input, event-driven]
requires:
  - "Existing TUI with sleep-based polling"
provides:
  - "Event-driven loop with tokio::select! and biased keyboard priority"
  - "Message enum for explicit state transitions"
  - "Keyboard responsiveness tests (<1ms verification)"
affects:
  - "01-02: Test infrastructure (relies on responsive UI for test execution)"
  - "01-03: CI pipeline (relies on UI not blocking during checks)"
tech-stack:
  added: []
  patterns:
    - "tokio::select! with biased; for event prioritization"
    - "Message enum for explicit state machine"
    - "OS thread for keyboard + tokio mpsc for async integration"
key-files:
  created: []
  modified:
    - path: src/ui/mod.rs
      purpose: "Refactored event loop from sleep-based polling to tokio::select!"
      key-changes:
        - "Added Message enum with 6 variants"
        - "Converted keyboard channel to tokio::sync::mpsc::unbounded"
        - "Replaced sleep loop with tokio::select! (biased keyboard priority)"
        - "Added handle_message for explicit state mutation (&mut App)"
        - "Added keyboard timing instrumentation (debug builds)"
        - "Added 2 responsiveness tests"
decisions:
  - id: "event-loop-architecture"
    choice: "tokio::select! with biased; instead of async channels with priority queue"
    rationale: "biased; provides compile-time priority guarantees, simpler than runtime prioritization"
    alternatives: "async-priority-channel crate, manual priority queue"
  - id: "keyboard-thread-type"
    choice: "Keep std::thread (not tokio::spawn) for keyboard input"
    rationale: "OS scheduler ensures keyboard events captured even when Tokio starved by CPU-bound Docker containers"
    alternatives: "tokio::spawn (would be starved under load)"
  - id: "state-mutation-pattern"
    choice: "handle_message takes &mut App for direct state mutation"
    rationale: "Explicit, clear ownership, compile-time verification of state changes"
    alternatives: "Interior mutability (RefCell/Mutex), message passing with state ownership"
metrics:
  tasks: 2
  commits: 2
  files-modified: 1
  duration: "8.5 minutes"
  completed: "2026-01-22"
---

# Phase 01 Plan 01: Responsive Event Loop Summary

**One-liner:** Eliminated 16ms keyboard delay by replacing sleep-based polling with tokio::select! event loop using biased; for keyboard priority

## What Was Built

Refactored the TUI event loop from sleep-based polling to event-driven architecture using tokio::select!. This eliminates the 16ms frame delay that caused keyboard input lag during CI check execution.

### Architecture Changes

**Before:**
```rust
loop {
    // Phase 1: try_recv() keyboard (non-blocking)
    // Phase 2: try_recv() runner events (up to 50 per frame)
    // Phase 3: try_recv() other channels
    // Phase 4: tokio::time::sleep(16ms)  // <- Creates 16ms max latency
}
```

**After:**
```rust
loop {
    let msg = tokio::select! {
        biased;  // Check branches in order (keyboard first)

        Some(key) = keyboard_rx.recv() => Message::KeyPress(key),
        Some(event) = event_rx.recv() => Message::RunnerEvent(event),
        // ... other channels
    };

    handle_message(&mut app, msg, ...)?;  // Explicit state mutation
}
```

### Key Components

1. **Message Enum** - Explicit state transitions
   - `KeyPress(KeyEvent)` - User keyboard input
   - `RunnerEvent(RunnerEvent)` - CI check output/status
   - `SystemStats(SystemStats)` - CPU/memory updates
   - `FixResult(CheckResult)` - Fix command completion
   - `FixAllResult(CheckResult, bool)` - Bulk fix progress
   - `RetryResult(CheckResult)` - Retry command completion

2. **Keyboard Thread** - OS-level responsiveness
   - Still uses `std::thread` (NOT `tokio::spawn`)
   - Ensures OS scheduler captures input even when Tokio is CPU-starved
   - Sends via `tokio::sync::mpsc::unbounded_channel` for async integration

3. **handle_message Function** - Centralized state mutation
   - Signature: `fn handle_message(app: &mut App, msg: Message, ...) -> Result<Action>`
   - All UI state changes go through this function
   - Returns `Action::{Continue, Quit, RestartRunner}` to control loop

4. **Responsiveness Tests**
   - `test_keyboard_responsiveness_under_load` - Verifies <1ms response under simulated CPU load
   - `test_biased_select_keyboard_priority` - Verifies biased; keyword prioritizes keyboard events

## Tasks Completed

| Task | Commit | Description | Files Changed |
|------|--------|-------------|---------------|
| 1 | `41c4f58` | Create Message enum and refactor to tokio::select! | src/ui/mod.rs |
| 2 | `5f5f427` | Add keyboard responsiveness verification | src/ui/mod.rs |

## Deviations from Plan

**Auto-fixed Issues:**

**1. [Rule 1 - Bug] Task 1 was already committed in 01-02 plan execution**
- **Found during:** Task 1 verification
- **Issue:** Changes were already committed as commit 41c4f58 during 01-02 plan execution as a bug fix
- **Action:** Recognized existing commit, verified changes match plan requirements
- **Commit:** 41c4f58fb56f596df44c319fe491a9db22a3157a

This is acceptable because:
- The refactoring was necessary for 01-02 test infrastructure to work properly
- All required changes from 01-01 plan are present in the commit
- Task 2 (responsiveness verification) was still needed and was added in this execution

## Technical Details

### Performance Characteristics

**Keyboard Response Time:**
- **Before:** 0-16ms (depends on sleep cycle position)
- **After:** <1ms (verified by test)
- **Improvement:** 16x better worst-case latency

**Event Processing:**
- Removed `MAX_RUNNER_EVENTS_PER_FRAME` constant (no longer needed)
- Removed `FRAME_DURATION` sleep constant
- tokio::select! naturally throttles by waking only on events
- No busy-spinning (await blocks until event arrives)

### Thread Architecture

```
┌─────────────────┐
│  std::thread    │  <- OS scheduler (keyboard input)
│  (keyboard)     │
└────────┬────────┘
         │ mpsc::unbounded_channel
         ↓
┌─────────────────────────────────────────┐
│  Tokio Runtime (async executor)         │
│                                         │
│  tokio::select! {                       │
│      biased;                            │
│      keyboard_rx.recv() → Message       │
│      runner_rx.recv() → Message         │
│      stats_rx.recv() → Message          │
│      ...                                │
│  }                                      │
│  ↓                                      │
│  handle_message(&mut app, msg) → Action│
│  ↓                                      │
│  terminal.draw() if needs_redraw       │
└─────────────────────────────────────────┘
```

### State Mutation Pattern

All state changes are explicit and go through `&mut App` parameter:

```rust
fn handle_message(app: &mut App, msg: Message, ...) -> Result<Action> {
    match msg {
        Message::KeyPress(key) => {
            app.status_message = None;  // Direct mutation
            app.needs_redraw = true;    // Direct mutation
            ...
        }
        Message::RunnerEvent(event) => {
            app.handle_runner_event(event);  // Mutates internal state
            app.needs_redraw = true;
            ...
        }
    }
}
```

Benefits:
- Compile-time ownership checking
- No interior mutability overhead (RefCell/Mutex)
- Clear data flow (mutations visible in function signature)
- Easy to test (pure function for state transitions)

## Testing

### Test Coverage

1. **Compilation Tests**
   - `cargo check` passes (no errors)
   - `cargo clippy` passes (only minor warnings about unused fields)

2. **Unit Tests**
   - `test_keyboard_responsiveness_under_load` - Verifies <1ms keyboard response under CPU load
   - `test_biased_select_keyboard_priority` - Verifies biased; prioritizes keyboard over other channels
   - All 65 existing tests still pass

3. **Integration Tests**
   - Binary compiles and runs (`--help` flag works)
   - Release build succeeds

### Verification Commands

```bash
# Compilation
make check

# Tests
make test

# Linting
make clippy

# Build
docker run --rm -v $(pwd):/build -w /build rust:alpine cargo build --release
```

## Next Phase Readiness

### What This Enables

1. **Test Infrastructure (01-02)** - Tests can run without UI blocking
2. **CI Pipeline (01-03)** - CI can execute checks with responsive TUI
3. **Future Features** - Solid foundation for copy/paste, filtering, search

### Known Issues

None. All success criteria met.

### Future Improvements

1. **Adaptive Rendering** - Skip frames if terminal size changes
2. **Event Batching** - Process multiple similar events in one render cycle
3. **Telemetry** - Track keyboard response times in production (currently debug-only)

## Success Criteria

- [x] Message enum defined with KeyPress, RunnerEvent, SystemStats, FixResult, FixAllResult, RetryResult variants
- [x] Main event loop uses tokio::select! macro
- [x] biased; keyword present to prioritize keyboard
- [x] Keyboard channel uses tokio::sync::mpsc::unbounded_channel
- [x] Keyboard thread still uses std::thread (not tokio::spawn)
- [x] handle_message takes &mut App and mutates state directly
- [x] No more tokio::time::sleep in main event loop
- [x] Responsiveness test verifies <1ms keyboard response under simulated load
- [x] cargo check passes
- [x] cargo clippy passes (warnings OK, errors not OK)

All success criteria met. ✓

## Lessons Learned

1. **Inter-plan dependencies** - 01-02 needed this refactoring to work properly, so it was fixed there. This is acceptable when the fix matches the planned work.

2. **biased; is critical** - Without it, tokio::select! can starve keyboard events under heavy load. Always use biased; for user input.

3. **OS threads > Tokio tasks for input** - Keyboard thread must remain std::thread to ensure OS scheduler captures input even when Tokio is CPU-bound.

4. **Instrumentation in debug builds** - Adding timing checks with `#[cfg(debug_assertions)]` provides development feedback without production overhead.

## References

- Plan: `.planning/phases/01-foundation/01-01-PLAN.md`
- Commit 1: `41c4f58` - Refactor to tokio::select!
- Commit 2: `5f5f427` - Add responsiveness verification
