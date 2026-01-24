//! Configuration loading and parsing from YAML files.
//!
//! This module handles all configuration-related types and loading logic for CI-TUI.
//! The configuration defines Docker settings, git base branches, file patterns for
//! triggering checks, and the checks themselves organized into execution groups.
//!
//! # Key Types
//!
//! - [`CiConfig`]: Root configuration type containing all settings
//! - [`GroupConfig`]: Configuration for a group of checks
//! - [`CheckDefinition`]: Definition of a single CI check
//! - [`TestDiscoveryConfig`]: Settings for automatic test file discovery
//!
//! # Example
//!
//! ```ignore
//! let config = load_config(Path::new("ci-tui.yaml"))?;
//! for (group_name, group) in config.groups() {
//!     println!("Group: {}", group.display_name(group_name));
//! }
//! ```

use anyhow::{Context, Result};
use indexmap::IndexMap;
use regex::Regex;
use serde::Deserialize;
use std::collections::HashMap;
use std::path::Path;
use std::sync::OnceLock;

/// Root configuration type for CI-TUI.
///
/// Contains all settings needed to run CI checks including Docker configuration,
/// git settings, file patterns, and check definitions organized into groups.
#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct CiConfig {
    pub version: u32,
    pub docker: DockerConfig,
    pub git: GitConfig,
    pub file_patterns: HashMap<String, FilePattern>,
    /// Check groups in execution order (YAML key order preserved)
    pub checks: IndexMap<String, GroupConfig>,
    #[serde(default)]
    pub ignore_patterns: Vec<String>,
    /// Cached compiled ignore patterns (lazily initialized)
    #[serde(skip)]
    compiled_ignore_patterns: OnceLock<Vec<Regex>>,
}

/// File pattern definition with optional color for UI display
#[derive(Debug, Clone, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct FilePattern {
    /// Regex pattern for matching files
    pub pattern: String,
    /// Optional color for files matching this pattern (e.g., "blue", "green")
    #[serde(default)]
    pub color: Option<String>,
}

impl Clone for CiConfig {
    fn clone(&self) -> Self {
        Self {
            version: self.version,
            docker: self.docker.clone(),
            git: self.git.clone(),
            file_patterns: self.file_patterns.clone(),
            checks: self.checks.clone(),
            ignore_patterns: self.ignore_patterns.clone(),
            // Reset cache on clone - will be lazily recomputed
            compiled_ignore_patterns: OnceLock::new(),
        }
    }
}

/// Docker configuration for running checks in containers.
///
/// Supports both `docker exec` (for running containers) and `docker run`
/// (for standalone execution).
#[derive(Debug, Clone, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct DockerConfig {
    pub project_dir: String,
    /// Default Docker service name (defaults to "app" if not specified)
    #[serde(default = "default_service")]
    pub service: String,
    /// Container name for docker exec (e.g., "myproject-app-1")
    /// If not set, derived from project_dir + service using compose naming convention
    #[serde(default)]
    pub container: Option<String>,
    /// Docker image for standalone run (e.g., "rust:latest")
    /// If set, use this image directly instead of deriving from container name
    #[serde(default)]
    pub image: Option<String>,
    /// Volume mount string for standalone run (e.g., ".:/build")
    /// Full volume specification including source:dest
    #[serde(default)]
    pub volume_mount: Option<String>,
    /// Working directory inside container (e.g., "/build")
    /// Overrides the default /app workdir
    #[serde(default)]
    pub work_dir: Option<String>,
    /// Environment variables to pass to all docker exec commands
    #[serde(default)]
    pub env: std::collections::HashMap<String, String>,
}

