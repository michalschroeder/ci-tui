# CI-TUI Code Quality Improvement

## What This Is

A terminal UI application for running CI checks on changed files, now with <1ms keyboard responsiveness, comprehensive testing infrastructure, idiomatic Rust patterns, and a clean architecture. The AI-generated codebase has been thoroughly cleaned up with test fixtures, deduplicated utilities, and refactored complex functions, validated with 360 tests at 65% function coverage.

## Core Value

Developers can confidently modify any module without fear of breaking things, and the TUI remains responsive during check execution.

## Requirements

### Validated

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
- ✓ Fix TUI responsiveness during check execution — v1.0 (<1ms response)
- ✓ Remove unused code and dead imports — v1.0
- ✓ Apply idiomatic Rust patterns consistently — v1.0
- ✓ Follow ratatui/TUI application conventions — v1.0
- ✓ Improve code readability and module organization — v1.0
- ✓ Add test coverage for core logic — v1.0 (65% coverage)
- ✓ Make modules testable in isolation — v1.0 (GitExecutor, CommandExecutor traits)
- ✓ Document non-obvious code paths — v1.0 (module docs)
- ✓ Consolidate test infrastructure — v2.0 (ConfigBuilder, CheckBuilder, 5 fixtures)
- ✓ Eliminate code duplication — v2.0 (utils module, 86 lines removed)
- ✓ Add unit tests to untested modules — v2.0 (54 new tests for runner, simple, fix)
- ✓ Refactor large functions — v2.0 (determine_checks: 155→46 lines)
- ✓ Enable complexity lints — v2.0 (too_many_lines: 100, excessive_nesting: 3)

### Active

(None — define in next milestone)

### Out of Scope

- Copy/paste and text selection — deferred, focus on stability first
- New features — v1.0 and v2.0 were cleanup milestones
- Performance optimization beyond responsiveness fix — premature
- Refactoring working UI rendering — if it works, leave it
- Module splitting (UI module) — research shows large modules maintain strong cohesion

## Context

**Current state (v2.0):** 12,603 lines of Rust code, 360 tests passing, 65.34% function coverage.

**Tech stack:** Rust 2021, ratatui 0.30, crossterm 0.28, tokio, clap 4, serde/serde_yaml, anyhow/thiserror.

**Testing infrastructure:** cargo-nextest, mockall, rstest, pretty_assertions, cargo-llvm-cov.

**Test fixtures:** ConfigBuilder, CheckBuilder in tests/common/configs.rs, 5 base fixtures.

**Utilities:** utils::time (format, format_from_duration), utils::docker (is_running).

**CI pipeline:** GitHub Actions with test, lint, coverage reporting.

**Core business logic coverage:** config (95%), checks (94%), test_discovery (99%), git (89%), ui/app (82%).

## Constraints

- **Tech stack**: Rust, ratatui, crossterm — no framework changes
- **Compatibility**: Must continue working with existing YAML config format
- **Docker**: Execution model via docker exec/run must be preserved
- **Test architecture**: Library tests use inline fixtures (cargo fmt Docker limitation)

## Key Decisions

| Decision | Rationale | Outcome |
|----------|-----------|---------|
| Focus on DX before features | AI-generated code needs audit before investing more | ✓ Good — solid foundation |
| Skip copy/paste for now | Responsiveness is more critical | ✓ Good — can revisit later |
| Prioritize testability | Enables confident future changes | ✓ Good — 360 tests, 65% coverage |
| tokio::select! with biased; | Compile-time priority for keyboard events | ✓ Good — <1ms response |
| Keep std::thread for keyboard | OS scheduler beats Tokio under CPU load | ✓ Good — reliable input |
| Clippy deny level | Zero tolerance for lint violations | ✓ Good — clean codebase |
| GitExecutor/CommandExecutor traits | Mock-based testing without real services | ✓ Good — fast CI |
| TEA-lite state management | Centralized App::update() for predictable state | ✓ Good — testable state |
| rstest for parameterized tests | Cleaner than test loops, comprehensive coverage | ✓ Good — 8+ cases per test |
| Widget tests with TestBackend | Terminal rendering verification without TUI | ✓ Good — 31 widget tests |
| ConfigBuilder fluent API | Chainable methods more ergonomic than setters | ✓ Good — <10 line test setup |
| utils module pub(crate) | Keep utilities internal, not public API | ✓ Good — clean separation |
| Characterization tests before refactoring | Safety net for determine_checks() changes | ✓ Good — 18 tests caught 0 regressions |
| Function extraction pattern | Domain-term naming (process_, match_, build_) | ✓ Good — self-documenting helpers |
| Inline fixtures in lib tests | cargo fmt Docker limitation prevents #[path] imports | — Tradeoff — documented pattern |
| Accept 78-line orchestrator | process_triggered_check passes Clippy 100-line threshold | — Tradeoff — well-structured |

---
*Last updated: 2026-01-31 after v2.0 milestone*
