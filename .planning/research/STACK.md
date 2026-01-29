# Stack Research: Test Infrastructure & Refactoring

**Project:** CI-TUI v2.0 Comprehensive Cleanup
**Researched:** 2026-01-29
**Focus:** Test fixture consolidation and safe refactoring patterns
**Overall confidence:** HIGH

## Executive Summary

For consolidating 40+ duplicate YAML test configs and deduplicating test utilities, the path forward relies primarily on **patterns over dependencies**. The existing stack (rstest 0.26, mockall 0.14) is already well-equipped for modern test fixtures. Key additions needed: `derive_builder` for complex config builders and leveraging `std::sync::OnceLock` (Rust 1.70+) for expensive shared fixtures. Refactoring relies on rust-analyzer's built-in capabilities and clippy lints rather than specialized tools.

**Key finding:** The "40 duplicate YAML configs" are inline string literals in test helpers (as seen in `tests/common/mod.rs`), not separate files. The solution is builder patterns for composable config fixtures, not file consolidation.

## Recommended Additions

### derive_builder 0.20.2
- **What:** Procedural macro for automatic builder pattern implementation
- **Why:** Eliminates boilerplate when creating flexible test fixture builders for `CiConfig` structs with many optional fields
- **Integration:**
  - Add as dev-dependency only
  - Creates `ConfigBuilder` type with sensible defaults
  - Allows test-specific field overrides without 40+ lines of config YAML
- **Usage pattern:**
  ```rust
  #[cfg(test)]
  #[derive(Builder)]
  #[builder(build_fn(validate = "Self::validate"))]
  struct TestConfig {
      #[builder(default = "String::from(\"./test\")")]
      project_dir: String,
      #[builder(default = "String::from(\"app\")")]
      service: String,
      // ... other fields with defaults
  }
  ```
