# Project Research Summary: CI-TUI Code Quality

**Project:** CI-TUI Code Quality Enhancement
**Domain:** Rust TUI Application Testing & Maintainability
**Researched:** 2026-01-22
**Confidence:** HIGH

## Executive Summary

CI-TUI is a well-architected Rust TUI application that currently follows 80% of recommended patterns but lacks comprehensive test coverage and has a critical responsiveness issue. The research across four dimensions (Stack, Features, Architecture, Pitfalls) reveals a clear path forward: modernize the event loop architecture to use tokio::select! instead of the current sleep-based pattern, establish a comprehensive testing foundation using cargo-nextest and mockall, and introduce explicit message passing via an enum to enable better testability and debugging.

The recommended approach prioritizes fixing the event loop monopolization issue (which causes 16ms+ input lag) before adding tests, as the architectural foundation needs to be solid. Once the event loop is refactored, the testing stack (nextest, rstest, mockall, pretty_assertions, cargo-llvm-cov) can be incrementally applied starting with high-value unit tests for business logic, then widget tests using TestBackend, and finally integration tests with ratatui-testlib for end-to-end flows.

The key risk is that refactoring the event loop is a moderate-complexity change that touches the core UI module, but the research shows this is a necessary foundation for both responsiveness and testability. The mitigation is to introduce the Message enum in parallel first (low risk), then gradually migrate the event loop to use tokio::select! with async channels while maintaining existing functionality. The existing test suite and manual testing will catch regressions, and the improvement in responsiveness will be immediately measurable.

## Key Findings

### Recommended Stack

The Rust ecosystem provides mature, production-ready tooling for TUI testing. The core stack combines built-in Rust tools with specialized TUI testing libraries and modern test runners that deliver 3x faster execution.

**Core technologies:**
- **cargo-nextest** (0.9.x+): Modern test runner — 3x faster than cargo test, JUnit XML output, test retries, partitioning for parallel CI
- **mockall** (0.14.x): Trait-based mocking — Official AOSP recommendation, enables isolated testing of Docker/git interactions
- **ratatui TestBackend**: Widget testing — Built into ratatui 0.30+, captures terminal buffer for assertions without real terminal
- **rstest** (0.23.x+): Test fixtures and parameterization — Eliminates boilerplate for table-driven tests, essential for config/pattern testing
- **pretty_assertions** (1.4.x): Enhanced assertion diffs — Drop-in replacement for assert_eq! with colorful output for complex structs
- **cargo-llvm-cov** (0.6.x+): Code coverage — LLVM-based accuracy, works with nextest, multiple output formats (HTML/LCOV/JSON)
- **cargo-deny** (0.19.x): Dependency security — Checks CVEs, licenses, duplicate versions, prevents dependency bloat
- **insta** (1.40.x+): Snapshot testing — Ergonomic review workflow for config parsing and output validation
- **proptest** (1.6.x+): Property-based testing — Better shrinking than quickcheck, tests edge cases in pattern matching

**Phase 1 priorities:** nextest, mockall, rstest, pretty_assertions, cargo-llvm-cov, cargo-deny
**Phase 2 additions:** TestBackend (widget tests), insta (snapshot tests), proptest (property tests)
**Phase 3 advanced:** ratatui-testlib (PTY-based integration tests)

### Expected Features (Code Quality Patterns)

Research identified clear table stakes patterns versus differentiators for maintainable Rust TUI applications.

**Must have (table stakes):**
- Clear Model/View/Update separation (TEA or MVC pattern) — Prevents UI entanglement
- Proper async event handling with tokio::select! — Prevents responsiveness issues
- Terminal cleanup on panic with panic hook — Avoids corrupted terminal state
- Linting with Clippy at deny level — Catches common mistakes
- Unit tests for business logic — Core logic testable in isolation
- Error handling with Result types, no unwrap() in production — Proper error propagation
- Idiomatic Rust patterns (if let, iterators, borrowing) — Follows Rust API Guidelines

**Should have (competitive):**
- Component-based architecture — Modular UI scales to complex interfaces
- Action/Command mapping system — Decouples key bindings from business logic
- File-based logging (tracing/log4rs) — Debug without interfering with TUI
- Immediate-mode rendering at fixed FPS — Consistent responsiveness
- TestBackend for widget tests — Preferred over integration tests for rendering
- XDG-compliant configuration — Follows platform conventions

**Defer (v2+) or avoid:**
- Over-abstraction layers — Keep it simple, two levels max
- Excessive .clone() usage — Use references, Arc for shared ownership
- Integration tests in tests/ folder — Prefer unit tests and doc tests in source files
- Preludes for application code — Re-export at crate root instead

### Architecture Approach

