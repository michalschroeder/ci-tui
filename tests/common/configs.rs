//! Shared test fixtures and builders for CI configuration
//!
//! This module provides reusable configuration builders and fixtures for tests,
//! eliminating the need for 40+ lines of inline YAML config per test.
//!
//! # When to use what
//!
//! - **Fixtures**: Use predefined fixtures like `minimal_config()`, `php_project_config()`,
//!   or `rust_project_config()` for common test scenarios
//! - **Builder**: Use `ConfigBuilder::new().with_*().build()` when you need custom
//!   configuration with specific patterns, checks, or settings
//! - **Inline**: Keep truly unique edge-case configs inline with `// Edge case: ...` comment
//!
//! # Basic usage examples
//!
//! ```rust
//! use crate::common::configs::{minimal_config, ConfigBuilder, CheckBuilder};
//!
//! // Use a fixture for common scenarios
//! let config = rust_project_config();
//!
//! // Build custom config with builder pattern
//! let config = ConfigBuilder::new()
//!     .with_file_pattern("php", r"\.php$", Some("yellow"))
//!     .with_check("lint", "phpcs", CheckBuilder::new("PHP CodeSniffer", "phpcs")
//!         .with_file_pattern_trigger("php")
//!         .build())
//!     .build();
//! ```

use ci_tui::config::{
    CheckDefinition, CheckTriggers, CiConfig, DockerConfig, FilePattern, GitConfig, GroupConfig,
    PreCommand, TestDiscoveryConfig,
};
use indexmap::IndexMap;
use std::collections::HashMap;

/// Builder for creating `CiConfig` instances in tests
///
/// Starts with minimal valid defaults and allows customization through fluent API.
pub struct ConfigBuilder {
    version: u32,
    docker_project_dir: String,
    docker_service: String,
    docker_shell: String,
    docker_env: HashMap<String, String>,
    git_base: String,
    git_fallback: String,
    file_patterns: HashMap<String, FilePattern>,
    checks: IndexMap<String, GroupConfig>,
    ignore_patterns: Vec<String>,
}

impl ConfigBuilder {
    /// Create a new builder with minimal valid configuration
    pub fn new() -> Self {
        Self {
            version: 2,
            docker_project_dir: "./test".to_string(),
            docker_service: "app".to_string(),
            docker_shell: "bash".to_string(),
            docker_env: HashMap::new(),
            git_base: "main".to_string(),
            git_fallback: "HEAD~1".to_string(),
            file_patterns: HashMap::new(),
            checks: IndexMap::new(),
            ignore_patterns: Vec::new(),
        }
    }

    /// Set Docker configuration
    pub fn with_docker(mut self, project_dir: &str, service: &str) -> Self {
        self.docker_project_dir = project_dir.to_string();
        self.docker_service = service.to_string();
        self
    }

    /// Set git branch configuration
    pub fn with_git_branches(mut self, base: &str, fallback: &str) -> Self {
        self.git_base = base.to_string();
        self.git_fallback = fallback.to_string();
        self
    }

    /// Add a file pattern
    pub fn with_file_pattern(mut self, name: &str, pattern: &str, color: Option<&str>) -> Self {
        self.file_patterns.insert(
            name.to_string(),
            FilePattern {
                pattern: pattern.to_string(),
                color: color.map(String::from),
            },
        );
        self
    }

