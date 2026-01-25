# CI-TUI Code Quality Improvement

## What This Is

A terminal UI application for running CI checks on changed files, now with <1ms keyboard responsiveness, comprehensive testing infrastructure, and idiomatic Rust patterns. The AI-generated codebase has been cleaned up and validated with 250 tests at 65% coverage.

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

### Active

(None — start next milestone with `/gsd:new-milestone`)

### Out of Scope

- Copy/paste and text selection — deferred, focus on stability first
- New features — v1.0 was a cleanup milestone
- Performance optimization beyond responsiveness fix — premature
- Refactoring working UI rendering — if it works, leave it

## Context

**Current state (v1.0):** 9,945 lines of Rust code, 250 tests passing, 65% overall coverage.

**Tech stack:** Rust 2021, ratatui 0.30, crossterm 0.28, tokio, clap 4, serde/serde_yaml, anyhow/thiserror.

**Testing infrastructure:** cargo-nextest, mockall, rstest, pretty_assertions, cargo-llvm-cov.

**CI pipeline:** GitHub Actions with test, lint, coverage reporting.

**Core business logic coverage:** config (95%), checks (94%), test_discovery (99%), git (89%), ui/app (82%).

## Constraints

- **Tech stack**: Rust, ratatui, crossterm — no framework changes
- **Compatibility**: Must continue working with existing YAML config format
- **Docker**: Execution model via docker exec/run must be preserved

## Key Decisions

| Decision | Rationale | Outcome |
|----------|-----------|---------|
| Focus on DX before features | AI-generated code needs audit before investing more | ✓ Good — solid foundation |
| Skip copy/paste for now | Responsiveness is more critical | ✓ Good — can revisit in v1.1 |
| Prioritize testability | Enables confident future changes | ✓ Good — 250 tests, 65% coverage |
| tokio::select! with biased; | Compile-time priority for keyboard events | ✓ Good — <1ms response |
| Keep std::thread for keyboard | OS scheduler beats Tokio under CPU load | ✓ Good — reliable input |
| Clippy deny level | Zero tolerance for lint violations | ✓ Good — clean codebase |
| GitExecutor/CommandExecutor traits | Mock-based testing without real services | ✓ Good — fast CI |
| TEA-lite state management | Centralized App::update() for predictable state | ✓ Good — testable state |
| rstest for parameterized tests | Cleaner than test loops, comprehensive coverage | ✓ Good — 8+ cases per test |
| Widget tests with TestBackend | Terminal rendering verification without TUI | ✓ Good — 31 widget tests |

---
*Last updated: 2026-01-25 after v1.0 milestone*
