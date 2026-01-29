//! Logic for determining which CI checks to run based on changed files.
//!
//! This module analyzes the configuration and changed files to build a list
//! of checks that should be executed. It handles file pattern matching,
//! test discovery, and on-demand check logic.
//!
//! # Key Types
//!
//! - [`CheckToRun`]: A check prepared for execution with resolved commands
//!
//! # Key Functions
//!
//! - [`determine_checks`]: Main entry point for determining which checks to run
//! - [`group_checks`]: Groups checks by their execution group for ordered execution

use crate::config::{CheckDefinition, CiConfig};
use crate::git::ChangedFiles;
use crate::test_discovery;
use std::collections::HashSet;
use std::path::Path;

/// A CI check that has been determined to run, with resolved commands
#[derive(Debug, Clone)]
pub struct CheckToRun {
    /// Check ID (from config key)
    pub id: String,
    /// Group this check belongs to
    pub group: String,
    /// The check definition from configuration
    pub definition: CheckDefinition,
    /// Docker service to use (resolved from check/group/global default)
    pub service: String,
    /// Files that triggered this check (may be empty for always-run checks)
    pub files: Vec<String>,
    /// The fully resolved command to execute
    pub resolved_command: String,
    /// The fully resolved fix command (if available)
    pub resolved_fix_command: Option<String>,
    /// If true, this check won't run automatically - user must trigger it manually
    pub on_demand: bool,
    /// If true, this check was skipped because it uses {files} placeholder but no files matched
    pub skipped_no_files: bool,
}

impl CheckToRun {
    /// Get the unique identifier for this check
    pub fn id(&self) -> &str {
        &self.id
    }

    /// Get the display name for this check
    pub fn name(&self) -> &str {
        &self.definition.name
    }

    /// Get the execution group this check belongs to
    pub fn group(&self) -> &str {
        &self.group
    }

    /// Check if this check has a fix command available
    pub fn has_fix(&self) -> bool {
        self.resolved_fix_command.is_some()
    }

    /// Check if this is an on-demand check (requires manual trigger)
    pub fn is_on_demand(&self) -> bool {
        self.on_demand
    }

    /// Get command with {files} placeholder removed (for running against all files)
    ///
    /// This returns the command with {files} replaced by empty string, trimmed.
    /// Useful for running a check against all files instead of just changed ones.
    pub fn get_command_for_all_files(&self) -> String {
        self.definition
            .command
            .replace("{files}", "")
            .split_whitespace()
            .collect::<Vec<_>>()
            .join(" ")
    }
}

