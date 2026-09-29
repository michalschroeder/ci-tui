//! Integration tests for determine_checks(): full-config, black-box
//! (config + changed files in, triggered/on-demand/skipped checks out).
//! Unit-level coverage of the individual build/process helpers lives in
//! src/checks/determine.rs #[cfg(test)].

use ci_tui::checks::{determine_checks, select_checks, CheckFiles, CheckToRun};
use ci_tui::config::CiConfig;
use ci_tui::git::ChangedFiles;
use std::path::PathBuf;

mod common;
use common::configs::{
    checks_test_config, php_files_filter_config, rust_grep_discovery_config, CheckBuilder,
    ConfigBuilder, FOO_FIXTURES, FOO_TEST,
};

// ============================================================================
// Helper Functions
// ============================================================================

/// Create ChangedFiles from a list of file paths
fn make_changed_files(files: Vec<&str>) -> ChangedFiles {
    ChangedFiles {
        files: files.into_iter().map(String::from).collect(),
        base_ref: "development".to_string(),
    }
}

/// Assert that a check with given ID is triggered (not on-demand)
fn assert_check_triggered(checks: &[CheckToRun], id: &str) {
    let check = checks.iter().find(|c| c.id() == id);
    assert!(
        check.is_some(),
        "Check '{}' should be present in results",
        id
    );
    let check = check.unwrap();
    assert!(
        !check.is_on_demand(),
        "Check '{}' should be triggered (not on-demand), but was on-demand",
        id
    );
}

/// Assert that a check with given ID is on-demand (not automatically triggered)
fn assert_check_on_demand(checks: &[CheckToRun], id: &str) {
    let check = checks.iter().find(|c| c.id() == id);
    assert!(
        check.is_some(),
        "Check '{}' should be present in results",
        id
    );
    let check = check.unwrap();
    assert!(
        check.is_on_demand(),
        "Check '{}' should be on-demand, but was triggered",
        id
    );
}

/// Assert that a check with given ID exists in results
fn assert_check_exists<'a>(checks: &'a [CheckToRun], id: &str) -> &'a CheckToRun {
    checks
        .iter()
        .find(|c| c.id() == id)
        .unwrap_or_else(|| panic!("Check '{}' should exist in results", id))
}

// ============================================================================
// Basic Behavior Tests
// ============================================================================

#[test]
fn test_empty_changed_files_returns_only_always_run_checks() {
    let config = checks_test_config();
    let changed_files = make_changed_files(vec![]);
    let project_root = PathBuf::from("/tmp/project");

    let checks = determine_checks(&config, &changed_files, &project_root);

    // cache-warmup has no triggers, so it should always run
    assert_check_triggered(&checks, "cache-warmup");

    // All checks with triggers should be on-demand when no files change
    assert_check_on_demand(&checks, "php-lint");
    assert_check_on_demand(&checks, "yaml-lint");
    assert_check_on_demand(&checks, "phpstan");
    assert_check_on_demand(&checks, "phpunit");
}

#[test]
fn test_single_php_file_triggers_php_checks() {
    let config = checks_test_config();
    let changed_files = make_changed_files(vec!["src/Service/Foo.php"]);
    let project_root = PathBuf::from("/tmp/project");

    let checks = determine_checks(&config, &changed_files, &project_root);

    // PHP-related checks should be triggered
    assert_check_triggered(&checks, "php-lint");
    assert_check_triggered(&checks, "phpstan");

    // Non-PHP checks should be on-demand
    assert_check_on_demand(&checks, "yaml-lint");
    assert_check_on_demand(&checks, "phpunit"); // No test files changed

    // Always-run check should still run
    assert_check_triggered(&checks, "cache-warmup");
}

#[test]
fn test_multiple_file_types_trigger_multiple_checks() {
    let config = checks_test_config();
    let changed_files = make_changed_files(vec![
        "src/Service/Foo.php",
        "config/services.yaml",
        "tests/Service/FooTest.php",
    ]);
    let project_root = PathBuf::from("/tmp/project");

    let checks = determine_checks(&config, &changed_files, &project_root);

    // All type-specific checks should be triggered
    assert_check_triggered(&checks, "php-lint"); // src/Service/Foo.php
    assert_check_triggered(&checks, "yaml-lint"); // config/services.yaml
    assert_check_triggered(&checks, "phpstan"); // src/Service/Foo.php
    assert_check_triggered(&checks, "phpunit"); // tests/Service/FooTest.php
    assert_check_triggered(&checks, "cache-warmup"); // Always runs
}

