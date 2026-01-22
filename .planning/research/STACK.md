# Technology Stack: Rust Code Quality & Testing

**Project:** CI-TUI
**Researched:** 2026-01-22
**Domain:** Rust TUI application code quality improvement
**Confidence:** HIGH

## Executive Summary

The Rust ecosystem in 2025-2026 has mature, standardized tooling for code quality and testing. For TUI applications like CI-TUI, the recommended stack combines:

1. **Built-in Rust tooling** (rustfmt, clippy) remains foundational
2. **Modern test runner** (cargo-nextest) for 3x faster test execution
3. **TUI-specific testing** (ratatui TestBackend + ratatui-testlib) for widget and integration testing
4. **Property-based testing** (proptest) for complex logic validation
5. **Comprehensive coverage** (cargo-llvm-cov) for LLVM-based code coverage
6. **Strict dependency management** (cargo-deny) for security and licensing

This stack is production-ready, widely adopted, and specifically addresses brownfield Rust TUI projects requiring test coverage retrofitting.

---

## Core Testing Framework

### Built-in Rust Test Framework
| Technology | Version | Purpose | Why |
|------------|---------|---------|-----|
| Rust built-in `#[test]` | Stable (Rust 2021+) | Unit testing foundation | Zero-setup, integrated with cargo, universally supported. No reason to replace. |
| `#[cfg(test)]` modules | Stable | Test organization | Standard pattern for unit test isolation, collocates tests with implementation |

**Rationale:** Rust's built-in testing is sufficient for basic unit tests. Build on this foundation rather than replacing it.

**Confidence:** HIGH (official Rust tooling)

### Enhanced Test Runner

| Technology | Version | Purpose | Why |
|------------|---------|---------|-----|
| cargo-nextest | 0.9.x+ | Modern test runner | 3x faster than `cargo test`, cleaner output, JUnit XML for CI, test retries, partitioning for parallel CI runs |

**Installation:**
```bash
cargo install cargo-nextest --locked
```

**Usage:**
```bash
cargo nextest run
```

**Limitations:** Does not support doctests (run separately with `cargo test --doc`).

**Rationale:** For a project with growing test coverage, nextest's performance and CI features are invaluable. The Tokio project uses it, signaling production readiness.

**Confidence:** HIGH (official docs verified, widely adopted)