- **Source:** [derive_builder 0.20.2 docs](https://docs.rs/derive_builder/latest/derive_builder/)
- **Confidence:** HIGH (actively maintained, current version verified via WebFetch)

### No Additional Testing Libraries Needed
- **Why:** rstest 0.26.1 already provides fixture injection and parameterization
- **Why:** mockall 0.14 already provides trait mocking
- **Why:** pretty_assertions 1.4, tempfile 3 already cover assertions and temp directories
- **Confidence:** HIGH (current versions verified via WebFetch)

## Patterns (No New Dependencies)

### Pattern 1: Shared Test Config Factory with std::sync::OnceLock
- **Problem:** Multiple test files need expensive-to-parse base configurations
- **Solution:** Use `std::sync::OnceLock` (stable since Rust 1.70) for lazy, thread-safe initialization
- **Example:**
  ```rust
  // tests/common/configs.rs
  use std::sync::OnceLock;
  static BASE_CONFIG: OnceLock<CiConfig> = OnceLock::new();

  pub fn base_config() -> &'static CiConfig {
      BASE_CONFIG.get_or_init(|| {
          serde_yaml::from_str(include_str!("fixtures/base.yaml"))
              .expect("valid base config")
      })
  }
  ```
- **Why OnceLock over once_cell:** Standard library since Rust 1.70, no external dependency needed
- **Source:** [OnceLock documentation](https://doc.rust-lang.org/std/sync/struct.OnceLock.html)
- **Confidence:** HIGH

### Pattern 2: Builder Pattern for Test Configs
- **Problem:** 40+ lines of duplicate config YAML across tests, each varying slightly
- **Solution:** Create composable builders that start from sensible defaults
- **Implementation:**
  ```rust
  // With derive_builder
  impl TestConfigBuilder {
      pub fn minimal() -> Self {
          Self::default()
      }

      pub fn with_php_checks() -> Self {
          Self::default()
              .checks(/* PHP-specific checks */)
              .file_patterns(/* PHP patterns */)
      }

      pub fn with_rust_checks() -> Self {
          // Rust-specific preset
      }
  }

  // In tests
  let config = TestConfigBuilder::minimal()
      .service("custom-service")
      .build()
      .unwrap();
  ```
- **Benefit:** Each test expresses only what differs from defaults, eliminating duplication
- **Source:** [Builder Pattern in Rust](https://rust-unofficial.github.io/patterns/patterns/creational/builder.html), [Testing with Builder Pattern](https://dan.munckton.co.uk/blog/2018/03/01/testing-rust-using-the-builder-pattern-for-complex-fixtures/)
- **Confidence:** HIGH (established Rust testing pattern)

### Pattern 3: Test Helper Module Consolidation
- **Problem:** 40+ lines of duplicate utility functions spread across test files
- **Solution:** Consolidate into `tests/common/` with focused submodules
- **Structure:**
  ```
  tests/
    common/
      mod.rs          # Re-exports
      configs.rs      # Config builders and factories
      mocks.rs        # Mock setup helpers (already exists as inline)
      assertions.rs   # Custom assertion helpers
      fixtures.rs     # Shared test data
  ```
- **Why this works:** Rust's `tests/common/mod.rs` pattern prevents common code from becoming test executables
- **Source:** [Test Organization - Rust Book](https://doc.rust-lang.org/book/ch11-03-test-organization.html)
- **Confidence:** HIGH (official Rust best practice)

### Pattern 4: Declarative Macros for Repetitive Test Setup
- **Problem:** Similar test structure repeated across many test functions
- **Solution:** Use `macro_rules!` for common test patterns
- **Example:**
  ```rust
  macro_rules! test_check_matching {
      ($name:ident, $pattern:expr, $files:expr, $expected:expr) => {
          #[test]
          fn $name() {
              let config = TestConfigBuilder::minimal()
                  .file_pattern("test", $pattern)
                  .build()
                  .unwrap();
              let result = determine_checks(&config, $files);
              assert_eq!(result.len(), $expected);
          }
      };
  }

  test_check_matching!(matches_rust, r"\.rs$", vec!["src/main.rs"], 1);
  test_check_matching!(matches_yaml, r"\.yaml$", vec!["config.yaml"], 1);
  ```
- **When to use:** When test structure is identical but data varies (complement to rstest's `#[case]`)
- **When NOT to use:** When test logic differs between cases (use rstest parameterization instead)
- **Source:** [Rust Macros for Testing](https://rust-exercises.com/advanced-testing/08_macros/00_intro)
- **Confidence:** MEDIUM (pattern works but requires balancing macro complexity vs readability)

### Pattern 5: rstest Fixtures Over Inline Setup
- **Problem:** Each test duplicates the same setup code
- **Solution:** Leverage existing rstest more extensively with composed fixtures
- **Example:**
  ```rust
  #[fixture]
  fn base_config() -> CiConfig {
      TestConfigBuilder::minimal().build().unwrap()
  }

  #[fixture]
  fn changed_rust_files() -> ChangedFiles {
      ChangedFiles {
          files: vec!["src/main.rs".into(), "src/lib.rs".into()],
          base_ref: "main".into(),
      }
  }

  #[rstest]
  fn test_with_fixtures(base_config: CiConfig, changed_rust_files: ChangedFiles) {
      // Test uses fixtures without setup code
  }
  ```
- **Already available:** rstest 0.26 is already in dev-dependencies
- **Confidence:** HIGH (already using rstest, just needs broader adoption in codebase)

## Refactoring Tools & Workflow

### IDE Tooling: rust-analyzer (Built-in)
- **What:** LSP-based refactoring built into rust-analyzer (standard for VS Code, RustRover, etc.)
- **Capabilities:**
  - Extract function/variable
  - Rename symbols project-wide
  - Inline function/variable
  - Move module to file
  - Extract module
  - Auto-import
- **Recent improvements (2026):** Extract function now preserves `#[cfg]` and `#[track_caller]` attributes (v0.3.2761, Jan 2026)
- **Usage:** Automatic in any editor using rust-analyzer (VS Code rust-analyzer extension, RustRover, etc.)
- **Source:** [rust-analyzer changelog](https://rust-analyzer.github.io/thisweek/2026/01/19/changelog-311.html)
- **Confidence:** HIGH (standard tooling, recently updated)

### Automated Refactoring Workflow
1. **Identify duplication:**
   ```bash
   cargo clippy -- -W clippy::branches_sharing_code
   ```
   - Use clippy's `branches_sharing_code` (nursery), `collapsible_if`, `collapsible_match` lints
   - Clippy has 800+ lints, ~5 specifically target code duplication patterns

2. **Safe extraction:**
   - Use rust-analyzer's "Extract Function" for duplicated code blocks
   - Keyboard shortcut in VS Code: Ctrl+Shift+R for refactor menu
   - Each extraction is validated by compiler (ownership/borrowing checked)

3. **Validation:**
   - Already using `cargo fmt`, `cargo clippy`, `cargo nextest` in CI
   - Run after each refactoring step to ensure correctness

- **Source:** [Clippy Lints](https://rust-lang.github.io/rust-clippy/master/index.html)
- **Confidence:** HIGH (established workflow, tools already in use)

### Why NOT Specialized Refactoring Tools

Several specialized Rust refactoring tools were evaluated:

- **Rerast (Google)** - DEPRECATED, replaced by rust-analyzer's Structured Search Replace
- **rust-refactor** - Research project (2017), unmaintained
- **REM (Extract Method)** - Academic research tool (Jan 2026), requires IntelliJ plugin setup, overkill for this project
- **c2rust refactor** - Specific to C-to-Rust translation, not general refactoring

**Verdict:** rust-analyzer provides sufficient refactoring for this cleanup. Specialized tools add complexity without clear benefit for consolidating test fixtures.

**Source:** [Rust refactoring discussion](https://users.rust-lang.org/t/we-need-a-great-general-refactoring-tool/103985)
**Confidence:** HIGH (rust-analyzer is current standard, alternatives are niche/deprecated)

## NOT Recommended

### once_cell crate
- **Why not:** Functionality available in `std::sync::OnceLock` since Rust 1.70
- **When you WOULD use it:** If targeting Rust < 1.70 or needing features not in std (unlikely for test fixtures)
- **Confidence:** HIGH (official recommendation from once_cell maintainer)

### lazy_static crate
- **Why not:** Superseded by once_cell, then by std library (Rust 1.70+)
- **Official guidance:** "If your MSRV is at least 1.70, use std. Otherwise use once_cell. Don't use lazy_static."
- **Source:** [once_cell documentation](https://docs.rs/once_cell/latest/once_cell/)
- **Confidence:** HIGH

### typed-builder crate
- **Why not:** derive_builder is more mature, more flexible, and better documented
- **When you WOULD use it:** If you need compile-time type-state enforcement (overkill for test fixtures)
- **Trade-off:** typed-builder provides stronger compile-time guarantees but adds complexity
- **Confidence:** MEDIUM (both are valid, derive_builder is better fit here)

### galvanic-test crate
- **Why not:** Unmaintained since 2019, superseded by rstest
- **Source:** [galvanic-test GitHub](https://github.com/mindsbackyard/galvanic-test)
- **Confidence:** HIGH (clearly abandoned)

### Procedural Macro for Custom Fixtures
- **Why not (for this project):** Overkill for 10k line codebase with straightforward fixtures
- **When you WOULD use it:** Large codebases with domain-specific fixture patterns repeated 100+ times
- **Trade-off:** Procedural macros add compile time and complexity; declarative macros + derive_builder are simpler
- **Confidence:** HIGH (right-sized tooling for project scale)

## Integration Plan

### Phase 1: Add derive_builder
```toml
[dev-dependencies]
derive_builder = "0.20"
```

### Phase 2: Create Builder for Test Configs
```rust
// tests/common/configs.rs
#[cfg(test)]
#[derive(Builder, Clone)]
#[builder(pattern = "owned", build_fn(validate = "Self::validate"))]
pub struct TestCiConfig {
    #[builder(default = "default_docker()")]
    pub docker: DockerConfig,
    #[builder(default = "default_git()")]
    pub git: GitConfig,
    // ... with sensible defaults for all fields
}

impl TestCiConfigBuilder {
    pub fn minimal() -> Self { /* ... */ }
    pub fn with_php() -> Self { /* ... */ }
    pub fn with_rust() -> Self { /* ... */ }
}
```

### Phase 3: Migrate Tests Incrementally
- Replace inline YAML strings with builder calls
- Consolidate duplicate helpers into `tests/common/` submodules
- Use rstest fixtures for repeated setup

### Phase 4: Add Clippy Lints for Ongoing Quality
```toml
[lints.clippy]
branches_sharing_code = "warn"
collapsible_if = "warn"
collapsible_match = "warn"
```

## Sources

### Library Documentation (HIGH confidence)
- [derive_builder 0.20.2 documentation](https://docs.rs/derive_builder/latest/derive_builder/)
- [rstest 0.26.1 documentation](https://docs.rs/rstest/latest/rstest/)
- [std::sync::OnceLock documentation](https://doc.rust-lang.org/std/sync/struct.OnceLock.html)
- [Clippy lints reference](https://rust-lang.github.io/rust-clippy/master/index.html)

### Official Rust Documentation (HIGH confidence)
- [Test Organization - Rust Book](https://doc.rust-lang.org/book/ch11-03-test-organization.html)
- [Builder Pattern - Rust Design Patterns](https://rust-unofficial.github.io/patterns/patterns/creational/builder.html)

### Community Resources (MEDIUM confidence, verified patterns)
- [Testing with Builder Pattern for Fixtures](https://dan.munckton.co.uk/blog/2018/03/01/testing-rust-using-the-builder-pattern-for-complex-fixtures/)
- [Testing With Fixtures in Rust](https://dawchihliou.github.io/articles/testing-with-fixtures-in-rust)
- [rust-analyzer changelog #311 (Jan 2026)](https://rust-analyzer.github.io/thisweek/2026/01/19/changelog-311.html)

### Tool Evaluations (MEDIUM-HIGH confidence)
- [Rust refactoring tools discussion](https://users.rust-lang.org/t/we-need-a-great-general-refactoring-tool/103985)
- [once_cell vs lazy_static guidance](https://docs.rs/once_cell/latest/once_cell/)
- [RustRover refactoring documentation](https://www.jetbrains.com/help/rust/refactoring-source-code.html)

## Confidence Assessment

| Area | Confidence | Reason |
|------|------------|--------|
| derive_builder recommendation | HIGH | Current version verified via WebFetch, widely used pattern |
| std::sync::OnceLock over once_cell | HIGH | Official Rust docs, stable since 1.70 |
| Builder pattern for configs | HIGH | Established Rust testing pattern, multiple authoritative sources |
| rust-analyzer for refactoring | HIGH | Standard tooling, recent version verified |
| NOT using specialized tools | MEDIUM-HIGH | Ecosystem survey shows alternatives are deprecated/niche |
| Macro patterns | MEDIUM | Pattern works but requires judgment on when to apply |

## Implementation Notes

**Critical insight:** The existing `tests/common/mod.rs` already demonstrates the duplication problem. Functions like `widget_test_config_yaml()` return 40+ lines of inline YAML. The solution is NOT file consolidation but programmatic config generation via builders.

**Recommended approach:**
1. Start with ONE test file (likely `widget_tests.rs` since it uses `widget_test_config_yaml()`)
2. Create `TestCiConfigBuilder` with derive_builder
3. Replace `widget_test_config_yaml()` with builder calls
4. Measure: lines of code, test clarity, maintenance burden
5. If successful, expand pattern to other test files

**Success metrics:**
- Reduction in duplicate config YAML (target: 40+ lines to <10 lines per test)
- Consolidation of test helpers (target: 40+ lines to reusable modules)
- Maintainability improvement (new tests require less boilerplate)
- No regression in test coverage or execution time
