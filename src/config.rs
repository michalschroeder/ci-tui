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

use anyhow::{bail, ensure, Context, Result};
use indexmap::IndexMap;
use regex::Regex;
use serde::Deserialize;
use std::collections::HashMap;
use std::path::Path;
use std::sync::OnceLock;
use std::time::Duration;

/// Root configuration type for CI-TUI.
///
/// Contains all settings needed to run CI checks including Docker configuration,
/// git settings, file patterns, and check definitions organized into groups.
/// Deserialized via [`RawCiConfig`] so the runner mode is resolved at parse time.
#[derive(Debug, Deserialize)]
#[serde(try_from = "RawCiConfig")]
pub struct CiConfig {
    pub version: u32,
    /// Where checks execute (`runner:` + its `docker` / `local` section)
    pub runner: ExecTarget,
    pub git: GitConfig,
    pub file_patterns: HashMap<String, FilePattern>,
    /// Check groups in execution order (YAML key order preserved)
    pub checks: IndexMap<String, GroupConfig>,
    pub ignore_patterns: Vec<String>,
    /// Max lines kept per stdout/stderr stream on a `CheckResult` (last N lines
    /// kept; older lines dropped with a "... X lines truncated" marker). Bounds
    /// memory for runaway commands. Defaults to [`DEFAULT_MAX_OUTPUT_LINES`].
    pub max_output_lines: usize,
    /// Compiled ignore patterns. Populated eagerly in `load_config`; lazy fallback for
    /// test-built/cloned configs panics on invalid patterns.
    pub(crate) compiled_ignore_patterns: OnceLock<Vec<Regex>>,
    /// Compiled file_patterns regexes keyed by pattern name. Populated eagerly in `load_config`.
    pub(crate) compiled_file_patterns: OnceLock<HashMap<String, Regex>>,
}

/// Default cap on stored lines per stdout/stderr stream when `max_output_lines`
/// is omitted from the config. Generous enough for normal CI logs while still
/// bounding a runaway command.
pub const DEFAULT_MAX_OUTPUT_LINES: usize = 10_000;

/// On-disk YAML shape of [`CiConfig`]: flat `runner` flag + optional sections.
/// Converted to [`CiConfig`] (with a typed [`ExecTarget`]) right after parsing.
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct RawCiConfig {
    version: u32,
    #[serde(default)]
    runner: RunnerMode,
    /// Required when runner is docker (the default)
    #[serde(default)]
    docker: Option<DockerConfig>,
    /// Settings for runner: local
    #[serde(default)]
    local: LocalConfig,
    git: GitConfig,
    file_patterns: HashMap<String, FilePattern>,
    checks: IndexMap<String, GroupConfig>,
    #[serde(default)]
    ignore_patterns: Vec<String>,
    /// Default timeout for checks and pre-commands that set none
    #[serde(default, deserialize_with = "deserialize_timeout")]
    timeout: Option<Duration>,
    /// Max lines kept per stdout/stderr stream (see [`DEFAULT_MAX_OUTPUT_LINES`])
    #[serde(default)]
    max_output_lines: Option<usize>,
}

impl TryFrom<RawCiConfig> for CiConfig {
    type Error = String;

    fn try_from(raw: RawCiConfig) -> std::result::Result<Self, Self::Error> {
        let runner = match (raw.runner, raw.docker) {
            (RunnerMode::Docker, Some(docker)) => ExecTarget::Docker(docker),
            (RunnerMode::Docker, None) => {
                return Err(
                    "`docker` section is required when runner is `docker` (the default). \
                     Set `runner: local` to run checks on the host instead."
                        .to_string(),
                )
            }
            // A `docker` section in local mode is parsed (must be valid) but ignored
            (RunnerMode::Local, _) => ExecTarget::Local(raw.local),
        };
        let mut checks = raw.checks;
        if let Some(default) = raw.timeout {
            apply_default_timeout(&mut checks, default);
        }
        Ok(Self {
            version: raw.version,
            runner,
            git: raw.git,
            file_patterns: raw.file_patterns,
            checks,
            ignore_patterns: raw.ignore_patterns,
            max_output_lines: raw.max_output_lines.unwrap_or(DEFAULT_MAX_OUTPUT_LINES),
            compiled_ignore_patterns: OnceLock::new(),
            compiled_file_patterns: OnceLock::new(),
        })
    }
}

/// Fill `timeout` on every check and pre-command that has none with the
/// top-level `timeout:`. Resolved once at parse time so runners only read the
/// item's own field (retry paths never see `CiConfig`).
fn apply_default_timeout(checks: &mut IndexMap<String, GroupConfig>, default: Duration) {
    for group in checks.values_mut() {
        let pre = group.pre_commands.iter_mut().map(|p| &mut p.timeout);
        let chk = group.checks.values_mut().map(|c| &mut c.timeout);
        for timeout in pre.chain(chk) {
            timeout.get_or_insert(default);
        }
    }
}

/// Deserialize an optional `timeout:` duration string (`30s`, `10m`, `1h`).
/// serde_yaml prefixes the error with the parent path (e.g. `checks.g.checks.c`), so the
/// message names the field itself: `checks.g.checks.c: `timeout`: invalid duration ...`.
fn deserialize_timeout<'de, D>(deserializer: D) -> std::result::Result<Option<Duration>, D::Error>
where
    D: serde::Deserializer<'de>,
{
    Option::<String>::deserialize(deserializer)?
        .map(|raw| {
            crate::utils::time::parse_duration(&raw)
                .map_err(|e| serde::de::Error::custom(format!("`timeout`: {e}")))
        })
        .transpose()
}

/// Where check commands execute, resolved from `runner:` at parse time.
///
/// Design: one enum instead of a `runner` flag + `Option<DockerConfig>` so
/// "docker mode without a docker section" is unrepresentable after parsing and
/// every consumer matches exhaustively (no panicking accessor). Enum rather than
/// a trait: the set of backends is closed (no others planned), and exhaustive
/// `match` keeps docker/local differences visible at each call site.
/// Command building lives in `runner.rs` (`impl ExecTarget`).
#[derive(Debug, Clone)]
pub enum ExecTarget {
    /// Run inside Docker: `docker exec` into a running container, else `docker run`
    Docker(DockerConfig),
    /// Run directly on the host via `local.shell -c` from the repo root
    Local(LocalConfig),
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
            runner: self.runner.clone(),
            git: self.git.clone(),
            file_patterns: self.file_patterns.clone(),
            checks: self.checks.clone(),
            ignore_patterns: self.ignore_patterns.clone(),
            max_output_lines: self.max_output_lines,
            // Reset caches on clone — repopulated via load_config or lazy fallback
            compiled_ignore_patterns: OnceLock::new(),
            compiled_file_patterns: OnceLock::new(),
        }
    }
}

/// Raw `runner:` value; resolved into [`ExecTarget`] during parsing.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize, Default)]
#[serde(rename_all = "lowercase")]
enum RunnerMode {
    #[default]
    Docker,
    Local,
}

/// Configuration for local (host) execution when `runner: local`.
#[derive(Debug, Clone, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct LocalConfig {
    /// Shell used to run check commands (default: "bash")
    #[serde(default = "default_local_shell")]
    pub shell: String,
    /// Environment variables for all local check commands
    #[serde(default)]
    pub env: HashMap<String, String>,
}

impl Default for LocalConfig {
    fn default() -> Self {
        Self {
            shell: default_local_shell(),
            env: HashMap::new(),
        }
    }
}