// ============================================================================
// Edge Case Tests
// ============================================================================

#[test]
fn test_empty_config_returns_empty_list() {
    let config = ConfigBuilder::new().build(); // Minimal config with no checks
    let changed_files = make_changed_files(vec!["src/Foo.php", "config.yaml"]);
    let project_root = PathBuf::from("/tmp/project");

    let checks = determine_checks(&config, &changed_files, &project_root);

    assert_eq!(
        checks.len(),
        0,
        "Empty config (no checks) should return empty list"
    );
}

#[test]
fn test_on_demand_config_field_ignored_when_files_match() {
    // This test documents CURRENT behavior: the on_demand config field
    // is ignored when files match - check becomes triggered automatically.
    let config = ConfigBuilder::new()
        .with_file_pattern("php", r"\.php$", None)
        .with_check(
            "expensive",
            "expensive-check",
            CheckBuilder::new("Expensive Check", "expensive-analysis")
                .with_file_pattern_trigger("php")
                .on_demand() // Explicitly marked as on-demand in config
                .build(),
        )
        .build();
    let changed_files = make_changed_files(vec!["src/Foo.php"]); // PHP file matches
    let project_root = PathBuf::from("/tmp/project");

    let checks = determine_checks(&config, &changed_files, &project_root);

    // CURRENT BEHAVIOR: Even though config has on_demand=true,
    // when files match the pattern, the check is triggered (not on-demand)
    let check = assert_check_exists(&checks, "expensive-check");
    assert!(
        !check.is_on_demand(),
        "Check marked on_demand in config becomes triggered when files match (current behavior)"
    );
}

// ============================================================================
// Test Discovery Scenarios
// ============================================================================

#[test]
fn test_source_file_change_triggers_test_discovery() {
    // Config with test_discovery trigger
    let config_yaml = r#"
version: 2
docker:
  project_dir: ./test
  service: php
  shell: bash
git:
  base_branch: development
  fallback_branch: HEAD~1
file_patterns:
  php_src:
    pattern: '^src/.*\.php$'
checks:
  tests:
    checks:
      phpunit:
        name: PHPUnit
        command: phpunit {files}
        triggers:
          test_discovery:
            source_pattern: php_src
            strategies:
              - type: path_mapping
                rules:
                  - source: src/{path}.php
                    tests:
                      - tests/Unit/{path}Test.php
"#;
    let config: CiConfig = serde_yaml::from_str(config_yaml).expect("Failed to parse config");
    let changed_files = make_changed_files(vec!["src/Service/Foo.php"]);

    // Create temp dir with test file
    let temp_dir = tempfile::tempdir().expect("Failed to create temp dir");
    let test_file = temp_dir.path().join("tests/Unit/Service/FooTest.php");
    std::fs::create_dir_all(test_file.parent().unwrap()).expect("Failed to create dir");
    std::fs::write(&test_file, "<?php // test").expect("Failed to write test file");

    let checks = determine_checks(&config, &changed_files, temp_dir.path());

    // Test discovery should find the exact mapped test file
    let check = assert_check_exists(&checks, "phpunit");
    assert!(!check.is_on_demand(), "Check should be triggered");
    assert!(
        check
            .files
            .paths()
            .contains(&"tests/Unit/Service/FooTest.php".to_string()),
        "Check should include the discovered test file at its mapped path"
    );
}

#[test]
fn test_command_with_files_placeholder_and_no_tests_found_is_skipped() {
    let config_yaml = r#"
version: 2
docker:
  project_dir: ./test
  service: php
  shell: bash
git:
  base_branch: development
  fallback_branch: HEAD~1
file_patterns:
  php_src:
    pattern: '^src/.*\.php$'
checks:
  tests:
    checks:
      phpunit:
        name: PHPUnit
        command: phpunit {files}
        triggers:
          test_discovery:
            source_pattern: php_src
            strategies:
              - type: path_mapping
                rules:
                  - source: src/{path}.php
                    tests:
                      - tests/Unit/{path}Test.php
"#;
    let config: CiConfig = serde_yaml::from_str(config_yaml).expect("Failed to parse config");
    let changed_files = make_changed_files(vec!["src/Foo.php"]);
    let temp_dir = tempfile::tempdir().expect("Failed to create temp dir");
    // No test file exists

    let checks = determine_checks(&config, &changed_files, temp_dir.path());

    let check = assert_check_exists(&checks, "phpunit");
    assert!(
        check.is_on_demand(),
        "Check with {{files}} and no tests found should be on-demand"
    );
    assert!(
        check.is_skipped_no_files(),
        "Should have skipped_no_files=true"
    );
}