    /// Add a check to a group (creates group if it doesn't exist)
    pub fn with_check(mut self, group_id: &str, check_id: &str, check: CheckDefinition) -> Self {
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

    /// Create or update a group to run checks in parallel
    pub fn with_parallel_group(mut self, group_id: &str) -> Self {
        self.checks
            .entry(group_id.to_string())
            .or_insert_with(|| GroupConfig {
                name: None,
                parallel: true,
                stop_on_failure: false,
                pre_commands: Vec::new(),
                checks: IndexMap::new(),
            })
            .parallel = true;
        self
    }

    /// Set the display name for a group
    pub fn with_group_name(mut self, group_id: &str, name: &str) -> Self {
        self.checks
            .entry(group_id.to_string())
            .or_insert_with(|| GroupConfig {
                name: None,
                parallel: false,
                stop_on_failure: false,
                pre_commands: Vec::new(),
                checks: IndexMap::new(),
            })
            .name = Some(name.to_string());
        self
    }

    /// Set ignore patterns for file filtering
    pub fn with_ignore_patterns(mut self, patterns: Vec<&str>) -> Self {
        self.ignore_patterns = patterns.into_iter().map(String::from).collect();
        self
    }

    /// Add a pre-command to a group (creates group if it doesn't exist)
    #[allow(dead_code)]
    pub fn with_pre_command(mut self, group_id: &str, name: &str, command: &str) -> Self {
        self.checks
            .entry(group_id.to_string())
            .or_insert_with(|| GroupConfig {
                name: None,
                parallel: false,
                stop_on_failure: false,
                pre_commands: Vec::new(),
                checks: IndexMap::new(),
            })
            .pre_commands
            .push(PreCommand {
                name: name.to_string(),
                command: command.to_string(),
                service: None,
                container: None,
                exec: false,
                env: std::collections::HashMap::new(),
            });
        self
    }

    /// Build the final CiConfig
    pub fn build(self) -> CiConfig {
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
                env: self.docker_env,
            },
            GitConfig {
                base_branch: self.git_base,
                fallback_branch: self.git_fallback,
            },
            self.file_patterns,
            self.checks,
            self.ignore_patterns,
        )
    }
}

impl Default for ConfigBuilder {
    fn default() -> Self {
        Self::new()
    }
}

/// Builder for creating `CheckDefinition` instances in tests
///
/// Provides fluent API for constructing check definitions with optional fields.
pub struct CheckBuilder {
    name: String,
    command: String,
    service: Option<String>,
    fix_command: Option<String>,
    triggers: Option<CheckTriggers>,
    on_demand: bool,
    env: HashMap<String, String>,
}

impl CheckBuilder {
    /// Create a new check builder with required fields
    pub fn new(name: &str, command: &str) -> Self {
        Self {
            name: name.to_string(),
            command: command.to_string(),
            service: None,
            fix_command: None,
            triggers: None,
            on_demand: false,
            env: HashMap::new(),
        }
    }

    /// Set the Docker service for this check
    pub fn with_service(mut self, service: &str) -> Self {
        self.service = Some(service.to_string());
        self
    }

    /// Add a fix command
    pub fn with_fix_command(mut self, fix_cmd: &str) -> Self {
        self.fix_command = Some(fix_cmd.to_string());
        self
    }

    /// Set file pattern trigger
    pub fn with_file_pattern_trigger(mut self, pattern_name: &str) -> Self {
        let triggers = self.triggers.get_or_insert_with(CheckTriggers::default);
        triggers.file_pattern = Some(pattern_name.to_string());
        self
    }

    /// Set test discovery configuration
    #[allow(dead_code)]
    pub fn with_test_discovery(mut self, config: TestDiscoveryConfig) -> Self {
        let triggers = self.triggers.get_or_insert_with(CheckTriggers::default);
        triggers.test_discovery = Some(config);
        self
    }

    /// Mark this check as on-demand
    pub fn on_demand(mut self) -> Self {
        self.on_demand = true;
        self
    }

    /// Build the final CheckDefinition
    pub fn build(self) -> CheckDefinition {
        CheckDefinition {
            name: self.name,
            command: self.command,
            service: self.service,
            container: None,
            fix_command: self.fix_command,
            triggers: self.triggers,
            on_demand: self.on_demand,
            env: self.env,
        }
    }
}

/// Minimal valid config with docker, git, no checks
pub fn minimal_config() -> CiConfig {
    ConfigBuilder::new().build()
}

