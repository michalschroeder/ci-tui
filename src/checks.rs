use crate::config::{CheckDefinition, CiConfig};
use crate::git::ChangedFiles;
use crate::test_discovery;
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

/// Determine which checks should run based on changed files
/// Returns ALL checks from config - those that match are set to run,
/// those that don't match are marked as skipped (can be run on-demand)
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
                                } else {
                                    // Fall back to running all when no related tests found
                                    matched_files
                                        .push("(source files changed - running all)".to_string());
                                }
                            }
                        }
                    }
                }

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
    use std::path::PathBuf;

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
    name: Cache Warmup
    checks:
      cache-warmup:
        name: Cache warmup
        command: bin/console cache:warmup

  fast:
    name: Fast Checks
    parallel: true
    checks:
      php-lint:
        name: PHP syntax check
        command: parallel-lint {files}
        triggers:
          file_pattern: php

      yaml-lint:
        name: YAML syntax check
        command: yaml-lint {files}
        triggers:
          file_pattern: yaml

  analysis:
    checks:
      phpstan:
        name: PHPStan
        command: phpstan analyse {files}
        fix_command: phpstan fix {files}
        triggers:
          file_pattern: php
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
        let config = parse_config();
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
        let config = parse_config();
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
        let config = parse_config();
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
        let config = parse_config();
        let changed_files = make_changed_files(vec!["src/Service/Foo.php", "src/Service/Bar.php"]);
        let project_root = PathBuf::from("/tmp/project");

        let checks = determine_checks(&config, &changed_files, &project_root);

        let php_lint = checks.iter().find(|c| c.id() == "php-lint").unwrap();
        assert!(php_lint.resolved_command.contains("src/Service/Foo.php"));
        assert!(php_lint.resolved_command.contains("src/Service/Bar.php"));
    }

    #[test]
    fn test_check_has_fix_command() {
        let config = parse_config();
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
        let config = parse_config();
        let changed_files = make_changed_files(vec!["src/Service/Foo.php"]);
        let project_root = PathBuf::from("/tmp/project");

        let checks = determine_checks(&config, &changed_files, &project_root);
        let grouped = group_checks(&checks);

        let group_names: Vec<&str> = grouped.iter().map(|(name, _)| *name).collect();
        assert_eq!(group_names, vec!["warmup", "fast", "analysis"]);
    }

    #[test]
    fn test_check_to_run_accessors() {
        let config = parse_config();
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
        let config = parse_config();
        let changed_files = make_changed_files(vec!["src/Service/Foo.php"]);
        let project_root = PathBuf::from("/tmp/project");

        let checks = determine_checks(&config, &changed_files, &project_root);

        let php_lint = checks.iter().find(|c| c.id() == "php-lint").unwrap();
        let all_files_cmd = php_lint.get_command_for_all_files();

        // Should have {files} replaced with empty and cleaned up
        assert_eq!(all_files_cmd, "parallel-lint");
        assert!(!all_files_cmd.contains("{files}"));
    }
}