fn default_local_shell() -> String {
    "bash".to_string()
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
    /// Shell to use inside container (e.g., "bash" or "/bin/sh" for Alpine)
    pub shell: String,
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
        let project_name = self.compose_project_name();

        // Docker Compose naming convention: {project}-{service}-1
        format!("{}-{}-1", project_name, self.service)
    }

    /// Compose project name, in docker compose's own precedence order:
    /// 1. `COMPOSE_PROJECT_NAME` env var
    /// 2. top-level `name:` key in a compose file found in `project_dir`
    /// 3. `project_dir`'s last path component
    ///
    /// LIMITATION: does not honor `COMPOSE_FILE` / `-f` overrides or multiple/override
    /// compose files — only the first default-named file in `project_dir` is read.
    pub(crate) fn compose_project_name(&self) -> String {
        let name = std::env::var("COMPOSE_PROJECT_NAME")
            .ok()
            .filter(|name| !name.is_empty())
            .or_else(|| compose_file_name(&self.project_dir))
            .unwrap_or_else(|| self.derive_project_name());
        let normalized = normalize_project_name(&name);
        if normalized.is_empty() {
            // Nothing valid left (e.g. "___", non-ASCII) - same fallback as non-UTF8 paths
            return "project".to_string();
        }
        normalized
    }

    /// Extract project name from project_dir (last path component)
    /// Handle "." and relative paths by resolving to absolute path first
    fn derive_project_name(&self) -> String {
        if self.project_dir == "." || self.project_dir == ".." {
            return resolve_project_name_from_cwd(&self.project_dir)
                .unwrap_or_else(|| "project".to_string());
        }
        let path = std::path::Path::new(&self.project_dir);
        path.file_name()
            .and_then(|n| n.to_str())
            .unwrap_or("project")
            .to_string()
    }

    /// Get the Docker image name to use
    /// Returns explicit image if set, otherwise derives from container_name by stripping the trailing "-1"
    pub fn image_name(&self) -> String {
        if let Some(ref image) = self.image {
            return image.clone();
        }

        // Derive image from container name (strip exactly one trailing "-1")
        let name = self.container_name();
        name.strip_suffix("-1").unwrap_or(&name).to_string()
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

/// Resolve project name from current working directory.
/// Only `"."` (cwd) and `".."` (cwd's parent) are meaningful; any other input returns `None`.
fn resolve_project_name_from_cwd(project_dir: &str) -> Option<String> {
    let cwd = std::env::current_dir().ok()?;
    let resolved = match project_dir {
        "." => cwd,
        ".." => cwd.parent()?.to_path_buf(),
        _ => return None,
    };
    resolved.file_name()?.to_str().map(String::from)
}

/// Compose file names to look for in `project_dir`, in docker compose's own
/// default discovery order. Only the first one found is read.
const COMPOSE_FILE_NAMES: &[&str] = &[
    "compose.yaml",
    "compose.yml",
    "docker-compose.yaml",
    "docker-compose.yml",
];

/// Minimal shape for reading just the top-level `name:` key from a compose file.
/// `deny_unknown_fields` is deliberately omitted — compose files have many other
/// top-level keys (`services`, `volumes`, ...) we don't care about here.
#[derive(Deserialize)]
struct ComposeFileName {
    name: Option<String>,
}

/// Read the top-level `name:` key from the first compose file found in `project_dir`.
/// Returns `None` if no compose file is found, it fails to parse, or it has no
/// (non-empty) `name:` key — callers fall back to the basename-derived name.
fn compose_file_name(project_dir: &str) -> Option<String> {
    let dir = Path::new(project_dir);
    for filename in COMPOSE_FILE_NAMES {
        let Ok(contents) = std::fs::read_to_string(dir.join(filename)) else {
            continue;
        };
        // Compose only ever reads the first file it finds, so stop here regardless
        // of whether this file has a usable `name:` key.
        return serde_yaml::from_str::<ComposeFileName>(&contents)
            .ok()
            .and_then(|parsed| parsed.name)
            .filter(|name| !name.is_empty());
    }
    None
}

/// Normalize a compose project name like docker compose (compose-go
/// `NormalizeProjectName`): lowercase, keep only `[a-z0-9_-]`, trim leading `_`/`-`
fn normalize_project_name(name: &str) -> String {
    name.to_lowercase()
        .chars()
        .filter(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || *c == '_' || *c == '-')
        .collect::<String>()
        .trim_start_matches(['_', '-'])
        .to_string()
}

fn default_service() -> String {
    "app".to_string()
}

/// Expand environment variables in format ${VAR_NAME}
fn expand_env_vars(input: &str) -> String {
    static ENV_VAR_RE: OnceLock<Regex> = OnceLock::new();
    let re = ENV_VAR_RE.get_or_init(|| Regex::new(r"\$\{([^}]+)\}").expect("static regex"));

    let mut result = input.to_string();
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
    /// Max runtime (`30s`, `10m`, `1h`); on expiry the process is killed and the
    /// check reports `TimedOut`. `None` = unbounded. Top-level `timeout:` fills it.
    #[serde(default, deserialize_with = "deserialize_timeout")]
    pub timeout: Option<Duration>,
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
    /// If true, run command directly on host instead of inside Docker
    #[serde(default)]
    pub host: bool,
    /// Additional environment variables for this command (merged with global env)
    #[serde(default)]
    pub env: std::collections::HashMap<String, String>,
    /// Max runtime; expiry fails the pre-command (and so the group). `None` = unbounded.
    #[serde(default, deserialize_with = "deserialize_timeout")]
    pub timeout: Option<Duration>,
}

/// Config schema version this build understands.
pub const SUPPORTED_VERSION: u32 = 2;

/// Compile every entry in `file_patterns` into a Regex, erroring on the first invalid pattern.
fn compile_file_patterns(
    patterns: &HashMap<String, FilePattern>,
) -> Result<HashMap<String, Regex>> {
    patterns
        .iter()
        .map(|(key, fp)| {
            let re = Regex::new(&fp.pattern).with_context(|| {
                format!("invalid regex for file_patterns.{key}: '{}'", fp.pattern)
            })?;
            Ok((key.clone(), re))
        })
        .collect()
}

fn compile_ignore_patterns(patterns: &[String]) -> Result<Vec<Regex>> {
    patterns
        .iter()
        .map(|p| Regex::new(p).with_context(|| format!("invalid regex in ignore_patterns: '{p}'")))
        .collect()
}

/// First pattern key referenced by `triggers` that is missing from `patterns`,
/// as `(field, key)`.
fn dangling_pattern_ref<'a>(
    triggers: &'a CheckTriggers,
    patterns: &HashMap<String, FilePattern>,
) -> Option<(&'static str, &'a str)> {
    let discovery = triggers
        .test_discovery
        .as_ref()
        .map(|d| d.source_pattern.as_str());
    [
        ("file_pattern", triggers.file_pattern.as_deref()),
        ("test_discovery.source_pattern", discovery),
    ]
    .into_iter()
    .find_map(|(field, key)| {
        key.filter(|k| !patterns.contains_key(*k))
            .map(|k| (field, k))
    })
}

/// Check one group's pre-commands and checks for `service`/`container` set, which is
/// disallowed under `runner: local`. Extracted to keep `check_no_docker_targets` nesting low.
fn ensure_group_has_no_docker_targets(g: &str, group: &GroupConfig) -> Result<()> {
    for (i, pre) in group.pre_commands.iter().enumerate() {
        if pre.service.is_some() || pre.container.is_some() {
            bail!(
                "`{g}.pre_commands[{i}]` sets `service`/`container`, which is not allowed with `runner: local`"
            );
        }
    }
    for (c, check) in group.checks.iter() {
        if check.service.is_some() || check.container.is_some() {
            bail!(
                "`{g}.{c}` sets `service`/`container`, which is not allowed with `runner: local`"
            );
        }
    }
    Ok(())
}

/// Load configuration from a YAML file.
///
/// # Errors
///
/// Returns an error if the file cannot be read, parsed, or contains invalid regexes.
pub fn load_config(path: &Path) -> Result<CiConfig> {
    let content = std::fs::read_to_string(path)
        .with_context(|| format!("Failed to read config file: {}", path.display()))?;

    let config: CiConfig = serde_yaml::from_str(&content)
        .with_context(|| format!("Failed to parse config file: {}", path.display()))?;
    config
        .validate_and_compile()
        .with_context(|| format!("Invalid config file: {}", path.display()))?;
    Ok(config)
}

impl CiConfig {
    /// Create a new CiConfig (primarily for tests)
    ///
    /// This constructor allows creating CiConfig instances programmatically
    /// without going through YAML deserialization.
    #[allow(clippy::too_many_arguments)]
    pub fn new(
        version: u32,
        docker: DockerConfig,
        git: GitConfig,
        file_patterns: HashMap<String, FilePattern>,
        checks: IndexMap<String, GroupConfig>,
        ignore_patterns: Vec<String>,
    ) -> Self {
        Self {
            version,
            runner: ExecTarget::Docker(docker),
            git,
            file_patterns,
            checks,
            ignore_patterns,
            max_output_lines: DEFAULT_MAX_OUTPUT_LINES,
            compiled_ignore_patterns: OnceLock::new(),
            compiled_file_patterns: OnceLock::new(),
        }
    }

    /// Docker config; `None` in local mode.
    pub fn docker(&self) -> Option<&DockerConfig> {
        match &self.runner {
            ExecTarget::Docker(docker) => Some(docker),
            ExecTarget::Local(_) => None,
        }
    }

    /// Check the schema version, then compile and cache every regex in
    /// `file_patterns` and `ignore_patterns`.
    ///
    /// Called from [`load_config`] so that invalid regexes surface at startup instead
    /// of being silently dropped on first use.
    ///
    /// First call wins: the `OnceLock::set` results are deliberately discarded, so a
    /// second call never replaces already-cached patterns (even if `file_patterns`
    /// was mutated in between). Call at most once per config instance.
    pub fn validate_and_compile(&self) -> Result<()> {
        if self.version != SUPPORTED_VERSION {
            bail!(
                "unsupported config version {} (expected {SUPPORTED_VERSION})",
                self.version
            );
        }
        match &self.runner {
            // Docker section presence is enforced at parse time (RawCiConfig)
            ExecTarget::Docker(_) => {}
            ExecTarget::Local(local) => {
                ensure!(
                    !local.shell.trim().is_empty(),
                    "`local.shell` must not be empty when runner is `local`"
                );
                self.check_no_docker_targets()?;
            }
        }
        self.check_pattern_references()?;
        let _ = self
            .compiled_file_patterns
            .set(compile_file_patterns(&self.file_patterns)?);
        let _ = self
            .compiled_ignore_patterns
            .set(compile_ignore_patterns(&self.ignore_patterns)?);
        Ok(())
    }

