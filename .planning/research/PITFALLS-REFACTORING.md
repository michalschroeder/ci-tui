# Pitfalls Research: Rust Refactoring & Test Migration

**Domain:** Rust codebase refactoring and test infrastructure consolidation
**Researched:** 2026-01-29
**Overall confidence:** HIGH (verified with official docs + community practices)

## Executive Summary

Refactoring Rust codebases presents unique challenges due to the borrow checker, strict visibility rules, and the lack of comprehensive refactoring tooling. The most critical risk for CI-TUI's cleanup is **test breakage through module visibility changes** when consolidating fixtures and extracting utilities. Secondary risks include **borrow checker conflicts from function splitting** and **accidental behavior changes** when refactoring the 155-line `determine_checks` function. The good news: Rust's compiler catches most errors at compile-time, making mistakes visible immediately rather than at runtime.

## Critical Pitfalls

### 1. Module Visibility Breakage During Refactor

**Risk:** Tests break after moving code between modules because items that were `pub` in one context become inaccessible in another, or `#[cfg(test)]` utilities aren't visible to integration tests.

**Why it happens:**
- Rust's privacy model is complex: items need to be `pub` at every level of the module tree
- `#[cfg(test)]` code is only compiled during testing but NOT visible from integration tests (`tests/` directory)
- Integration tests are separate crates and can only see `pub` items from `lib.rs`
- Refactoring changes paths (`super::`, `crate::`), breaking existing imports

**Warning signs:**
- Compile errors: "module `X` is private" or "function `Y` is not found in scope"
- Tests that worked in `src/` modules suddenly fail when moved to `tests/common/`
- Integration tests can't access helper functions that unit tests can

**Prevention:**
1. **Before moving test utilities to `tests/common/`**: Mark them `pub` and remove `#[cfg(test)]`
2. **Use `pub(crate)` by default** for internal helpers - makes future refactoring easier
3. **Prefer `crate::` over `super::`** for imports - absolute paths survive refactoring better
4. **Keep test helpers in `tests/common/mod.rs`** - Cargo won't treat it as a separate test crate
5. **For CI-TUI specifically**: The existing `tests/common/mod.rs` already follows this pattern - extend it rather than creating new patterns

**Recovery:**
- Add `pub` visibility incrementally up the module tree until accessible
- Use `pub use` re-exports to create stable paths that won't break if you move code again
- Run `cargo test` after each module move to catch breaks immediately

**Phase relevance:** Phase 1 (Test fixture consolidation) - This is THE critical risk for consolidating 40 inline YAML configs