/// PHP project with lint check and common patterns (matches original minimal_config_yaml from config.rs tests)
pub fn php_project_config() -> CiConfig {
    ConfigBuilder::new()
        .with_docker("./infrastructure", "php")
        .with_git_branches("development", "HEAD~1")
        .with_file_pattern("php", r"\.php$", None)
        .with_file_pattern("php_src", r"^src/", Some("blue"))
        .with_file_pattern("tests", r"tests/.*\.php$", Some("green"))
        .with_ignore_patterns(vec![r"\.md$", r"\.github/"])
        .with_check(
            "fast",
            "php-lint",
            CheckBuilder::new("PHP syntax check", "php-lint {files}")
                .with_file_pattern_trigger("php")
                .build(),
        )
        .with_group_name("fast", "Fast Checks")
        .with_parallel_group("fast")
        .with_check(
            "tests",
            "phpunit",
            CheckBuilder::new("PHPUnit Tests", "phpunit {files}")
                .with_service("custom-service")
                .with_file_pattern_trigger("tests")
                .build(),
        )
        .build()
}

/// Rust project with clippy, fmt, and test checks
pub fn rust_project_config() -> CiConfig {
    ConfigBuilder::new()
        .with_file_pattern("rust", r"\.rs$", Some("yellow"))
        .with_file_pattern("toml", r"\.toml$", Some("cyan"))
        .with_parallel_group("lint")
        .with_check(
            "lint",
            "clippy",
            CheckBuilder::new("Clippy", "cargo clippy")
                .with_file_pattern_trigger("rust")
                .build(),
        )
        .with_check(
            "lint",
            "fmt",
            CheckBuilder::new("Format Check", "cargo fmt --check")
                .with_fix_command("cargo fmt")
                .with_file_pattern_trigger("rust")
                .build(),
        )
        .with_check(
            "test",
            "unit",
            CheckBuilder::new("Unit Tests", "cargo test")
                .with_file_pattern_trigger("rust")
                .build(),
        )
        .build()
}

/// Config with common ignore patterns (\.md$, \.github/) - matches test expectations
pub fn config_with_ignore_patterns() -> CiConfig {
    ConfigBuilder::new()
        .with_ignore_patterns(vec![r"\.md$", r"\.github/"])
        .build()
}

/// Config with custom ignore patterns for filtering tests
pub fn config_with_custom_ignore_patterns(patterns: Vec<&str>) -> CiConfig {
    ConfigBuilder::new().with_ignore_patterns(patterns).build()
}

/// Config with a check that always runs (no triggers)
pub fn config_with_always_run_check() -> CiConfig {
    ConfigBuilder::new()
        .with_check(
            "always",
            "always-run",
            CheckBuilder::new("Always Run Check", "echo 'always runs'").build(),
        )
        .build()
}

/// Config for widget tests with Rust lint and test groups
///
/// Provides a Rust project config matching the original widget_test_config_yaml():
/// - lint group (parallel): clippy, fmt (with fix_command)
/// - test group: unit tests
pub fn widget_test_config() -> CiConfig {
    ConfigBuilder::new()
        .with_docker("./test", "app")
        .with_git_branches("main", "HEAD~1")
        .with_file_pattern("rust", r"\.rs$", Some("yellow"))
        .with_file_pattern("toml", r"\.toml$", Some("cyan"))
        .with_parallel_group("lint")
        .with_group_name("lint", "Lint")
        .with_check(
            "lint",
            "clippy",
            CheckBuilder::new("Clippy", "cargo clippy")
                .with_file_pattern_trigger("rust")
                .build(),
        )
        .with_check(
            "lint",
            "fmt",
            CheckBuilder::new("Format Check", "cargo fmt --check")
                .with_fix_command("cargo fmt")
                .with_file_pattern_trigger("rust")
                .build(),
        )
        .with_group_name("test", "Tests")
        .with_check(
            "test",
            "unit",
            CheckBuilder::new("Unit Tests", "cargo test")
                .with_file_pattern_trigger("rust")
                .build(),
        )
        .build()
}

