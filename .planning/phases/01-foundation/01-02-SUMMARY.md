---
phase: 01-foundation
plan: 02
title: "Testing Infrastructure"
subsystem: development-tooling
tags: [testing, cargo-nextest, mockall, rstest, test-infrastructure]

requires:
  - phase: 01-foundation
    plan: 01
    what: Cargo project structure

provides:
  - artifact: dev-dependencies
    what: Testing libraries (mockall, rstest, pretty_assertions)
  - artifact: nextest-config
    what: Fast test execution configuration
  - capability: test-execution
    what: Developers can run tests with cargo test or nextest

affects:
  - phase: 01-foundation
    plans: [04, 05]
    why: All TDD tasks will use these testing dependencies
  - phase: 02-core-logic
    plans: all
    why: Core logic will be developed with TDD

tech-stack:
  added:
    - name: mockall
      version: "0.13"
      purpose: Trait-based mocking for test isolation
      key-features: "#[automock] macro, async trait support"
    - name: rstest
      version: "0.23"
      purpose: Test fixtures and parameterized tests
      key-features: "#[case] for test cases, fixture injection"
    - name: pretty_assertions
      version: "1.4"
      purpose: Colorful assertion diffs for test failures
      key-features: "assert_eq! with colored output"
    - name: tokio-test
      version: "0.4"
      purpose: Async testing utilities
      key-features: "Test harness for tokio code"
    - name: cargo-nextest
      version: n/a
      purpose: Fast test execution (3x faster than cargo test)
      key-features: "Parallel execution, retries, JUnit output"
  patterns: []

key-files:
  created:
    - path: .config/nextest.toml
      role: Test execution configuration
      consumers: [cargo-nextest]
  modified:
    - path: Cargo.toml
      changes: Added [dev-dependencies] section
    - path: src/lib.rs
      changes: Added test module with placeholder tests
    - path: src/ui/mod.rs
      changes: Added missing UI constants (bug fix)

decisions:
  - id: test-framework
    what: Use cargo-nextest as primary test runner
    why: 3x faster than cargo test, better CI integration with retries and JUnit output
    alternatives: cargo test (standard but slower)
    impact: Developers get faster test feedback loop

  - id: mocking-library
    what: Use mockall for trait-based mocking
    why: Mature library with #[automock] macro support and async trait compatibility
    alternatives: mockito (HTTP mocking), manually-written mocks
    impact: Can test modules in isolation without full dependency graph

  - id: parameterized-tests
    what: Use rstest for test fixtures and parameterization
    why: Cleaner syntax than test loops, better test discovery
    alternatives: test loops, proptest (property testing)
    impact: Can write table-driven tests efficiently

metrics:
  tasks: 3
  commits: 4
  files_created: 1
  files_modified: 3
  duration: "4 minutes"
  completed: 2026-01-22
---

# Phase 1 Plan 2: Testing Infrastructure Summary

**One-liner:** Added mockall, rstest, pretty_assertions test dependencies with cargo-nextest configuration for fast parallel test execution

## What Was Built

This plan established the testing infrastructure that enables test-driven development in subsequent phases:

### Testing Dependencies
- **mockall 0.13**: Trait-based mocking with `#[automock]` macro for test isolation
- **rstest 0.23**: Fixtures and parameterized tests for cleaner test code
- **pretty_assertions 1.4**: Colorful assertion diffs for better test failure readability
- **tokio-test 0.4**: Async testing utilities (required by mockall for async traits)

### Test Execution Configuration
- **nextest default profile**: Fast local feedback (fail-fast, no retries, 60s timeout)
- **nextest ci profile**: Thorough CI testing (2 retries, JUnit XML output, 120s timeout)
- **Integration test overrides**: Extended timeout for integration tests

### Verification Tests
- Basic test infrastructure verification
- rstest parameterized test example (2 test cases)
- pretty_assertions import verification

All tests pass with both `cargo test --lib` and `cargo nextest run --lib`.

## Deviations from Plan

### Auto-fixed Issues

**1. [Rule 1 - Bug] Fixed missing UI constants**
- **Found during:** Task 1 verification (cargo check)
- **Issue:** `src/ui/mod.rs` referenced `MAX_RUNNER_EVENTS_PER_FRAME` and `FRAME_DURATION` constants that were not defined
- **Fix:** Added missing constants:
  - `MAX_RUNNER_EVENTS_PER_FRAME = 50` (prevents UI starvation during high output)
  - `FRAME_DURATION = Duration::from_millis(16)` (~60fps refresh rate)
