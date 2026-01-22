# CI-TUI Code Quality Improvement

## What This Is

A code quality, DX, and reliability improvement pass on CI-TUI — a terminal UI application for running CI checks on changed files. The codebase was AI-generated and needs cleanup to ensure maintainability, testability, and correct behavior before further feature development.

## Core Value

Developers can confidently modify any module without fear of breaking things, and the TUI remains responsive during check execution.

## Requirements

### Validated

<!-- Existing working functionality inferred from codebase -->

- ✓ Detect git changes against base branch — existing
- ✓ Match changed files against configurable file patterns — existing
- ✓ Discover related tests via path mapping and grep search — existing
- ✓ Execute checks in Docker containers via docker compose exec — existing
- ✓ Stream check output in real-time — existing
- ✓ Group checks with sequential/parallel execution — existing
- ✓ TUI mode with ratatui rendering — existing
- ✓ Simple console mode for CI pipelines — existing
- ✓ On-demand checks triggered manually — existing
- ✓ YAML configuration with preserved key ordering — existing

### Active

<!-- Current scope: code quality and DX improvements -->

- [ ] Fix TUI responsiveness during check execution (keyboard shortcuts work without delay)
- [ ] Remove unused code and dead imports
- [ ] Apply idiomatic Rust patterns consistently
- [ ] Follow ratatui/TUI application conventions
- [ ] Improve code readability and module organization
- [ ] Add test coverage for core logic (checks, test_discovery, config)
- [ ] Make modules testable in isolation (reduce coupling)
- [ ] Document non-obvious code paths

### Out of Scope

- Copy/paste and text selection — deferred, focus on stability first
- New features — this is a cleanup milestone
- Performance optimization beyond responsiveness fix — premature
- Refactoring working UI rendering — if it works, leave it

## Context

**Codebase origin:** AI-generated code, likely has characteristic issues (over-abstraction, inconsistent patterns, dead code, missing idioms).

**Known bug:** TUI becomes unresponsive during CI check execution. Keyboard shortcuts don't work or have significant delay. Architecture docs claim there's a dedicated OS thread for keyboard input, but it's not working as intended.

**Architecture:** Event-driven with mpsc channels between runner and UI. Tokio async runtime for Docker execution. Crossterm for terminal handling.

**Stack:** Rust 2021, ratatui 0.30, crossterm 0.28, tokio, clap 4, serde/serde_yaml, anyhow/thiserror.

**Testing:** No test coverage currently. Core logic in checks.rs, test_discovery.rs, config.rs should be testable.

## Constraints

- **Tech stack**: Rust, ratatui, crossterm — no framework changes
- **Compatibility**: Must continue working with existing YAML config format
- **Docker**: Execution model via docker compose exec must be preserved

## Key Decisions

<!-- Decisions made during project initialization -->

| Decision | Rationale | Outcome |
|----------|-----------|---------|
| Focus on DX before features | AI-generated code needs audit before investing more | — Pending |
| Skip copy/paste for now | Responsiveness is more critical | — Pending |
| Prioritize testability | Enables confident future changes | — Pending |

---
*Last updated: 2026-01-22 after initialization*
