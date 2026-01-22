# Testing Patterns

**Analysis Date:** 2026-01-22

## Test Framework

**Runner:**
- Rust built-in test framework (no external test runner)
- Cargo test integration: `cargo test` runs all tests
- Config: `Makefile` target at line 46: `docker run --rm -v $(PWD):/build -w /build rust:alpine cargo test`

**Assertion Library:**
- Rust standard assertions: `assert!()`, `assert_eq!()`, `assert_ne!()`
- No external assertion library - using built-in only

**Run Commands:**
```bash
cargo test                 # Run all tests (via Make: make test)
cargo test <test_name>     # Run single test by name
cargo test -- --nocapture # Run with println! output visible
```

## Test File Organization

**Location:**
- Co-located with source code using `#[cfg(test)]` modules at bottom of each file
- Test modules are internal to the implementation file, not in separate test directories

**Files with tests:**
- `src/config.rs`: 13 test cases
- `src/git.rs`: 14 test cases
- `src/checks.rs`: 8 test cases
- `src/test_discovery.rs`: Tests present (search-related)
- `src/ui/app.rs`: Tests present for app state

**Naming:**
- Test function names start with `test_`: `test_get_file_pattern()`, `test_should_ignore_file()`
- Descriptive names indicate what is tested: `test_triggered_check_with_matching_files()`
- Helper functions are lowercase utility names: `parse_test_config()`, `make_changed_files()`

**Structure:**
```
src/
├── config.rs
│   └── #[cfg(test)]
│       └── mod tests { ... }
├── git.rs
│   └── #[cfg(test)]
│       └── mod tests { ... }
├── checks.rs
│   └── #[cfg(test)]
│       └── mod tests { ... }
```

## Test Structure

**Suite Organization:**
```rust
#[cfg(test)]
mod tests {
    use super::*;
    use crate::other_modules;

    // Helper functions (test fixtures)
    fn parse_test_config() -> CiConfig {
        // ...
    }

    // Test functions
    #[test]
    fn test_feature_behavior() {
        // Arrange
        let config = parse_test_config();

        // Act
        let result = config.some_method();

        // Assert
        assert_eq!(result, expected);
    }
}
```

**Patterns:**
- Setup: Test fixtures created via helper functions like `parse_test_config()` and `make_changed_files()`
- Teardown: None required - test data is isolated and dropped at scope end
- Assertion: Standard Rust assertions with explicit assertions for each behavior

**Example from config.rs (lines 246-366):**
```rust
#[cfg(test)]
mod tests {
    use super::*;

    fn minimal_config_yaml() -> &'static str {
        r#"
version: 2

docker:
  project_dir: ./infrastructure
  service: php

git:
  base_branch: development
  fallback_branch: HEAD~1

file_patterns:
  php:
    pattern: '\.php$'
  # ...
checks:
  fast:
    name: Fast Checks
    parallel: true
    checks:
      php-lint:
        name: PHP syntax check
        command: php-lint {files}
        triggers:
          file_pattern: php
  # ...
"#
    }

    fn parse_test_config() -> CiConfig {
        serde_yaml::from_str(minimal_config_yaml()).expect("Failed to parse test config")
    }

    #[test]
    fn test_get_file_pattern() {
        let config = parse_test_config();

        assert_eq!(config.get_file_pattern("php"), Some(r"\.php$"));
        assert_eq!(config.get_file_pattern("php_src"), Some("^src/"));
        assert_eq!(config.get_file_pattern("nonexistent"), None);
    }
}
```

## Mocking

**Framework:** No external mocking library used

**Patterns:**
- Use test fixtures and helper constructors to build test data
- Create inline test structs that implement needed traits
- Example from `git.rs` (line 151):
```rust
fn make_changed_files(files: Vec<&str>) -> ChangedFiles {
    ChangedFiles {
        files: files.into_iter().map(String::from).collect(),
        base_ref: "origin/development".to_string(),
    }
}
```

- Configuration mocking via YAML strings that are parsed:
```rust
fn test_config_yaml() -> &'static str {
    r#"
version: 2
docker:
  project_dir: ./infrastructure
  service: php
# ... rest of YAML
"#
}

fn parse_config() -> CiConfig {
    serde_yaml::from_str(test_config_yaml()).expect("Failed to parse")
}
```

**What to Mock:**
- File system operations can be tested with actual files or YAML fixtures
- Configuration is tested via real YAML parsing to catch serde issues
- Git commands are NOT tested (external process - left for integration testing)

**What NOT to Mock:**
- Serde YAML deserialization - test with real YAML strings to validate parsing works
- Configuration struct behavior - test with realistic configurations
- Path operations - can use real PathBuf creation

## Fixtures and Factories

**Test Data:**
- YAML fixture strings defined as `&'static str` functions returning hardcoded YAML
- Simple data builders for objects without serde: `make_changed_files(vec!["file.rs"])`
- Configuration parser wrapper: `parse_config()` wraps serde_yaml::from_str

