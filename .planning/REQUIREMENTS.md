# Requirements: CI-TUI Code Quality

**Defined:** 2026-01-22
**Core Value:** Developers can confidently modify any module without fear of breaking things, and the TUI remains responsive during check execution.

## v1 Requirements

Requirements for code quality milestone. Each maps to roadmap phases.

### Event Loop & Responsiveness

- [x] **EVNT-01**: Refactor event loop to use `tokio::select!` instead of sleep-based pattern
- [x] **EVNT-02**: Convert keyboard channel from `std::sync::mpsc` to `tokio::sync::mpsc`
- [x] **EVNT-03**: Introduce Message enum for explicit state transitions
- [x] **EVNT-04**: Keyboard input responds within 1ms during check execution (was 16ms+)

### Testing Infrastructure

- [x] **TEST-01**: Install and configure cargo-nextest as test runner
- [x] **TEST-02**: Add mockall to dev-dependencies for trait-based mocking
- [x] **TEST-03**: Add rstest for test fixtures and parameterized tests
- [x] **TEST-04**: Add pretty_assertions for better assertion output
- [x] **TEST-05**: Install cargo-llvm-cov for code coverage measurement
- [x] **TEST-06**: Configure CI pipeline with test, lint, and coverage reporting

### Code Quality

- [x] **QUAL-01**: Enable Clippy at deny level (warnings fail build)
- [x] **QUAL-02**: Remove all unused code and dead imports
- [x] **QUAL-03**: Fix all Clippy warnings across codebase
- [x] **QUAL-04**: Replace excessive `.clone()` with references or Arc where appropriate
- [x] **QUAL-05**: Replace `unwrap()` and `expect()` in production code with proper error handling
- [x] **QUAL-06**: Apply idiomatic Rust patterns (if let, iterators, proper borrowing)
- [x] **QUAL-07**: Document module boundaries and responsibilities
- [x] **QUAL-08**: Add doc comments to public API functions

### Test Coverage

- [x] **COVR-01**: Unit tests for `checks.rs` (check determination logic)
- [x] **COVR-02**: Unit tests for `test_discovery.rs` (path mapping, grep search)
- [x] **COVR-03**: Unit tests for `config.rs` (YAML parsing, pattern compilation)
- [ ] **COVR-04**: Mock-based tests for `runner.rs` (Docker execution without containers)
- [ ] **COVR-05**: Mock-based tests for `git.rs` (git operations without repo)
- [ ] **COVR-06**: Widget tests for UI components using TestBackend
- [ ] **COVR-07**: Achieve 60%+ code coverage for core modules

### Architecture

- [ ] **ARCH-01**: Centralize state update logic in single `update()` function
- [ ] **ARCH-02**: Verify panic hook exists for terminal restoration
- [ ] **ARCH-03**: Ensure dirty flag optimization preserved during refactoring

## v2 Requirements

Deferred to future milestone. Tracked but not in current roadmap.

### Advanced Testing

- **ADVT-01**: Property-based tests with proptest for pattern matching
- **ADVT-02**: Snapshot tests with insta for config parsing
- **ADVT-03**: Integration tests with ratatui-testlib for E2E flows
- **ADVT-04**: Doc tests for public API

### Security & Dependencies

- **SECD-01**: Install cargo-deny for dependency security checks
- **SECD-02**: Configure Dependabot for automated dependency updates
- **SECD-03**: Periodic cargo-geiger audits for unsafe code

### Architecture Polish

- **ARCV-01**: Command pattern for async operations (full TEA)
- **ARCV-02**: Component-based refactoring if complexity grows

## Out of Scope

Explicitly excluded. Documented to prevent scope creep.

| Feature | Reason |
|---------|--------|
| Copy/paste functionality | User deferred, focus on stability first |
| New features | This is a cleanup milestone |
| Performance optimization beyond responsiveness | Premature until responsiveness fixed |
| UI rendering refactoring | If it works, leave it |
| ratatui-testlib integration tests | Defer to v2, tool is early stage (0.1.x) |

## Traceability

Which phases cover which requirements. Updated during roadmap creation.

| Requirement | Phase | Status |
|-------------|-------|--------|
| EVNT-01 | Phase 1 | Complete |
| EVNT-02 | Phase 1 | Complete |
| EVNT-03 | Phase 1 | Complete |
| EVNT-04 | Phase 1 | Complete |
| TEST-01 | Phase 1 | Complete |
| TEST-02 | Phase 1 | Complete |
| TEST-03 | Phase 1 | Complete |
| TEST-04 | Phase 1 | Complete |
| TEST-05 | Phase 1 | Complete |
| TEST-06 | Phase 1 | Complete |
| QUAL-01 | Phase 2 | Complete |
| QUAL-02 | Phase 2 | Complete |
| QUAL-03 | Phase 2 | Complete |
| QUAL-04 | Phase 2 | Complete |
| QUAL-05 | Phase 2 | Complete |
| QUAL-06 | Phase 2 | Complete |
| QUAL-07 | Phase 2 | Complete |
| QUAL-08 | Phase 2 | Complete |
| COVR-01 | Phase 3 | Complete |
| COVR-02 | Phase 3 | Complete |
| COVR-03 | Phase 3 | Complete |
| COVR-04 | Phase 4 | Pending |
| COVR-05 | Phase 4 | Pending |
| COVR-06 | Phase 5 | Pending |
| COVR-07 | Phase 5 | Pending |
| ARCH-01 | Phase 5 | Pending |
| ARCH-02 | Phase 5 | Pending |
| ARCH-03 | Phase 5 | Pending |

**Coverage:**
- v1 requirements: 28 total
- Mapped to phases: 28
- Unmapped: 0

---
*Requirements defined: 2026-01-22*
*Last updated: 2026-01-24 after Phase 3 completion*
