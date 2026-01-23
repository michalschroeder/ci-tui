# Roadmap: CI-TUI Code Quality

## Overview

This milestone transforms CI-TUI from AI-generated code into a maintainable, well-tested Rust TUI application. The journey fixes critical TUI responsiveness issues, establishes comprehensive testing infrastructure, cleans up the codebase to idiomatic Rust standards, and achieves 60%+ test coverage across business logic, external dependencies, and UI components. Each phase builds on the previous: first we fix the event loop architecture and install testing tools, then clean up code quality, then systematically add test coverage from unit tests through widget tests.

## Phases

**Phase Numbering:**
- Integer phases (1, 2, 3): Planned milestone work
- Decimal phases (2.1, 2.2): Urgent insertions (marked with INSERTED)

Decimal phases appear between their surrounding integers in numeric order.

- [x] **Phase 1: Foundation** - Event loop refactoring + testing infrastructure
- [ ] **Phase 2: Code Quality Baseline** - Clean up codebase to idiomatic Rust
- [ ] **Phase 3: Unit Test Coverage** - Tests for business logic modules
- [ ] **Phase 4: Mock-Based Tests** - Tests for external dependencies
- [ ] **Phase 5: Widget Tests & Architecture** - UI tests + architectural polish

## Phase Details

### Phase 1: Foundation
**Goal**: TUI responds to keyboard input within 1ms during check execution, and testing infrastructure is ready for test-driven development

**Depends on**: Nothing (first phase)

**Requirements**: EVNT-01, EVNT-02, EVNT-03, EVNT-04, TEST-01, TEST-02, TEST-03, TEST-04, TEST-05, TEST-06

**Success Criteria** (what must be TRUE):
  1. Developer presses keyboard shortcut during CI check execution and UI responds within 1ms (measurable with instrumentation)
  2. Event loop uses tokio::select! pattern instead of sleep-based polling
  3. All state transitions are represented by explicit Message enum variants
  4. Developer runs `cargo nextest run` and sees 3x faster test execution than cargo test
  5. CI pipeline runs tests, lints, and coverage reporting on every commit

**Plans:** 3 plans

Plans:
- [x] 01-01-PLAN.md — Refactor event loop to tokio::select! with Message enum
- [x] 01-02-PLAN.md — Add test dependencies and configure nextest
- [x] 01-03-PLAN.md — Create CI pipeline with tests, linting, coverage

### Phase 2: Code Quality Baseline
**Goal**: Codebase follows idiomatic Rust patterns with no Clippy warnings, all dead code removed, and proper error handling throughout

**Depends on**: Phase 1

**Requirements**: QUAL-01, QUAL-02, QUAL-03, QUAL-04, QUAL-05, QUAL-06, QUAL-07, QUAL-08

**Success Criteria** (what must be TRUE):
  1. Running `cargo clippy -- -D warnings` passes with zero warnings (deny level enabled)
  2. All unused imports, dead code, and unreachable code paths have been removed from codebase
  3. Production code uses proper error propagation with Result types instead of unwrap/expect
  4. Every public API function and module has documentation comments explaining purpose and usage
  5. Code review shows consistent use of idiomatic Rust patterns (if let, iterators, proper borrowing)

**Plans:** 2 plans

Plans:
- [ ] 02-01-PLAN.md — Configure Clippy deny level and fix all warnings
- [ ] 02-02-PLAN.md — Fix production unwrap calls and enhance documentation

### Phase 3: Unit Test Coverage
**Goal**: Business logic modules (checks, test_discovery, config) have comprehensive unit tests with fast feedback loop

**Depends on**: Phase 2

**Requirements**: COVR-01, COVR-02, COVR-03

**Success Criteria** (what must be TRUE):
  1. Developer modifies pattern matching logic in checks.rs and existing tests catch regressions
  2. Developer changes path mapping rules in test_discovery.rs and parameterized tests verify behavior across multiple scenarios
  3. Developer adds new YAML config structure and parser tests validate both success and error cases
  4. All unit tests run in under 1 second providing immediate feedback during development

**Plans**: TBD

Plans:
- [ ] 03-01: TBD during planning

### Phase 4: Mock-Based Tests
**Goal**: Modules with external dependencies (Docker, Git) are testable in isolation without containers or repositories

**Depends on**: Phase 3

**Requirements**: COVR-04, COVR-05

**Success Criteria** (what must be TRUE):
  1. Developer runs runner.rs tests without Docker installed and all tests pass using mocked Docker execution
  2. Developer runs git.rs tests in directory without Git repository and all tests pass using mocked Git operations
  3. Test suite executes completely in CI environment without Docker daemon or Git repositories

**Plans**: TBD

Plans:
- [ ] 04-01: TBD during planning

### Phase 5: Widget Tests & Architecture
**Goal**: UI components have widget tests using TestBackend, architecture follows centralized state update pattern, and 60%+ code coverage is achieved

**Depends on**: Phase 4

**Requirements**: COVR-06, COVR-07, ARCH-01, ARCH-02, ARCH-03

**Success Criteria** (what must be TRUE):
  1. Developer modifies dashboard rendering logic and widget tests verify terminal buffer output without running TUI
  2. All state updates flow through centralized update() function enabling predictable state transitions
  3. Panic during check execution triggers panic hook that restores terminal to normal mode (user can see their shell)
  4. Running `cargo llvm-cov --html` shows 60%+ code coverage across core modules with detailed line-by-line report
  5. Dirty flag optimization (needs_redraw) is preserved and verified during all UI refactoring

**Plans**: TBD

Plans:
- [ ] 05-01: TBD during planning

## Progress

**Execution Order:**
Phases execute in numeric order: 1 -> 2 -> 3 -> 4 -> 5

| Phase | Plans Complete | Status | Completed |
|-------|----------------|--------|-----------|
| 1. Foundation | 3/3 | Complete | 2026-01-23 |
| 2. Code Quality Baseline | 0/2 | Ready | - |
| 3. Unit Test Coverage | 0/TBD | Not started | - |
| 4. Mock-Based Tests | 0/TBD | Not started | - |
| 5. Widget Tests & Architecture | 0/TBD | Not started | - |

---
*Roadmap created: 2026-01-22*
*Last updated: 2026-01-24*