impl DockerConfig {
    /// Get the container name to use for docker exec
    /// Returns explicit container name if set, otherwise derives from project_dir + service
    pub fn container_name(&self) -> String {
        if let Some(ref container) = self.container {
            return container.clone();
        }

        // Derive container name from project_dir and service
        // Extract project name from project_dir (last path component)
        // Handle "." and relative paths by resolving to absolute path first
        let path = std::path::Path::new(&self.project_dir);
        let project_name = if self.project_dir == "." || self.project_dir == ".." {
            // Resolve relative path to get actual directory name
            std::env::current_dir()
                .ok()
                .and_then(|cwd| {
                    let resolved = if self.project_dir == "." {
                        cwd
                    } else {
                        cwd.parent()?.to_path_buf()
                    };
                    resolved.file_name()?.to_str().map(String::from)
                })
                .unwrap_or_else(|| "project".to_string())
        } else {
            path.file_name()
                .and_then(|n| n.to_str())
                .unwrap_or("project")
                .to_string()
        };

        // Docker Compose naming convention: {project}-{service}-1
        format!("{}-{}-1", project_name, self.service)
    }

    /// Get the Docker image name to use
    /// Returns explicit image if set, otherwise derives from container_name by stripping -1 suffix
    pub fn image_name(&self) -> String {
        if let Some(ref image) = self.image {
            return image.clone();
        }

        // Derive image from container name (strip -1 suffix)
        self.container_name().trim_end_matches("-1").to_string()
    }

    /// Get volume mount arguments for docker run
    /// Returns "-v {volume_mount}" if volume_mount is set, None otherwise
    /// Expands environment variables in format ${VAR_NAME}
    pub fn volume_args(&self) -> Option<String> {
        self.volume_mount.as_ref().map(|mount| {
            let expanded = expand_env_vars(mount);
            format!("-v {}", expanded)
        })
    }

    /// Get the working directory inside container
    /// Returns work_dir if set, otherwise "/app"
    pub fn working_dir(&self) -> &str {
        self.work_dir.as_deref().unwrap_or("/app")
    }
}

fn default_service() -> String {
    "app".to_string()
}

/// Expand environment variables in format ${VAR_NAME}
fn expand_env_vars(input: &str) -> String {
    let mut result = input.to_string();
    let re = Regex::new(r"\$\{([^}]+)\}").unwrap();

    for cap in re.captures_iter(input) {
        let var_name = &cap[1];
        if let Ok(value) = std::env::var(var_name) {
            result = result.replace(&cap[0], &value);
        }
    }

    result
}

/// Git configuration for change detection.
#[derive(Debug, Clone, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct GitConfig {
    pub base_branch: String,
    pub fallback_branch: String,
}

/// Configuration for an execution group
#[derive(Debug, Clone, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct GroupConfig {
    /// Optional display name for the group (uses key name if not set)
    #[serde(default)]
    pub name: Option<String>,
    /// Run checks in this group in parallel
    #[serde(default)]
    pub parallel: bool,
    /// Stop all execution if any check in this group fails
    #[serde(default)]
    pub stop_on_failure: bool,
    /// Commands to run before checks in this group (e.g., DB init)
    #[serde(default)]
    pub pre_commands: Vec<PreCommand>,
    /// Checks in this group (key is check ID)
    pub checks: IndexMap<String, CheckDefinition>,
}

/// Definition of a single CI check.
///
/// Contains the command to run, optional fix command, triggers for when
/// to run, and other configuration options.
#[derive(Debug, Clone, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct CheckDefinition {
    pub name: String,
    pub command: String,
    /// Docker service to run this check in (overrides group/global default)
    #[serde(default)]
    pub service: Option<String>,
    /// Explicit container name (overrides service-based derivation)
    #[serde(default)]
    pub container: Option<String>,
    #[serde(default)]
    pub fix_command: Option<String>,
    #[serde(default)]
    pub triggers: Option<CheckTriggers>,
    /// If true, check requires manual trigger when source files change but no specific tests found
    #[serde(default)]
    pub on_demand: bool,
    /// Additional environment variables for this check (merged with global env)
    #[serde(default)]
    pub env: std::collections::HashMap<String, String>,
}

