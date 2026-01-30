//! Characterization tests for determine_checks()
//!
//! These tests capture the current behavior of the determine_checks() function
//! before refactoring in Plan 08-02. They serve as a safety net to ensure that
//! refactoring doesn't change the external behavior.
//!
//! The tests are black-box style: given specific inputs (config + changed files),
//! we verify the exact output (which checks are triggered, on-demand, skipped, etc.).

use ci_tui::checks::{determine_checks, CheckToRun};
use ci_tui::config::{
    CheckDefinition, CheckTriggers, CiConfig, DockerConfig, FilePattern, GitConfig, GroupConfig,
};
use ci_tui::git::ChangedFiles;
use indexmap::IndexMap;
use std::collections::HashMap;
use std::path::PathBuf;

// ============================================================================
// Test Fixtures and Builders
// ============================================================================

/// Builder for creating `CheckDefinition` instances in tests
struct CheckBuilder {
    name: String,
    command: String,
    service: Option<String>,
    fix_command: Option<String>,
    triggers: Option<CheckTriggers>,
    on_demand: bool,
}

impl CheckBuilder {
    fn new(name: &str, command: &str) -> Self {
        Self {
            name: name.to_string(),
            command: command.to_string(),
            service: None,
            fix_command: None,
            triggers: None,
            on_demand: false,
        }
    }

    #[allow(dead_code)]
    fn with_service(mut self, service: &str) -> Self {
        self.service = Some(service.to_string());
        self
    }

    #[allow(dead_code)]
    fn with_fix_command(mut self, fix_cmd: &str) -> Self {
        self.fix_command = Some(fix_cmd.to_string());
        self
    }

    fn with_file_pattern_trigger(mut self, pattern_name: &str) -> Self {
        let triggers = self.triggers.get_or_insert_with(CheckTriggers::default);
        triggers.file_pattern = Some(pattern_name.to_string());
        self
    }

    fn on_demand(mut self) -> Self {
        self.on_demand = true;
        self
    }

    fn build(self) -> CheckDefinition {
        CheckDefinition {
            name: self.name,
            command: self.command,
            service: self.service,
            container: None,
            fix_command: self.fix_command,
            triggers: self.triggers,
            on_demand: self.on_demand,
            env: HashMap::new(),
        }
    }
}

/// Builder for creating `CiConfig` instances in tests
struct ConfigBuilder {
    version: u32,
    docker_project_dir: String,
    docker_service: String,
    docker_shell: String,
    git_base: String,
    git_fallback: String,
    file_patterns: HashMap<String, FilePattern>,
    checks: IndexMap<String, GroupConfig>,
}

impl ConfigBuilder {
    fn new() -> Self {
        Self {
            version: 2,
            docker_project_dir: "./test".to_string(),
            docker_service: "app".to_string(),
            docker_shell: "bash".to_string(),
            git_base: "main".to_string(),
            git_fallback: "HEAD~1".to_string(),
            file_patterns: HashMap::new(),
            checks: IndexMap::new(),
        }
    }

    fn with_file_pattern(mut self, name: &str, pattern: &str, color: Option<&str>) -> Self {
        self.file_patterns.insert(
            name.to_string(),
            FilePattern {
                pattern: pattern.to_string(),
                color: color.map(String::from),
            },
        );
        self
    }

    fn with_check(mut self, group_id: &str, check_id: &str, check: CheckDefinition) -> Self {
        self.checks
            .entry(group_id.to_string())
            .or_insert_with(|| GroupConfig {
                name: None,
                parallel: false,
                stop_on_failure: false,
                pre_commands: Vec::new(),
                checks: IndexMap::new(),
            })
            .checks
            .insert(check_id.to_string(), check);
        self
    }

    fn build(self) -> CiConfig {
        CiConfig::new(
            self.version,
            DockerConfig {
                project_dir: self.docker_project_dir,
                service: self.docker_service,
                container: None,
                image: None,
                volume_mount: None,
                work_dir: None,
                shell: self.docker_shell,
                env: HashMap::new(),
            },
            GitConfig {
                base_branch: self.git_base,
                fallback_branch: self.git_fallback,
            },
            self.file_patterns,
            self.checks,
            Vec::new(),
        )
    }
}

