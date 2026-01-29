# Project Research Summary: CI-TUI v2.0 Cleanup

**Project:** CI-TUI v2.0 Comprehensive Cleanup
**Domain:** Rust Test Infrastructure & Code Organization
**Researched:** 2026-01-29
**Confidence:** HIGH

## Executive Summary

CI-TUI has a solid foundation with 250 tests and 65% coverage, but faces technical debt in test organization and code duplication. The research reveals that the core issue is not architectural complexity but systematic duplication: 37 inline YAML test configs scattered across modules, and utility functions like `format_duration()` duplicated 3x. The solution follows established Rust testing patterns: consolidate test fixtures into `tests/common/configs.rs` using builder patterns (leveraging `derive_builder`), and extract shared utilities to dedicated modules. Module splitting is explicitly NOT recommended despite large file sizes - the codebase maintains strong cohesion and splitting would reduce maintainability.

The most critical finding is that CI-TUI's async event loop architecture has a responsiveness pitfall: the main loop uses `tokio::time::sleep(16ms)` which creates input lag under CPU load. While this isn't a v2.0 cleanup issue per se, it's documented as a foundation-level concern that should be monitored. For the cleanup milestone, focus should be on test consolidation (Phase 1) and strategic refactoring of the 155-line `determine_checks()` function (Phase 2), while explicitly avoiding unnecessary module splits.

The recommended approach is incremental: start with test fixture consolidation (high impact, low risk), then tackle function complexity with clippy lints as objective gates, and defer any "nice-to-have" improvements like advanced fixture frameworks (rstest parameterization) to post-v2.0.

## Key Findings

### Recommended Stack

**Core addition: derive_builder for test fixtures**

The existing testing stack (rstest 0.26, mockall 0.14, nextest, pretty_assertions) is already comprehensive. The one valuable addition is `derive_builder` 0.20 as a dev-dependency to eliminate boilerplate in test config builders. The current problem is 37+ inline YAML configs in test modules - these should be replaced with programmatic builders that start from sensible defaults and allow test-specific overrides.

**Key insight:** The "40 duplicate YAML configs" are inline string literals in test helpers, not separate files. The solution is builder patterns for composable config fixtures, not file consolidation.

**Patterns over dependencies:**
- Use `std::sync::OnceLock` (Rust 1.70+) for lazy shared fixtures - no external dependency needed
- Leverage rust-analyzer's built-in refactoring capabilities (extract function, rename, inline) - no specialized tools needed
- Use clippy lints (`too_many_lines`, `excessive_nesting`) as complexity gates - avoid deprecated `cognitive_complexity`

**Not recommended:**
- `once_cell` or `lazy_static` (superseded by stdlib)
- Specialized refactoring tools (rust-analyzer is sufficient)
- Procedural macros for fixtures (overkill for 10k LOC codebase)

### Expected Features (Code Quality Patterns)