/// Determine which checks should run based on changed files.
///
/// Returns ALL checks from config - those that match are set to run,
/// those that don't match are marked as skipped (can be run on-demand).
/// Checks are returned in config order for predictable execution.
pub fn determine_checks(
    config: &CiConfig,
    changed_files: &ChangedFiles,
    project_root: &Path,
) -> Vec<CheckToRun> {
    let mut checks_to_run = Vec::new();
    let default_service = config.default_service();

    // Iterate over groups in config order
    for (group_name, group_config) in config.groups() {
        // Iterate over checks in this group
        for (check_id, check) in &group_config.checks {
            let service = check.service_or_default(default_service).to_string();

            // Always run checks with no triggers
            if check.always_run() {
                let resolved = resolve_command(config, check, &[], false);
                let resolved_fix = check
                    .fix_command
                    .as_ref()
                    .map(|_| resolve_command(config, check, &[], true));
                checks_to_run.push(CheckToRun {
                    id: check_id.clone(),
                    group: group_name.to_string(),
                    definition: check.clone(),
                    service,
                    files: vec![],
                    resolved_command: resolved,
                    resolved_fix_command: resolved_fix,
                    on_demand: false,
                    skipped_no_files: false,
                });
                continue;
            }

            // Check if this check should run based on triggers
            if let Some(triggers) = &check.triggers {
                let mut matched_files = Vec::new();
                let mut has_source_trigger = false;

                // Check file pattern trigger (direct test file changes)
                if let Some(pattern_key) = &triggers.file_pattern {
                    if let Some(pattern) = config.get_file_pattern(pattern_key) {
                        let files = changed_files.filter_by_pattern(pattern);
                        matched_files.extend(files.into_iter().map(String::from));
                    }
                }

                // Check test_discovery trigger (for running tests when src changes)
                if let Some(discovery) = &triggers.test_discovery {
                    has_source_trigger = true;
                    if let Some(pattern) = config.get_file_pattern(&discovery.source_pattern) {
                        let source_files = changed_files.filter_by_pattern(pattern);
                        if !source_files.is_empty() {
                            // Source files changed - find related tests using inline strategies
                            let related_tests = test_discovery::find_related_tests(
                                &discovery.strategies,
                                &source_files,
                                project_root,
                            );

                            if !related_tests.is_empty() {
                                // Found specific tests to run
                                matched_files.extend(related_tests);
                            } else if matched_files.is_empty() {
                                if check.on_demand {
                                    // Check is configured as on-demand - add but require manual trigger
                                    let resolved = resolve_command(config, check, &[], false);
                                    let resolved_fix = check
                                        .fix_command
                                        .as_ref()
                                        .map(|_| resolve_command(config, check, &[], true));
                                    checks_to_run.push(CheckToRun {
                                        id: check_id.clone(),
                                        group: group_name.to_string(),
                                        definition: check.clone(),
                                        service: service.clone(),
                                        files: vec!["(on-demand - press 't' to run)".to_string()],
                                        resolved_command: resolved,
                                        resolved_fix_command: resolved_fix,
                                        on_demand: true,
                                        skipped_no_files: false,
                                    });
                                    continue;
                                } else if !check.command.contains("{files}") {
                                    // Fall back to running all when no related tests found
                                    // BUT only if the command doesn't use {files} placeholder
                                    // (running with empty {files} would run against entire codebase)
                                    matched_files
                                        .push("(source files changed - running all)".to_string());
                                }
                                // If command uses {files} and no tests found, matched_files stays empty
                                // and check will be added as skipped below
                            }
                        }
                    }
                }

                // Deduplicate matched files (file_pattern and test_discovery may find same file)
                let matched_files: Vec<String> = matched_files
                    .into_iter()
                    .collect::<HashSet<_>>()
                    .into_iter()
                    .collect();

                // If files matched, add this check to run
                if !matched_files.is_empty() {
                    let file_list: Vec<&str> = matched_files.iter().map(|s| s.as_str()).collect();
                    let resolved = resolve_command(config, check, &file_list, false);
                    let resolved_fix = check
                        .fix_command
                        .as_ref()
                        .map(|_| resolve_command(config, check, &file_list, true));
                    checks_to_run.push(CheckToRun {
                        id: check_id.clone(),
                        group: group_name.to_string(),
                        definition: check.clone(),
                        service,
                        files: matched_files,
                        resolved_command: resolved,
                        resolved_fix_command: resolved_fix,
                        on_demand: false,
                        skipped_no_files: false,
                    });
                } else {
                    // No files matched - add as skipped (on-demand)
                    // Only add if it has triggers but none matched
                    let has_file_trigger = triggers.file_pattern.is_some();
                    if has_file_trigger || has_source_trigger {
                        let resolved = resolve_command(config, check, &[], false);
                        let resolved_fix = check
                            .fix_command
                            .as_ref()
                            .map(|_| resolve_command(config, check, &[], true));
                        // Check if command uses {files} placeholder
                        let has_files_placeholder = check.command.contains("{files}");
                        checks_to_run.push(CheckToRun {
                            id: check_id.clone(),
                            group: group_name.to_string(),
                            definition: check.clone(),
                            service,
                            files: vec!["(skipped - no matching files)".to_string()],
                            resolved_command: resolved,
                            resolved_fix_command: resolved_fix,
                            on_demand: true, // Can be run on-demand
                            skipped_no_files: has_files_placeholder,
                        });
                    }
                }
            }
        }
    }

    checks_to_run
}

/// Resolve placeholders in check command
///
/// Supported placeholders:
/// - `{files}` - replaced with space-separated list of matched files
fn resolve_command(
    _config: &CiConfig,
    check: &CheckDefinition,
    files: &[&str],
    use_fix: bool,
) -> String {
    let mut command = if use_fix {
        check
            .fix_command
            .clone()
            .unwrap_or_else(|| check.command.clone())
    } else {
        check.command.clone()
    };

    // Replace {files} placeholder
    let files_str = if files.is_empty() || files.iter().any(|f| f.starts_with('(')) {
        String::new()
    } else {
        files.join(" ")
    };
    command = command.replace("{files}", &files_str);

    command.trim().to_string()
}