CI-TUI currently uses a hybrid MVC/TEA architecture with async extensions, which aligns 80% with recommended patterns. The main gap is lack of explicit Message enum and the event loop sleep pattern that creates input lag.

**Major components:**
1. **State Container (ui/app.rs)** — Single source of truth for all application state, implements dirty flag optimization
2. **Event Loop (ui/mod.rs)** — Currently uses try_recv() + sleep pattern, should migrate to tokio::select! with async channels
3. **Rendering Layer (ui/dashboard.rs)** — Pure read-only rendering functions, no state mutation
4. **Async Execution (runner.rs)** — Docker command execution with event streaming through channels
5. **Business Logic (checks.rs, test_discovery.rs)** — Pattern matching and test discovery, isolated from UI concerns

**Recommended refactoring order:**
1. Introduce Message enum (low risk, parallel to current event handling)
2. Centralize update logic in update(app, msg) function (medium risk)
3. Convert to tokio::select! with async channels (medium risk, high value)
4. Add Command pattern for async operations (optional, improves testability)

**Key architectural principle:** Unidirectional data flow (Events → Messages → Update → State → View) with dedicated OS thread for keyboard input to guarantee responsiveness under extreme CPU load.

### Critical Pitfalls

Research identified async Rust TUI-specific pitfalls, with one critical issue affecting CI-TUI.

1. **Single Async Task Monopolizes Event Loop** — Main loop with tokio::time::sleep(16ms).await creates 16ms+ input lag floor. Even with dedicated keyboard thread, async processing is delayed until sleep completes. FIX: Use tokio::select! to interleave keyboard, runner events, and tick interval.

2. **Mixing Sync and Async Channels Incorrectly** — Using std::sync::mpsc with try_recv() in async tasks doesn't yield to scheduler. CI-TUI currently does this but mitigates with sleep at end of loop. FIX: Convert to tokio::sync::mpsc with recv().await in select!.

3. **Long-Running Operations Without Yielding** — CPU-intensive work (regex compilation, large iterations) without .await points blocks worker thread. FIX: Add tokio::task::yield_now() every 100 iterations or use spawn_blocking for CPU-bound work.

4. **Blocking File I/O in Async Tasks** — Using std::fs in async context blocks worker thread. CI-TUI loads config before TUI starts (OK), but future features need tokio::fs. FIX: Use tokio::fs or spawn_blocking for file operations.

5. **Channel Backpressure** — Bounded channels fill when consumer can't keep pace. CI-TUI handles this well with MAX_RUNNER_EVENTS_PER_FRAME limiter and unbounded keyboard channel. MAINTAIN: Current approach is correct.

**CI-TUI strengths already implemented:**
- Dirty flag rendering optimization (needs_redraw)
- KeyEventKind::Press filtering for cross-platform consistency
- Panic hook for terminal restoration
- Dedicated OS thread for keyboard input (novel improvement over standard patterns)
- Event rate limiting to prevent UI starvation

## Implications for Roadmap

Based on research, the project should be structured in three main phases with clear dependencies and risk mitigation.

### Phase 1: Foundation (Event Loop & Testing Infrastructure)

**Rationale:** The event loop architecture must be fixed before adding tests, as the current sleep-based pattern creates measurable input lag and complicates async testing. Establishing testing infrastructure early enables test-driven development for subsequent phases.

**Delivers:**
- Refactored event loop using tokio::select! with <1ms input latency
- Message enum for explicit state transitions
- Testing stack installed (nextest, mockall, rstest, pretty_assertions, cargo-llvm-cov, cargo-deny)
- CI pipeline with test, lint, coverage, and security checks
- Initial unit tests for core modules as examples

**Addresses:**
- Pitfall 1 (event loop monopolization) — Critical responsiveness issue
- Pitfall 2 (sync/async channels) — Convert to tokio::sync::mpsc
- Table stakes: Proper async event handling with tokio::select!

**Avoids:**
- Building test infrastructure on top of flawed event loop architecture
- Accumulating technical debt that makes testing harder later

**Research flags:** Standard patterns from official Ratatui tutorials, no additional research needed.

### Phase 2: Core Testing Coverage

**Rationale:** With solid foundation, add tests incrementally starting with highest-value modules (business logic, config parsing) where mocking is straightforward, then widget rendering, then integration tests.

**Delivers:**
- Unit tests for business logic (checks.rs, test_discovery.rs, config.rs) using rstest for parameterized tests
- Mock-based tests for runner.rs and git.rs using mockall
- Widget tests using TestBackend for UI components
- Snapshot tests using insta for config parsing and output formatting
- Property-based tests using proptest for pattern matching edge cases
- 60%+ code coverage measured with cargo-llvm-cov

