# Roadmap: CI-TUI v2.0 Comprehensive Cleanup

## Milestones

- ✅ **v1.0 Code Quality** - Phases 1-5 (shipped 2026-01-25)
- 🚧 **v2.0 Comprehensive Cleanup** - Phases 6-9 (in progress)

## Phases

<details>
<summary>✅ v1.0 Code Quality (Phases 1-5) - SHIPPED 2026-01-25</summary>

### Phase 1: Foundation & Test Infrastructure
**Goal**: Establish testability patterns for core logic modules
**Plans**: 3 plans
**Status**: Complete

Plans:
- [x] 01-01: Extract traits and add unit tests to config.rs
- [x] 01-02: Add unit tests to checks.rs
- [x] 01-03: Add unit tests to test_discovery.rs

### Phase 2: Git & App Testing
**Goal**: Achieve comprehensive test coverage for git operations and UI state management
**Plans**: 2 plans
**Status**: Complete

Plans:
- [x] 02-01: Add unit tests to git.rs
- [x] 02-02: Add unit tests to ui/app.rs

### Phase 3: Code Quality Improvements
**Goal**: Apply idiomatic Rust patterns and remove dead code
**Plans**: 4 plans
**Status**: Complete

Plans:
- [x] 03-01: Fix Clippy warnings across codebase
- [x] 03-02: Remove dead code and unused imports
- [x] 03-03: Apply idiomatic patterns (iterator chains, error handling)
- [x] 03-04: Document non-obvious code paths

### Phase 4: TUI Responsiveness Fix
**Goal**: Achieve consistent sub-millisecond keyboard response during check execution
**Plans**: 2 plans
**Status**: Complete

Plans:
- [x] 04-01: Profile keyboard input latency under CPU load
- [x] 04-02: Refactor event loop for prioritized input processing

### Phase 5: Widget Testing & Documentation
**Goal**: Verify TUI rendering behavior and document architectural patterns
**Plans**: 4 plans
**Status**: Complete

Plans:
- [x] 05-01: Add widget tests for UI rendering
- [x] 05-02: Document TUI architecture patterns
- [x] 05-03: Add integration tests for end-to-end flows
- [x] 05-04: Update CLAUDE.md with validation workflow

</details>

### 🚧 v2.0 Comprehensive Cleanup (In Progress)

**Milestone Goal:** Eliminate technical debt from test infrastructure, code duplication, and function complexity to enable confident future modifications.

#### Phase 6: Test Infrastructure Consolidation

**Goal**: Replace 37 inline YAML test configs with shared fixtures and builders.

**Depends on**: Phase 5

**Requirements**:
- TEST-01: Extract all 37 inline YAML test configs to shared fixtures in tests/common/configs.rs
- TEST-02: Create builder pattern for programmatic config generation
- TEST-03: Document test fixture usage patterns
- TEST-04: Migrate config.rs tests (27 configs) to use shared fixtures
- TEST-05: Migrate checks.rs tests (9 configs) to use shared fixtures
- TEST-06: Migrate remaining test files to use shared fixtures

**Success Criteria** (what must be TRUE):
1. All inline YAML configs from src/ modules are extracted to tests/common/configs.rs
2. New tests require less than 10 lines of config setup (vs current 40+ lines)
3. All 250 existing tests pass with no behavior changes
4. Test modules have reduced line counts due to fixture consolidation
5. Builder pattern enables composable config creation with sensible defaults

**Plans**: 3 plans

Plans:
- [ ] 06-01-PLAN.md — Create fixture infrastructure (ConfigBuilder, CheckBuilder, base fixtures, docs)
- [ ] 06-02-PLAN.md — Migrate config.rs tests (27 configs)
- [ ] 06-03-PLAN.md — Migrate checks.rs and tests/common tests (10 configs)

#### Phase 7: Code Deduplication

**Goal**: Consolidate duplicate utility functions into shared modules.

**Depends on**: Phase 6

