# Project State

## Project Reference

See: .planning/PROJECT.md (updated 2026-01-22)

**Core value:** Developers can confidently modify any module without fear of breaking things, and the TUI remains responsive during check execution.

**Current focus:** Phase 4: Mock-Based Tests

## Current Position

Phase: 4 of 5 (Mock-Based Tests)
Plan: 1 of TBD in current phase
Status: In progress
Last activity: 2026-01-24 — Completed 04-01-PLAN.md (GitExecutor trait with mock tests)

Progress: [██████░░░░] 60%

## Performance Metrics

**Velocity:**
- Total plans completed: 26
- Average duration: 4.4m
- Total execution time: 1.96 hours

**By Phase:**

| Phase | Plans | Total | Avg/Plan |
|-------|-------|-------|----------|
| 01-foundation | 3 | 12.5m | 4.2m |
| 02-code-quality-baseline | 2 | 10m | 5m |
| 03-unit-test-coverage | 3 | 11m | 3.7m |
| 04-mock-based-tests | 1 | 11m | 11m |

**Recent Trend:**
- Last 5 plans: 04-01 (11m), 03-03 (3m), 03-02 (2.5m), 03-01 (5.5m), 02-02 (8m)
- Trend: Mock infrastructure setup takes longer than simple unit tests

*Updated after each plan completion*

## Accumulated Context

### Decisions

Decisions are logged in PROJECT.md Key Decisions table.
Recent decisions affecting current work:

- Initialization: Focus on DX before features (AI-generated code needs audit)
- Initialization: Skip copy/paste for now (responsiveness is more critical)
- Initialization: Prioritize testability (enables confident future changes)
- 01-01: tokio::select! with biased; instead of async priority channels (compile-time priority)
- 01-01: Keep std::thread for keyboard (OS scheduler beats Tokio under CPU load)
- 01-01: handle_message with &mut App for explicit state mutation (no interior mutability)
- 01-03: Use cargo-nextest for CI test execution (faster, better CI integration)
- 01-03: Three parallel CI jobs (test, lint, coverage) for faster feedback
- 01-03: Make Codecov optional (fail_ci_if_error: false) to unblock CI usage
- 01-02: Use cargo-nextest as primary test runner (3x faster than cargo test)
- 01-02: Use mockall for trait-based mocking (enables isolated unit tests)
- 01-02: Use rstest for parameterized tests (cleaner than test loops)
- quick-001: Add .dockerignore to exclude build artifacts (build speed over image size)
- quick-007: Reuse show_full_command flag for both command and files expansion ('e' hotkey)
- quick-008: Two-phase self-hosting strategy (compose now, run mode later)
- quick-009: Use docker exec when container running, docker run when not (eliminates docker-compose dependency)
- quick-009: Derive container name from project_dir + service by default (backward compatibility)
- quick-011: Add image/volume_mount/work_dir as optional fields (maintains backward compatibility)
- quick-011: Derive image name from container_name if not explicit (preserves docker-compose behavior)
- quick-012: Track output_visible_lines in App state and update during render (dynamic scroll bounds)
- quick-012: Show scroll position indicator only when content exceeds visible area (avoid clutter)
- quick-016: Auto-skip checks with {files} placeholder when no matches (prevents accidental full-codebase runs)
- quick-016: Keep skipped checks visible with on-demand trigger capability (user awareness + manual override)
- quick-020: Single comprehensive reference file over multiple small files (easier to search/browse)
- 02-01: Use default Clippy lints only, no pedantic (balanced strictness)
- 02-01: All warnings denied, no allow attributes (zero tolerance for lint violations)
- 02-02: Use let-else guard clauses for safe Option handling (cleaner than match)
- 02-02: Allow unwrap on regex literals and thread spawn (guaranteed cases)
- 02-02: Add //! module docs with Key Types/Functions sections (discoverability)
- 03-01: Use nested test modules to organize tests by function (improved organization)
- 03-01: Import pretty_assertions in each nested module (avoids ambiguity with use super::*)
- 04-01: Use unconditional #[mockall::automock] with optional mockall dependency (enables integration tests)
- 04-01: Make executor-accepting functions public for testability (maintains backward compatibility)
- 04-01: Use Vec<String> in trait signatures to avoid lifetime issues (cleaner than &[&str])

### Pending Todos

None yet.

### Blockers/Concerns

None yet.

### Quick Tasks Completed