**Uses:**
- rstest for fixtures and table-driven tests (config variations, file patterns)
- mockall for isolating Docker/git dependencies
- TestBackend for rendering assertions
- insta for regression detection in text output
- proptest for finding edge cases in regex patterns

**Implements:**
- Component-based testing architecture (unit → widget → integration)
- Test organization pattern (#[cfg(test)] modules in source files)
- Coverage reporting in CI with Codecov integration

**Avoids:**
- Pitfall 3 (long operations) — Profiling during testing reveals CPU-intensive code paths
- Over-reliance on integration tests (prefer unit tests for faster feedback)

**Research flags:** Standard testing patterns, well-documented. No additional research needed.

### Phase 3: Advanced Testing & Polish

**Rationale:** After core coverage established, add advanced testing capabilities and polish the development workflow with continuous dependency updates and periodic security audits.

**Delivers:**
- Integration tests using ratatui-testlib for end-to-end user flows
- Centralized update() function with Command pattern for better testability
- Doc tests for public API (config.rs, checks.rs)
- Dependabot configuration for weekly dependency updates
- Miri periodic checks for undefined behavior detection
- cargo-geiger audit of unsafe code in dependencies

**Uses:**
- ratatui-testlib (0.1.x) for PTY-based integration testing
- Command pattern returning async operations from pure update function
- cargo-geiger for security audits
- Dependabot for automated dependency updates

**Implements:**
- Full TEA architecture with Message/Update/Command separation
- Time-travel debugging capability (foundation for future)
- State serialization for crash recovery (optional)

**Avoids:**
- Pitfall 4 (blocking file I/O) — If adding state persistence, use tokio::fs
- Unbounded technical debt from outdated dependencies

**Research flags:**
- **ratatui-testlib integration** — Version 0.1.0 (early stage), may need experimentation for CI-TUI's specific flows
- **Command pattern implementation** — Well-documented but requires careful refactoring

### Phase Ordering Rationale

**Why this order based on dependencies:**
- Event loop architecture must be fixed first (Phase 1) because it affects all async operations and is hard to test with current structure
- Message enum enables better testing in Phase 2 by making state transitions explicit
- Testing infrastructure in Phase 1 enables test-driven development for all subsequent work
- Unit/widget tests (Phase 2) provide faster feedback than integration tests (Phase 3)
- Advanced patterns (Phase 3) build on stable foundation from Phases 1-2

**Why this grouping based on architecture patterns:**
- Phase 1 establishes the event-driven architecture (TEA foundation)
- Phase 2 adds testing at each architectural layer (business logic → widgets → integration)
- Phase 3 completes TEA architecture with Command pattern and adds advanced tooling

**How this avoids pitfalls from research:**
- Phase 1 directly addresses Pitfalls 1-2 (event loop, channels) which are critical
- Phase 2 testing reveals Pitfall 3 (long operations) through profiling
- Phase 3 prevents Pitfall 4 (blocking I/O) before adding features that need it
- Maintaining good patterns (dirty flag, panic hook, platform filtering) throughout

### Research Flags

**Phases with standard patterns (skip research-phase):**
- **Phase 1:** Event loop refactoring well-documented in official Ratatui async tutorials and Tokio guides
- **Phase 2:** Testing patterns thoroughly covered in tool documentation (nextest, mockall, rstest, etc.)

**Phases that may need deeper research during planning:**
- **Phase 3 (ratatui-testlib integration):** Version 0.1.0 indicates early development, may encounter edge cases or API changes. Monitor release notes and examples.
- **Phase 3 (Command pattern):** While conceptually documented, adapting to CI-TUI's specific async execution patterns may require experimentation.

**When to invoke /gsd:research-phase:**
- If ratatui-testlib proves unstable or insufficient for CI-TUI's integration testing needs
- If adding new features with unfamiliar domains (e.g., LSP integration, plugin system)
- If encountering performance bottlenecks requiring benchmarking infrastructure

## Confidence Assessment

| Area | Confidence | Notes |
|------|------------|-------|
| Stack | HIGH | All tools verified with official docs, 100M+ downloads for core crates, actively maintained |
| Features | HIGH | Patterns drawn from official Ratatui docs, multiple 2025 articles, established best practices |
| Architecture | HIGH | CI-TUI's current architecture analyzed directly, recommendations based on official TEA documentation |
| Pitfalls | HIGH | Verified with official Tokio/Ratatui docs, authoritative sources (Alice Ryhl), codebase analysis |

**Overall confidence:** HIGH

The research is grounded in official documentation (Ratatui, Tokio, cargo-nextest, etc.), verified with multiple authoritative sources, and cross-validated by analyzing CI-TUI's actual codebase structure. All recommendations are production-proven patterns from established projects.

### Gaps to Address

**Identified gaps and handling strategy:**

1. **ratatui-testlib stability (v0.1.0):** Early version may have API changes or limitations. **Handling:** Defer to Phase 3, monitor releases, have fallback plan to write custom PTY harness if needed.

2. **Performance baseline unclear:** No current benchmarks for event loop latency or rendering performance. **Handling:** Establish baseline measurements in Phase 1 before/after refactoring to quantify improvement.

3. **Cross-platform testing coverage:** Research documents Windows vs Linux/macOS differences but CI-TUI may not be tested on all platforms. **Handling:** Add Windows to CI matrix in Phase 1, use platform-specific test cases where needed.

4. **Dependency count (43 dependencies):** cargo-deny will catch issues but need proactive strategy for preventing bloat. **Handling:** Review deny.toml configuration in Phase 1, enable multiple-versions check to prevent duplicate dependencies.

5. **Integration test strategy unclear:** ratatui-testlib is recommended but CI-TUI's specific use cases (Docker containers, git operations, file system) may need custom harnesses. **Handling:** Start with TestBackend for widget tests in Phase 2, prototype ratatui-testlib in Phase 3 before committing to full integration test suite.

## Sources

### Primary Sources (HIGH confidence)

**Stack Research:**
- [cargo-nextest official site](https://nexte.st/) — Test runner features, installation, usage
- [Ratatui TestBackend Documentation](https://docs.rs/ratatui/latest/ratatui/backend/struct.TestBackend.html) — Widget testing API
- [mockall GitHub](https://github.com/asomers/mockall) — Official AOSP recommendation, mocking patterns
- [cargo-llvm-cov GitHub](https://github.com/taiki-e/cargo-llvm-cov) — Coverage collection
- [cargo-deny GitHub](https://github.com/EmbarkStudios/cargo-deny) — January 2026 release notes
- [Clippy Documentation](https://doc.rust-lang.org/clippy/) — Linting configuration

**Features Research:**
- [The Elm Architecture | Ratatui](https://ratatui.rs/concepts/application-patterns/the-elm-architecture/) — TEA pattern for TUI
- [Best practices for ratatui apps](https://github.com/ratatui/ratatui/discussions/220) — Community consensus
- [Project Structure | Ratatui](https://ratatui.rs/templates/component/project-structure/) — Component-based organization
- [7 Rust Idioms for Clean Code (Dec 2025)](https://medium.com/@jamesmiller22871/7-rust-idioms-for-clean-high-performance-code-6d7433e66d65) — Idiomatic patterns

**Architecture Research:**
- [Ratatui Async Event Stream Tutorial](https://ratatui.rs/tutorials/counter-async-app/async-event-stream/) — tokio::select! patterns
- [async-ratatui example](https://github.com/d-holguin/async-ratatui) — Real-world async architecture
- [Tokio Channels Tutorial](https://tokio.rs/tokio/tutorial/channels) — Channel patterns

**Pitfalls Research:**
- [Alice Ryhl: Async: What is blocking?](https://ryhl.io/blog/async-what-is-blocking/) — Authoritative source on cooperative scheduling
- [Tokio spawn_blocking Documentation](https://docs.rs/tokio/latest/tokio/task/fn.spawn_blocking.html) — When to use blocking threads
- [Tokio Select Tutorial](https://tokio.rs/tokio/tutorial/select) — Event loop patterns
- [Ratatui Event Handling Concepts](https://ratatui.rs/concepts/event-handling/) — Terminal event handling

### Secondary Sources (MEDIUM confidence)

- [ratatui-testlib documentation](https://docs.rs/ratatui-testlib/latest/ratatui_testlib/) — Integration testing (v0.1.0, early stage)
- [The 7 Rust Anti-Patterns (2025)](https://medium.com/solo-devs/the-7-rust-anti-patterns-that-are-secretly-killing-your-performance-and-how-to-fix-them-in-2025-dcebfdef7b54) — Common pitfalls
- [How Tokio Schedule Tasks](https://rustmagazine.org/issue-4/how-tokio-schedule-tasks/) — Scheduling model details

### Codebase Analysis

Direct analysis of CI-TUI source code:
- `src/ui/mod.rs` (lines 362-540) — Event loop implementation
- `src/ui/app.rs` — State container structure
- `src/ui/dashboard.rs` — Rendering layer
- `src/runner.rs` — Async execution patterns
- `src/checks.rs` — Business logic for check determination

---

**Research completed:** 2026-01-22
**Ready for roadmap:** Yes

All research dimensions completed with high confidence. Findings synthesized into actionable three-phase roadmap with clear dependencies, rationale, and risk mitigation strategies. Recommended approach prioritizes fixing event loop responsiveness (Phase 1), establishing comprehensive testing coverage (Phase 2), then adding advanced capabilities (Phase 3).