    /// Every `triggers.file_pattern` / `test_discovery.source_pattern` must name a
    /// key in `file_patterns`; otherwise the check would be silently skipped.
    fn check_pattern_references(&self) -> Result<()> {
        let dangling = self
            .checks
            .iter()
            .flat_map(|(g, group)| group.checks.iter().map(move |(c, check)| (g, c, check)))
            .find_map(|(g, c, check)| {
                let (field, key) =
                    dangling_pattern_ref(check.triggers.as_ref()?, &self.file_patterns)?;
                Some(format!(
                    "checks.{g}.checks.{c}.triggers.{field}: unknown file pattern '{key}'"
                ))
            });
        match dangling {
            Some(msg) => bail!(msg),
            None => Ok(()),
        }
    }

    /// In `runner: local` mode, no group may set `service`/`container` on a check or
    /// pre-command — those only make sense for Docker execution.
    fn check_no_docker_targets(&self) -> Result<()> {
        for (g, group) in self.checks.iter() {
            ensure_group_has_no_docker_targets(g, group)?;
        }
        Ok(())
    }

    /// Get the raw regex string for a file pattern key
    pub fn get_file_pattern(&self, key: &str) -> Option<&str> {
        self.file_patterns.get(key).map(|fp| fp.pattern.as_str())
    }

    /// Get the compiled regex for a file pattern key.
    ///
    /// Uses the eagerly populated cache from `load_config`. For configs built via
    /// `CiConfig::new` or cloned (caches reset on clone), lazily compiles on first
    /// access and PANICS on an invalid pattern — production configs are validated
    /// at load time, so a panic here indicates a broken test fixture.
    pub fn get_compiled_file_pattern(&self, key: &str) -> Option<&Regex> {
        let map = self.compiled_file_patterns.get_or_init(|| {
            compile_file_patterns(&self.file_patterns)
                .expect("invalid regex in file_patterns (validate_and_compile not called)")
        });
        map.get(key)
    }

    /// Get the compiled ignore regexes.
    ///
    /// Eagerly populated in `load_config`; lazy fallback for test/cloned configs
    /// PANICS on invalid patterns instead of silently dropping them.
    pub fn compiled_ignore_patterns(&self) -> &[Regex] {
        self.compiled_ignore_patterns.get_or_init(|| {
            compile_ignore_patterns(&self.ignore_patterns)
                .expect("invalid regex in ignore_patterns (validate_and_compile not called)")
        })
    }

    /// Check if a file should be ignored based on ignore_patterns
    pub fn should_ignore_file(&self, path: &str) -> bool {
        self.compiled_ignore_patterns()
            .iter()
            .any(|re| re.is_match(path))
    }

    /// Get color name for a file based on file_patterns
    pub fn get_file_color(&self, path: &str) -> &str {
        self.file_patterns
            .iter()
            .filter_map(|(key, fp)| {
                let color = fp.color.as_ref()?;
                let re = self.get_compiled_file_pattern(key)?;
                re.is_match(path).then_some(color.as_str())
            })
            .next()
            .unwrap_or("white")
    }