/// Config for checks.rs tests with warmup, fast, analysis, and test groups
fn checks_test_config() -> CiConfig {
    let mut file_patterns = HashMap::new();
    file_patterns.insert(
        "php".to_string(),
        FilePattern {
            pattern: r"\.php$".to_string(),
            color: None,
        },
    );
    file_patterns.insert(
        "php_src".to_string(),
        FilePattern {
            pattern: r"^src/.*\.php$".to_string(),
            color: None,
        },
    );
    file_patterns.insert(
        "tests".to_string(),
        FilePattern {
            pattern: r"tests/.*\.php$".to_string(),
            color: None,
        },
    );
    file_patterns.insert(
        "yaml".to_string(),
        FilePattern {
            pattern: r"\.ya?ml$".to_string(),
            color: None,
        },
    );

    let mut checks = IndexMap::new();

    // warmup group
    let mut warmup_group = GroupConfig {
        name: Some("Cache Warmup".to_string()),
        parallel: false,
        stop_on_failure: false,
        pre_commands: Vec::new(),
        checks: IndexMap::new(),
    };
    warmup_group.checks.insert(
        "cache-warmup".to_string(),
        CheckDefinition {
            name: "Cache warmup".to_string(),
            command: "bin/console cache:warmup".to_string(),
            service: None,
            container: None,
            fix_command: None,
            triggers: None,
            on_demand: false,
            env: HashMap::new(),
        },
    );
    checks.insert("warmup".to_string(), warmup_group);

    // fast group (parallel)
    let mut fast_group = GroupConfig {
        name: Some("Fast Checks".to_string()),
        parallel: true,
        stop_on_failure: false,
        pre_commands: Vec::new(),
        checks: IndexMap::new(),
    };
    fast_group.checks.insert(
        "php-lint".to_string(),
        CheckDefinition {
            name: "PHP syntax check".to_string(),
            command: "parallel-lint {files}".to_string(),
            service: None,
            container: None,
            fix_command: None,
            triggers: Some(CheckTriggers {
                file_pattern: Some("php".to_string()),
                test_discovery: None,
            }),
            on_demand: false,
            env: HashMap::new(),
        },
    );
    fast_group.checks.insert(
        "yaml-lint".to_string(),
        CheckDefinition {
            name: "YAML syntax check".to_string(),
            command: "yaml-lint {files}".to_string(),
            service: None,
            container: None,
            fix_command: None,
            triggers: Some(CheckTriggers {
                file_pattern: Some("yaml".to_string()),
                test_discovery: None,
            }),
            on_demand: false,
            env: HashMap::new(),
        },
    );
    checks.insert("fast".to_string(), fast_group);

    // analysis group
    let mut analysis_group = GroupConfig {
        name: None,
        parallel: false,
        stop_on_failure: false,
        pre_commands: Vec::new(),
        checks: IndexMap::new(),
    };
    analysis_group.checks.insert(
        "phpstan".to_string(),
        CheckDefinition {
            name: "PHPStan".to_string(),
            command: "phpstan analyse {files}".to_string(),
            service: None,
            container: None,
            fix_command: Some("phpstan fix {files}".to_string()),
            triggers: Some(CheckTriggers {
                file_pattern: Some("php".to_string()),
                test_discovery: None,
            }),
            on_demand: false,
            env: HashMap::new(),
        },
    );
    checks.insert("analysis".to_string(), analysis_group);

    // tests group
    let mut tests_group = GroupConfig {
        name: None,
        parallel: false,
        stop_on_failure: false,
        pre_commands: Vec::new(),
        checks: IndexMap::new(),
    };
    tests_group.checks.insert(
        "phpunit".to_string(),
        CheckDefinition {
            name: "PHPUnit".to_string(),
            command: "phpunit {files}".to_string(),
            service: None,
            container: None,
            fix_command: None,
            triggers: Some(CheckTriggers {
                file_pattern: Some("tests".to_string()),
                test_discovery: None,
            }),
            on_demand: false,
            env: HashMap::new(),
        },
    );
    checks.insert("tests".to_string(), tests_group);

    CiConfig::new(
        2,
        DockerConfig {
            project_dir: "./infrastructure".to_string(),
            service: "php".to_string(),
            container: None,
            image: None,
            volume_mount: None,
            work_dir: None,
            shell: "bash".to_string(),
            env: HashMap::new(),
        },
        GitConfig {
            base_branch: "development".to_string(),
            fallback_branch: "HEAD~1".to_string(),
        },
        file_patterns,
        checks,
        Vec::new(),
    )
}

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
        !check.on_demand,
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
        check.on_demand,
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
// Basic Behavior Tests (5 tests)
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
fn test_single_yaml_file_triggers_yaml_check() {
    let config = checks_test_config();
    let changed_files = make_changed_files(vec!["config/services.yaml"]);
    let project_root = PathBuf::from("/tmp/project");

    let checks = determine_checks(&config, &changed_files, &project_root);

    // YAML check should be triggered
    assert_check_triggered(&checks, "yaml-lint");

    // PHP checks should be on-demand (no PHP files changed)
    assert_check_on_demand(&checks, "php-lint");
    assert_check_on_demand(&checks, "phpstan");
    assert_check_on_demand(&checks, "phpunit");

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

#[test]
fn test_no_matching_patterns_results_in_on_demand_only() {
    let config = checks_test_config();
    let changed_files = make_changed_files(vec!["README.md", "LICENSE", "docs/guide.txt"]);
    let project_root = PathBuf::from("/tmp/project");

    let checks = determine_checks(&config, &changed_files, &project_root);

    // All checks with triggers should be on-demand
    assert_check_on_demand(&checks, "php-lint");
    assert_check_on_demand(&checks, "yaml-lint");
    assert_check_on_demand(&checks, "phpstan");
    assert_check_on_demand(&checks, "phpunit");

    // Always-run check should still run
    assert_check_triggered(&checks, "cache-warmup");
}

// ============================================================================
// Edge Case Tests (5 tests)
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
fn test_check_with_no_triggers_always_runs() {
    let config = ConfigBuilder::new()
        .with_check(
            "always",
            "always-run",
            CheckBuilder::new("Always Run", "echo 'runs always'").build(),
        )
        .build();
    let changed_files = make_changed_files(vec![]); // No changes
    let project_root = PathBuf::from("/tmp/project");

    let checks = determine_checks(&config, &changed_files, &project_root);

    assert_eq!(checks.len(), 1, "Should have one check");
    assert_check_triggered(&checks, "always-run");
    assert!(
        checks[0].files.is_empty(),
        "Always-run check should have empty files list"
    );
}

#[test]
fn test_check_with_triggers_but_no_matches_becomes_on_demand() {
    let config = ConfigBuilder::new()
        .with_file_pattern("php", r"\.php$", None)
        .with_check(
            "lint",
            "php-lint",
            CheckBuilder::new("PHP Lint", "phplint {files}")
                .with_file_pattern_trigger("php")
                .build(),
        )
        .build();
    let changed_files = make_changed_files(vec!["config.yaml", "README.md"]); // No PHP files
    let project_root = PathBuf::from("/tmp/project");

    let checks = determine_checks(&config, &changed_files, &project_root);

    assert_eq!(checks.len(), 1, "Should have one check");
    assert_check_on_demand(&checks, "php-lint");
}

#[test]
fn test_files_placeholder_with_no_matches() {
    let config = ConfigBuilder::new()
        .with_file_pattern("php", r"\.php$", None)
        .with_check(
            "lint",
            "php-lint",
            CheckBuilder::new("PHP Lint", "phplint {files}")
                .with_file_pattern_trigger("php")
                .build(),
        )
        .build();
    let changed_files = make_changed_files(vec!["config.yaml"]);
    let project_root = PathBuf::from("/tmp/project");

    let checks = determine_checks(&config, &changed_files, &project_root);

    let check = assert_check_exists(&checks, "php-lint");
    assert!(check.on_demand, "Check should be on-demand");
    assert!(
        check.skipped_no_files,
        "Check with {{files}} placeholder and no matches should have skipped_no_files=true"
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
        !check.on_demand,
        "Check marked on_demand in config becomes triggered when files match (current behavior)"
    );
}

// ============================================================================
// Test Discovery Scenarios (5 tests)
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

    // Test discovery should find the test file
    let check = assert_check_exists(&checks, "phpunit");
    assert!(!check.on_demand, "Check should be triggered");
    assert!(
        check
            .files
            .iter()
            .any(|f: &String| f.contains("FooTest.php")),
        "Check should include discovered test file"
    );
}

