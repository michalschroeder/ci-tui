---
type: quick-task
id: 001
subsystem: infra
tags: [makefile, cargo, docker, ci]

key-files:
  modified: [Makefile]

key-decisions:
  - "Remove Docker-based cargo execution (slow, inconsistent with CI)"
  - "Use cargo-nextest as primary test runner (3x faster)"
  - "Docker targets only for CI-TUI image management"

duration: <1min
completed: 2026-01-23
---

# Quick Task 001: Refactor Makefile for Consistent Cargo Approach

**Makefile now uses local cargo for all development/CI tasks with clear organization; Docker reserved for image management only**

## Performance

- **Duration:** <1 min (52 seconds)
- **Started:** 2026-01-23T01:54:10Z
- **Completed:** 2026-01-23T01:55:02Z
- **Tasks:** 2
- **Files modified:** 1

## Accomplishments
- Removed redundant Docker-based cargo targets (check, fmt, clippy, test)
- Established cargo-nextest as primary test runner via `make test`
- Organized Makefile into clear sections (Development, CI, Docker Image)
- Removed monorepo remnants (PROJECT_ROOT, CONFIG_FILE variables, dev target)

## Task Commits

Each task was committed atomically:

1. **Task 1: Reorganize Makefile with local cargo approach** - `e406114` (refactor)

_Note: Task 2 was verification only, no code changes_

## Files Created/Modified
- `Makefile` - Complete refactor with local cargo execution and clear section organization

## Decisions Made

**1. Remove all Docker-based cargo execution**
- **Rationale:** Docker-based targets were slow (clippy reinstalled component each run), inconsistent with CI pipeline (which uses local cargo), and confusing (mixed approaches)
- **Impact:** Developers use same tooling locally as CI uses

**2. Make cargo-nextest the primary test runner**
- **Rationale:** Phase 1 established cargo-nextest as 3x faster than cargo test
- **Implementation:** `test-local` → `test` (standard name), kept `test-ci` for CI profile
- **Impact:** `make test` now runs nextest by default

**3. Docker targets only for CI-TUI image management**
- **Rationale:** Docker is for building/deploying the CI-TUI application itself, not for running cargo
- **Kept targets:** build, build-no-cache, run, run-local, push, pull, clean
- **Impact:** Clear separation of concerns

**4. Remove monorepo patterns**
- **Rationale:** PROJECT_ROOT navigation assumed this was in `tools/ci/tui/` subdirectory of larger repo
- **Removed:** PROJECT_ROOT variable, CONFIG_FILE variable, dev target
- **Impact:** Makefile works as standalone project (which it is)

## Deviations from Plan

None - plan executed exactly as written.

## Issues Encountered

None - straightforward refactor with clear requirements.

## Changes Summary

**Removed:**
- Docker-based cargo targets: `check`, `fmt` (Docker version), `clippy` (Docker version), `test` (Docker version)
- Variables: `PROJECT_ROOT`, `CONFIG_FILE`
- Target: `dev` (monorepo pattern)

**Added:**
- Local `fmt` target: `cargo fmt`
- Local `clippy` target: `cargo clippy -- -D warnings`

**Renamed:**
- `test-local` → `test` (primary test target)

**Updated:**
- `ci` target: Now runs `fmt-check`, `clippy`, `test` (all local)
- `run-local`: Uses `PWD` instead of `PROJECT_ROOT`

**Organized:**
- Section comments: Development, CI, Docker Image
- All targets properly categorized

## Verification Performed

✅ `make help` displays organized, sorted target list
✅ `make fmt-check` executes `cargo fmt -- --check`
✅ `make clippy` executes `cargo clippy -- -D warnings`
✅ `make test` executes `cargo nextest run`
✅ `make ci` runs fmt-check, clippy, test in sequence
✅ No Docker-based cargo commands remain (grep verified)
✅ Docker image management targets unchanged

## Next Phase Readiness

Makefile is now consistent with Phase 1 tooling decisions and CI pipeline. Ready for Phase 2 code quality work with clean, fast local development workflow.

---
*Type: quick-task*
*Completed: 2026-01-23*
