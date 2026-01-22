---
phase: 01-foundation
plan: 03
subsystem: ci-infrastructure
tags: [github-actions, ci-cd, testing, coverage, automation]
requires: [rust-toolchain]
provides: [ci-pipeline, coverage-reports, quality-gates]
affects: [02-test-infrastructure, 03-test-coverage]
tech-stack:
  added: [cargo-nextest, cargo-llvm-cov, github-actions]
  patterns: [ci-cd-pipeline, test-automation, coverage-reporting]
key-files:
  created: [.github/workflows/ci.yml]
  modified: [Makefile]
decisions:
  - id: use-nextest
    choice: cargo-nextest for test execution
    rationale: Faster test execution and better CI integration than standard cargo test
  - id: parallel-jobs
    choice: Three independent CI jobs (test, lint, coverage)
    rationale: Faster feedback by running jobs in parallel
  - id: codecov-optional
    choice: fail_ci_if_error false for codecov
    rationale: Allow CI to pass even if codecov is not configured
metrics:
  tasks: 2
  commits: 2
  duration: 1m
  completed: 2026-01-22
---

# Phase 1 Plan 3: CI Pipeline Summary

**One-liner:** GitHub Actions CI with cargo-nextest tests, clippy linting, and cargo-llvm-cov coverage reporting

## What Was Built

Created a complete CI pipeline that automatically validates code quality on every push and pull request:

1. **GitHub Actions workflow** (.github/workflows/ci.yml) with three parallel jobs:
   - **Test job**: Runs tests with cargo-nextest (faster than cargo test) plus doctests
   - **Lint job**: Validates formatting (cargo fmt --check) and runs clippy with warnings-as-errors
   - **Coverage job**: Generates coverage reports with cargo-llvm-cov and uploads to Codecov

2. **Local CI commands** added to Makefile:
   - `make ci`: Runs fmt-check, clippy, and test-local (mirrors CI locally)
   - `make test-local`: Quick local test run with nextest
   - `make test-ci`: Test with CI profile
   - `make coverage`: Generate HTML coverage report and open in browser
   - `make coverage-lcov`: Generate LCOV report for CI

**Key design decisions:**
- Cargo caching to speed up subsequent CI runs
- Separate doctest execution (nextest doesn't run doctests)
- Clippy with -D warnings to catch code quality issues early
- Optional Codecov upload (doesn't fail CI if not configured)

## Tasks Completed

| Task | Description | Commit | Files |
|------|-------------|--------|-------|
| 1 | Create GitHub Actions CI workflow | c87b548 | .github/workflows/ci.yml |
| 2 | Add local CI commands to Makefile | dfe0fe5 | Makefile |

## Deviations from Plan

None - plan executed exactly as written.

## Decisions Made

**Decision 1: Use cargo-nextest for test execution**
- **Context:** Standard cargo test works but nextest offers better CI integration
- **Choice:** Use cargo-nextest via taiki-e/install-action
- **Rationale:** Faster test execution, better output formatting, reliable CI installation
- **Impact:** Requires separate doctest run (nextest doesn't support doctests)

**Decision 2: Three parallel CI jobs**
- **Context:** Could run all checks sequentially in one job
- **Choice:** Split into test, lint, coverage jobs that run in parallel
- **Rationale:** Faster feedback (jobs run concurrently), easier to identify which check failed
- **Impact:** Slightly higher GitHub Actions compute usage

**Decision 3: Make Codecov optional**
- **Context:** Codecov requires configuration and account setup
- **Choice:** Set fail_ci_if_error: false on codecov upload
- **Rationale:** Allow CI to pass even if Codecov is not configured, unblock immediate PR workflows
- **Impact:** Coverage upload failures don't block CI

## Technical Implementation

### CI Workflow Structure

```yaml
jobs:
  test:     # Runs tests with nextest + doctests
  lint:     # Checks formatting and runs clippy -D warnings
  coverage: # Generates coverage and uploads to Codecov
```

All three jobs run in parallel on ubuntu-latest with Rust stable.

### Caching Strategy

Each job caches:
- `~/.cargo/registry` - Downloaded crate registry
- `~/.cargo/git` - Git dependencies
- `target/` - Compiled dependencies

Cache key uses `hashFiles('**/Cargo.lock')` to invalidate when dependencies change.

### Tool Installation

Uses `taiki-e/install-action` for reliable installation of:
- cargo-nextest (test and coverage jobs)
- cargo-llvm-cov (coverage job only)

This is more reliable than cargo install in CI.

### Local Development Workflow

Developers can run `make ci` before pushing to catch issues:

```bash
make ci           # Full CI check suite
make fmt-check    # Just formatting
make clippy       # Just linting
make test-local   # Just tests
make coverage     # Generate and view coverage
```

## Verification Results

All verification checks passed:

- ✓ .github/workflows/ci.yml exists and is valid YAML
- ✓ CI triggers on push and PR to master/main branches
- ✓ Three jobs defined: test, lint, coverage
- ✓ cargo-nextest used in test and coverage jobs
- ✓ cargo-llvm-cov used in coverage job
- ✓ Clippy runs with -D warnings
- ✓ Makefile has ci, test-local, coverage targets
- ✓ `make --dry-run ci` shows expected commands

## Integration Points

**With Phase 2 (Test Infrastructure):**
- CI will run the unit tests created in Phase 2
- Clippy warnings will be addressed when cleaning up code
- Coverage reports will show baseline coverage

**With Phase 3 (Test Coverage):**
- Coverage job will track improvements as Phase 3 adds tests
- Integration tests added in Phase 3 will run in CI

**With Phase 4 (Refactoring):**
- CI provides safety net for refactoring
- Ensures no regressions during code cleanup

## Next Phase Readiness

**Ready for Phase 2:**
- CI pipeline ready to validate new tests
- Local `make ci` workflow established
- Coverage tracking in place

**No blockers identified.**

**Notes:**
- Clippy will likely fail initially (existing code may have warnings)
- This is expected and will be addressed in Phase 2
- For now, the workflow is defined but may need clippy disabled temporarily

## Files Modified

### Created
- `.github/workflows/ci.yml` - GitHub Actions CI workflow with test/lint/coverage jobs

### Modified
- `Makefile` - Added local CI commands (ci, test-local, test-ci, fmt-check, coverage, coverage-lcov)

## Commands Reference

**CI workflow triggers:**
- Push to master/main
- Pull request to master/main

**Local commands:**
```bash
make ci              # Run all CI checks locally
make test-local      # Run tests with nextest
make test-ci         # Run tests with CI profile
make fmt-check       # Check formatting
make coverage        # Generate HTML coverage report
make coverage-lcov   # Generate LCOV report
```

## Success Metrics

- ✅ 2/2 tasks completed
- ✅ 2 atomic commits created
- ✅ All verification checks passed
- ✅ CI workflow ready for immediate use
- ✅ Local development workflow established