#[test]
fn test_command_without_files_placeholder_and_no_tests_runs_all() {
    let config_yaml = r#"
version: 2
docker:
  project_dir: ./test
  service: php
  shell: bash
git:
  base_branch: development
  fallback_branch: HEAD~1
file_patterns:
  php_src:
    pattern: '^src/.*\.php$'
checks:
  tests:
    checks:
      phpunit:
        name: PHPUnit
        command: phpunit --all
        triggers:
          test_discovery:
            source_pattern: php_src
            strategies:
              - type: path_mapping
                rules:
                  - source: src/{path}.php
                    tests:
                      - tests/Unit/{path}Test.php
"#;
    let config: CiConfig = serde_yaml::from_str(config_yaml).expect("Failed to parse config");
    let changed_files = make_changed_files(vec!["src/Foo.php"]);
    let temp_dir = tempfile::tempdir().expect("Failed to create temp dir");
    // No test file exists

    let checks = determine_checks(&config, &changed_files, temp_dir.path());

    let check = assert_check_exists(&checks, "phpunit");
    // Command doesn't have {files}, so should fall back to running all
    assert!(
        !check.is_on_demand(),
        "Check without {{files}} should run all when no tests found"
    );
    assert_eq!(
        check.files,
        CheckFiles::RunAll,
        "Should indicate running all tests"
    );
}

// ============================================================================
// Grouping and Ordering Tests
// ============================================================================

#[test]
fn test_checks_are_grouped_by_execution_group() {
    let config = checks_test_config();
    let changed_files = make_changed_files(vec!["src/Foo.php", "config.yaml"]);
    let project_root = PathBuf::from("/tmp/project");

    let checks = determine_checks(&config, &changed_files, &project_root);

    // Verify checks are assigned to correct groups
    let warmup = checks.iter().find(|c| c.id() == "cache-warmup").unwrap();
    assert_eq!(warmup.group(), "warmup");

    let php_lint = checks.iter().find(|c| c.id() == "php-lint").unwrap();
    assert_eq!(php_lint.group(), "fast");

    let yaml_lint = checks.iter().find(|c| c.id() == "yaml-lint").unwrap();
    assert_eq!(yaml_lint.group(), "fast");

    let phpstan = checks.iter().find(|c| c.id() == "phpstan").unwrap();
    assert_eq!(phpstan.group(), "analysis");
}

#[test]
fn test_group_order_matches_config_definition_order() {
    let config = checks_test_config();
    let changed_files = make_changed_files(vec!["src/Foo.php", "tests/FooTest.php"]);
    let project_root = PathBuf::from("/tmp/project");

    let checks = determine_checks(&config, &changed_files, &project_root);

    // Extract groups in order they appear
    let mut seen_groups = Vec::new();
    for check in &checks {
        if !seen_groups.contains(&check.group()) {
            seen_groups.push(check.group());
        }
    }

    // Config order is: warmup, fast, analysis, tests
    assert_eq!(
        seen_groups,
        vec!["warmup", "fast", "analysis", "tests"],
        "Groups should appear in config definition order"
    );
}

// ============================================================================
// Test discovery warnings
// ============================================================================

#[test]
fn test_grep_failure_is_warning_and_other_strategies_still_find_tests() {
    // `\(` is an unmatched group in grep BRE -> grep exits 2
    let config = rust_grep_discovery_config(r"\({basename}");
    let root = tempfile::tempdir().unwrap();
    std::fs::create_dir_all(root.path().join("tests")).unwrap();
    std::fs::write(root.path().join("tests/foo_test.rs"), "foo").unwrap();

    let selected = select_checks(
        &config,
        &make_changed_files(vec!["src/foo.rs"]),
        root.path(),
        None,
    );

    let a = assert_check_exists(&selected.checks, "a");
    assert_eq!(
        a.files,
        CheckFiles::Files(vec!["tests/foo_test.rs".to_string()])
    );
    // Both checks hit the same failure; reported once
    assert_eq!(selected.warnings.len(), 1, "{:?}", selected.warnings);
    assert!(
        selected.warnings[0].starts_with("test discovery: grep in `tests` failed: exit status 2"),
        "{}",
        selected.warnings[0]
    );
}