/// Group checks by their execution group
///
/// Returns groups in the order they appear in checks (which is config order)
pub fn group_checks(checks: &[CheckToRun]) -> Vec<(&str, Vec<&CheckToRun>)> {
    // Use IndexMap to preserve insertion order
    let mut groups: indexmap::IndexMap<&str, Vec<&CheckToRun>> = indexmap::IndexMap::new();

    for check in checks {
        groups.entry(check.group()).or_default().push(check);
    }

    groups.into_iter().collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::CiConfig;
    use crate::git::ChangedFiles;
    use rstest::rstest;
    use std::path::PathBuf;

    // Test fixture matching tests/common/configs.rs::checks_test_config()
    // NOTE: Cannot use #[path] to import from tests/ due to cargo fmt limitations in Docker
    // See: https://github.com/rust-lang/rustfmt/issues/4656
    fn checks_test_config() -> CiConfig {
        use crate::config::{
            CheckDefinition, CheckTriggers, DockerConfig, FilePattern, GitConfig, GroupConfig,
        };
        use indexmap::IndexMap;
        use std::collections::HashMap;
        use std::sync::OnceLock;

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

        CiConfig {
            version: 2,
            docker: DockerConfig {
                project_dir: "./infrastructure".to_string(),
                service: "php".to_string(),
                container: None,
                image: None,
                volume_mount: None,
                work_dir: None,
                shell: "bash".to_string(),
                env: HashMap::new(),
            },
            git: GitConfig {
                base_branch: "development".to_string(),
                fallback_branch: "HEAD~1".to_string(),
            },
            file_patterns,
            checks,
            ignore_patterns: Vec::new(),
            compiled_ignore_patterns: OnceLock::new(),
        }
    }

    fn make_changed_files(files: Vec<&str>) -> ChangedFiles {
        ChangedFiles {
            files: files.into_iter().map(String::from).collect(),
            base_ref: "development".to_string(),
        }
    }

    // Helper to create CheckToRun for tests
    fn make_check(
        id: &str,
        group: &str,
        name: &str,
        command: &str,
        on_demand: bool,
        skipped_no_files: bool,
    ) -> CheckToRun {
        CheckToRun {
            id: id.to_string(),
            group: group.to_string(),
            definition: crate::config::CheckDefinition {
                name: name.to_string(),
                command: command.to_string(),
                service: None,
                container: None,
                fix_command: None,
                triggers: None,
                on_demand: false,
                env: std::collections::HashMap::new(),
            },
            service: "php".to_string(),
            files: vec![],
            resolved_command: command.to_string(),
            resolved_fix_command: None,
            on_demand,
            skipped_no_files,
        }
    }

    #[test]
    fn test_always_run_check() {
        let config = checks_test_config();
        let changed_files = make_changed_files(vec!["src/Service/Foo.php"]);
        let project_root = PathBuf::from("/tmp/project");

        let checks = determine_checks(&config, &changed_files, &project_root);

        // Cache warmup should always run (no triggers)
        let cache_warmup = checks.iter().find(|c| c.id() == "cache-warmup");
        assert!(cache_warmup.is_some());
        let cache_warmup = cache_warmup.unwrap();
        assert!(!cache_warmup.on_demand);
        assert!(cache_warmup.files.is_empty());
    }

    #[test]
    fn test_triggered_check_with_matching_files() {
        let config = checks_test_config();
        let changed_files = make_changed_files(vec!["src/Service/Foo.php"]);
        let project_root = PathBuf::from("/tmp/project");

        let checks = determine_checks(&config, &changed_files, &project_root);

        // PHP lint should be triggered
        let php_lint = checks.iter().find(|c| c.id() == "php-lint");
        assert!(php_lint.is_some());
        let php_lint = php_lint.unwrap();
        assert!(!php_lint.on_demand);
        assert!(php_lint.files.contains(&"src/Service/Foo.php".to_string()));
    }

    #[test]
    fn test_triggered_check_without_matching_files() {
        let config = checks_test_config();
        let changed_files = make_changed_files(vec!["config/services.yaml"]);
        let project_root = PathBuf::from("/tmp/project");

        let checks = determine_checks(&config, &changed_files, &project_root);

        // PHP lint should be skipped (on-demand) since no PHP files changed
        let php_lint = checks.iter().find(|c| c.id() == "php-lint");
        assert!(php_lint.is_some());
        let php_lint = php_lint.unwrap();
        assert!(php_lint.on_demand);

        // YAML lint should be triggered
        let yaml_lint = checks.iter().find(|c| c.id() == "yaml-lint");
        assert!(yaml_lint.is_some());
        let yaml_lint = yaml_lint.unwrap();
        assert!(!yaml_lint.on_demand);
        assert!(yaml_lint
            .files
            .contains(&"config/services.yaml".to_string()));
    }

    #[test]
    fn test_resolve_command_with_files() {
        let config = checks_test_config();
        let changed_files = make_changed_files(vec!["src/Service/Foo.php", "src/Service/Bar.php"]);
        let project_root = PathBuf::from("/tmp/project");

        let checks = determine_checks(&config, &changed_files, &project_root);

        let php_lint = checks.iter().find(|c| c.id() == "php-lint").unwrap();
        assert!(php_lint.resolved_command.contains("src/Service/Foo.php"));
        assert!(php_lint.resolved_command.contains("src/Service/Bar.php"));
    }

    #[test]
    fn test_check_has_fix_command() {
        let config = checks_test_config();
        let changed_files = make_changed_files(vec!["src/Service/Foo.php"]);
        let project_root = PathBuf::from("/tmp/project");

        let checks = determine_checks(&config, &changed_files, &project_root);

        let php_lint = checks.iter().find(|c| c.id() == "php-lint").unwrap();
        assert!(!php_lint.has_fix());

        let phpstan = checks.iter().find(|c| c.id() == "phpstan").unwrap();
        assert!(phpstan.has_fix());
        assert!(phpstan.resolved_fix_command.is_some());
    }

    #[test]
    fn test_group_checks_preserves_order() {
        let config = checks_test_config();
        let changed_files = make_changed_files(vec!["src/Service/Foo.php"]);
        let project_root = PathBuf::from("/tmp/project");

        let checks = determine_checks(&config, &changed_files, &project_root);
        let grouped = group_checks(&checks);

        let group_names: Vec<&str> = grouped.iter().map(|(name, _)| *name).collect();
        // Groups: warmup (always), fast (php matches), analysis (php matches), tests (skipped - no match)
        assert_eq!(group_names, vec!["warmup", "fast", "analysis", "tests"]);
    }

    #[test]
    fn test_check_to_run_accessors() {
        let config = checks_test_config();
        let changed_files = make_changed_files(vec!["src/Service/Foo.php"]);
        let project_root = PathBuf::from("/tmp/project");

        let checks = determine_checks(&config, &changed_files, &project_root);

        let php_lint = checks.iter().find(|c| c.id() == "php-lint").unwrap();
        assert_eq!(php_lint.id(), "php-lint");
        assert_eq!(php_lint.name(), "PHP syntax check");
        assert_eq!(php_lint.group(), "fast");
        assert!(!php_lint.is_on_demand());
    }

    #[test]
    fn test_get_command_for_all_files() {
        let config = checks_test_config();
        let changed_files = make_changed_files(vec!["src/Service/Foo.php"]);
        let project_root = PathBuf::from("/tmp/project");

        let checks = determine_checks(&config, &changed_files, &project_root);

        let php_lint = checks.iter().find(|c| c.id() == "php-lint").unwrap();
        let all_files_cmd = php_lint.get_command_for_all_files();

        // Should have {files} replaced with empty and cleaned up
        assert_eq!(all_files_cmd, "parallel-lint");
        assert!(!all_files_cmd.contains("{files}"));
    }

    #[test]
    fn test_check_with_files_placeholder_no_matches_is_skipped() {
        let config = checks_test_config();
        // Change only YAML files - no PHP files
        let changed_files = make_changed_files(vec!["config/services.yaml"]);
        let project_root = PathBuf::from("/tmp/project");

        let checks = determine_checks(&config, &changed_files, &project_root);

        // php-lint uses {files} placeholder and no PHP files matched
        let php_lint = checks.iter().find(|c| c.id() == "php-lint").unwrap();
        assert!(php_lint.on_demand, "Check should be on-demand");
        assert!(
            php_lint.skipped_no_files,
            "Check should have skipped_no_files=true"
        );

        // phpstan also uses {files} placeholder and no PHP files matched
        let phpstan = checks.iter().find(|c| c.id() == "phpstan").unwrap();
        assert!(phpstan.on_demand, "Check should be on-demand");
        assert!(
            phpstan.skipped_no_files,
            "Check should have skipped_no_files=true"
        );
    }

    #[test]
    fn test_check_without_files_placeholder_no_matches_not_skipped() {
        // Test config with a check that doesn't use {files} placeholder
        let config_yaml = r#"
version: 2

docker:
  project_dir: ./infrastructure
  service: php
  shell: bash

git:
  base_branch: development
  fallback_branch: HEAD~1

file_patterns:
  php:
    pattern: '\.php$'

checks:
  tests:
    checks:
      phpunit:
        name: PHPUnit All Tests
        command: phpunit --all
        triggers:
          file_pattern: php
"#;
        let config: CiConfig =
            serde_yaml::from_str(config_yaml).expect("Failed to parse test config");

        // Change only YAML files - no PHP files
        let changed_files = make_changed_files(vec!["config/services.yaml"]);
        let project_root = PathBuf::from("/tmp/project");

        let checks = determine_checks(&config, &changed_files, &project_root);

        // phpunit doesn't use {{files}} placeholder, so shouldn't be auto-skipped
        let phpunit = checks.iter().find(|c| c.id() == "phpunit").unwrap();
        assert!(phpunit.on_demand, "Check should be on-demand");
        assert!(
            !phpunit.skipped_no_files,
            "Check should NOT have skipped_no_files=true (no {{files}} placeholder)"
        );
    }

    #[test]
    fn test_test_discovery_no_matches_with_files_placeholder_is_skipped() {
        // Test scenario: source file changes, test discovery runs but finds no matching tests,
        // and command uses {files} placeholder - should be skipped to avoid running all tests
        let config_yaml = r#"
version: 2

docker:
  project_dir: ./infrastructure
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
        name: PHPUnit Tests
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
        let config: CiConfig =
            serde_yaml::from_str(config_yaml).expect("Failed to parse test config");

        // Source file changes but has no matching test (test file doesn't exist)
        let changed_files = make_changed_files(vec!["src/SomeClass.php"]);
        let project_root = PathBuf::from("/tmp/nonexistent");

        let checks = determine_checks(&config, &changed_files, &project_root);

        // phpunit uses {files} placeholder and no tests were found
        // should be skipped to avoid running entire test suite
        let phpunit = checks.iter().find(|c| c.id() == "phpunit").unwrap();
        assert!(phpunit.on_demand, "Check should be on-demand");
        assert!(
            phpunit.skipped_no_files,
            "Check should have skipped_no_files=true (uses {{files}} but no tests found)"
        );
        assert!(
            phpunit.files[0].contains("skipped"),
            "Files should indicate skipped status"
        );
    }

    #[test]
    fn test_test_discovery_no_matches_without_files_placeholder_runs_all() {
        // Test scenario: source file changes, test discovery runs but finds no matching tests,
        // and command does NOT use {files} placeholder - should run all tests
        let config_yaml = r#"
version: 2

docker:
  project_dir: ./infrastructure
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
        name: PHPUnit Tests
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
        let config: CiConfig =
            serde_yaml::from_str(config_yaml).expect("Failed to parse test config");

        // Source file changes but has no matching test
        let changed_files = make_changed_files(vec!["src/SomeClass.php"]);
        let project_root = PathBuf::from("/tmp/nonexistent");

        let checks = determine_checks(&config, &changed_files, &project_root);

        // phpunit does NOT use {files} placeholder
        // should fall back to running all tests
        let phpunit = checks.iter().find(|c| c.id() == "phpunit").unwrap();
        assert!(
            !phpunit.on_demand,
            "Check should NOT be on-demand (runs all)"
        );
        assert!(
            !phpunit.skipped_no_files,
            "Check should NOT have skipped_no_files=true"
        );
        assert!(
            phpunit.files[0].contains("running all"),
            "Files should indicate running all"
        );
    }

    #[test]
    fn test_no_duplicate_test_files_from_multiple_triggers() {
        // Test scenario: A test file changes directly (matches file_pattern)
        // AND its corresponding source file also changes (triggers test_discovery which finds same test file)
        // Result: CheckToRun.files should contain NO duplicates

        let config_yaml = r#"
version: 2

docker:
  project_dir: ./infrastructure
  service: php
  shell: bash

git:
  base_branch: development
  fallback_branch: HEAD~1

file_patterns:
  phpunit:
    pattern: 'tests/.*Test\.php$'
  php_src:
    pattern: '^src/.*\.php$'

checks:
  tests:
    checks:
      phpunit:
        name: PHPUnit Tests
        command: phpunit {files}
        triggers:
          file_pattern: phpunit
          test_discovery:
            source_pattern: php_src
            strategies:
              - type: path_mapping
                rules:
                  - source: src/{path}.php
                    tests:
                      - tests/Unit/{path}Test.php
"#;
        let config: CiConfig =
            serde_yaml::from_str(config_yaml).expect("Failed to parse test config");

        // Create temporary directory with actual test files (test_discovery checks file existence)
        let temp_dir = tempfile::tempdir().expect("Failed to create temp dir");
        let test_file_path = temp_dir.path().join("tests/Unit/FooTest.php");
        std::fs::create_dir_all(test_file_path.parent().unwrap())
            .expect("Failed to create test dir");
        std::fs::write(&test_file_path, "<?php // test content")
            .expect("Failed to write test file");

        // Changed files: BOTH the test file AND its source file
        // - tests/Unit/FooTest.php matches file_pattern "phpunit"
        // - src/Foo.php matches php_src and test_discovery finds tests/Unit/FooTest.php
        let changed_files = make_changed_files(vec!["tests/Unit/FooTest.php", "src/Foo.php"]);

        let checks = determine_checks(&config, &changed_files, temp_dir.path());

        // Should have exactly one phpunit check
        let phpunit = checks.iter().find(|c| c.id() == "phpunit").unwrap();
        assert!(!phpunit.on_demand, "Check should not be on-demand");
        assert!(!phpunit.skipped_no_files, "Check should not be skipped");

        // The key assertion: files should contain NO duplicates
        let files = &phpunit.files;
        let unique_files: std::collections::HashSet<_> = files.iter().collect();

        assert_eq!(
            files.len(),
            unique_files.len(),
            "CheckToRun.files contains duplicates! Files: {:?}",
            files
        );

        // Should contain the test file exactly once
        let test_file_count = files
            .iter()
            .filter(|f| f.contains("tests/Unit/FooTest.php"))
            .count();
        assert_eq!(
            test_file_count, 1,
            "Expected 'tests/Unit/FooTest.php' to appear exactly once, but found {} occurrences in {:?}",
            test_file_count, files
        );
    }

    mod test_determine_checks {
        use super::*;
        use pretty_assertions::assert_eq;

        #[rstest]
        #[case("src/Foo.php", "php", true)]
        #[case("src/Foo.php", "yaml", false)]
        #[case("config.yaml", "yaml", true)]
        #[case("config.yml", "yaml", true)]
        #[case("README.md", "php", false)]
        #[case("src/Bar/Baz.php", "php", true)]
        #[case("tests/FooTest.php", "tests", true)]
        #[case("", "php", false)]
        fn test_file_pattern_trigger(
            #[case] file: &str,
            #[case] pattern_key: &str,
            #[case] should_trigger: bool,
        ) {
            let config = checks_test_config();
            let changed_files = if file.is_empty() {
                make_changed_files(vec![])
            } else {
                make_changed_files(vec![file])
            };
            let project_root = PathBuf::from("/tmp/project");

            let checks = determine_checks(&config, &changed_files, &project_root);

            // Find checks that have the pattern_key trigger
            let matching_checks: Vec<_> = checks
                .iter()
                .filter(|c| {
                    c.definition
                        .triggers
                        .as_ref()
                        .and_then(|t| t.file_pattern.as_ref())
                        .map(|p| p == pattern_key)
                        .unwrap_or(false)
                })
                .collect();

            if should_trigger {
                // At least one check with this pattern should not be on-demand
                let has_triggered = matching_checks.iter().any(|c| !c.on_demand);
                assert!(
                    has_triggered,
                    "Expected file '{}' to trigger pattern '{}', but all checks are on-demand",
                    file, pattern_key
                );
            } else {
                // All checks with this pattern should be on-demand (skipped)
                let all_on_demand = matching_checks.iter().all(|c| c.on_demand);
                assert!(
                    all_on_demand,
                    "Expected file '{}' NOT to trigger pattern '{}', but found triggered checks",
                    file, pattern_key
                );
            }
        }

        #[test]
        fn test_empty_changed_files_returns_only_always_run_checks() {
            let config = checks_test_config();
            let changed_files = make_changed_files(vec![]);
            let project_root = PathBuf::from("/tmp/project");

            let checks = determine_checks(&config, &changed_files, &project_root);

            // Should have at least the cache-warmup check (always run)
            let always_run_checks: Vec<_> = checks.iter().filter(|c| !c.on_demand).collect();
            assert!(
                !always_run_checks.is_empty(),
                "Should have always-run checks"
            );

            // Verify cache-warmup is present and not on-demand
            let cache_warmup = checks.iter().find(|c| c.id() == "cache-warmup");
            assert!(cache_warmup.is_some(), "Cache warmup should be present");
            assert!(
                !cache_warmup.unwrap().on_demand,
                "Cache warmup should not be on-demand"
            );

            // All checks with triggers should be on-demand (skipped)
            for check in checks.iter() {
                if check.definition.triggers.is_some() {
                    assert!(
                        check.on_demand,
                        "Check '{}' with triggers should be on-demand when no files match",
                        check.id()
                    );
                }
            }
        }

        #[test]
        fn test_empty_config_returns_empty() {
            let config_yaml = r#"
version: 2

docker:
  project_dir: ./infrastructure
  service: php
  shell: bash

git:
  base_branch: development
  fallback_branch: HEAD~1

file_patterns: {}
checks: {}
"#;
            let config: CiConfig =
                serde_yaml::from_str(config_yaml).expect("Failed to parse empty config");
            let changed_files = make_changed_files(vec!["src/Foo.php"]);
            let project_root = PathBuf::from("/tmp/project");

            let checks = determine_checks(&config, &changed_files, &project_root);

            assert_eq!(checks.len(), 0, "Empty config should return no checks");
        }

        #[test]
        fn test_no_matching_patterns_marks_as_on_demand() {
            let config = checks_test_config();
            // Change a file that matches no patterns in config
            let changed_files = make_changed_files(vec!["README.txt", "data.json"]);
            let project_root = PathBuf::from("/tmp/project");

            let checks = determine_checks(&config, &changed_files, &project_root);

            // All checks with triggers should be on-demand
            let checks_with_triggers: Vec<_> = checks
                .iter()
                .filter(|c| c.definition.triggers.is_some())
                .collect();

            assert!(
                !checks_with_triggers.is_empty(),
                "Config should have checks with triggers"
            );

            for check in checks_with_triggers {
                assert!(
                    check.on_demand,
                    "Check '{}' should be on-demand when no files match its patterns",
                    check.id()
                );
            }
        }

        #[test]
        fn test_both_file_pattern_and_test_discovery_triggers() {
            // This test uses the existing test_no_duplicate_test_files_from_multiple_triggers
            // config which has both trigger types
            let config_yaml = r#"
version: 2

docker:
  project_dir: ./infrastructure
  service: php
  shell: bash

git:
  base_branch: development
  fallback_branch: HEAD~1

file_patterns:
  phpunit:
    pattern: 'tests/.*Test\.php$'
  php_src:
    pattern: '^src/.*\.php$'

checks:
  tests:
    checks:
      phpunit:
        name: PHPUnit Tests
        command: phpunit {files}
        triggers:
          file_pattern: phpunit
          test_discovery:
            source_pattern: php_src
            strategies:
              - type: path_mapping
                rules:
                  - source: src/{path}.php
                    tests:
                      - tests/Unit/{path}Test.php
"#;
            let config: CiConfig =
                serde_yaml::from_str(config_yaml).expect("Failed to parse test config");

            let temp_dir = tempfile::tempdir().expect("Failed to create temp dir");
            let test_file_path = temp_dir.path().join("tests/Unit/FooTest.php");
            std::fs::create_dir_all(test_file_path.parent().unwrap())
                .expect("Failed to create test dir");
            std::fs::write(&test_file_path, "<?php // test content")
                .expect("Failed to write test file");

            // Change only a test file (file_pattern trigger)
            let changed_files = make_changed_files(vec!["tests/Unit/FooTest.php"]);
            let checks = determine_checks(&config, &changed_files, temp_dir.path());

            let phpunit = checks.iter().find(|c| c.id() == "phpunit").unwrap();
            assert!(!phpunit.on_demand, "Check should be triggered");
            assert!(
                phpunit
                    .files
                    .contains(&"tests/Unit/FooTest.php".to_string()),
                "Should include the test file"
            );
        }

        #[test]
        fn test_multiple_checks_same_pattern() {
            let config_yaml = r#"
version: 2

docker:
  project_dir: ./infrastructure
  service: php
  shell: bash

git:
  base_branch: development
  fallback_branch: HEAD~1

file_patterns:
  php:
    pattern: '\.php$'

checks:
  group1:
    checks:
      check1:
        name: Check 1
        command: check1 {files}
        triggers:
          file_pattern: php
      check2:
        name: Check 2
        command: check2 {files}
        triggers:
          file_pattern: php
"#;
            let config: CiConfig =
                serde_yaml::from_str(config_yaml).expect("Failed to parse test config");
            let changed_files = make_changed_files(vec!["src/Foo.php"]);
            let project_root = PathBuf::from("/tmp/project");

            let checks = determine_checks(&config, &changed_files, &project_root);

            // Both checks should be triggered
            let check1 = checks.iter().find(|c| c.id() == "check1");
            let check2 = checks.iter().find(|c| c.id() == "check2");

            assert!(check1.is_some(), "check1 should be present");
            assert!(check2.is_some(), "check2 should be present");

            let check1 = check1.unwrap();
            let check2 = check2.unwrap();

            assert!(!check1.on_demand, "check1 should be triggered");
            assert!(!check2.on_demand, "check2 should be triggered");

            assert!(
                check1.files.contains(&"src/Foo.php".to_string()),
                "check1 should have matching file"
            );
            assert!(
                check2.files.contains(&"src/Foo.php".to_string()),
                "check2 should have matching file"
            );
        }
    }

    mod test_resolve_command {
        use super::*;
        use pretty_assertions::assert_eq;

        #[test]
        fn test_files_placeholder_replaced() {
            let config = checks_test_config();
            let check = config
                .groups()
                .find_map(|(_, g)| g.checks.get("php-lint"))
                .expect("php-lint should exist");

            let files = vec!["src/Foo.php", "src/Bar.php"];
            let resolved = resolve_command(&config, check, &files, false);

            assert!(
                resolved.contains("src/Foo.php"),
                "Resolved command should contain first file"
            );
            assert!(
                resolved.contains("src/Bar.php"),
                "Resolved command should contain second file"
            );
            assert!(
                !resolved.contains("{files}"),
                "Resolved command should not contain placeholder"
            );
        }

        #[test]
        fn test_files_placeholder_empty_when_no_files() {
            let config = checks_test_config();
            let check = config
                .groups()
                .find_map(|(_, g)| g.checks.get("php-lint"))
                .expect("php-lint should exist");

            let resolved = resolve_command(&config, check, &[], false);

            assert_eq!(
                resolved, "parallel-lint",
                "Resolved command should have {{files}} replaced with empty string"
            );
        }

        #[test]
        fn test_command_trimmed() {
            let config_yaml = r#"
version: 2

docker:
  project_dir: ./infrastructure
  service: php
  shell: bash

git:
  base_branch: development
  fallback_branch: HEAD~1

file_patterns:
  php:
    pattern: '\.php$'

checks:
  test:
    checks:
      test-check:
        name: Test
        command: "  phpunit   {files}  "
        triggers:
          file_pattern: php
"#;
            let config: CiConfig =
                serde_yaml::from_str(config_yaml).expect("Failed to parse test config");
            let check = config
                .groups()
                .find_map(|(_, g)| g.checks.get("test-check"))
                .expect("test-check should exist");

            let resolved = resolve_command(&config, check, &[], false);

            // Should be trimmed
            assert_eq!(resolved, "phpunit", "Command should be trimmed");
            assert!(
                !resolved.starts_with(' '),
                "Command should not start with space"
            );
            assert!(
                !resolved.ends_with(' '),
                "Command should not end with space"
            );
        }

        #[test]
        fn test_fix_command_resolved() {
            let config = checks_test_config();
            let check = config
                .groups()
                .find_map(|(_, g)| g.checks.get("phpstan"))
                .expect("phpstan should exist");

            let files = vec!["src/Foo.php"];
            let resolved_fix = resolve_command(&config, check, &files, true);

            assert!(
                resolved_fix.contains("phpstan fix"),
                "Fix command should be resolved"
            );
            assert!(
                resolved_fix.contains("src/Foo.php"),
                "Fix command should contain file"
            );
        }
    }

    mod test_group_checks {
        use super::*;
        use pretty_assertions::assert_eq;

        #[test]
        fn test_empty_checks_returns_empty_groups() {
            let checks: Vec<CheckToRun> = vec![];
            let grouped = group_checks(&checks);

            assert_eq!(grouped.len(), 0, "Empty checks should return empty groups");
        }

        #[test]
        fn test_single_group() {
            let checks = vec![
                make_check("check1", "group1", "Check 1", "cmd1", false, false),
                make_check("check2", "group1", "Check 2", "cmd2", false, false),
                make_check("check3", "group1", "Check 3", "cmd3", false, false),
            ];

            let grouped = group_checks(&checks);

            assert_eq!(grouped.len(), 1, "Should have one group");
            assert_eq!(grouped[0].0, "group1", "Group name should match");
            assert_eq!(grouped[0].1.len(), 3, "Group should have 3 checks");
        }

        #[test]
        fn test_multiple_groups_preserve_order() {
            let checks = vec![
                make_check("check1", "warmup", "Check 1", "cmd1", false, false),
                make_check("check2", "fast", "Check 2", "cmd2", false, false),
                make_check("check3", "analysis", "Check 3", "cmd3", false, false),
                make_check("check4", "fast", "Check 4", "cmd4", false, false),
            ];

            let grouped = group_checks(&checks);

            assert_eq!(grouped.len(), 3, "Should have 3 groups");

            // Groups should appear in order of first occurrence
            let group_names: Vec<&str> = grouped.iter().map(|(name, _)| *name).collect();
            assert_eq!(
                group_names,
                vec!["warmup", "fast", "analysis"],
                "Groups should be in order of first occurrence"
            );

            // fast group should have 2 checks
            let fast_group = grouped.iter().find(|(name, _)| *name == "fast").unwrap();
            assert_eq!(fast_group.1.len(), 2, "fast group should have 2 checks");
        }
    }
}