/// Config for checks.rs tests with warmup, fast, analysis, and test groups
///
/// Provides a PHP project config matching the original test_config_yaml() structure:
/// - warmup group: cache-warmup (always run, no triggers)
/// - fast group (parallel): php-lint, yaml-lint (with file pattern triggers)
/// - analysis group: phpstan (with fix_command)
/// - tests group: phpunit (with tests pattern trigger)
#[allow(dead_code)]
pub fn checks_test_config() -> CiConfig {
    ConfigBuilder::new()
        .with_docker("./infrastructure", "php")
        .with_git_branches("development", "HEAD~1")
        .with_file_pattern("php", r"\.php$", None)
        .with_file_pattern("php_src", r"^src/.*\.php$", None)
        .with_file_pattern("tests", r"tests/.*\.php$", None)
        .with_file_pattern("yaml", r"\.ya?ml$", None)
        // warmup group - always runs (no triggers)
        .with_check(
            "warmup",
            "cache-warmup",
            CheckBuilder::new("Cache warmup", "bin/console cache:warmup").build(),
        )
        .with_group_name("warmup", "Cache Warmup")
        // fast group - parallel execution
        .with_parallel_group("fast")
        .with_group_name("fast", "Fast Checks")
        .with_check(
            "fast",
            "php-lint",
            CheckBuilder::new("PHP syntax check", "parallel-lint {files}")
                .with_file_pattern_trigger("php")
                .build(),
        )
        .with_check(
            "fast",
            "yaml-lint",
            CheckBuilder::new("YAML syntax check", "yaml-lint {files}")
                .with_file_pattern_trigger("yaml")
                .build(),
        )
        // analysis group - has fix command
        .with_check(
            "analysis",
            "phpstan",
            CheckBuilder::new("PHPStan", "phpstan analyse {files}")
                .with_fix_command("phpstan fix {files}")
                .with_file_pattern_trigger("php")
                .build(),
        )
        // tests group
        .with_check(
            "tests",
            "phpunit",
            CheckBuilder::new("PHPUnit", "phpunit {files}")
                .with_file_pattern_trigger("tests")
                .build(),
        )
        .build()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_fixtures_are_valid() {
        let _ = minimal_config();
        let _ = php_project_config();
        let _ = rust_project_config();
        let _ = config_with_ignore_patterns();
        let _ = config_with_custom_ignore_patterns(vec![r"\.md$"]);
        let _ = config_with_always_run_check();
    }

    #[test]
    fn test_minimal_config_has_required_fields() {
        let config = minimal_config();
        assert_eq!(config.version, 2);
        assert_eq!(config.docker.project_dir, "./test");
        assert_eq!(config.docker.service, "app");
        assert_eq!(config.git.base_branch, "main");
        assert_eq!(config.git.fallback_branch, "HEAD~1");
        assert!(config.checks.is_empty());
    }

    #[test]
    fn test_rust_project_has_expected_checks() {
        let config = rust_project_config();
        assert_eq!(config.checks.len(), 2); // lint and test groups
        assert!(config.checks.contains_key("lint"));
        assert!(config.checks.contains_key("test"));

        let lint_group = &config.checks["lint"];
        assert!(lint_group.parallel);
        assert_eq!(lint_group.checks.len(), 2); // clippy and fmt
    }

    #[test]
    fn test_builder_allows_customization() {
        let config = ConfigBuilder::new()
            .with_docker("./custom", "web")
            .with_git_branches("develop", "HEAD~5")
            .with_file_pattern("custom", r"\.custom$", Some("green"))
            .build();

        assert_eq!(config.docker.project_dir, "./custom");
        assert_eq!(config.docker.service, "web");
        assert_eq!(config.git.base_branch, "develop");
        assert_eq!(config.git.fallback_branch, "HEAD~5");
        assert!(config.file_patterns.contains_key("custom"));
    }

    #[test]
    fn test_check_builder_creates_valid_definition() {
        let check = CheckBuilder::new("Test Check", "test command")
            .with_service("test-service")
            .with_fix_command("fix command")
            .with_file_pattern_trigger("test_pattern")
            .on_demand()
            .build();

        assert_eq!(check.name, "Test Check");
        assert_eq!(check.command, "test command");
        assert_eq!(check.service, Some("test-service".to_string()));
        assert_eq!(check.fix_command, Some("fix command".to_string()));
        assert!(check.on_demand);
        assert!(check.triggers.is_some());
    }
}