#[test]
fn test_no_discovery_warnings_when_grep_succeeds() {
    let config = rust_grep_discovery_config("{basename}");
    let root = tempfile::tempdir().unwrap();
    std::fs::create_dir_all(root.path().join("tests")).unwrap();
    std::fs::write(root.path().join("tests/foo_test.rs"), "foo").unwrap();

    let selected = select_checks(
        &config,
        &make_changed_files(vec!["src/foo.rs"]),
        root.path(),
        None,
    );
    let a = assert_check_exists(&selected.checks, "a");
    assert_eq!(
        a.files,
        CheckFiles::Files(vec!["tests/foo_test.rs".to_string()])
    );
    assert!(selected.warnings.is_empty());
}

// ============================================================================
// triggers.files_filter (#210)
// ============================================================================

/// Tempdir holding `paths`, each mentioning `FooRule` (grep discovery finds them)
fn linter_root(paths: &[&str]) -> tempfile::TempDir {
    let root = tempfile::tempdir().unwrap();
    for path in paths {
        let file = root.path().join(path);
        std::fs::create_dir_all(file.parent().unwrap()).unwrap();
        std::fs::write(file, "<?php // FooRule").unwrap();
    }
    root
}

#[test]
fn test_files_filter_drops_fixtures_from_files() {
    // Issue #210 repro: fixtures match `file_pattern` and grep discovery
    let root = linter_root(&[FOO_TEST, FOO_FIXTURES[0], FOO_FIXTURES[1]]);
    let mut changed = vec!["tools/linter/src/Rule/FooRule.php", FOO_TEST];
    changed.extend(FOO_FIXTURES);

    let checks = determine_checks(
        &php_files_filter_config(),
        &make_changed_files(changed),
        root.path(),
    );

    // grep also found both fixtures: filter keeps only the test
    let unit = assert_check_exists(&checks, "unit");
    assert_eq!(unit.files, CheckFiles::Files(vec![FOO_TEST.to_string()]));
    assert_eq!(unit.resolved_command, format!("phpunit {FOO_TEST}"));
}

#[test]
fn test_files_filter_dropping_every_file_pattern_match_applies_no_files_fallback() {
    // Only fixtures changed: `file_pattern` matched them, filter drops all,
    // so the trigger fired with nothing for `{files}` (skip / on_demand / run all)
    let root = linter_root(&FOO_FIXTURES);
    let checks = determine_checks(
        &php_files_filter_config(),
        &make_changed_files(FOO_FIXTURES.to_vec()),
        root.path(),
    );

    let unit = assert_check_exists(&checks, "unit");
    assert_eq!(unit.files, CheckFiles::SkippedNoMatch);
    assert!(unit.is_skipped_no_files());
    assert_eq!(
        assert_check_exists(&checks, "slow").files,
        CheckFiles::OnDemand
    );
    let suite = assert_check_exists(&checks, "suite");
    assert_eq!(suite.files, CheckFiles::RunAll);
    assert_eq!(suite.resolved_command, "phpunit");
}

#[test]
fn test_files_filter_dropping_all_discovered_tests_is_no_tests_found() {
    // Source changed, grep discovers only fixtures: filter drops them, so
    // the no-tests fallbacks apply (skip / on_demand / run full command)
    let root = linter_root(&FOO_FIXTURES);
    let checks = determine_checks(
        &php_files_filter_config(),
        &make_changed_files(vec!["tools/linter/src/Rule/FooRule.php"]),
        root.path(),
    );

    assert_eq!(
        assert_check_exists(&checks, "unit").files,
        CheckFiles::SkippedNoMatch
    );
    assert_eq!(
        assert_check_exists(&checks, "slow").files,
        CheckFiles::OnDemand
    );
    assert_eq!(
        assert_check_exists(&checks, "suite").files,
        CheckFiles::RunAll
    );
}