- **Files modified:** `src/ui/mod.rs`
- **Commit:** 41c4f58
- **Why it's a bug:** Code wouldn't compile without these constants. The event loop explicitly references them for performance tuning.

## Technical Decisions

### Why cargo-nextest?
Standard `cargo test` runs tests sequentially by default. Nextest provides:
- **3x faster execution** through intelligent parallelization
- **Per-test timeout** detection for hanging tests
- **CI integration** with retries and JUnit XML output
- **Better flake handling** with configurable retries

### Why mockall?
Trait-based mocking enables:
- **Isolated unit tests** - Test a module without its real dependencies
- **Clean API** - `#[automock]` macro generates mocks automatically
- **Async support** - Works with async traits (critical for this async-heavy codebase)

### Why rstest?
Parameterized tests reduce boilerplate:
```rust
// Before (manual parameterization)
#[test] fn test_case_1() { assert_eq!(add(2, 2), 4); }
#[test] fn test_case_2() { assert_eq!(add(3, 3), 6); }

// After (rstest)
#[rstest]
#[case(2, 2, 4)]
#[case(3, 3, 6)]
fn test_add(#[case] a: i32, #[case] b: i32, #[case] expected: i32) {
    assert_eq!(add(a, b), expected);
}
```

Each `#[case]` becomes a separate test in the test runner.

## Next Phase Readiness

### Blockers
None - infrastructure is ready for TDD.

### Concerns
None - all dependencies resolved correctly.

### Recommendations
1. **Install cargo-nextest locally** for developers:
   ```bash
   cargo install cargo-nextest
   ```
2. **CI pipeline** (plan 01-03) should use nextest ci profile:
   ```bash
   cargo nextest run --profile ci
   ```

## Impact Analysis

### Immediate Impact
- **Developers can write tests** with mocking, fixtures, and parameterization
- **Fast test feedback** via nextest parallel execution
- **CI pipeline ready** with JUnit output for test reporting

### Enables Future Work
- **Plan 01-04 (config module)**: Can use TDD with mockall for YAML parsing
- **Plan 01-05 (git module)**: Can use rstest for git command test cases
- **All Phase 2 plans**: Core logic developed via TDD with full test infrastructure

### Architecture Changes
None - purely additive. Existing tests continue to work with standard cargo test.

## Verification

### Success Criteria Met
- [x] Cargo.toml has [dev-dependencies] with mockall, rstest, pretty_assertions, tokio-test
- [x] .config/nextest.toml exists with default and ci profiles
- [x] JUnit output configured in ci profile
- [x] Placeholder tests in src/lib.rs pass with cargo test
- [x] Placeholder tests pass with cargo nextest run (rstest + nextest integration verified)
- [x] cargo check passes

### Test Results
```bash
$ cargo test --lib
test result: ok. 63 passed; 0 failed; 0 ignored; 0 measured; 0 filtered out

$ cargo test --lib -- --list | grep -E "test_infrastructure|test_parameterized"
tests::test_infrastructure_works: test
tests::test_parameterized::case_1: test
tests::test_parameterized::case_2: test
```

rstest parameterization working correctly: 1 basic test + 2 parameterized cases = 3 tests total.

## Commits

| Commit  | Type  | Description                                     |
| ------- | ----- | ----------------------------------------------- |
| eeda6a9 | chore | Add test dependencies to Cargo.toml             |
| 41c4f58 | fix   | Add missing UI constants                        |
| 36e8c02 | chore | Create nextest configuration                    |
| 8eb4ef6 | test  | Add placeholder tests to verify infrastructure  |

## Files Modified

**Created:**
- `.config/nextest.toml` (658 bytes) - Test execution profiles

**Modified:**
- `Cargo.toml` - Added [dev-dependencies] section (9 lines)
- `src/lib.rs` - Added test module with placeholder tests (20 lines)
- `src/ui/mod.rs` - Added missing constants (2 lines)

## Learning Notes

### For AI Agents
- **Bug fixes during setup are normal**: Pre-existing compilation errors should be fixed immediately (Rule 1)
- **Test infrastructure must be verified**: Don't just add dependencies - write and run tests to prove they work
- **nextest requires installation**: It's a cargo subcommand, not a library dependency
- **Docker builds work without local cargo**: Use `make check`, `make test` for Docker-based verification

### For Developers
- **Use nextest for local development**: Significantly faster than cargo test
- **Use pretty_assertions by default**: Replace `assert_eq!` imports with `pretty_assertions::assert_eq!`
- **Use rstest for table-driven tests**: Cleaner than manual test loops
- **Use mockall sparingly**: Only mock external dependencies, prefer real implementations for internal modules