/// Triggers that determine when a check should run.
#[derive(Debug, Clone, Deserialize, Default)]
#[serde(deny_unknown_fields)]
pub struct CheckTriggers {
    #[serde(default)]
    pub file_pattern: Option<String>,
    #[serde(default)]
    pub test_discovery: Option<TestDiscoveryConfig>,
}

/// Inline test discovery configuration for a check
#[derive(Debug, Clone, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct TestDiscoveryConfig {
    /// Source file pattern key that triggers discovery (references file_patterns)
    pub source_pattern: String,
    /// Strategies to find related tests
    pub strategies: Vec<TestDiscoveryStrategy>,
}

/// Strategy for discovering related test files
#[derive(Debug, Clone, Deserialize)]
#[serde(tag = "type", deny_unknown_fields)]
pub enum TestDiscoveryStrategy {
    /// Map source paths to test paths by pattern
    #[serde(rename = "path_mapping")]
    PathMapping { rules: Vec<PathMappingRule> },
    /// Search test files for content matching a pattern
    #[serde(rename = "grep_search")]
    GrepSearch {
        /// Directories to search in
        search_dirs: Vec<String>,
        /// Regex pattern with placeholders: {basename}, {filename}, {extension}, {dirname}, {path}
        pattern: String,
    },
}

/// Rule for mapping source paths to test paths
#[derive(Debug, Clone, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct PathMappingRule {
    /// Source file pattern with {path} placeholder (e.g., "src/{path}.php")
    pub source: String,
    /// Test file patterns with {path} placeholder (e.g., "tests/{path}Test.php")
    pub tests: Vec<String>,
}

/// A command to run before checks in a group (e.g., database initialization).
#[derive(Debug, Clone, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct PreCommand {
    /// Display name for the command
    pub name: String,
    /// The command to execute
    pub command: String,
    /// Docker service to run in (uses group/global default if not specified)
    #[serde(default)]
    pub service: Option<String>,
    /// Explicit container name (overrides service-based derivation)
    #[serde(default)]
    pub container: Option<String>,
    /// If true, use `docker compose exec` (existing container) instead of `run` (new container)
    #[serde(default)]
    pub exec: bool,
    /// Additional environment variables for this command (merged with global env)
    #[serde(default)]
    pub env: std::collections::HashMap<String, String>,
}

/// Load configuration from a YAML file.
///
/// # Errors
///
/// Returns an error if the file cannot be read or parsed.
pub fn load_config(path: &Path) -> Result<CiConfig> {
    let content = std::fs::read_to_string(path)
        .with_context(|| format!("Failed to read config file: {}", path.display()))?;

    serde_yaml::from_str(&content)
        .with_context(|| format!("Failed to parse config file: {}", path.display()))
}

impl CiConfig {
    /// Get the regex pattern for a file pattern key
    pub fn get_file_pattern(&self, key: &str) -> Option<&str> {
        self.file_patterns.get(key).map(|fp| fp.pattern.as_str())
    }

    /// Check if a file should be ignored based on ignore_patterns
    pub fn should_ignore_file(&self, path: &str) -> bool {
        let patterns = self.compiled_ignore_patterns.get_or_init(|| {
            self.ignore_patterns
                .iter()
                .filter_map(|p| Regex::new(p).ok())
                .collect()
        });
        patterns.iter().any(|re| re.is_match(path))
    }

    /// Get color name for a file based on file_patterns
    pub fn get_file_color(&self, path: &str) -> &str {
        // Check if any file_pattern with a color matches this file
        for fp in self.file_patterns.values() {
            if let Some(ref color) = fp.color {
                if let Ok(re) = Regex::new(&fp.pattern) {
                    if re.is_match(path) {
                        return color.as_str();
                    }
                }
            }
        }
        "white"
    }