**Must have (table stakes):**
1. **Shared test fixtures in tests/common/** - DRY principle for test data, extracting 37 duplicate configs
2. **Clippy complexity lints enabled** - `too_many_lines` (100 line threshold), `excessive_nesting` to prevent 155-line functions from recurring
3. **Module boundaries by responsibility** - Split only when mixing unrelated concerns, not for line counts
4. **Cross-module utilities in src/utils/** - Single implementation of `format_duration()` instead of 3 copies

**Should have (competitive excellence):**
1. **rstest fixture-based testing** - Parameterized tests with `#[case]` attributes to replace repetitive test functions
2. **Utility module hierarchy** - `src/utils/formatting.rs`, `src/utils/...` for cross-cutting concerns
3. **Builder patterns for test configs** - Using derive_builder for composable fixtures

**Defer (post-v2.0):**
1. **ratatui-testlib snapshot testing** - Valuable but requires learning new paradigm, current 65% coverage is solid
2. **Component-based TUI refactor** - Current ui/app.rs + ui/dashboard.rs + ui/mod.rs structure works well
3. **Action/Command mapping system** - Only if keybindings need to be configurable

**Anti-patterns explicitly avoided:**
- Splitting modules by arbitrary line count (CI-TUI's 1,647 line app.rs is fine with high cohesion)
- Using `tests/common.rs` instead of `tests/common/mod.rs` (creates empty test output)
- Relying on `cognitive_complexity` lint (flawed, moved to restriction category by clippy maintainers)

### Architecture Approach

**Current structure is sound - focus on test organization, not module splitting.**

CI-TUI has clean separation: 12 modules with no circular dependencies. Files like `src/checks.rs` (1,261 lines) and `src/ui/app.rs` (1,647 lines) are large but maintain strong single responsibility. The Rust community has no official line count standard - projects commonly have 2,000-5,000 line files when cohesion is high. Splitting these would create artificial boundaries and reduce maintainability.

**Recommended test fixture structure:**
```
tests/
├── common/
│   ├── mod.rs          # Re-exports
│   ├── configs.rs      # Shared YAML configs (NEW - consolidates 37 duplicates)
│   ├── builders.rs     # Test data builders (FUTURE - if needed)
│   └── mocks.rs        # Mock utilities (FUTURE - extract from mod.rs if >500 lines)
├── git_tests.rs
├── runner_tests.rs
├── widget_tests.rs
└── panic_hook_tests.rs
```

**For duplicate utility functions:**
Create `src/utils/formatting.rs` for `format_duration()` and similar cross-module functions. Use `pub(crate)` for internal visibility.

**Module splitting decision tree:**
- Split when: Multiple unrelated concerns, poor test isolation, difficulty naming module
- DON'T split when: High cohesion, well-organized tests, single responsibility maintained
- Current modules (checks.rs, config.rs, ui/*.rs) all meet the "don't split" criteria

### Critical Pitfalls

1. **Event loop monopolization (async TUI-specific)** - Current architecture uses `tokio::time::sleep(16ms)` at end of main loop, creating input lag floor. Solution: Use `tokio::select!` with async channels or reduce sleep duration. NOT a v2.0 cleanup priority but worth monitoring.

2. **Test data duplication brittleness** - 37 inline YAML configs mean every config structure change requires N updates. Solution: Extract canonical fixtures to `tests/common/configs.rs` with builder helpers. This is the PRIMARY target for Phase 1.

3. **Complexity creep without objective gates** - 155-line functions exist because no lint enforcement. Solution: Enable `too_many_lines` (threshold 100) and `excessive_nesting` lints. Use rust-analyzer's "Extract Function" refactoring for violations.

4. **Premature module splitting** - Large files might trigger reflexive splitting, but Rust community pattern is to maintain cohesion over arbitrary line limits. Solution: Only split when multiple unrelated responsibilities emerge, not for aesthetics.

5. **Mixing sync/async channels incorrectly** - Using `std::sync::mpsc` with tight `try_recv()` loops in async contexts wastes CPU. Current CI-TUI implementation is acceptable (has sleep yield point) but could be improved with `tokio::sync::mpsc` and `recv().await` in `select!` block.

## Implications for Roadmap

### Phase 1: Test Fixture Consolidation
**Rationale:** Highest ROI with lowest risk. Addresses the PRIMARY technical debt (37 duplicate configs) using well-established Rust patterns (tests/common/ convention from official Rust Book). Foundation for all other improvements.

**Delivers:**
- `tests/common/configs.rs` with canonical test fixtures
- Helper functions: `minimal_config()`, `php_project_config()`, `rust_project_config()`, etc.
- Migration of all inline YAML configs from `src/config.rs` (27 configs), `src/checks.rs` (9 configs), `src/ui/app.rs` (1 config)
- Zero inline test configs remaining in src/ modules

**Addresses:**
- FEATURES.md: Must-have #2 (DRY test fixtures)
- PITFALLS.md: Pitfall #2 (test data duplication brittleness)

**Success criteria:**
- All tests pass with no behavior changes
- Reduced LOC in test modules
- New tests require <10 lines of config setup (vs current 40+)

**Research flag:** Standard pattern, skip research. Official Rust Book documentation is definitive.

---

### Phase 2: Function Complexity Reduction
**Rationale:** Enable objective complexity gates before they become enforcement burden. The 155-line `determine_checks()` function is already pushing limits - establish clippy lint infrastructure now.

**Delivers:**
- Clippy lints enabled: `too_many_lines = "warn"` (threshold 100), `excessive_nesting = "warn"`
- Refactoring of `determine_checks()` function using rust-analyzer's "Extract Function"
- Documentation of when to split functions (objective criteria)

**Addresses:**
- FEATURES.md: Must-have #3 (Clippy complexity lints)
- PITFALLS.md: Pitfall #3 (complexity creep without gates)

**Implementation approach:**
1. Add lints to Cargo.toml
2. Run `cargo clippy` to identify violations
3. Use rust-analyzer refactoring (select code, Ctrl+Shift+R, "Extract Function")
4. Validate with `--simple` after each extraction

**Research flag:** Skip research. Clippy documentation is comprehensive, rust-analyzer refactoring is standard tooling.

---

### Phase 3: Utility Module Extraction
**Rationale:** Eliminate function duplication (3x `format_duration()`). Low complexity but requires careful module organization.

**Delivers:**
- `src/utils/mod.rs` with submodules
- `src/utils/formatting.rs` containing `format_duration()` (single implementation)
- Updates to modules using these utilities to import from `crate::utils`

**Addresses:**
- FEATURES.md: Must-have #4 (cross-module utilities)
- FEATURES.md: Should-have #12 (utility module hierarchy)

**Implementation notes:**
- Use `pub(crate)` visibility for internal utilities
- Keep utilities free of domain logic (pure functions)
- Add unit tests in `#[cfg(test)]` module alongside utilities

**Research flag:** Skip research. Standard Rust module organization from official docs.

---

### Phase 4: Test Excellence (Optional)
**Rationale:** Nice-to-have improvements that enhance testing but aren't critical for v2.0. Can defer if timeline tight.

**Delivers:**
- Builder pattern implementation using derive_builder for test configs
- rstest parameterization for repetitive test scenarios
- Enhanced test documentation

**Addresses:**
- FEATURES.md: Should-have #11 (rstest fixture-based testing)
- FEATURES.md: Should-have #17 (better test organization)
- STACK.md: derive_builder recommendation

**Dependencies:**
- Requires Phase 1 complete (fixtures extracted)
- Adds new dev-dependency: `derive_builder = "0.20"`

**Research flag:** Skip research. Library documentation is comprehensive, pattern is well-established.

---

### Phase Ordering Rationale

**Sequential dependencies:**
- Phase 1 MUST come first: Test fixture consolidation establishes the foundation for Phase 4's builder patterns
- Phase 2 can run parallel to Phase 1: Clippy lints and function extraction don't conflict with test refactoring
- Phase 3 depends on nothing: Utility extraction is independent but lower priority than test debt

**Risk mitigation:**
- Phase 1 (highest impact) has lowest risk: moving test data doesn't change logic
- Phase 2 requires validation after each function extraction: use `--simple` mode to verify
- Phase 3 is pure refactoring: compiler enforces correctness via ownership/borrowing

**Effort distribution:**
- Phase 1: 4-6 hours (migrate 37 configs incrementally, one module at a time)
- Phase 2: 2-3 hours (enable lints, refactor 1-2 functions)
- Phase 3: 1-2 hours (extract utilities, update imports)
- Phase 4: 3-4 hours (add derive_builder, refactor to builders)

**Critical insight:** DO NOT add a "module splitting" phase. The research conclusively shows CI-TUI's large modules maintain high cohesion and should NOT be split. This saves significant time and avoids creating maintenance burden.

### Research Flags

**Phases with standard patterns (skip research-phase):**
- **Phase 1 (Test Fixtures):** Official Rust Book chapter on test organization is definitive. Pattern is tests/common/mod.rs with fixture helpers.
- **Phase 2 (Complexity):** Clippy lint documentation and rust-analyzer refactoring guides are comprehensive. No ambiguity.
- **Phase 3 (Utilities):** Standard Rust module organization. Official guidance in Rust Book chapter 7.
- **Phase 4 (Test Excellence):** derive_builder and rstest both have excellent documentation and examples.

**Phases needing deeper research:** NONE for this cleanup milestone.

All patterns are well-documented with authoritative sources (official Rust Book, Clippy docs, library documentation). The research phase already provided sufficient depth.

## Confidence Assessment

| Area | Confidence | Notes |
|------|------------|-------|
| Stack | HIGH | Current versions verified via WebFetch (rstest 0.26, mockall 0.14, derive_builder 0.20). OnceLock pattern is stdlib since Rust 1.70. |
| Features | HIGH | Official Rust Book defines test organization patterns. Clippy maintainers explicitly document lint recommendations. Multiple authoritative sources agree. |
| Architecture | HIGH | Direct analysis of CI-TUI codebase structure. Module sizes and cohesion assessment based on official Rust API guidelines (RFC 430). |
| Pitfalls | HIGH | Async TUI pitfalls verified against official Tokio docs, Ratatui tutorials, and authoritative technical articles (Alice Ryhl). Test duplication is self-evident from codebase inspection. |

**Overall confidence:** HIGH

All recommendations are grounded in:
- Official Rust documentation (Rust Book, API Guidelines)
- Standard library capabilities (OnceLock, module system)
- Established community patterns (tests/common/, clippy lints)
- Authoritative library docs (derive_builder, rstest, tokio)
- Direct codebase analysis (no speculation about structure)

### Gaps to Address

**No significant gaps identified.** Research was thorough across all dimensions:

✓ Test organization patterns are definitive (official Rust Book)
✓ Refactoring tooling is standard (rust-analyzer, clippy)
✓ Module organization guidelines are clear (API Guidelines RFC 430)
✓ Async TUI pitfalls are well-documented (Tokio docs, Ratatui tutorials)

**Minor validation points during implementation:**
1. **derive_builder ergonomics:** Verify builder pattern feels natural with CI-TUI's config structure. If not, fall back to hand-written builders (still better than 37 YAML strings).
2. **clippy lint thresholds:** Default `too_many_lines` threshold is 100 - may need tuning to 120-150 based on codebase style. Adjust in Cargo.toml if needed.
3. **rstest adoption:** Evaluate cost/benefit after Phase 1. If existing fixture functions work well, defer rstest to post-v2.0.

**Monitoring point (not v2.0 blocker):**
- Event loop responsiveness under high CPU load. Current architecture has known 16ms input lag floor. If users report sluggish keyboard during v2.0 work, consider async channel upgrade (converts `std::sync::mpsc` to `tokio::sync::mpsc` with `select!` pattern). But this is a separate enhancement, not cleanup.

## Sources

### Primary (HIGH confidence)

**Official Rust Documentation:**
- [Test Organization - The Rust Programming Language](https://doc.rust-lang.org/book/ch11-03-test-organization.html) - Definitive guide for tests/common/ pattern
- [Refactoring to Improve Modularity - The Rust Programming Language](https://doc.rust-lang.org/book/ch12-03-improving-error-handling-and-modularity.html) - Module splitting guidance
- [Rust API Guidelines - Naming Conventions (RFC 430)](https://rust-lang.github.io/api-guidelines/naming.html) - Official standards
- [std::sync::OnceLock documentation](https://doc.rust-lang.org/std/sync/struct.OnceLock.html) - Lazy static fixture pattern

**Authoritative Library Documentation:**
- [derive_builder 0.20.2 documentation](https://docs.rs/derive_builder/latest/derive_builder/) - Current version verified via WebFetch
- [rstest 0.26.1 documentation](https://docs.rs/rstest/latest/rstest/) - Fixture framework
- [Clippy Lints](https://rust-lang.github.io/rust-clippy/master/index.html) - Lint recommendations including cognitive_complexity deprecation rationale

**TUI Stack:**
- [Ratatui FAQ](https://ratatui.rs/faq/) - Platform differences (Windows vs Linux/macOS)
- [Ratatui - The Elm Architecture](https://ratatui.rs/concepts/application-patterns/the-elm-architecture/) - UI organization patterns
- [Tokio Runtime Documentation](https://docs.rs/tokio/latest/tokio/runtime/index.html) - Async scheduling model
- [Tokio Bridging with Sync Code](https://tokio.rs/tokio/topics/bridging) - Channel selection guidance

### Secondary (MEDIUM confidence)

**Community Best Practices:**
- [Long-term Rust Project Maintenance](https://corrode.dev/blog/long-term-rust-maintenance/) - Visibility and maintenance patterns
- [Testing with Builder Pattern for Fixtures](https://dan.munckton.co.uk/blog/2018/03/01/testing-rust-using-the-builder-pattern-for-complex-fixtures/) - Practical examples
- [Do we have standard for LOC per file? - Rust Forum](https://users.rust-lang.org/t/do-we-have-standard-for-loc-per-file/63509) - Community consensus on file sizes

**Technical Deep Dives:**
- [Alice Ryhl: Async: What is blocking?](https://ryhl.io/blog/async-what-is-blocking/) - Authoritative async Rust explanation
- [rust-analyzer changelog #311 (Jan 2026)](https://rust-analyzer.github.io/thisweek/2026/01/19/changelog-311.html) - Recent refactoring improvements
- [How Tokio Schedule Tasks: A Hard Lesson Learnt](https://rustmagazine.org/issue-4/how-tokio-schedule-tasks/) - Cooperative scheduling pitfalls

### Tertiary (codebase analysis)

**CI-TUI Source Code Inspection:**
- Direct analysis of `tests/common/mod.rs` (223 lines, current shared utilities)
- Inline YAML config count: `src/config.rs` (27x), `src/checks.rs` (9x), `src/ui/app.rs` (1x)
- Module sizes: checks.rs (1261 lines), config.rs (1227 lines), ui/app.rs (1647 lines)
- Event loop structure: `src/ui/mod.rs` lines 362-540 (async function with sleep pattern)

---

**Research completed:** 2026-01-29
**Ready for roadmap:** Yes

**Summary for orchestrator:**

Research synthesis complete. CI-TUI v2.0 cleanup should focus on **test fixture consolidation** (37 duplicate YAML configs) and **complexity gates** (clippy lints), while explicitly **avoiding module splitting** (high cohesion maintained). Recommended 3-4 phases with clear dependencies: fixtures first (foundation), complexity lints second (gates), utilities third (duplication), builders fourth (excellence/optional). All patterns are standard Rust practices with high-confidence sources. No research-phase needed during planning - official docs are definitive.