| # | Description | Date | Commit | Directory |
|---|-------------|------|--------|-----------|
| 001 | Refactor Makefile for consistent cargo approach | 2026-01-23 | b961357 | [001-refactor-makefile](./quick/001-refactor-makefile-for-consistent-cargo-a/) |
| 002 | Add version display showing git hash + build date | 2026-01-23 | dc4016f | [002-add-version-display](./quick/002-add-version-display-git-hash-and-build-d/) |
| 003 | Remove unused run.sh and Makefile targets | 2026-01-23 | d812197 | [003-review-and-remove](./quick/003-review-and-remove-unused-run-sh-and-make/) |
| 001 | Optimize Docker build context with .dockerignore | 2026-01-23 | eada4fe | [001-shrink-docker-image](./quick/001-shrink-docker-image-following-best-pract/) |
| 004 | Fix f-hotkey to only select failed items | 2026-01-23 | ac2ea17 | [004-fix-f-hotkey](./quick/004-fix-f-hotkey-to-only-select-failed-check/) |
| 005 | Show full datetime in build footer | 2026-01-23 | 255aeec | [005-show-full-datetime](./quick/005-show-full-datetime-in-build-footer/) |
| 006 | r-hotkey refresh git changes before retry | 2026-01-23 | c02b7fe | [006-r-hotkey-refresh](./quick/006-r-hotkey-refresh-git-changes-before-retr/) |
| 007 | Remove '+N more' from files section, use 'e' to expand | 2026-01-23 | a6a2498 | [007-remove-n-more](./quick/007-remove-n-more-from-files-section-use-e-h/) |
| 008 | Research CI-TUI self-hosting requirement | 2026-01-23 | 05aa45e | [008-research-ci-tui-self-hosting-requirement](./quick/008-research-ci-tui-self-hosting-requirement/) |
| 009 | Replace docker-compose exec with docker exec/run | 2026-01-23 | a3859bf | [009-replace-docker-compose-exec-with-docker-](./quick/009-replace-docker-compose-exec-with-docker-/) |
| 010 | Remove docker-compose from Docker image | 2026-01-23 | 76ac35b | [010-remove-docker-compose-from-docker-image](./quick/010-remove-docker-compose-from-docker-image/) |
| 011 | Create CI-TUI config for self-hosting | 2026-01-23 | e42634c | [011-create-ci-tui-config-for-self-hosting](./quick/011-create-ci-tui-config-for-self-hosting/) |
| 012 | Fix check output panel scrolling | 2026-01-23 | e59c0f4 | [012-fix-check-output-panel-scrolling](./quick/012-fix-check-output-panel-scrolling/) |
| 013 | Add config validation with schema error | 2026-01-23 | f66485a | [013-add-config-validation-with-schema-error-](./quick/013-add-config-validation-with-schema-error-/) |
| 014 | Add CI/CD pipeline with GitHub Actions | 2026-01-23 | a3be1b1 | [014-add-ci-cd-pipeline](./quick/014-add-ci-cd-pipeline-with-github-actions/) |
| 015 | Add Release Please for automated releases | 2026-01-23 | 7a5530d | [015-add-release-please](./quick/015-add-release-please-for-automated-release/) |
| 016 | Skip checks with {files} placeholder when no matches | 2026-01-23 | c469cd4 | [016-skip-checks-with-files-placeholder-when](./quick/016-skip-checks-with-files-placeholder-when/) |
| 017 | Optimize Docker builds with native ARM64 runners | 2026-01-23 | 2c9be2a | [017-optimize-docker-build-with-matrix-strat](./quick/017-optimize-docker-build-with-matrix-strat/) |
| 018 | TDD test for duplicate test file detection | 2026-01-23 | 96dca68 | [018-tdd-test-for-duplicate-test-file-detecti](./quick/018-tdd-test-for-duplicate-test-file-detecti/) |
| 019 | Add --fix parameter to run only fix commands | 2026-01-23 | c4c47f5 | [019-add-fix-parameter-to-run-only-fix-comman](./quick/019-add-fix-parameter-to-run-only-fix-comman/) |
| 020 | Add docs directory with config file documentation | 2026-01-23 | f2a276b | [020-add-docs-directory-with-config-file-docu](./quick/020-add-docs-directory-with-config-file-docu/) |
| 021 | Replace Laravel example with Symfony example | 2026-01-23 | e760c38 | [021-replace-laravel-example-with-symfony-in-](./quick/021-replace-laravel-example-with-symfony-in-/) |
| 022 | Add grep_search tests for test discovery | 2026-01-24 | 8ada5b3 | [022-add-grep-search-tests](./quick/022-add-grep-search-tests/) |
| 023 | Fix grep_search missing path argument | 2026-01-24 | b62ce7c | - |

## Session Continuity

Last session: 2026-01-24T11:46:26Z
Stopped at: Completed 04-01-PLAN.md (GitExecutor trait with mock tests)
Resume file: None