    /// Get the default Docker service (from docker config)
    pub fn default_service(&self) -> &str {
        &self.docker.service
    }

    /// Get group config by name
    pub fn get_group(&self, group_name: &str) -> Option<&GroupConfig> {
        self.checks.get(group_name)
    }

    /// Iterate over groups in execution order
    pub fn groups(&self) -> impl Iterator<Item = (&str, &GroupConfig)> {
        self.checks.iter().map(|(k, v)| (k.as_str(), v))
    }
}

impl GroupConfig {
    /// Get display name (uses provided name or falls back to key)
    pub fn display_name<'a>(&'a self, key: &'a str) -> &'a str {
        self.name.as_deref().unwrap_or(key)
    }
}

impl CheckDefinition {
    /// Get the service for this check, with fallback to default
    pub fn service_or_default<'a>(&'a self, default: &'a str) -> &'a str {
        self.service.as_deref().unwrap_or(default)
    }

    /// Check if this check should always run (no triggers defined)
    pub fn always_run(&self) -> bool {
        self.triggers.is_none()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use pretty_assertions::assert_eq;
    use rstest::rstest;

    fn minimal_config_yaml() -> &'static str {
        r#"
version: 2

docker:
  project_dir: ./infrastructure
  service: php
  # container: myproject-php-1  # Optional: explicit container name for docker exec

git:
  base_branch: development
  fallback_branch: HEAD~1

file_patterns:
  php:
    pattern: '\.php$'
  php_src:
    pattern: '^src/'
    color: blue
  tests:
    pattern: 'tests/.*\.php$'
    color: green

ignore_patterns:
  - '\.md$'
  - '\.github/'

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

  tests:
    checks:
      phpunit:
        name: PHPUnit Tests
        command: phpunit {files}
        service: custom-service
        triggers:
          file_pattern: tests
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

    #[test]
    fn test_should_ignore_file() {
        let config = parse_test_config();

        // Should ignore markdown files
        assert!(config.should_ignore_file("README.md"));
        assert!(config.should_ignore_file("docs/CONTRIBUTING.md"));

        // Should ignore .github directory
        assert!(config.should_ignore_file(".github/workflows/ci.yml"));

        // Should not ignore other files
        assert!(!config.should_ignore_file("src/Service/Foo.php"));
        assert!(!config.should_ignore_file("tests/FooTest.php"));
    }

    #[test]
    fn test_get_file_color() {
        let config = parse_test_config();

        // Files matching patterns with colors
        assert_eq!(config.get_file_color("src/Service/Foo.php"), "blue");
        assert_eq!(config.get_file_color("tests/Unit/FooTest.php"), "green");

        // Files not matching any color pattern
        assert_eq!(config.get_file_color("composer.json"), "white");
    }

    #[test]
    fn test_default_service() {
        let config = parse_test_config();
        assert_eq!(config.default_service(), "php");
    }

    #[test]
    fn test_get_group() {
        let config = parse_test_config();

        let fast = config.get_group("fast");
        assert!(fast.is_some());
        assert_eq!(fast.unwrap().name, Some("Fast Checks".to_string()));

        let tests = config.get_group("tests");
        assert!(tests.is_some());
        assert_eq!(tests.unwrap().name, None);

        assert!(config.get_group("nonexistent").is_none());
    }

    #[test]
    fn test_groups_preserves_order() {
        let config = parse_test_config();

        let group_names: Vec<&str> = config.groups().map(|(name, _)| name).collect();
        assert_eq!(group_names, vec!["fast", "tests"]);
    }

    #[test]
    fn test_group_display_name() {
        let config = parse_test_config();

        let fast = config.get_group("fast").unwrap();
        assert_eq!(fast.display_name("fast"), "Fast Checks");

        let tests = config.get_group("tests").unwrap();
        assert_eq!(tests.display_name("tests"), "tests"); // Falls back to key
    }

    #[test]
    fn test_check_service_or_default() {
        let config = parse_test_config();

        let fast = config.get_group("fast").unwrap();
        let php_lint = fast.checks.get("php-lint").unwrap();
        assert_eq!(php_lint.service_or_default("default"), "default");

        let tests = config.get_group("tests").unwrap();
        let phpunit = tests.checks.get("phpunit").unwrap();
        assert_eq!(phpunit.service_or_default("default"), "custom-service");
    }

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

    #[test]
    fn test_parallel_and_stop_on_failure() {
        let config = parse_test_config();

        let fast = config.get_group("fast").unwrap();
        assert!(fast.parallel);
        assert!(!fast.stop_on_failure);

        let tests = config.get_group("tests").unwrap();
        assert!(!tests.parallel);
    }

    #[test]
    fn test_unknown_field_rejected_top_level() {
        let yaml = r#"
version: 2
docker:
  project_dir: .
git:
  base_branch: main
  fallback_branch: HEAD~1
file_patterns: {}
checks: {}
typo_field: oops
"#;
        let result: Result<CiConfig, _> = serde_yaml::from_str(yaml);
        assert!(result.is_err());
        let err = result.unwrap_err().to_string();
        assert!(
            err.contains("typo_field") || err.contains("unknown field"),
            "Error should mention unknown field: {}",
            err
        );
    }

    #[test]
    fn test_unknown_field_rejected_nested() {
        let yaml = r#"
version: 2
docker:
  project_dir: .
  service: app
  unknown_docker_field: invalid
git:
  base_branch: main
  fallback_branch: HEAD~1
file_patterns: {}
checks: {}
"#;
        let result: Result<CiConfig, _> = serde_yaml::from_str(yaml);
        assert!(result.is_err());
        let err = result.unwrap_err().to_string();
        assert!(
            err.contains("unknown_docker_field") || err.contains("unknown field"),
            "Error should mention unknown field in docker config: {}",
            err
        );
    }

    #[test]
    fn test_typo_produces_helpful_error() {
        let yaml = r#"
version: 2
docker:
  project_dir: .
git:
  base_branc: main
  fallback_branch: HEAD~1
file_patterns: {}
checks: {}
"#;
        let result: Result<CiConfig, _> = serde_yaml::from_str(yaml);
        assert!(result.is_err());
        let err = result.unwrap_err().to_string();
        // Should mention the typo field or that base_branch is missing
        assert!(
            err.contains("base_branc")
                || err.contains("unknown field")
                || err.contains("base_branch"),
            "Error should mention typo field or missing required field: {}",
            err
        );
    }

    mod test_file_patterns {
        use super::*;

        #[rstest]
        #[case("php", Some(r"\.php$"))]
        #[case("php_src", Some("^src/"))]
        #[case("tests", Some(r"tests/.*\.php$"))]
        #[case("nonexistent", None)]
        #[case("", None)] // Empty key
        fn test_get_file_pattern(#[case] key: &str, #[case] expected: Option<&str>) {
            let config = parse_test_config();
            assert_eq!(config.get_file_pattern(key), expected);
        }
    }

    mod test_should_ignore_file {
        use super::*;

        #[rstest]
        #[case("README.md", true)] // Matches \.md$
        #[case("docs/CONTRIBUTING.md", true)]
        #[case(".github/workflows/ci.yml", true)] // Matches \.github/
        #[case("src/Service/Foo.php", false)]
        #[case("tests/FooTest.php", false)]
        #[case("", false)] // Empty path
        fn test_should_ignore_file(#[case] path: &str, #[case] should_ignore: bool) {
            let config = parse_test_config();
            assert_eq!(config.should_ignore_file(path), should_ignore);
        }
    }

    mod test_get_file_color {
        use super::*;

        #[rstest]
        #[case("src/Service/Foo.php", "blue")] // Matches php_src with blue
        #[case("tests/Unit/FooTest.php", "green")] // Matches tests with green
        #[case("composer.json", "white")] // No color defined
        #[case("", "white")] // Empty path
        fn test_get_file_color(#[case] path: &str, #[case] expected_color: &str) {
            let config = parse_test_config();
            assert_eq!(config.get_file_color(path), expected_color);
        }
    }

    mod test_docker_config {
        use super::*;

        #[test]
        fn test_container_name_explicit() {
            let yaml = r#"
version: 2
docker:
  project_dir: ./infrastructure
  service: php
  container: explicit-container-name
git:
  base_branch: main
  fallback_branch: HEAD~1
file_patterns: {}
checks: {}
"#;
            let config: CiConfig = serde_yaml::from_str(yaml).unwrap();
            assert_eq!(config.docker.container_name(), "explicit-container-name");
        }

        #[test]
        fn test_container_name_derived() {
            let yaml = r#"
version: 2
docker:
  project_dir: ./infrastructure
  service: php
git:
  base_branch: main
  fallback_branch: HEAD~1
file_patterns: {}
checks: {}
"#;
            let config: CiConfig = serde_yaml::from_str(yaml).unwrap();
            assert_eq!(config.docker.container_name(), "infrastructure-php-1");
        }

        #[test]
        fn test_container_name_current_dir() {
            let yaml = r#"
version: 2
docker:
  project_dir: .
  service: app
git:
  base_branch: main
  fallback_branch: HEAD~1
file_patterns: {}
checks: {}
"#;
            let config: CiConfig = serde_yaml::from_str(yaml).unwrap();
            let container_name = config.docker.container_name();
            // Should derive from current directory name
            assert!(container_name.ends_with("-app-1"));
            assert_ne!(container_name, ".-app-1"); // Should not use literal "."
        }

        #[test]
        fn test_image_name_explicit() {
            let yaml = r#"
version: 2
docker:
  project_dir: .
  service: app
  image: rust:latest
git:
  base_branch: main
  fallback_branch: HEAD~1
file_patterns: {}
checks: {}
"#;
            let config: CiConfig = serde_yaml::from_str(yaml).unwrap();
            assert_eq!(config.docker.image_name(), "rust:latest");
        }

        #[test]
        fn test_image_name_derived() {
            let yaml = r#"
version: 2
docker:
  project_dir: ./myproject
  service: web
git:
  base_branch: main
  fallback_branch: HEAD~1
file_patterns: {}
checks: {}
"#;
            let config: CiConfig = serde_yaml::from_str(yaml).unwrap();
            // Image derived from container name by stripping -1 suffix
            assert_eq!(config.docker.image_name(), "myproject-web");
        }

        #[test]
        fn test_volume_args() {
            let yaml = r#"
version: 2
docker:
  project_dir: .
  service: app
  volume_mount: ".:/build"
git:
  base_branch: main
  fallback_branch: HEAD~1
file_patterns: {}
checks: {}
"#;
            let config: CiConfig = serde_yaml::from_str(yaml).unwrap();
            assert_eq!(config.docker.volume_args(), Some("-v .:/build".to_string()));
        }

        #[test]
        fn test_working_dir_default() {
            let yaml = r#"
version: 2
docker:
  project_dir: .
  service: app
git:
  base_branch: main
  fallback_branch: HEAD~1
file_patterns: {}
checks: {}
"#;
            let config: CiConfig = serde_yaml::from_str(yaml).unwrap();
            assert_eq!(config.docker.working_dir(), "/app");
        }

        #[test]
        fn test_working_dir_custom() {
            let yaml = r#"
version: 2
docker:
  project_dir: .
  service: app
  work_dir: "/custom/path"
git:
  base_branch: main
  fallback_branch: HEAD~1
file_patterns: {}
checks: {}
"#;
            let config: CiConfig = serde_yaml::from_str(yaml).unwrap();
            assert_eq!(config.docker.working_dir(), "/custom/path");
        }
    }
}