**Sources:**
- [cargo-nextest official site](https://nexte.st/)
- [GitHub: nextest-rs/nextest](https://github.com/nextest-rs/nextest)

---

## TUI-Specific Testing

### Ratatui TestBackend (Unit Testing)

| Technology | Version | Purpose | Why |
|------------|---------|---------|-----|
| `ratatui::backend::TestBackend` | Built into ratatui 0.30+ | Widget/layout unit testing | Built-in, zero-config, captures terminal buffer for assertions |

**Usage:**
```rust
use ratatui::backend::TestBackend;
use ratatui::Terminal;

#[test]
fn test_widget_rendering() {
    let backend = TestBackend::new(80, 24);
    let mut terminal = Terminal::new(backend).unwrap();

    terminal.draw(|frame| {
        // Render widgets
    }).unwrap();

    // Assert on backend.buffer()
}
```

**Rationale:** Perfect for testing widget rendering logic, layout calculations, and visual output without real terminal interaction.

**Confidence:** HIGH (built into ratatui)

### ratatui-testlib (Integration Testing)

| Technology | Version | Purpose | Why |
|------------|---------|---------|-----|
| ratatui-testlib | 0.1.x (early stage) | PTY-based integration testing | Real terminal emulation, tests keyboard input → output flow, snapshot testing integration |

**Caution:** Version 0.1.0 indicates early development. Monitor stability before heavy reliance.

**Usage:**
```rust
use ratatui_testlib::Terminal;

#[test]
fn test_interactive_flow() {
    let term = Terminal::spawn("./target/debug/ci-tui")?;
    term.send_keys("j"); // Send 'j' key
    term.assert_contains("Selected: check 2")?; // Verify output
}
```

**Rationale:** TestBackend cannot test real terminal behavior (keyboard input, async event handling, crossterm integration). ratatui-testlib fills this gap with PTY emulation.

**Recommendation for CI-TUI:**
- Use TestBackend for widget unit tests (checks.rs rendering, UI component tests)
- Use ratatui-testlib for end-to-end flows (keyboard navigation, check selection, status updates)

**Confidence:** MEDIUM (early version, but solves critical TUI testing gap)

**Sources:**
- [ratatui-testlib documentation](https://docs.rs/ratatui-testlib/latest/ratatui_testlib/)
- [Testing Strategies for Ratatui TUI Applications](https://lib.rs/crates/ratatui-testlib)

---

## Unit Testing Enhancements

### Test Fixtures & Parameterization

| Technology | Version | Purpose | Why |
|------------|---------|---------|-----|
| rstest | 0.23.x+ | Fixtures and parameterized tests | 48M+ downloads, ergonomic `#[rstest]` syntax, `#[case]` for table-driven tests, async support |

**Usage:**
```rust
use rstest::*;

#[fixture]
fn sample_config() -> CiConfig {
    CiConfig::from_yaml("test-fixtures/config.yaml").unwrap()
}

#[rstest]
#[case("src/main.rs", vec!["clippy", "rustfmt"])]
#[case("tests/integration.rs", vec!["test"])]
fn test_determine_checks(sample_config: CiConfig, #[case] path: &str, #[case] expected: Vec<&str>) {
    let checks = determine_checks(&sample_config, &[path]);
    assert_eq!(checks.len(), expected.len());
}
```

**Rationale:** CI-TUI's `checks.rs` and `test_discovery.rs` have complex input combinations (file patterns, config variations). rstest eliminates boilerplate for parameterized testing.

**Confidence:** HIGH (top 3 most popular Rust testing crate)

**Sources:**
- [GitHub: la10736/rstest](https://github.com/la10736/rstest)
- [Rstest Rust Guide 2025](https://generalistprogrammer.com/tutorials/rstest-rust-crate-guide)

### Enhanced Assertions

| Technology | Version | Purpose | Why |
|------------|---------|---------|-----|
| pretty_assertions | 1.4.x | Colorful assertion diffs | 101M+ downloads, drop-in replacement for `assert_eq!`, highlights differences in complex structs |

**Usage:**
```rust
use pretty_assertions::assert_eq;

#[test]
fn test_config_parsing() {
    let actual = parse_config("config.yaml");
    let expected = CiConfig { /* ... */ };
    assert_eq!(actual, expected); // Colorful diff on failure
}
```

**Rationale:** Config parsing tests will compare large `CiConfig` structs. Standard `assert_eq!` is unreadable for failures. pretty_assertions is a one-line change for massive clarity improvement.

**Confidence:** HIGH (101M+ downloads, stable API)

**Sources:**
- [pretty_assertions on lib.rs](https://lib.rs/crates/pretty_assertions)
- [Rust Tests with pretty_assertions](https://medium.com/@adamszpilewicz/rust-tests-with-pretty-assertions-dc7462748ec0)

### Snapshot Testing

| Technology | Version | Purpose | Why |
|------------|---------|---------|-----|
| insta | 1.40.x+ | Snapshot testing | De facto standard for snapshot testing, ergonomic review workflow via `cargo-insta`, excellent for config parsing and text output validation |

**Installation:**
```bash
cargo install cargo-insta
```

**Usage:**
```rust
use insta::assert_snapshot;

#[test]
fn test_check_output_format() {
    let output = format_check_result(&check);
    assert_snapshot!(output);
}
```

**Workflow:**
```bash
cargo insta test    # Run tests and capture snapshots
cargo insta review  # Review new/changed snapshots
```

**Rationale:** CI-TUI formats check output, error messages, and UI text. Snapshot testing captures expected output without manual string construction. Excellent for regression detection.

**Confidence:** HIGH (mature library, active maintenance, refreshed docs in Jan 2026)

**Sources:**
- [Insta Snapshots official site](https://insta.rs/)
- [GitHub: mitsuhiko/insta](https://github.com/mitsuhiko/insta)

---

## Advanced Testing

### Mocking

| Technology | Version | Purpose | Why |
|------------|---------|---------|-----|
| mockall | 0.14.x | Mock generation for traits | Most feature-rich Rust mocking library, 100% safe Rust, official Android (AOSP) recommendation, `#[automock]` for trait-based mocks |

**Usage:**
```rust
use mockall::*;

#[automock]
trait DockerRunner {
    fn exec(&self, cmd: &str) -> Result<Output>;
}

#[test]
fn test_check_runner() {
    let mut mock = MockDockerRunner::new();
    mock.expect_exec()
        .with(eq("docker compose exec app cargo clippy"))
        .times(1)
        .returning(|_| Ok(Output::success()));

    let runner = CheckRunner::new(Box::new(mock));
    runner.run_check(&check).unwrap();
}
```

**Rationale:** CI-TUI interacts with Docker, git, and the filesystem. Mocking these dependencies enables fast, isolated unit tests for `runner.rs` and `git.rs` without Docker containers.

**When NOT to use:** Don't mock internal application logic. Mock external dependencies (Docker, git, filesystem).

**Alternative considered:** mockito (simpler but less powerful). Mockall's features justify the learning curve.

**Confidence:** HIGH (AOSP official recommendation, comprehensive feature set)

**Sources:**
- [GitHub: asomers/mockall](https://github.com/asomers/mockall)
- [Mocking in Rust: Mockall and alternatives](https://blog.logrocket.com/mocking-rust-mockall-alternatives/)

### Property-Based Testing

| Technology | Version | Purpose | Why |
|------------|---------|---------|-----|
| proptest | 1.6.x+ | Property testing | More flexible than quickcheck, better shrinking, explicit strategies, hypothesis-inspired, MSRV 1.84.0 |

**Usage:**
```rust
use proptest::prelude::*;

proptest! {
    #[test]
    fn test_file_pattern_matching(path in ".*\\.rs", pattern in "[a-z]+") {
        let config = CiConfig { file_patterns: vec![pattern.clone()] };
        // Property: matching is deterministic
        let result1 = matches_pattern(&config, &path);
        let result2 = matches_pattern(&config, &path);
        assert_eq!(result1, result2);
    }
}
```

**Rationale:** CI-TUI's regex pattern matching and file path logic has edge cases. Property testing finds corner cases by generating hundreds of random inputs. Complements example-based tests.

**When to use:** `checks.rs` (pattern matching), `test_discovery.rs` (path manipulation), `config.rs` (validation).

**Confidence:** HIGH (mature, widely used, official docs)

**Sources:**
- [GitHub: proptest-rs/proptest](https://github.com/proptest-rs/proptest)
- [Proptest Rust Guide 2025](https://generalistprogrammer.com/tutorials/proptest-rust-crate-guide)

---

## Async Testing (Tokio)

### Tokio Test Support

| Technology | Version | Purpose | Why |
|------------|---------|---------|-----|
| `#[tokio::test]` macro | Built into tokio 1.x | Async test execution | Already a dependency, zero-config, handles runtime setup automatically |
| tokio::time::pause | Built into tokio (test-util feature) | Deterministic time control | Eliminate timing-based flakiness in async tests |

**Usage:**
```rust
#[tokio::test]
async fn test_async_check_runner() {
    let runner = CheckRunner::new();
    let result = runner.run_check_async(&check).await;
    assert!(result.is_ok());
}

#[tokio::test(start_paused = true)]
async fn test_check_timeout() {
    tokio::time::pause();

    let runner = CheckRunner::new();
    let fut = runner.run_with_timeout(&check, Duration::from_secs(5));

    // Advance time without real delays
    tokio::time::advance(Duration::from_secs(6)).await;

    assert!(fut.await.is_err()); // Should timeout
}
```

**Rationale:** CI-TUI uses tokio for async runtime. The `#[tokio::test]` macro is the standard way to test async code. `tokio::time::pause` makes time-based tests deterministic and fast.

**Confidence:** HIGH (official Tokio feature, TokioConf 2026 announced)

**Sources:**
- [Tokio Unit Testing](https://tokio.rs/tokio/topics/testing)
- [Mastering Asynchronous Testing in Rust](https://moldstud.com/articles/p-mastering-asynchronous-testing-in-rust-strategies-and-frameworks-with-tokio)

---

## Code Coverage

### LLVM-Based Coverage

| Technology | Version | Purpose | Why |
|------------|---------|---------|-----|
| cargo-llvm-cov | 0.6.x+ | Code coverage reporting | LLVM instrumentation (most accurate), supports cargo-nextest, multiple output formats (HTML, JSON, LCOV), cross-platform |

**Installation:**
```bash
cargo install cargo-llvm-cov --locked
```

**Usage:**
```bash
# HTML report
cargo llvm-cov --html

# CI-friendly output
cargo llvm-cov --lcov --output-path lcov.info

# With nextest
cargo llvm-cov nextest
```

**Output formats:** HTML (human review), LCOV (CI integration like Codecov), JSON (programmatic analysis)

**Rationale:** LLVM instrumentation is more accurate than ptrace-based tools (cargo-tarpaulin). Works on all platforms (Linux, macOS, Windows). Integrates with cargo-nextest for fast coverage collection.

**Alternative considered:** cargo-tarpaulin (Linux-only by default, less accurate). cargo-llvm-cov is superior.

**Confidence:** HIGH (1.3k stars, active maintenance, LLVM-backed accuracy)

**Sources:**
- [GitHub: taiki-e/cargo-llvm-cov](https://github.com/taiki-e/cargo-llvm-cov)
- [Rust Code Coverage Comparison 2025](https://rustprojectprimer.com/measure/coverage.html)

---

## Linting & Formatting

### Core Linting

| Technology | Version | Purpose | Why |
|------------|---------|---------|-----|
| rustfmt | Built into Rust toolchain | Code formatting | Official formatter, zero-config, community standard, prevents style debates |
| clippy | Built into Rust toolchain | Lint collection | 700+ lints, catches common mistakes, suggests idiomatic code, extensible with custom lints |

**Configuration (.clippy.toml):**
```toml
# Recommended for brownfield projects
warn-on-all-wildcard-imports = true
disallowed-names = ["foo", "bar", "baz"]
```

**Recommended clippy configuration (Cargo.toml):**
```toml
[lints.clippy]
# Enable pedantic group but allow specific lints
pedantic = { level = "warn", priority = -1 }
module_name_repetitions = "allow"
missing_errors_doc = "allow"
must_use_candidate = "allow"

# Enforce stricter lints
unwrap_used = "deny"
expect_used = "deny"
panic = "deny"
```

**Rationale:**
- **pedantic group:** More opinionated lints, good for brownfield cleanup. Set to "warn" not "deny" to avoid blocking compilation during migration.
- **Deny unwrap/expect/panic:** CI-TUI already uses `Result` types. Enforcing this prevents future regressions.
- **Allow module_name_repetitions:** Common in Rust projects, too noisy.

**Confidence:** HIGH (official tooling)

**Sources:**
- [Clippy Documentation](https://doc.rust-lang.org/clippy/)
- [Rust Linting Best Practices 2025](https://rust-lang.github.io/rust-clippy/master/index.html)

### Additional Linting (Optional, Lower Priority)

| Technology | Version | Purpose | Why |
|------------|---------|---------|-----|
| clippy::nursery | Built into clippy | Experimental lints | Cherry-pick lints from nursery group, not ready for whole-group enablement |
| cargo-hack | 0.6.x | Feature combination testing | Validate all feature flag combinations work, prevent "works on my machine" issues |

**cargo-hack usage:**
```bash
# Test all feature combinations
cargo hack --feature-powerset check

# Test on minimal versions
cargo hack --version-range 1.70..
```

**Rationale:** CI-TUI doesn't use feature flags heavily yet. Add cargo-hack if feature flags are introduced. Lower priority for initial testing phase.

**Confidence:** MEDIUM (useful but not essential for current CI-TUI state)

**Sources:**
- [GitHub: taiki-e/cargo-hack](https://github.com/taiki-e/cargo-hack)

---

## Dependency Management & Security

### Security Auditing

| Technology | Version | Purpose | Why |
|------------|---------|---------|-----|
| cargo-deny | 0.19.x (released Jan 2026) | Dependency linting | Checks licenses, advisories (security vulns), bans, and sources. Embark Studios production tool. |
| cargo-audit | 0.21.x | Security vulnerability scanning | RustSec database integration, focused on CVEs |

**Recommendation:** Use **cargo-deny** (superset of cargo-audit functionality + license/ban checks).

**Installation:**
```bash
cargo install cargo-deny --locked
```

**Setup:**
```bash
cargo deny init  # Creates deny.toml
```

**Configuration (deny.toml):**
```toml
[advisories]
version = 2
ignore = []  # Ignore specific CVEs if false positives

[licenses]
allow = ["MIT", "Apache-2.0", "BSD-3-Clause"]
deny = ["GPL-3.0"]

[bans]
multiple-versions = "deny"  # Prevent duplicate versions
```

**Usage:**
```bash
cargo deny check advisories  # Check for CVEs
cargo deny check licenses    # Check for license violations
cargo deny check bans        # Check for duplicate versions
cargo deny check              # Run all checks
```

**Rationale:** CI-TUI has 43 dependencies (from Cargo.toml). Security vulnerabilities in dependencies are a real risk. cargo-deny catches these in CI before production. The "bans" check prevents dependency bloat from duplicate versions.

**Confidence:** HIGH (2.2k stars, actively maintained, production-ready)

**Sources:**
- [GitHub: EmbarkStudios/cargo-deny](https://github.com/EmbarkStudios/cargo-deny)
- [Rust Dependency Management Best Practices](https://lurklurk.org/effective-rust/dep-graph.html)

### Dependency Maintenance

| Technology | Version | Purpose | Why |
|------------|---------|---------|-----|
| cargo-outdated | 0.17.x | Outdated dependency detection | Shows available updates for dependencies |
| Dependabot | GitHub native | Automated dependency updates | Auto-creates PRs for dependency updates |

**Recommendation:** Use **Dependabot** (free on GitHub, zero maintenance).

**Setup (.github/dependabot.yml):**
```yaml
version: 2
updates:
  - package-ecosystem: "cargo"
    directory: "/"
    schedule:
      interval: "weekly"
```

**Rationale:** Manual dependency updates are tedious. Dependabot automates this with weekly PRs. CI runs tests on each PR, ensuring updates don't break functionality.

**Confidence:** HIGH (GitHub-native, widely used)

---

## Static Analysis & Advanced Tooling

### Memory Safety & Undefined Behavior

| Technology | Version | Purpose | Why |
|------------|---------|---------|-----|
| miri | Built into rustup | Undefined behavior detection | Experimental but official, detects memory safety violations at runtime in tests |
| cargo-geiger | 0.11.x | Unsafe code usage detection | Visualizes unsafe block usage in dependency tree |

**Miri usage:**
```bash
rustup +nightly component add miri
cargo +nightly miri test
```

**Rationale:** CI-TUI doesn't use unsafe code, but dependencies might. Miri catches UB that even Rust's type system misses. Run occasionally (not in every CI run due to slowness).

**When to use:**
- After major dependency updates
- If experiencing unexplained crashes
- For paranoia-level safety verification

**cargo-geiger usage:**
```bash
cargo install cargo-geiger
cargo geiger
```

**Rationale:** Visualizes unsafe code in dependencies. Useful for security audits. Low priority for active development.

**Confidence:** MEDIUM (miri is experimental, cargo-geiger is niche but useful)

**Sources:**
- [Rust Static Analysis Tools Comparison 2025](https://markaicode.com/rust-static-analysis-tools-comparison-2025/)
- [Rust Auditing Tools 2025](https://markaicode.com/rust-auditing-tools-2025-automated-security-scanning/)

---

## Documentation Testing

### Doc Tests

| Technology | Version | Purpose | Why |
|------------|---------|---------|-----|
| `cargo test --doc` | Built into Rust | Documentation example testing | Ensures code examples in doc comments compile and run, prevents outdated documentation |

**Usage in code:**
```rust
/// Parses a CI config from YAML.
///
/// # Example
/// ```
/// use ci_tui::config::CiConfig;
///
/// let yaml = "docker:\n  project_dir: /app\n  service: app";
/// let config = CiConfig::from_yaml(yaml).unwrap();
/// assert_eq!(config.docker.service, "app");
/// ```
pub fn from_yaml(yaml: &str) -> Result<CiConfig> {
    // ...
}
```

**Run doc tests:**
```bash
cargo test --doc
```

**Rationale:** Doc tests serve dual purpose: documentation AND tests. Critical for public APIs. CI-TUI is an internal tool, so doc tests are lower priority than unit tests, but useful for complex modules like `config.rs` and `checks.rs`.

**Limitation:** cargo-nextest does NOT run doc tests. Must run separately.

**Confidence:** HIGH (official Rust feature)

**Sources:**
- [Rust Documentation Testing](https://doc.rust-lang.org/rust-by-example/testing/doc_testing.html)
- [How to Write Effective Doc Tests 2025](https://moldstud.com/articles/p-how-to-write-effective-documentation-tests-in-rust-a-comprehensive-guide)

---

## Benchmarking (Optional)

### Performance Testing

| Technology | Version | Purpose | Why |
|------------|---------|---------|-----|
| criterion | 0.5.x | Benchmarking | De facto standard, statistical analysis, historical tracking, HTML reports |
| divan | 0.1.x+ | Modern benchmarking | Simpler API, faster execution, ergonomic parameterization |

**Recommendation:** **Criterion** for now (mature, widely adopted). Consider **divan** for future (modern ergonomics).

**Criterion usage:**
```rust
use criterion::{black_box, criterion_group, criterion_main, Criterion};

fn bench_determine_checks(c: &mut Criterion) {
    let config = load_sample_config();
    let changed_files = vec!["src/main.rs", "tests/test.rs"];

    c.bench_function("determine_checks", |b| {
        b.iter(|| determine_checks(black_box(&config), black_box(&changed_files)))
    });
}

criterion_group!(benches, bench_determine_checks);
criterion_main!(benches);
```

**When to use:** If `determine_checks()` or `find_related_tests()` become performance bottlenecks. Not a priority for initial testing phase.

**Confidence:** MEDIUM (useful but optional for CI-TUI's current needs)

**Sources:**
- [Criterion.rs Documentation](https://bheisler.github.io/criterion.rs/book/)
- [Divan vs Criterion Comparison](https://nikolaivazquez.com/blog/divan/)

---

## Anti-Recommendations (What NOT to Use)

| Tool/Practice | Why Avoid |
|---------------|-----------|
| cargo-tarpaulin as primary coverage | Linux-only by default (ptrace), less accurate than LLVM. Use cargo-llvm-cov instead. |
| quickcheck for new property tests | Less flexible than proptest, worse shrinking. Proptest is superior. |
| mockito for complex mocking | Less powerful than mockall. Use mockall for trait mocking. |
| Custom test harness | Rust's built-in testing + cargo-nextest is sufficient. Custom harnesses add maintenance burden. |
| Enabling ALL clippy::pedantic lints without review | Generates noise and false positives. Cherry-pick useful lints. |
| Enabling ALL clippy::nursery lints | These are experimental. Too unstable for production use. |
| cargo-hack without feature flags | CI-TUI doesn't use features yet. Wait until features are added. |

---

## Installation Checklist

### Immediate Priority (Core Stack)

```bash
# Test runner
cargo install cargo-nextest --locked

# Coverage
cargo install cargo-llvm-cov --locked

# Security
cargo install cargo-deny --locked

# Snapshot testing
cargo install cargo-insta --locked
```

### Add to Cargo.toml (dev-dependencies)

```toml
[dev-dependencies]
# Core testing
tokio-test = "0.4"

# Enhanced assertions
pretty_assertions = "1.4"

# Fixtures and parameterization
rstest = "0.23"

# Mocking
mockall = "0.14"

# Property testing
proptest = "1.6"

# Snapshot testing
insta = "1.40"

# TUI testing (when ready for integration tests)
ratatui-testlib = "0.1"
```

### CI Configuration (.github/workflows/ci.yml)

```yaml
name: CI

on: [push, pull_request]

jobs:
  test:
    runs-on: ubuntu-latest
    steps:
      - uses: actions/checkout@v4

      - uses: dtolnay/rust-toolchain@stable

      - name: Install nextest
        run: cargo install cargo-nextest --locked

      - name: Run tests
        run: cargo nextest run

      - name: Run doc tests
        run: cargo test --doc

  coverage:
    runs-on: ubuntu-latest
    steps:
      - uses: actions/checkout@v4

      - uses: dtolnay/rust-toolchain@stable

      - name: Install llvm-cov
        run: cargo install cargo-llvm-cov --locked

      - name: Generate coverage
        run: cargo llvm-cov --lcov --output-path lcov.info

      - name: Upload to Codecov
        uses: codecov/codecov-action@v4
        with:
          files: lcov.info

  lint:
    runs-on: ubuntu-latest
    steps:
      - uses: actions/checkout@v4

      - uses: dtolnay/rust-toolchain@stable
        with:
          components: rustfmt, clippy

      - name: Check formatting
        run: cargo fmt -- --check

      - name: Run clippy
        run: cargo clippy -- -D warnings

  security:
    runs-on: ubuntu-latest
    steps:
      - uses: actions/checkout@v4

      - name: Install cargo-deny
        run: cargo install cargo-deny --locked

      - name: Check dependencies
        run: cargo deny check
```

---

## Confidence Assessment

| Category | Confidence | Rationale |
|----------|------------|-----------|
| Core Testing (rstest, nextest) | HIGH | Verified with official docs, 100M+ downloads, widely adopted |
| TUI Testing (TestBackend) | HIGH | Built into ratatui 0.30, official feature |
| TUI Testing (ratatui-testlib) | MEDIUM | Version 0.1.0 (early), but solves critical testing gap |
| Mocking (mockall) | HIGH | AOSP official recommendation, 1k+ stars, stable API |
| Coverage (cargo-llvm-cov) | HIGH | LLVM-backed, active maintenance, 1.3k stars |
| Security (cargo-deny) | HIGH | Production tool from Embark Studios, Jan 2026 release |
| Property Testing (proptest) | HIGH | Mature, widely used, official docs |
| Linting (clippy pedantic) | HIGH | Official tooling, documented best practices |
| Static Analysis (miri) | MEDIUM | Experimental but official, useful for periodic checks |

---

## Summary: Recommended Stack for CI-TUI

**Phase 1 (Immediate):**
- cargo-nextest (test runner)
- rstest (fixtures/parameterization)
- pretty_assertions (better diffs)
- mockall (mock Docker/git dependencies)
- cargo-llvm-cov (coverage)
- cargo-deny (security)

**Phase 2 (After basic tests working):**
- ratatui TestBackend (widget unit tests)
- insta (snapshot testing for output validation)
- proptest (property testing for checks.rs/test_discovery.rs)

**Phase 3 (Integration testing):**
- ratatui-testlib (PTY-based integration tests)
- cargo-geiger (unsafe code audit)

**CI Integration:**
- cargo-nextest in CI
- cargo-deny in CI (fail on security issues)
- cargo-llvm-cov for Codecov integration
- Dependabot for dependency updates

**Not Needed Yet:**
- cargo-hack (no feature flags)
- Benchmarking (no performance bottlenecks identified)
- miri (run occasionally, not in CI)

---

## Sources

### Testing Libraries
- [Rust Testing Best Practices 2025](https://medium.com/@asma.shaikh_19478/rust-testing-best-practices-unit-to-integration-965b39a8212f)
- [cargo-nextest official site](https://nexte.st/)
- [rstest GitHub](https://github.com/la10736/rstest)
- [ratatui-testlib documentation](https://docs.rs/ratatui-testlib/latest/ratatui_testlib/)

### Mocking & Property Testing
- [Mockall GitHub](https://github.com/asomers/mockall)
- [Mocking in Rust: Mockall and alternatives](https://blog.logrocket.com/mocking-rust-mockall-alternatives/)
- [Proptest GitHub](https://github.com/proptest-rs/proptest)

### Code Coverage
- [cargo-llvm-cov GitHub](https://github.com/taiki-e/cargo-llvm-cov)
- [Rust Code Coverage Comparison](https://rustprojectprimer.com/measure/coverage.html)

### Security & Dependencies
- [cargo-deny GitHub](https://github.com/EmbarkStudios/cargo-deny)
- [Rust Dependency Management](https://lurklurk.org/effective-rust/dep-graph.html)

### Linting & Static Analysis
- [Clippy Documentation](https://doc.rust-lang.org/clippy/)
- [Rust Static Analysis Tools 2025](https://markaicode.com/rust-static-analysis-tools-comparison-2025/)
- [Tokio Unit Testing](https://tokio.rs/tokio/topics/testing)

### Snapshot & Assertion Testing
- [Insta Snapshots](https://insta.rs/)
- [pretty_assertions on lib.rs](https://lib.rs/crates/pretty_assertions)