**Requirements**:
- DEDUP-01: Consolidate format_duration() implementations (3 copies → 1 in runner.rs)
- DEDUP-02: Consolidate format_duration_ms() implementations (2 copies → 1)
- DEDUP-03: Consolidate is_container_running() implementations (3 copies → 1)
- DEDUP-04: Create src/utils/ module for shared utilities
- DEDUP-05: Re-export consolidated utilities from lib.rs for crate-wide access

**Success Criteria** (what must be TRUE):
1. Only one implementation of format_duration() exists across codebase
2. Only one implementation of format_duration_ms() exists across codebase
3. Only one implementation of is_container_running() exists across codebase
4. src/utils/ module provides crate-wide access to shared utilities
5. All existing tests pass after consolidation with no behavior changes

**Plans**: TBD

Plans:
- [ ] 07-01: TBD during plan-phase

#### Phase 8: Complexity Reduction

**Goal**: Enable Clippy complexity gates and refactor functions exceeding thresholds.

**Depends on**: Phase 7

**Requirements**:
- CMPLX-01: Enable Clippy too_many_lines lint (threshold: 100 lines)
- CMPLX-02: Enable Clippy excessive_nesting lint
- CMPLX-03: Add characterization tests for determine_checks() before refactoring
- CMPLX-04: Split determine_checks() (155 lines) into focused functions (<50 lines each)
- CMPLX-05: Ensure all functions pass new lint thresholds

**Success Criteria** (what must be TRUE):
1. Clippy lints too_many_lines and excessive_nesting are enabled in Cargo.toml
2. Characterization tests document current determine_checks() behavior before refactoring
3. determine_checks() is split into focused functions under 50 lines each
4. All functions pass Clippy complexity lints with no violations
5. All 250+ existing tests pass after refactoring with identical behavior

**Plans**: TBD

Plans:
- [ ] 08-01: TBD during plan-phase

#### Phase 9: Test Coverage Expansion

**Goal**: Add unit tests to untested modules and maintain 65%+ overall coverage.

**Depends on**: Phase 8

**Requirements**:
- COV-01: Add unit tests to runner.rs (730 lines, currently no inline tests)
- COV-02: Add unit tests to simple.rs (382 lines, currently no inline tests)
- COV-03: Add unit tests to fix.rs (281 lines, currently no inline tests)
- COV-04: Maintain or improve overall coverage (currently 65%)

**Success Criteria** (what must be TRUE):
1. runner.rs has inline unit tests for core functions (CheckRunner, event handling)
2. simple.rs has inline unit tests for console output formatting
3. fix.rs has inline unit tests for fix mode logic
4. Overall test coverage is at 65% or higher
5. All new tests use shared fixtures from Phase 6

**Plans**: TBD

Plans:
- [ ] 09-01: TBD during plan-phase

## Progress

**Execution Order:**
Phases execute in numeric order: 6 → 7 → 8 → 9

| Phase | Milestone | Plans Complete | Status | Completed |
|-------|-----------|----------------|--------|-----------|
| 1. Foundation & Test Infrastructure | v1.0 | 3/3 | Complete | 2026-01-25 |
| 2. Git & App Testing | v1.0 | 2/2 | Complete | 2026-01-25 |
| 3. Code Quality Improvements | v1.0 | 4/4 | Complete | 2026-01-25 |
| 4. TUI Responsiveness Fix | v1.0 | 2/2 | Complete | 2026-01-25 |
| 5. Widget Testing & Documentation | v1.0 | 4/4 | Complete | 2026-01-25 |
| 6. Test Infrastructure Consolidation | v2.0 | 0/3 | Planned | - |
| 7. Code Deduplication | v2.0 | 0/TBD | Not started | - |
| 8. Complexity Reduction | v2.0 | 0/TBD | Not started | - |
| 9. Test Coverage Expansion | v2.0 | 0/TBD | Not started | - |

---
*Roadmap created: 2026-01-29 for v2.0 milestone*
*Last updated: 2026-01-29 after Phase 6 planning*