**Example from checks.rs (lines 243-309):**
```rust
fn test_config_yaml() -> &'static str {
    r#"
version: 2

docker:
  project_dir: ./infrastructure
  service: php

git:
  base_branch: development
  fallback_branch: HEAD~1

file_patterns:
  php:
    pattern: '\.php$'
  php_src:
    pattern: '^src/.*\.php$'
  tests:
    pattern: 'tests/.*\.php$'
  yaml:
    pattern: '\.ya?ml$'

checks:
  warmup:
    checks:
      cache-warmup:
        name: Cache warmup
        command: bin/console cache:warmup
  # ... more groups
"#
}

fn parse_config() -> CiConfig {
    serde_yaml::from_str(test_config_yaml()).expect("Failed to parse test config")
}

fn make_changed_files(files: Vec<&str>) -> ChangedFiles {
    ChangedFiles {
        files: files.into_iter().map(String::from).collect(),
        base_ref: "development".to_string(),
    }
}
```

**Location:**
- All fixtures are defined within the `#[cfg(test)]` module at the bottom of the source file
- No separate fixture files or directories
- Fixtures are simple helper functions, not complex factory builders

## Coverage

**Requirements:** No explicit coverage targets enforced

**View Coverage:**
- Not configured in this project (no tarpaulin or coverage.rs integration)
- Standard approach would be: `cargo tarpaulin --out Html` (if added)

## Test Types

**Unit Tests:**
- Scope: Testing individual functions and methods in isolation
- Approach: Most tests in the codebase are unit tests
- Example from `git.rs` (line 158):
```rust
#[test]
fn test_is_empty() {
    let empty = make_changed_files(vec![]);
    assert!(empty.is_empty());

    let non_empty = make_changed_files(vec!["src/Foo.php"]);
    assert!(!non_empty.is_empty());
}
```

- Example from `config.rs` (line 302):
```rust
#[test]
fn test_get_file_pattern() {
    let config = parse_test_config();

    assert_eq!(config.get_file_pattern("php"), Some(r"\.php$"));
    assert_eq!(config.get_file_pattern("php_src"), Some("^src/"));
    assert_eq!(config.get_file_pattern("nonexistent"), None);
}
```

**Integration Tests:**
- Not currently implemented in the codebase
- Would test full flows like config loading + check determination + runner execution
- Would require Docker setup for runner tests

**E2E Tests:**
- Not used
- Manual testing via actual CI config files in tools/ci/ci-config.yaml

## Common Patterns

**Async Testing:**
- No async tests currently (no #[tokio::test] attribute seen)
- Async code in runner.rs and ui/mod.rs is not unit-tested
- Main.rs is async but only integration-tested

**Error Testing:**
```rust
#[test]
fn test_filter_by_pattern_invalid_regex() {
    let files = make_changed_files(vec!["src/Foo.php"]);

    // Invalid regex should return empty vec
    let result = files.filter_by_pattern(r"[invalid");
    assert!(result.is_empty());
}
```

- Example from `git.rs` (line 204): Tests graceful handling of invalid regexes
- Pattern is to verify that invalid input produces safe defaults (empty results)

**Configuration Testing:**
```rust
#[test]
fn test_check_always_run() {
    let config = parse_test_config();

    let fast = config.get_group("fast").unwrap();
    let php_lint = fast.checks.get("php-lint").unwrap();
    assert!(!php_lint.always_run()); // Has triggers

    // Create a check without triggers to test always_run = true
    let yaml = r#"
version: 2
docker:
  project_dir: ./infrastructure
git:
  base_branch: dev
  fallback_branch: HEAD~1
file_patterns: {}
checks:
  warmup:
    checks:
      cache-warmup:
        name: Cache warmup
        command: bin/console cache:warmup
"#;
    let config: CiConfig = serde_yaml::from_str(yaml).unwrap();
    let warmup = config.get_group("warmup").unwrap();
    let cache = warmup.checks.get("cache-warmup").unwrap();
    assert!(cache.always_run()); // No triggers
}
```

- Multi-scenario testing with different YAML configurations
- Inline test configs demonstrate edge cases

**State Verification:**
```rust
#[test]
fn test_groups_preserves_order() {
    let config = parse_test_config();

    let group_names: Vec<&str> = config.groups().map(|(name, _)| name).collect();
    assert_eq!(group_names, vec!["fast", "tests"]);
}
```

- Tests that internal state (like IndexMap ordering) is preserved
- Example from `config.rs` (line 361)

## Test Quality Observations

**Strengths:**
- Configuration parsing thoroughly tested with realistic YAML fixtures
- Edge cases covered (invalid regex, empty collections, missing patterns)
- Helper functions reduce test boilerplate
- Clear test naming convention makes intent obvious

**Gaps:**
- No async/tokio test integration for runner, UI, and main modules
- No Docker-based integration tests for actual check execution
- No end-to-end tests with real CI configurations
- No error propagation tests for Result handling chains

---

*Testing analysis: 2026-01-22*