**Sources:**
- [Visibility and Privacy - Rust Reference](https://doc.rust-lang.org/reference/visibility-and-privacy.html)
- [Test Organization - The Rust Programming Language](https://doc.rust-lang.org/book/ch11-03-test-organization.html)
- [Cargo issue #8379: cfg(test) visibility across crates](https://github.com/rust-lang/cargo/issues/8379)

---

### 2. Lifetime/Borrow Checker Errors From Function Splitting

**Risk:** Refactoring a 155-line function into smaller pieces creates new borrow checker errors because you've changed the scope boundaries where borrows are checked.

**Why it happens:**
- Large functions have one big borrow scope - the compiler can see all borrows together
- Splitting into smaller functions creates separate borrow scopes with function boundaries
- The compiler now needs explicit lifetimes where it could previously infer them
- Mutable and immutable borrows that coexisted in one scope conflict across function boundaries

**Warning signs:**
- Compile error: "cannot borrow as mutable because it is also borrowed as immutable"
- Compile error: "borrowed value does not live long enough"
- Code that worked in one 155-line function breaks when split into 3 functions
- You're tempted to add `.clone()` everywhere to make errors go away

**Prevention:**
1. **Split incrementally** - Extract one small helper at a time, test after each extraction
2. **Minimize borrow scopes** - Use block scopes `{ let x = ...; }` to end borrows early
3. **Prefer small functions that take ownership** where possible (avoid lifetime parameters initially)
4. **For CI-TUI's `determine_checks`**: Look for natural boundaries where data is independent:
   - Extract `resolve_command` logic (already done)
   - Extract "match files by pattern" logic
   - Extract "handle test discovery" logic
   - Keep the main loop intact initially

**Recovery:**
- If stuck on lifetimes: make function take owned values (`String`) instead of references (`&str`)
- Use `#[allow(clippy::too_many_arguments)]` temporarily if ownership simplification requires more parameters
- Consider returning tuples instead of multiple mutable parameters
- As last resort: use `Rc<RefCell<>>` or `Arc<Mutex<>>` (but usually a sign of design issue)

**Phase relevance:** Phase 2 (Refactor large functions) - Will hit this when splitting `determine_checks`

**Sources:**
- [Refactoring Rust Code to Avoid Borrow Checker Conflicts - Sling Academy](https://www.slingacademy.com/article/refactoring-rust-code-to-avoid-borrow-checker-conflicts/)
- [Difficult/Long refactoring - Rust Forum](https://users.rust-lang.org/t/difficult-long-refactoring-suggestions/30900)
- [Borrow checker/lifetime error discussions - Rust Forum January 2026](https://users.rust-lang.org/t/borrow-checker-lifetime-error-in-iterator/137260)

---

### 3. Accidental Behavior Change During Logic Refactor

**Risk:** When extracting complex logic with multiple branches, you accidentally change the behavior because you misunderstood the original control flow.

**Why it happens:**
- 155-line functions have complex nested conditions that aren't obvious
- CI-TUI's `determine_checks` has tricky logic: "if test discovery finds nothing BUT check is on_demand THEN mark as on-demand ELSE IF command has no {files} placeholder THEN run all"
- Early returns, continue statements, and nested matches change behavior subtly
- You think you understand it, extract it, tests pass... but edge case behavior changed

**Warning signs:**
- Tests pass but behavior feels different in manual testing
- Edge cases that used to work now fail (or vice versa)
- Debug output shows different execution paths for same input
- You can't explain why the original code used `continue` vs `break` vs early return

**Prevention:**
1. **Add characterization tests BEFORE refactoring** - Test current behavior even if it seems wrong
2. **For `determine_checks` specifically**:
   - Add tests for: no files matched + on_demand = true
   - Add tests for: source changed + no tests found + command has {files}
   - Add tests for: source changed + no tests found + command has NO {files}
3. **Extract pure functions first** - Functions with no side effects, deterministic output (like `resolve_command`)
4. **Keep complex branching logic together initially** - Don't try to DRY up subtle differences
5. **Use `// TODO: Extract this` comments** during first pass rather than extracting immediately

**Recovery:**
- Revert the extraction, add more tests, re-attempt
- Use `git diff` to compare before/after logic side-by-side
- Add temporary debug logging at decision points to verify paths taken
- Consider extracting smaller pieces (one branch at a time)

**Phase relevance:** Phase 2 (Refactor large functions) - Most dangerous for `determine_checks` with its complex branching

**Sources:**
- [Refactoring to Improve Modularity - The Rust Programming Language](https://doc.rust-lang.org/book/ch12-03-improving-error-handling-and-modularity.html)
- [Rust Refactoring for Beginners - Better Programming](https://betterprogramming.pub/rust-refactoring-for-beginners-15a3270ce45d)

---

### 4. Test Fixture Duplication Instead of Consolidation

**Risk:** When consolidating test fixtures, you create helper functions that are subtly different, leading to more test fixtures than you started with, each slightly incompatible.

**Why it happens:**
- Each test had inline YAML with small variations
- You create "flexible" fixture functions with many parameters to handle variations
- Different tests need different combinations of parameters
- You end up with `make_config()`, `make_config_with_parallel()`, `make_config_no_git()`, etc.
- Now you have 10 fixture functions instead of 40 inline YAMLs, but they're harder to understand

**Warning signs:**
- Fixture functions have 5+ boolean parameters
- Multiple fixture functions with similar names but slight differences
- Tests pass individual parameters like `make_config(true, false, None, Some("x"), false)`
- You can't remember which fixture function does what
- New tests require new fixture variants

**Prevention:**
1. **Use rstest fixture composition** - Compose complex fixtures from smaller ones:
   ```rust
   #[fixture]
   fn base_config() -> CiConfig { ... }

   #[fixture]
   fn parallel_config(#[from(base_config)] mut config: CiConfig) -> CiConfig {
       config.groups_mut().get_mut("test").unwrap().parallel = true;
       config
   }
   ```
2. **Use builder pattern for complex configs**:
   ```rust
   TestConfigBuilder::new()
       .with_parallel_group("lint")
       .with_check("clippy", "cargo clippy")
       .build()
   ```
3. **For CI-TUI**: The existing `widget_test_config_yaml()` is good - expand it with variants like:
   - `base_test_config()` - Minimal valid config
   - `test_config_with_discovery()` - Adds test discovery
   - `multi_group_config()` - Multiple groups for grouping tests
4. **Keep inline YAML for edge cases** - Don't consolidate tests of invalid configs

**Recovery:**
- Delete the complicated fixtures
- Start over with ONE base fixture
- Add 2-3 variant fixtures for common patterns
- Keep inline YAML for anything else

**Phase relevance:** Phase 1 (Test fixture consolidation) - Easy to over-engineer here

**Sources:**
- [Testing With Fixtures in Rust](https://dawchihliou.github.io/articles/testing-with-fixtures-in-rust)
- [rstest fixture composition docs](https://docs.rs/rstest/latest/rstest/attr.fixture.html)
- [Rust Testing Superpowers: rstest - Medium](https://medium.com/@adamszpilewicz/rust-testing-superpowers-rstest-and-fixtures-7c17d7ec12df)

## Medium-Risk Pitfalls

### 5. Breaking Existing Tests During Utility Extraction

**Risk:** When extracting duplicate utility code from test modules, you change the behavior slightly and break tests in non-obvious ways.

**Why it happens:**
- Duplicate code had small intentional differences you didn't notice
- You extract the "common" part but lose the variations
- Example: two tests both build Docker commands but one quotes differently

**Warning signs:**
- Tests fail with "assertion failed" after extracting utilities
- Output format changes (extra quotes, different spacing)
- One test domain works but another breaks with same utility

**Prevention:**
1. **Run full test suite after each extraction** - `cargo test` must stay green
2. **Extract functions that are EXACTLY identical first** - Use diff tools
3. **Add tests FOR the utility functions** - Test the extracted code independently
4. **For CI-TUI**: The `tests/common/mod.rs` utilities are already tested indirectly - good pattern

**Recovery:**
- Revert, identify the difference, make utility handle both cases with a parameter
- Or keep two separate utilities if the difference is fundamental

**Phase relevance:** Phase 1 (Extract duplicate utility code)

---

### 6. Overuse of `.clone()` to Appease Borrow Checker

**Risk:** During refactoring, hitting borrow checker errors and adding `.clone()` everywhere to make it compile, causing performance regression and hiding design issues.

**Why it happens:**
- Borrow checker errors are frustrating when you're focused on logic
- `.clone()` is the easiest fix - just clone and move on
- But cloning large configs, file lists, etc. is expensive
- Excessive cloning means your ownership model is wrong

**Warning signs:**
- Performance regression after refactor
- Many `.clone()` calls added to make code compile
- Clippy warns: "redundant clone" or "unnecessary clone"
- You're cloning `String`, `Vec`, `HashMap` frequently

**Prevention:**
1. **Understand WHY the borrow checker is complaining** before adding `.clone()`
2. **Prefer passing references** (`&T`) when possible
3. **For CI-TUI's `CheckToRun` specifically**: This struct already clones `CheckDefinition` - that's fine, it's small and infrequent
4. **Use `Cow<'a, str>` for string data** that's sometimes owned, sometimes borrowed
5. **Consider `Arc<T>` for shared ownership** instead of cloning large data structures

**Recovery:**
- Profile with `cargo bench` or `cargo flamegraph` to find expensive clones
- Replace clones with references one at a time
- If you need shared ownership, use `Arc<T>` instead of cloning

**Phase relevance:** Phase 2 (Refactor large functions) - Will be tempted to clone when splitting functions

**Sources:**
- [Rust Common Mistakes - Medium](https://medium.com/@tzutoo/rust-common-mistakes-8e759c6e1dc)
- [10 Common Mistakes Rust Developers Make - Medium](https://medium.com/@giorgio.martinez1926/10-common-mistakes-rust-developers-make-and-how-to-avoid-them-2bb8250c7150)

---

### 7. Module Organization Causing Circular Dependencies

**Risk:** When moving code between modules, accidentally creating circular dependencies where module A imports B and B imports A.

**Why it happens:**
- Rust doesn't allow circular `mod` declarations
- Easy to create when extracting utilities: tests import utilities, utilities import test helpers
- Not caught until you try to compile

**Warning signs:**
- Compile error: "cyclic module dependency"
- Import that worked suddenly fails when you add a reverse import

**Prevention:**
1. **Keep dependency flow unidirectional**: `tests` → `common` → `src`, never reverse
2. **For utilities needed by both**: Create a shared module (`test_support`) in `src` with `#[cfg(test)]`
3. **For CI-TUI**: Keep `tests/common/` for test-only code, add `src/test_support/` if you need utilities in both places

**Recovery:**
- Create a third module that both depend on
- Move common types to a `types` module
- Use trait objects or dependency injection to break the cycle

**Phase relevance:** Phase 1 (Test fixture consolidation) - Easy to create when organizing shared code

**Sources:**
- [Understanding Rust Privacy and Visibility Model](https://iximiuz.com/en/posts/rust-privacy-and-visibility/)
- [Parent Module Imports in Rust 2025-2026](https://copyprogramming.com/howto/how-do-you-use-parent-module-imports-in-rust)

---

### 8. Test Execution Order Dependency

**Risk:** Tests pass individually but fail when run together because they share mutable state (especially in integration tests).

**Why it happens:**
- Rust tests run in parallel by default
- Integration tests might use shared files, environment variables, or Docker containers
- Race conditions when tests mutate shared resources

**Warning signs:**
- `cargo test` sometimes passes, sometimes fails
- Tests pass individually (`cargo test test_name`) but fail in suite
- Failures happen on CI but not locally (or vice versa)

**Prevention:**
1. **Avoid mutable global state** - Each test should be isolated
2. **For CI-TUI specifically**: Tests use Docker containers - ensure each test uses unique container names or services
3. **Use `#[serial]` attribute from `serial_test` crate** for tests that must run sequentially
4. **Don't rely on filesystem state** - Clean up temp files in each test

**Recovery:**
- Run with `cargo test -- --test-threads=1` to diagnose
- Add resource cleanup in test teardown
- Use unique resource names per test

**Phase relevance:** Phase 3 (Add tests to runner.rs, simple.rs, fix.rs) - These modules interact with Docker

**Sources:**
- [Writing efficient tests - Mozilla Application Services](https://mozilla.github.io/application-services/book/design/test-faster.html)
- [Everything you need to know about testing in Rust - Shuttle](https://www.shuttle.dev/blog/2024/03/21/testing-in-rust)

## Low-Risk (But Worth Noting)

### 9. Clippy Warnings Explosion After Refactor

**Risk:** Refactoring introduces many new Clippy warnings that obscure real issues.

**Why it happens:**
- Moving code to new modules triggers new lint contexts
- Extracted functions have different patterns that Clippy dislikes
- Not a real problem, but noise that hides actual issues

**Prevention:**
- Run `cargo clippy` after each refactor phase
- Fix warnings incrementally
- Use `#[allow(clippy::lint_name)]` judiciously for intentional patterns

**Phase relevance:** All phases - Run clippy after each change

---

### 10. Forgetting to Update Documentation

**Risk:** Doc comments become outdated after refactoring, misleading future developers.

**Why it happens:**
- Focused on code, forget to update comments
- Module-level docs reference old structure
- Function docs reference old behavior

**Prevention:**
- Update docs in same commit as code changes
- Run `cargo doc --open` to review generated docs
- For CI-TUI: Module-level docs in `checks.rs` are good - keep them updated

**Phase relevance:** All phases

---

### 11. Not Running Full Test Suite Before Committing

**Risk:** Assuming your targeted tests passing means everything works, missing breakage elsewhere.

**Why it happens:**
- Running `cargo test test_name` during development
- Forgetting to run full suite before commit
- CI catches it, but wastes time with round-trips

**Prevention:**
- CI-TUI already has validation commands - USE THEM:
  ```bash
  # Fix format first
  docker run --rm -v /var/run/docker.sock:/var/run/docker.sock -v "$(pwd)":/app -e HOST_PWD="$(pwd)" -w /app ghcr.io/michalschroeder/ci-tui:latest --config ./ci-tui.yaml --fix

  # Then validate
  docker run --rm -v /var/run/docker.sock:/var/run/docker.sock -v "$(pwd)":/app -e HOST_PWD="$(pwd)" -w /app ghcr.io/michalschroeder/ci-tui:latest --config ./ci-tui.yaml --simple
  ```
- Make this a habit: `--fix` then `--simple` before every commit

**Phase relevance:** All phases - Use after every change

## CI-TUI Specific Concerns

### The `determine_checks` Function (155 lines in checks.rs)

**Complexity characteristics:**
- Nested loops: iterates over groups, then checks within groups
- Multiple early continues: `if check.always_run() { ... continue; }`
- Complex conditional logic: test discovery fallback behavior
- State accumulation: builds `matched_files` across multiple conditions
- Critical domain logic: determines what runs, what's skipped, what's on-demand

**Refactoring strategy to avoid pitfalls:**
1. **Phase 2.1**: Add characterization tests for every branch path BEFORE extracting
2. **Phase 2.2**: Extract pure helper functions first:
   - `match_files_by_pattern(triggers, config, changed_files) -> Vec<String>`
   - `find_tests_for_sources(discovery, source_files, project_root) -> Vec<String>`
   - `should_be_on_demand(matched_files, check, has_source_trigger) -> bool`
3. **Phase 2.3**: Refactor the main loop to use extracted helpers, but keep loop structure intact
4. **Phase 2.4**: Only THEN consider extracting the entire loop body

**Red flags to watch for:**
- If you're cloning `CheckDefinition` more than once per check, something is wrong
- If extracted functions need 5+ parameters, they're too coupled to the original function
- If tests start failing mysteriously, you've changed control flow

### Test Structure (250 existing tests)

**Current organization:**
- `tests/common/mod.rs`: Shared utilities, mock builders (224 lines)
- `tests/runner_tests.rs`: Runner command building (uses rstest)
- `tests/git_tests.rs`: Git operations
- `tests/widget_tests.rs`: UI rendering
- `tests/panic_hook_tests.rs`: Panic handling

**Good patterns already present:**
- Integration test helpers in `tests/common/` (not treated as separate test crate)
- Mock executors using mockall trait mocking
- Fixture functions like `make_test_app()` for widget tests
- Using rstest for parameterized tests

**Anti-patterns to avoid when adding tests:**
- Don't create `tests/common.rs` (Cargo would treat it as a test crate) - use `tests/common/mod.rs` instead
- Don't mark fixtures as `#[cfg(test)]` if they need to be used from `tests/` directory
- Don't create inline YAML configs in the new tests - use the consolidated fixtures from Phase 1

### Untested Modules (runner.rs, simple.rs, fix.rs)

**Why they're untested:**
- **runner.rs**: Heavy Docker interaction, async complexity, event streaming
- **simple.rs**: Console output formatting, harder to assert
- **fix.rs**: Similar to runner but focused on fix commands

**Testing strategy to avoid pitfalls:**
1. **Mock the Docker executor** (already exists: `MockCommandExecutor`)
2. **Test command building separately from execution** (already done in `runner_tests.rs`)
3. **For simple.rs**: Capture stdout/stderr instead of mocking console
4. **For fix.rs**: Similar approach to runner tests, mock executor

**Don't test:**
- Actual Docker container execution (integration test territory)
- Real terminal rendering (widget tests cover the UI layer)
- Actual git commands (git_tests.rs uses mocks)

## Phase-Specific Warnings

| Phase | Topic | Primary Pitfall Risk | Mitigation |
|-------|-------|---------------------|------------|
| 1 | Consolidate test fixtures | Module visibility breakage (#1) | Keep fixtures in `tests/common/mod.rs`, mark `pub`, remove `#[cfg(test)]` |
| 1 | Extract duplicate utilities | Test fixture duplication (#4) | Use rstest composition, builder pattern, keep base simple |
| 2 | Refactor `determine_checks` | Accidental behavior change (#3) | Add characterization tests first, extract pure functions only initially |
| 2 | Split large function | Borrow checker errors (#2) | Split incrementally, test after each extraction, prefer ownership |
| 3 | Add runner.rs tests | Test execution order (#8) | Use unique Docker container names, clean up resources |
| 3 | Add simple.rs tests | Breaking existing tests (#5) | Capture stdout properly, don't change existing output format |
| 3 | Add fix.rs tests | Overuse of clone (#6) | Reuse runner mocks, avoid cloning CheckToRun unnecessarily |

## Success Metrics

**Refactoring is complete when:**
- [ ] All 250 existing tests still pass
- [ ] No new Clippy warnings introduced
- [ ] `cargo fmt --check` passes
- [ ] New tests added for previously untested modules
- [ ] No `.clone()` added without justification
- [ ] Module structure is clearer (fewer cross-module dependencies)
- [ ] The largest function is under 100 lines

**Red flags that indicate problems:**
- Test suite is slower after refactor (cloning or inefficiency introduced)
- New tests are more complex than the code they test
- You've created more test fixture functions than you had inline configs
- Borrow checker errors that you "fixed" with `unsafe` or `Arc<Mutex<>>`

## Sources

### High Confidence (Official Documentation + Community Consensus)

- [Refactoring to Improve Modularity - The Rust Programming Language](https://doc.rust-lang.org/book/ch12-03-improving-error-handling-and-modularity.html)
- [Test Organization - The Rust Programming Language](https://doc.rust-lang.org/book/ch11-03-test-organization.html)
- [Visibility and Privacy - The Rust Reference](https://doc.rust-lang.org/reference/visibility-and-privacy.html)
- [rstest Fixture Documentation](https://docs.rs/rstest/latest/rstest/attr.fixture.html)
- [Testing With Fixtures in Rust - Daw-Chih Liou](https://dawchihliou.github.io/articles/testing-with-fixtures-in-rust)

### Medium Confidence (Active Community Discussions, 2026)

- [Refactoring Rust Code to Avoid Borrow Checker Conflicts - Sling Academy](https://www.slingacademy.com/article/refactoring-rust-code-to-avoid-borrow-checker-conflicts/)
- [Rust Refactoring for Beginners - Better Programming](https://betterprogramming.pub/rust-refactoring-for-beginners-15a3270ce45d)
- [Difficult/Long refactoring - Rust Forum](https://users.rust-lang.org/t/difficult-long-refactoring-suggestions/30900)
- [Borrow checker lifetime error discussions - Rust Forum, Jan 2026](https://users.rust-lang.org/t/borrow-checker-lifetime-error-in-iterator/137260)
- [Cargo issue #8379: cfg(test) visibility](https://github.com/rust-lang/cargo/issues/8379)

### General Best Practices

- [Rust Common Mistakes - Medium (tzutoo)](https://medium.com/@tzutoo/rust-common-mistakes-8e759c6e1dc)
- [Common Newbie Mistakes - Michael F. Bryan](https://adventures.michaelfbryan.com/posts/rust-best-practices/bad-habits/)
- [10 Common Mistakes Rust Developers Make - Medium](https://medium.com/@giorgio.martinez1926/10-common-mistakes-rust-developers-make-and-how-to-avoid-them-2bb8250c7150)
- [Everything you need to know about testing in Rust - Shuttle](https://www.shuttle.dev/blog/2024/03/21/testing-in-rust)
- [Understanding Rust Privacy and Visibility Model - Ivan Zemlianskii](https://iximiuz.com/en/posts/rust-privacy-and-visibility/)