#[test]
fn test_test_discovery_finds_existing_test_file() {
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
    let test_file = temp_dir.path().join("tests/Unit/FooTest.php");
    std::fs::create_dir_all(test_file.parent().unwrap()).expect("Failed to create dir");
    std::fs::write(&test_file, "<?php // test").expect("Failed to write test file");

    let checks = determine_checks(&config, &changed_files, temp_dir.path());

    let check = assert_check_exists(&checks, "phpunit");
    assert!(!check.on_demand, "Check should be triggered");
    assert!(
        check.files.contains(&"tests/Unit/FooTest.php".to_string()),
        "Should find exact test file"
    );
}

#[test]
fn test_test_discovery_nonexistent_test_file_fallback() {
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
    // Don't create test file - it doesn't exist

    let checks = determine_checks(&config, &changed_files, temp_dir.path());

    let check = assert_check_exists(&checks, "phpunit");
    // Command doesn't have {files}, so should fall back to running all
    assert!(
        !check.on_demand,
        "Check should run all when no test found and no {{files}} placeholder"
    );
    assert!(
        check
            .files
            .iter()
            .any(|f: &String| f.contains("running all")),
        "Should indicate running all tests"
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
        check.on_demand,
        "Check with {{files}} and no tests found should be on-demand"
    );
    assert!(check.skipped_no_files, "Should have skipped_no_files=true");
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
    assert!(
        !check.on_demand,
        "Check without {{files}} should run all when no tests found"
    );
    assert!(
        check
            .files
            .iter()
            .any(|f: &String| f.contains("running all")),
        "Should indicate running all tests"
    );
}

// ============================================================================
// Grouping and Ordering Tests (3 tests)
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

#[test]
fn test_multiple_checks_in_same_group_preserved() {
    let config = checks_test_config();
    let changed_files = make_changed_files(vec!["src/Foo.php", "config.yaml"]);
    let project_root = PathBuf::from("/tmp/project");

    let checks = determine_checks(&config, &changed_files, &project_root);

    // Fast group should have both php-lint and yaml-lint
    let fast_checks: Vec<_> = checks.iter().filter(|c| c.group() == "fast").collect();
    assert!(
        fast_checks.len() >= 2,
        "Fast group should have multiple checks"
    );

    let check_ids: Vec<&str> = fast_checks.iter().map(|c| c.id()).collect();
    assert!(
        check_ids.contains(&"php-lint"),
        "Fast group should include php-lint"
    );
    assert!(
        check_ids.contains(&"yaml-lint"),
        "Fast group should include yaml-lint"
    );
}