    /// Default Docker service (from docker config); `None` in local mode.
    pub fn default_service(&self) -> Option<&str> {
        self.docker().map(|d| d.service.as_str())
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
    /// Get the service for this check, with fallback to default (`None` in local mode)
    pub fn service_or_default<'a>(&'a self, default: Option<&'a str>) -> Option<&'a str> {
        self.service.as_deref().or(default)
    }

    /// Check if this check should always run (no triggers defined)
    pub fn always_run(&self) -> bool {
        self.triggers.is_none()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use rstest::rstest;

    // Import builder types from the main config module
    use super::{
        CheckDefinition, CheckTriggers, CiConfig, DockerConfig, FilePattern, GitConfig,
        GroupConfig, DEFAULT_MAX_OUTPUT_LINES,
    };
    use indexmap::IndexMap;
    use std::collections::HashMap;
    use std::sync::OnceLock;

    // Inline ConfigBuilder for tests (simplified version of tests/common/configs.rs)
    struct ConfigBuilder {
        docker_project_dir: String,
        docker_service: String,
        git_base: String,
        git_fallback: String,
        file_patterns: HashMap<String, FilePattern>,
        checks: IndexMap<String, GroupConfig>,
        ignore_patterns: Vec<String>,
        max_output_lines: usize,
    }

    impl ConfigBuilder {
        fn new() -> Self {
            Self {
                docker_project_dir: "./test".to_string(),
                docker_service: "app".to_string(),
                max_output_lines: DEFAULT_MAX_OUTPUT_LINES,
                git_base: "main".to_string(),
                git_fallback: "HEAD~1".to_string(),
                file_patterns: HashMap::new(),
                checks: IndexMap::new(),
                ignore_patterns: Vec::new(),
            }
        }

        fn with_docker(mut self, project_dir: &str, service: &str) -> Self {
            self.docker_project_dir = project_dir.to_string();
            self.docker_service = service.to_string();
            self
        }

        fn with_git_branches(mut self, base: &str, fallback: &str) -> Self {
            self.git_base = base.to_string();
            self.git_fallback = fallback.to_string();
            self
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

        fn with_parallel_group(mut self, group_id: &str) -> Self {
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

        fn with_group_name(mut self, group_id: &str, name: &str) -> Self {
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

        fn with_ignore_patterns(mut self, patterns: Vec<&str>) -> Self {
            self.ignore_patterns = patterns.into_iter().map(String::from).collect();
            self
        }

        fn with_max_output_lines(mut self, max_output_lines: usize) -> Self {
            self.max_output_lines = max_output_lines;
            self
        }

        fn build(self) -> CiConfig {
            CiConfig {
                version: 2,
                runner: ExecTarget::Docker(DockerConfig {
                    project_dir: self.docker_project_dir,
                    service: self.docker_service,
                    container: None,
                    image: None,
                    volume_mount: None,
                    work_dir: None,
                    shell: "bash".to_string(),
                    env: HashMap::new(),
                }),
                git: GitConfig {
                    base_branch: self.git_base,
                    fallback_branch: self.git_fallback,
                },
                file_patterns: self.file_patterns,
                checks: self.checks,
                ignore_patterns: self.ignore_patterns,
                max_output_lines: self.max_output_lines,
                compiled_ignore_patterns: OnceLock::new(),
                compiled_file_patterns: OnceLock::new(),
            }
        }
    }

    struct CheckBuilder {
        name: String,
        command: String,
        service: Option<String>,
        fix_command: Option<String>,
        triggers: Option<CheckTriggers>,
    }

    impl CheckBuilder {
        fn new(name: &str, command: &str) -> Self {
            Self {
                name: name.to_string(),
                command: command.to_string(),
                service: None,
                fix_command: None,
                triggers: None,
            }
        }

        fn with_service(mut self, service: &str) -> Self {
            self.service = Some(service.to_string());
            self
        }

        fn with_file_pattern_trigger(mut self, pattern_name: &str) -> Self {
            let triggers = self.triggers.get_or_insert_with(CheckTriggers::default);
            triggers.file_pattern = Some(pattern_name.to_string());
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
                on_demand: false,
                env: HashMap::new(),
                timeout: None,
            }
        }
    }

    // Helper fixtures
    fn php_project_config() -> CiConfig {
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

    fn config_with_ignore_patterns() -> CiConfig {
        ConfigBuilder::new()
            .with_ignore_patterns(vec![r"\.md$", r"\.github/"])
            .build()
    }

    #[test]
    fn test_config_builder_with_max_output_lines() {
        let config = ConfigBuilder::new().with_max_output_lines(50).build();
        assert_eq!(config.max_output_lines, 50);
    }

    #[test]
    fn test_get_file_pattern() {
        let config = php_project_config();

        assert_eq!(config.get_file_pattern("php"), Some(r"\.php$"));
        assert_eq!(config.get_file_pattern("php_src"), Some("^src/"));
        assert_eq!(config.get_file_pattern("nonexistent"), None);
    }

    #[test]
    fn test_should_ignore_file() {
        let config = config_with_ignore_patterns();

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
        let config = php_project_config();

        // Files matching patterns with colors
        assert_eq!(config.get_file_color("src/Service/Foo.php"), "blue");
        assert_eq!(config.get_file_color("tests/Unit/FooTest.php"), "green");

        // Files not matching any color pattern
        assert_eq!(config.get_file_color("composer.json"), "white");
    }

    #[test]
    fn test_default_service() {
        let config = ConfigBuilder::new()
            .with_docker("./infrastructure", "php")
            .build();
        assert_eq!(config.default_service(), Some("php"));
    }

    #[test]
    fn test_get_group() {
        let config = php_project_config();

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
        let config = php_project_config();

        let group_names: Vec<&str> = config.groups().map(|(name, _)| name).collect();
        assert_eq!(group_names, vec!["fast", "tests"]);
    }

    #[test]
    fn test_group_display_name() {
        let config = php_project_config();

        let fast = config.get_group("fast").unwrap();
        assert_eq!(fast.display_name("fast"), "Fast Checks");

        let tests = config.get_group("tests").unwrap();
        assert_eq!(tests.display_name("tests"), "tests"); // Falls back to key
    }

    #[test]
    fn test_check_service_or_default() {
        let config = php_project_config();

        let fast = config.get_group("fast").unwrap();
        let php_lint = fast.checks.get("php-lint").unwrap();
        assert_eq!(
            php_lint.service_or_default(Some("default")),
            Some("default")
        );

        let tests = config.get_group("tests").unwrap();
        let phpunit = tests.checks.get("phpunit").unwrap();
        assert_eq!(
            phpunit.service_or_default(Some("default")),
            Some("custom-service")
        );
    }

    #[test]
    fn test_check_always_run() {
        let config = php_project_config();

        let fast = config.get_group("fast").unwrap();
        let php_lint = fast.checks.get("php-lint").unwrap();
        assert!(!php_lint.always_run()); // Has triggers

        // Create a check without triggers to test always_run = true
        let config = ConfigBuilder::new()
            .with_check(
                "warmup",
                "cache-warmup",
                CheckBuilder::new("Cache warmup", "bin/console cache:warmup").build(),
            )
            .build();
        let warmup = config.get_group("warmup").unwrap();
        let cache = warmup.checks.get("cache-warmup").unwrap();
        assert!(cache.always_run()); // No triggers
    }

    #[test]
    fn test_parallel_and_stop_on_failure() {
        let config = php_project_config();

        let fast = config.get_group("fast").unwrap();
        assert!(fast.parallel);
        assert!(!fast.stop_on_failure);

        let tests = config.get_group("tests").unwrap();
        assert!(!tests.parallel);
    }

    // Edge case: Tests YAML parsing rejection of unknown fields at top level - requires raw YAML to verify serde deny_unknown_fields behavior
    #[test]
    fn test_unknown_field_rejected_top_level() {
        let yaml = r#"
version: 2
docker:
  project_dir: .
  shell: bash
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

    // Edge case: Tests YAML parsing rejection of unknown nested fields - requires raw YAML to verify nested struct deny_unknown_fields
    #[test]
    fn test_unknown_field_rejected_nested() {
        let yaml = r#"
version: 2
docker:
  project_dir: .
  service: app
  shell: bash
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

    // Edge case: Tests error message quality for field name typos - requires raw YAML with intentional typo to verify error reporting
    #[test]
    fn test_typo_produces_helpful_error() {
        let yaml = r#"
version: 2
docker:
  project_dir: .
  shell: bash
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
            let config = php_project_config();
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
            let config = php_project_config();
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
            let config = php_project_config();
            assert_eq!(config.get_file_color(path), expected_color);
        }
    }

    mod test_docker_config {
        use super::*;

        // Edge case: Tests explicit container name configuration - requires raw YAML to verify container field parsing
        #[test]
        fn test_container_name_explicit() {
            let yaml = r#"
version: 2
docker:
  project_dir: ./infrastructure
  service: php
  shell: bash
  container: explicit-container-name
git:
  base_branch: main
  fallback_branch: HEAD~1
file_patterns: {}
checks: {}
"#;
            let config: CiConfig = serde_yaml::from_str(yaml).unwrap();
            assert_eq!(
                config.docker().unwrap().container_name(),
                "explicit-container-name"
            );
        }

        // Edge case: Tests container name derivation from project_dir and service - requires raw YAML to verify default derivation logic
        #[test]
        fn test_container_name_derived() {
            let yaml = r#"
version: 2
docker:
  project_dir: ./infrastructure
  service: php
  shell: bash
git:
  base_branch: main
  fallback_branch: HEAD~1
file_patterns: {}
checks: {}
"#;
            let config: CiConfig = serde_yaml::from_str(yaml).unwrap();
            assert_eq!(
                config.docker().unwrap().container_name(),
                "infrastructure-php-1"
            );
        }

        // Edge case: Tests container name derivation with "." project_dir - requires raw YAML to verify special path handling
        #[test]
        fn test_container_name_current_dir() {
            let yaml = r#"
version: 2
docker:
  project_dir: .
  service: app
  shell: bash
git:
  base_branch: main
  fallback_branch: HEAD~1
file_patterns: {}
checks: {}
"#;
            let config: CiConfig = serde_yaml::from_str(yaml).unwrap();
            let container_name = config.docker().unwrap().container_name();
            // Should derive from current directory name
            assert!(container_name.ends_with("-app-1"));
            assert_ne!(container_name, ".-app-1"); // Should not use literal "."
        }

        // Edge case: Tests explicit Docker image name configuration - requires raw YAML to verify image field parsing
        #[test]
        fn test_image_name_explicit() {
            let yaml = r#"
version: 2
docker:
  project_dir: .
  service: app
  shell: bash
  image: rust:latest
git:
  base_branch: main
  fallback_branch: HEAD~1
file_patterns: {}
checks: {}
"#;
            let config: CiConfig = serde_yaml::from_str(yaml).unwrap();
            assert_eq!(config.docker().unwrap().image_name(), "rust:latest");
        }

        // Edge case: Tests Docker image name derivation from container name - requires raw YAML to verify derivation logic
        #[test]
        fn test_image_name_derived() {
            let yaml = r#"
version: 2
docker:
  project_dir: ./myproject
  service: web
  shell: bash
git:
  base_branch: main
  fallback_branch: HEAD~1
file_patterns: {}
checks: {}
"#;
            let config: CiConfig = serde_yaml::from_str(yaml).unwrap();
            // Image derived from container name by stripping -1 suffix
            assert_eq!(config.docker().unwrap().image_name(), "myproject-web");
        }

        // Edge case: project_dir basename is normalized like docker compose
        // (lowercase, drop chars outside [a-z0-9_-], trim leading _/-)
        #[rstest]
        #[case("./MyApp", "myapp-web-1", "myapp-web")]
        #[case("./my.app", "myapp-web-1", "myapp-web")]
        #[case("./_My App!", "myapp-web-1", "myapp-web")]
        #[case("./my_proj-2", "my_proj-2-web-1", "my_proj-2-web")]
        #[case("./___", "project-web-1", "project-web")]
        #[case("./日本", "project-web-1", "project-web")]
        fn test_derived_names_normalized_like_compose(
            #[case] project_dir: &str,
            #[case] container: &str,
            #[case] image: &str,
        ) {
            let config = DockerConfig {
                project_dir: project_dir.to_string(),
                service: "web".to_string(),
                container: None,
                image: None,
                volume_mount: None,
                work_dir: None,
                shell: "bash".to_string(),
                env: Default::default(),
            };
            assert_eq!(config.container_name(), container);
            assert_eq!(config.image_name(), image);
        }

        // Edge case: COMPOSE_PROJECT_NAME overrides project_dir-derived compose project name.
        // Uses a unique var value; test mutates process env, so restore afterwards.
        #[test]
        fn test_container_name_honors_compose_project_name_env() {
            let yaml = r#"
version: 2
docker:
  project_dir: ./myproject
  service: web
  shell: bash
git:
  base_branch: main
  fallback_branch: HEAD~1
file_patterns: {}
checks: {}
"#;
            let config: CiConfig = serde_yaml::from_str(yaml).unwrap();
            std::env::set_var("COMPOSE_PROJECT_NAME", "customproj");
            let name = config.docker().unwrap().container_name();
            std::env::remove_var("COMPOSE_PROJECT_NAME");
            assert_eq!(name, "customproj-web-1");
        }

        // Shared by the compose-file-`name:`-key tests below: a `DockerConfig` pointed
        // at `dir`, varying only in project_dir.
        fn docker_config_for(dir: &std::path::Path) -> DockerConfig {
            DockerConfig {
                project_dir: dir.to_str().unwrap().to_string(),
                service: "web".to_string(),
                container: None,
                image: None,
                volume_mount: None,
                work_dir: None,
                shell: "bash".to_string(),
                env: Default::default(),
            }
        }

        // Compose file's top-level `name:` key is honored when COMPOSE_PROJECT_NAME is unset.
        #[test]
        fn test_compose_project_name_honors_compose_file_name_key() {
            let dir = tempfile::tempdir().unwrap();
            std::fs::write(
                dir.path().join("docker-compose.yml"),
                "name: FromCompose\nservices:\n  web: {}\n",
            )
            .unwrap();
            let config = docker_config_for(dir.path());
            // normalize_project_name lowercases, so "FromCompose" -> "fromcompose"
            assert_eq!(config.compose_project_name(), "fromcompose");
            assert_eq!(config.container_name(), "fromcompose-web-1");
        }

        // When both a canonical `compose.yaml` and a legacy `docker-compose.yml` are
        // present, the canonical file wins (matches Docker Compose's own discovery order).
        #[test]
        fn test_compose_project_name_prefers_canonical_compose_yaml_over_legacy() {
            let dir = tempfile::tempdir().unwrap();
            std::fs::write(dir.path().join("compose.yaml"), "name: Canonical\n").unwrap();
            std::fs::write(dir.path().join("docker-compose.yml"), "name: Legacy\n").unwrap();
            let config = docker_config_for(dir.path());
            assert_eq!(config.compose_project_name(), "canonical");
        }

        // COMPOSE_PROJECT_NAME still takes precedence over the compose file's `name:` key.
        #[test]
        fn test_compose_project_name_env_var_wins_over_name_key() {
            let dir = tempfile::tempdir().unwrap();
            std::fs::write(dir.path().join("docker-compose.yml"), "name: FromCompose\n").unwrap();
            let config = docker_config_for(dir.path());
            std::env::set_var("COMPOSE_PROJECT_NAME", "fromenv");
            let name = config.compose_project_name();
            std::env::remove_var("COMPOSE_PROJECT_NAME");
            assert_eq!(name, "fromenv");
        }

        // No compose file in project_dir: fall back to the basename-derived name.
        #[test]
        fn test_compose_project_name_missing_compose_file_falls_back_to_basename() {
            let dir = tempfile::tempdir().unwrap();
            let config = docker_config_for(dir.path());
            let expected =
                normalize_project_name(dir.path().file_name().unwrap().to_str().unwrap());
            assert_eq!(config.compose_project_name(), expected);
        }

        // Compose file present but with no top-level `name:` key (and a malformed one):
        // both fall back to the basename-derived name.
        #[rstest]
        #[case::no_name_key("services:\n  web: {}\n")]
        #[case::malformed_yaml("not: [valid: yaml")]
        fn test_compose_project_name_bad_compose_file_falls_back_to_basename(
            #[case] compose_contents: &str,
        ) {
            let dir = tempfile::tempdir().unwrap();
            std::fs::write(dir.path().join("docker-compose.yml"), compose_contents).unwrap();
            let config = docker_config_for(dir.path());
            let expected =
                normalize_project_name(dir.path().file_name().unwrap().to_str().unwrap());
            assert_eq!(config.compose_project_name(), expected);
        }

        // Edge case: container name itself ending in -1 must lose only ONE -1 suffix
        #[test]
        fn test_image_name_derived_strips_single_dash_one_suffix() {
            let yaml = r#"
version: 2
docker:
  project_dir: ./proj
  service: x-1
  shell: bash
git:
  base_branch: main
  fallback_branch: HEAD~1
file_patterns: {}
checks: {}
"#;
            let config: CiConfig = serde_yaml::from_str(yaml).unwrap();
            // container_name = "proj-x-1-1"; image must be "proj-x-1", not "proj-x"
            assert_eq!(config.docker().unwrap().image_name(), "proj-x-1");
        }

        // Edge case: Tests Docker volume mount configuration - requires raw YAML to verify volume_mount field parsing and formatting
        #[test]
        fn test_volume_args() {
            let yaml = r#"
version: 2
docker:
  project_dir: .
  service: app
  shell: bash
  volume_mount: ".:/build"
git:
  base_branch: main
  fallback_branch: HEAD~1
file_patterns: {}
checks: {}
"#;
            let config: CiConfig = serde_yaml::from_str(yaml).unwrap();
            assert_eq!(
                config.docker().unwrap().volume_args(),
                Some("-v .:/build".to_string())
            );
        }

        // Edge case: Tests default working directory when work_dir not specified - requires raw YAML to verify /app default
        #[test]
        fn test_working_dir_default() {
            let yaml = r#"
version: 2
docker:
  project_dir: .
  service: app
  shell: bash
git:
  base_branch: main
  fallback_branch: HEAD~1
file_patterns: {}
checks: {}
"#;
            let config: CiConfig = serde_yaml::from_str(yaml).unwrap();
            assert_eq!(config.docker().unwrap().working_dir(), "/app");
        }

        // Edge case: Tests custom working directory configuration - requires raw YAML to verify work_dir field parsing
        #[test]
        fn test_working_dir_custom() {
            let yaml = r#"
version: 2
docker:
  project_dir: .
  service: app
  shell: bash
  work_dir: "/custom/path"
git:
  base_branch: main
  fallback_branch: HEAD~1
file_patterns: {}
checks: {}
"#;
            let config: CiConfig = serde_yaml::from_str(yaml).unwrap();
            assert_eq!(config.docker().unwrap().working_dir(), "/custom/path");
        }

        // Edge case: Tests that shell field is required - requires raw YAML missing shell to verify error handling
        #[test]
        fn test_shell_required() {
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
            let result: Result<CiConfig, _> = serde_yaml::from_str(yaml);
            assert!(result.is_err());
            let err = result.unwrap_err().to_string();
            assert!(
                err.contains("shell") || err.contains("missing field"),
                "Error should mention missing shell field: {}",
                err
            );
        }

        // Edge case: Tests custom shell configuration (e.g., /bin/sh for Alpine) - requires raw YAML to verify shell field parsing
        #[test]
        fn test_shell_custom() {
            let yaml = r#"
version: 2
docker:
  project_dir: .
  service: app
  shell: /bin/sh
git:
  base_branch: main
  fallback_branch: HEAD~1
file_patterns: {}
checks: {}
"#;
            let config: CiConfig = serde_yaml::from_str(yaml).unwrap();
            assert_eq!(config.docker().unwrap().shell, "/bin/sh");
        }

        // volume_args() returns None when no volume_mount is configured
        #[test]
        fn test_volume_args_returns_none_without_mount() {
            let yaml = r#"
version: 2
docker:
  project_dir: .
  service: app
  shell: bash
git:
  base_branch: main
  fallback_branch: HEAD~1
file_patterns: {}
checks: {}
"#;
            let config: CiConfig = serde_yaml::from_str(yaml).unwrap();
            assert_eq!(config.docker().unwrap().volume_args(), None);
        }

        // volume_args() expands env vars when present
        #[test]
        fn test_volume_args_expands_env_vars() {
            std::env::set_var("CI_TUI_TEST_VOL", "/tmp/ci-tui-vol");
            let yaml = r#"
version: 2
docker:
  project_dir: .
  service: app
  shell: bash
  volume_mount: "${CI_TUI_TEST_VOL}:/build"
git:
  base_branch: main
  fallback_branch: HEAD~1
file_patterns: {}
checks: {}
"#;
            let config: CiConfig = serde_yaml::from_str(yaml).unwrap();
            assert_eq!(
                config.docker().unwrap().volume_args(),
                Some("-v /tmp/ci-tui-vol:/build".to_string())
            );
            std::env::remove_var("CI_TUI_TEST_VOL");
        }
    }

    mod test_yaml_parsing_errors {
        use super::*;

        // Edge case: Parameterized tests for unknown field rejection - requires raw YAML fragments to test multiple error scenarios
        #[rstest]
        #[case("typo_field: oops", "typo_field")] // Unknown top-level field
        #[case("docker:\n  unknown_field: x", "unknown")] // Unknown nested field
        #[case("docker:\n  project_dir: .\n  shell: bash", "git")] // Missing required git section
        fn test_unknown_fields_rejected(#[case] extra_yaml: &str, #[case] error_contains: &str) {
            let yaml = format!(
                r#"
version: 2
{}
file_patterns: {{}}
checks: {{}}
"#,
                extra_yaml
            );
            let result: Result<CiConfig, _> = serde_yaml::from_str(&yaml);
            assert!(result.is_err());
            let err = result.unwrap_err().to_string();
            assert!(
                err.contains(error_contains),
                "Error should contain '{}', got: {}",
                error_contains,
                err
            );
        }
    }

    mod test_invalid_regex {
        use super::*;

        // Invalid regex in file_patterns must fail validate_and_compile (surfaces at load_config)
        #[test]
        fn test_invalid_regex_in_file_pattern_rejected() {
            let yaml = r#"
version: 2
docker:
  project_dir: .
  shell: bash
git:
  base_branch: main
  fallback_branch: HEAD~1
file_patterns:
  bad:
    pattern: '[unclosed'
checks: {}
"#;
            let config: CiConfig = serde_yaml::from_str(yaml).unwrap();
            let err = config.validate_and_compile().unwrap_err().to_string();
            assert!(
                err.contains("file_patterns.bad") && err.contains("[unclosed"),
                "error should name the offending key and pattern: {err}"
            );
        }

        // Invalid regex in ignore_patterns must fail validate_and_compile
        #[test]
        fn test_invalid_regex_in_ignore_patterns_rejected() {
            let yaml = r#"
version: 2
docker:
  project_dir: .
  shell: bash
git:
  base_branch: main
  fallback_branch: HEAD~1
file_patterns: {}
ignore_patterns:
  - '\.md$'
  - '[unclosed'
checks: {}
"#;
            let config: CiConfig = serde_yaml::from_str(yaml).unwrap();
            let err = config.validate_and_compile().unwrap_err().to_string();
            assert!(
                err.contains("ignore_patterns") && err.contains("[unclosed"),
                "error should name ignore_patterns and the bad pattern: {err}"
            );
        }

        // After validate_and_compile, compiled patterns are accessible for all valid keys
        #[test]
        fn test_compiled_patterns_accessible_after_validation() {
            let yaml = r#"
version: 2
docker:
  project_dir: .
  shell: bash
git:
  base_branch: main
  fallback_branch: HEAD~1
file_patterns:
  rust:
    pattern: '\.rs$'
  toml:
    pattern: '\.toml$'
ignore_patterns:
  - '\.md$'
checks: {}
"#;
            let config: CiConfig = serde_yaml::from_str(yaml).unwrap();
            config.validate_and_compile().unwrap();

            let rust_re = config.get_compiled_file_pattern("rust").unwrap();
            assert!(rust_re.is_match("src/main.rs"));
            assert!(!rust_re.is_match("Cargo.toml"));

            let toml_re = config.get_compiled_file_pattern("toml").unwrap();
            assert!(toml_re.is_match("Cargo.toml"));

            assert!(config.get_compiled_file_pattern("missing").is_none());
            assert!(config.should_ignore_file("README.md"));
        }

        // load_config end-to-end: invalid file_patterns regex fails with a clear message
        #[test]
        fn test_load_config_rejects_invalid_file_pattern() {
            use std::io::Write;
            let yaml = r#"
version: 2
docker:
  project_dir: .
  shell: bash
git:
  base_branch: main
  fallback_branch: HEAD~1
file_patterns:
  foo:
    pattern: '[invalid'
checks: {}
"#;
            let mut tmp = tempfile::NamedTempFile::new().unwrap();
            tmp.write_all(yaml.as_bytes()).unwrap();
            let err = load_config(tmp.path()).unwrap_err();
            let msg = format!("{err:#}");
            assert!(
                msg.contains("file_patterns.foo") && msg.contains("[invalid"),
                "error chain should mention offending key + pattern: {msg}"
            );
        }

        // Unsupported schema version is rejected at load
        #[rstest]
        #[case(1)]
        #[case(99)]
        fn test_validate_rejects_unsupported_version(#[case] version: u32) {
            let yaml = format!(
                "version: {version}\ndocker:\n  project_dir: .\n  shell: bash\ngit:\n  base_branch: main\n  fallback_branch: HEAD~1\nfile_patterns: {{}}\nchecks: {{}}\n"
            );
            let config: CiConfig = serde_yaml::from_str(&yaml).unwrap();
            let msg = config.validate_and_compile().unwrap_err().to_string();
            assert!(msg.contains("unsupported config version"), "got: {msg}");
        }

        // Trigger referencing an undefined file_patterns key is rejected at load
        #[rstest]
        #[case::file_pattern("file_pattern: sorce", "triggers.file_pattern")]
        #[case::source_pattern(
            "test_discovery: {source_pattern: sorce, strategies: []}",
            "triggers.test_discovery.source_pattern"
        )]
        fn test_validate_rejects_unknown_pattern_ref(#[case] trigger: &str, #[case] field: &str) {
            let yaml = format!(
                "version: 2\ndocker:\n  project_dir: .\n  shell: bash\ngit:\n  base_branch: main\n  fallback_branch: HEAD~1\nfile_patterns:\n  source:\n    pattern: 'x'\nchecks:\n  g:\n    checks:\n      lint:\n        name: Lint\n        command: 'true'\n        triggers:\n          {trigger}\n"
            );
            let config: CiConfig = serde_yaml::from_str(&yaml).unwrap();
            let msg = config.validate_and_compile().unwrap_err().to_string();
            assert!(
                msg.contains(&format!("checks.g.checks.lint.{field}")) && msg.contains("'sorce'"),
                "got: {msg}"
            );
        }

        // load_config end-to-end: invalid ignore_patterns regex fails
        #[test]
        fn test_load_config_rejects_invalid_ignore_pattern() {
            use std::io::Write;
            let yaml = r#"
version: 2
docker:
  project_dir: .
  shell: bash
git:
  base_branch: main
  fallback_branch: HEAD~1
file_patterns: {}
ignore_patterns:
  - '[invalid'
checks: {}
"#;
            let mut tmp = tempfile::NamedTempFile::new().unwrap();
            tmp.write_all(yaml.as_bytes()).unwrap();
            let err = load_config(tmp.path()).unwrap_err();
            let msg = format!("{err:#}");
            assert!(
                msg.contains("ignore_patterns") && msg.contains("[invalid"),
                "error chain should mention ignore_patterns: {msg}"
            );
        }
    }

    mod test_boundary_cases {
        use super::*;

        // Edge case: Tests minimal config with empty file_patterns - requires raw YAML to verify empty map handling
        #[test]
        fn test_empty_file_patterns() {
            let yaml = r#"
version: 2
docker:
  project_dir: .
  shell: bash
git:
  base_branch: main
  fallback_branch: HEAD~1
file_patterns: {}
checks: {}
"#;
            let config: CiConfig = serde_yaml::from_str(yaml).unwrap();
            assert_eq!(config.get_file_pattern("anything"), None);
            assert_eq!(config.get_file_color("any/file.txt"), "white");
        }

        // Edge case: Tests minimal config with empty checks - requires raw YAML to verify empty map handling
        #[test]
        fn test_empty_checks() {
            let yaml = r#"
version: 2
docker:
  project_dir: .
  shell: bash
git:
  base_branch: main
  fallback_branch: HEAD~1
file_patterns: {}
checks: {}
"#;
            let config: CiConfig = serde_yaml::from_str(yaml).unwrap();
            assert_eq!(config.groups().count(), 0);
            assert!(config.get_group("anything").is_none());
        }

        // Edge case: Tests config with no ignore_patterns - requires raw YAML to verify default empty list behavior
        #[test]
        fn test_empty_ignore_patterns() {
            let yaml = r#"
version: 2
docker:
  project_dir: .
  shell: bash
git:
  base_branch: main
  fallback_branch: HEAD~1
file_patterns: {}
checks: {}
"#;
            let config: CiConfig = serde_yaml::from_str(yaml).unwrap();
            // Should not ignore anything
            assert!(!config.should_ignore_file("README.md"));
            assert!(!config.should_ignore_file(".github/workflow.yml"));
            assert!(!config.should_ignore_file("src/main.rs"));
        }

        // Edge case: Tests check definition with all optional fields populated - requires raw YAML to verify complete field parsing
        #[test]
        fn test_check_with_all_optional_fields() {
            let yaml = r#"
version: 2
docker:
  project_dir: .
  service: default_service
  shell: bash
git:
  base_branch: main
  fallback_branch: HEAD~1
file_patterns:
  rust:
    pattern: '\.rs$'
checks:
  comprehensive:
    checks:
      full-check:
        name: Full Featured Check
        command: cargo test
        service: custom_service
        container: custom_container
        fix_command: cargo fmt
        on_demand: true
        env:
          TEST_VAR: value
        triggers:
          file_pattern: rust
"#;
            let config: CiConfig = serde_yaml::from_str(yaml).unwrap();
            let group = config.get_group("comprehensive").unwrap();
            let check = group.checks.get("full-check").unwrap();

            assert_eq!(check.name, "Full Featured Check");
            assert_eq!(check.command, "cargo test");
            assert_eq!(check.service, Some("custom_service".to_string()));
            assert_eq!(check.container, Some("custom_container".to_string()));
            assert_eq!(check.fix_command, Some("cargo fmt".to_string()));
            assert!(check.on_demand);
            assert_eq!(check.env.get("TEST_VAR"), Some(&"value".to_string()));
            assert!(check.triggers.is_some());
        }

        // Edge case: Tests absolute minimum required config fields - requires raw YAML to verify required vs optional field defaults
        #[test]
        fn test_minimal_valid_config() {
            // Absolute minimum required fields
            let yaml = r#"
version: 2
docker:
  project_dir: .
  shell: bash
git:
  base_branch: main
  fallback_branch: HEAD~1
file_patterns: {}
checks: {}
"#;
            let config: CiConfig = serde_yaml::from_str(yaml).unwrap();
            assert_eq!(config.version, 2);
            assert_eq!(config.docker().unwrap().project_dir, ".");
            assert_eq!(config.docker().unwrap().service, "app"); // Default value
            assert_eq!(config.docker().unwrap().shell, "bash");
            assert_eq!(config.git.base_branch, "main");
            assert_eq!(config.git.fallback_branch, "HEAD~1");
        }
    }

    mod test_check_definition {
        use super::*;

        // Edge case: Tests always_run() returns false when triggers are defined - requires raw YAML to test trigger presence detection
        #[test]
        fn test_always_run_with_triggers() {
            let yaml = r#"
version: 2
docker:
  project_dir: .
  shell: bash
git:
  base_branch: main
  fallback_branch: HEAD~1
file_patterns:
  php:
    pattern: '\.php$'
checks:
  group:
    checks:
      check_with_triggers:
        name: Check
        command: test
        triggers:
          file_pattern: php
"#;
            let config: CiConfig = serde_yaml::from_str(yaml).unwrap();
            let group = config.get_group("group").unwrap();
            let check = group.checks.get("check_with_triggers").unwrap();
            assert!(!check.always_run()); // Has triggers = not always_run
        }

        // Edge case: Tests always_run() returns true when no triggers defined - requires raw YAML to test trigger absence detection
        #[test]
        fn test_always_run_without_triggers() {
            let yaml = r#"
version: 2
docker:
  project_dir: .
  shell: bash
git:
  base_branch: main
  fallback_branch: HEAD~1
file_patterns: {}
checks:
  group:
    checks:
      check_no_triggers:
        name: Check
        command: test
"#;
            let config: CiConfig = serde_yaml::from_str(yaml).unwrap();
            let group = config.get_group("group").unwrap();
            let check = group.checks.get("check_no_triggers").unwrap();
            assert!(check.always_run()); // No triggers = always_run
        }

        // Edge case: Tests service_or_default() returns explicit service - requires raw YAML to verify service field override
        #[test]
        fn test_service_or_default_with_service() {
            let yaml = r#"
version: 2
docker:
  project_dir: .
  shell: bash
git:
  base_branch: main
  fallback_branch: HEAD~1
file_patterns: {}
checks:
  group:
    checks:
      check_with_service:
        name: Check
        command: test
        service: custom
"#;
            let config: CiConfig = serde_yaml::from_str(yaml).unwrap();
            let group = config.get_group("group").unwrap();
            let check = group.checks.get("check_with_service").unwrap();
            assert_eq!(check.service_or_default(Some("default")), Some("custom"));
        }

        // Edge case: Tests service_or_default() returns default when no service specified - requires raw YAML to verify fallback behavior
        #[test]
        fn test_service_or_default_without_service() {
            let yaml = r#"
version: 2
docker:
  project_dir: .
  shell: bash
git:
  base_branch: main
  fallback_branch: HEAD~1
file_patterns: {}
checks:
  group:
    checks:
      check_no_service:
        name: Check
        command: test
"#;
            let config: CiConfig = serde_yaml::from_str(yaml).unwrap();
            let group = config.get_group("group").unwrap();
            let check = group.checks.get("check_no_service").unwrap();
            assert_eq!(
                check.service_or_default(Some("default_service")),
                Some("default_service")
            );
        }
    }

    mod test_lazy_compiled_fallback {
        use super::super::{CiConfig, DockerConfig, FilePattern, GitConfig};
        use indexmap::IndexMap;
        use std::collections::HashMap;

        fn cfg_with_patterns(patterns: Vec<(&str, &str)>, ignore: Vec<&str>) -> CiConfig {
            let mut file_patterns = HashMap::new();
            for (k, v) in patterns {
                file_patterns.insert(
                    k.to_string(),
                    FilePattern {
                        pattern: v.to_string(),
                        color: None,
                    },
                );
            }
            CiConfig::new(
                2,
                DockerConfig {
                    project_dir: ".".to_string(),
                    service: "app".to_string(),
                    container: None,
                    image: None,
                    volume_mount: None,
                    work_dir: None,
                    shell: "bash".to_string(),
                    env: HashMap::new(),
                },
                GitConfig {
                    base_branch: "main".to_string(),
                    fallback_branch: "HEAD~1".to_string(),
                },
                file_patterns,
                IndexMap::new(),
                ignore.into_iter().map(String::from).collect(),
            )
        }

        #[test]
        fn valid_file_pattern_compiles_lazily() {
            let cfg = cfg_with_patterns(vec![("rust", r"\.rs$")], vec![]);
            let re = cfg
                .get_compiled_file_pattern("rust")
                .expect("lazy compile should succeed");
            assert!(re.is_match("src/main.rs"));
            assert!(!re.is_match("Cargo.toml"));
        }

        #[test]
        #[should_panic(expected = "invalid regex")]
        fn invalid_file_pattern_panics_on_lazy_path() {
            let cfg = cfg_with_patterns(vec![("good", r"\.rs$"), ("bad", r"[unclosed")], vec![]);
            let _ = cfg.get_compiled_file_pattern("bad");
        }

        #[test]
        fn unknown_key_returns_none() {
            let cfg = cfg_with_patterns(vec![("rust", r"\.rs$")], vec![]);
            assert!(cfg.get_compiled_file_pattern("missing").is_none());
        }

        #[test]
        #[should_panic(expected = "invalid regex")]
        fn invalid_ignore_pattern_panics_on_lazy_path() {
            let cfg = cfg_with_patterns(vec![], vec![r"\.md$", r"[unclosed"]);
            let _ = cfg.should_ignore_file("README.md");
        }
    }

    mod test_resolve_project_name_from_cwd {
        use super::super::resolve_project_name_from_cwd;

        #[test]
        fn dot_returns_current_dir_name() {
            let cwd = std::env::current_dir().unwrap();
            let expected = cwd.file_name().unwrap().to_string_lossy().to_string();
            assert_eq!(resolve_project_name_from_cwd("."), Some(expected));
        }

        #[test]
        fn dotdot_returns_parent_dir_name() {
            let cwd = std::env::current_dir().unwrap();
            let expected_parent = cwd.parent().map(|p| {
                p.file_name()
                    .map(|n| n.to_string_lossy().to_string())
                    .unwrap_or_default()
            });
            if let Some(parent_name) = expected_parent {
                if !parent_name.is_empty() {
                    assert_eq!(resolve_project_name_from_cwd(".."), Some(parent_name));
                }
            }
        }

        #[test]
        fn non_dot_input_returns_none() {
            assert_eq!(resolve_project_name_from_cwd("weird"), None);
            assert_eq!(resolve_project_name_from_cwd("./sub"), None);
        }
    }

    mod test_runner_mode {
        use super::*;

        const LOCAL_MIN: &str = "version: 2\nrunner: local\ngit:\n  base_branch: main\n  fallback_branch: HEAD~1\nfile_patterns: {}\nchecks: {}\n";

        #[test]
        fn test_runner_defaults_to_docker() {
            let yaml = "version: 2\ndocker:\n  project_dir: .\n  shell: bash\ngit:\n  base_branch: main\n  fallback_branch: HEAD~1\nfile_patterns: {}\nchecks: {}\n";
            let config: CiConfig = serde_yaml::from_str(yaml).unwrap();
            config.validate_and_compile().unwrap();
            assert!(matches!(config.runner, ExecTarget::Docker(_)));
        }

        #[test]
        fn test_runner_local_without_docker_section_is_valid() {
            let config: CiConfig = serde_yaml::from_str(LOCAL_MIN).unwrap();
            config.validate_and_compile().unwrap();
            let ExecTarget::Local(local) = &config.runner else {
                panic!("expected local runner, got {:?}", config.runner);
            };
            assert_eq!(local.shell, "bash");
            assert_eq!(config.default_service(), None);
            assert!(config.docker().is_none());
        }

        #[test]
        fn test_docker_mode_requires_docker_section() {
            let yaml = "version: 2\ngit:\n  base_branch: main\n  fallback_branch: HEAD~1\nfile_patterns: {}\nchecks: {}\n";
            let err = serde_yaml::from_str::<CiConfig>(yaml)
                .unwrap_err()
                .to_string();
            assert!(err.contains("`docker` section is required"), "got: {err}");
            assert!(err.contains("runner: local"), "got: {err}");
        }

        #[test]
        fn test_local_config_custom_shell_and_env() {
            let yaml = LOCAL_MIN.replace(
                "runner: local\n",
                "runner: local\nlocal:\n  shell: /bin/sh\n  env:\n    APP_ENV: test\n",
            );
            let config: CiConfig = serde_yaml::from_str(&yaml).unwrap();
            let ExecTarget::Local(local) = &config.runner else {
                panic!("expected local runner");
            };
            assert_eq!(local.shell, "/bin/sh");
            assert_eq!(local.env.get("APP_ENV").unwrap(), "test");
        }

        #[test]
        fn test_local_mode_rejects_empty_shell() {
            let yaml =
                LOCAL_MIN.replace("runner: local\n", "runner: local\nlocal:\n  shell: '  '\n");
            let config: CiConfig = serde_yaml::from_str(&yaml).unwrap();
            let err = config.validate_and_compile().unwrap_err().to_string();
            assert!(err.contains("local.shell"), "got: {err}");
        }

        #[test]
        fn test_clone_keeps_runner_and_local() {
            let config: CiConfig = serde_yaml::from_str(LOCAL_MIN).unwrap();
            let cloned = config.clone();
            assert!(matches!(cloned.runner, ExecTarget::Local(_)));
            assert!(cloned.docker().is_none());
        }

        #[test]
        fn test_local_mode_rejects_check_service() {
            let yaml = LOCAL_MIN.replace(
                "file_patterns: {}\nchecks: {}\n",
                "file_patterns:\n  rust: { pattern: '\\.rs$' }\nchecks:\n  g:\n    checks:\n      lint:\n        name: Lint\n        command: echo\n        service: app\n        triggers: { file_pattern: rust }\n",
            );
            let config: CiConfig = serde_yaml::from_str(&yaml).unwrap();
            let err = config.validate_and_compile().unwrap_err().to_string();
            assert!(
                err.contains("g.lint") && err.contains("runner: local"),
                "got: {err}"
            );
        }

        #[test]
        fn test_local_mode_rejects_pre_command_container() {
            let yaml = LOCAL_MIN.replace(
                "checks: {}\n",
                "checks:\n  g:\n    pre_commands:\n      - name: init\n        command: echo\n        container: db\n    checks: {}\n",
            );
            let config: CiConfig = serde_yaml::from_str(&yaml).unwrap();
            let err = config.validate_and_compile().unwrap_err().to_string();
            assert!(
                err.contains("pre_commands") && err.contains("runner: local"),
                "got: {err}"
            );
        }
    }

    mod test_expand_env_vars {
        use super::super::expand_env_vars;

        #[test]
        fn no_placeholder_returns_input_unchanged() {
            assert_eq!(expand_env_vars("plain string"), "plain string");
        }

        #[test]
        fn empty_input_returns_empty() {
            assert_eq!(expand_env_vars(""), "");
        }

        #[test]
        fn expands_set_variable() {
            std::env::set_var("CI_TUI_TEST_EXPAND_A", "resolved");
            assert_eq!(
                expand_env_vars("prefix-${CI_TUI_TEST_EXPAND_A}-suffix"),
                "prefix-resolved-suffix"
            );
            std::env::remove_var("CI_TUI_TEST_EXPAND_A");
        }

        #[test]
        fn missing_variable_is_left_as_literal() {
            std::env::remove_var("CI_TUI_TEST_EXPAND_MISSING_XYZ");
            let result = expand_env_vars("a-${CI_TUI_TEST_EXPAND_MISSING_XYZ}-b");
            assert_eq!(result, "a-${CI_TUI_TEST_EXPAND_MISSING_XYZ}-b");
        }

        #[test]
        fn expands_multiple_placeholders() {
            std::env::set_var("CI_TUI_TEST_EXPAND_X", "one");
            std::env::set_var("CI_TUI_TEST_EXPAND_Y", "two");
            let result = expand_env_vars("${CI_TUI_TEST_EXPAND_X}-${CI_TUI_TEST_EXPAND_Y}");
            assert_eq!(result, "one-two");
            std::env::remove_var("CI_TUI_TEST_EXPAND_X");
            std::env::remove_var("CI_TUI_TEST_EXPAND_Y");
        }

        #[test]
        fn same_placeholder_repeated_is_replaced_each_time() {
            std::env::set_var("CI_TUI_TEST_EXPAND_DUP", "X");
            let result = expand_env_vars("${CI_TUI_TEST_EXPAND_DUP}-${CI_TUI_TEST_EXPAND_DUP}");
            assert_eq!(result, "X-X");
            std::env::remove_var("CI_TUI_TEST_EXPAND_DUP");
        }
    }

    mod test_timeout {
        use super::*;
        use std::io::Write;
        use std::time::Duration;

        /// Local-mode config with one check `c` + one pre-command in group `g`.
        /// `top`, `check`, `pre` are optional `timeout:` values (raw YAML scalars).
        fn yaml(top: Option<&str>, check: Option<&str>, pre: Option<&str>) -> String {
            let t = |v: Option<&str>, indent: &str| {
                v.map(|v| format!("{indent}timeout: {v}\n"))
                    .unwrap_or_default()
            };
            format!(
                "version: 2\nrunner: local\n{}git:\n  base_branch: main\n  fallback_branch: HEAD~1\nfile_patterns: {{}}\nchecks:\n  g:\n    pre_commands:\n      - name: warmup\n        command: 'true'\n{}    checks:\n      c:\n        name: C\n        command: 'true'\n{}",
                t(top, ""),
                t(pre, "        "),
                t(check, "        "),
            )
        }

        fn parse(yaml: &str) -> CiConfig {
            serde_yaml::from_str(yaml).unwrap()
        }

        fn timeouts(config: &CiConfig) -> (Option<Duration>, Option<Duration>) {
            let group = config.get_group("g").unwrap();
            (group.checks["c"].timeout, group.pre_commands[0].timeout)
        }

        #[rstest]
        #[case("1s", 1)]
        #[case("30s", 30)]
        #[case("10m", 600)]
        #[case("1h", 3600)]
        fn test_check_timeout_parses(#[case] raw: &str, #[case] secs: u64) {
            let config = parse(&yaml(None, Some(raw), None));
            assert_eq!(timeouts(&config).0, Some(Duration::from_secs(secs)));
        }

        #[test]
        fn test_pre_command_timeout_parses() {
            let config = parse(&yaml(None, None, Some("45s")));
            assert_eq!(timeouts(&config).1, Some(Duration::from_secs(45)));
        }

        #[test]
        fn test_no_timeout_anywhere_is_unbounded() {
            assert_eq!(timeouts(&parse(&yaml(None, None, None))), (None, None));
        }

        #[test]
        fn test_global_timeout_is_default_and_item_overrides() {
            let s = Duration::from_secs;
            // Global applies where the item has none
            let config = parse(&yaml(Some("5m"), None, None));
            assert_eq!(timeouts(&config), (Some(s(300)), Some(s(300))));
            // Per-item value wins over global
            let config = parse(&yaml(Some("5m"), Some("10s"), Some("1h")));
            assert_eq!(timeouts(&config), (Some(s(10)), Some(s(3600))));
        }

        #[rstest]
        #[case::check(None, Some("abc"), None, "checks.g.checks.c: `timeout`")]
        #[case::unit(None, Some("10x"), None, "checks.g.checks.c: `timeout`")]
        #[case::zero(None, Some("0s"), None, "checks.g.checks.c: `timeout`")]
        #[case::empty(None, Some("''"), None, "checks.g.checks.c: `timeout`")]
        #[case::pre(None, None, Some("abc"), "checks.g.pre_commands[0]: `timeout`")]
        #[case::global(Some("-1s"), None, None, "`timeout`: invalid duration")]
        fn test_load_config_rejects_bad_timeout(
            #[case] top: Option<&str>,
            #[case] check: Option<&str>,
            #[case] pre: Option<&str>,
            #[case] path: &str,
        ) {
            let mut tmp = tempfile::NamedTempFile::new().unwrap();
            tmp.write_all(yaml(top, check, pre).as_bytes()).unwrap();
            let msg = format!("{:#}", load_config(tmp.path()).unwrap_err());
            assert!(msg.contains(path), "error should name `{path}`: {msg}");
            assert!(msg.contains("duration"), "error should explain: {msg}");
        }
    }

    mod test_max_output_lines {
        use super::*;

        /// Minimal local-mode config; `max_output_lines` line optional.
        fn yaml(max_output_lines: Option<usize>) -> String {
            let extra = max_output_lines
                .map(|n| format!("max_output_lines: {n}\n"))
                .unwrap_or_default();
            format!(
                "version: 2\nrunner: local\n{extra}git:\n  base_branch: main\n  fallback_branch: HEAD~1\nfile_patterns: {{}}\nchecks: {{}}\n"
            )
        }

        #[test]
        fn defaults_when_omitted() {
            let config: CiConfig = serde_yaml::from_str(&yaml(None)).unwrap();
            assert_eq!(config.max_output_lines, DEFAULT_MAX_OUTPUT_LINES);
        }

        #[test]
        fn custom_value_is_applied() {
            let config: CiConfig = serde_yaml::from_str(&yaml(Some(500))).unwrap();
            assert_eq!(config.max_output_lines, 500);
        }
    }
}
