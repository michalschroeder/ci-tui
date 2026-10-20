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

use crate::exit::ConfigError;
use anyhow::{bail, ensure, Context, Result};
use indexmap::IndexMap;
use regex::Regex;
use schemars::JsonSchema;
use serde::Deserialize;
use std::collections::HashMap;
use std::num::NonZeroUsize;
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
    /// Cap on concurrent checks per `parallel: true` group (`--jobs` overrides
    /// in main). `None` = CPU count; see [`parallel_limit`].
    pub max_parallel: Option<NonZeroUsize>,
    /// TUI end-of-run bell + desktop notification (or `--notify`); see
    /// [`crate::ui::notify`]
    pub notify: bool,
    /// Hash of the config file content, set by [`load_config`] (0 otherwise);
    /// a result cache key input, see [`crate::cache`] and [`cache_text`]
    pub(crate) source_hash: u64,
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
/// Also the root of the config JSON Schema (`ci-tui schema`); the `if`/`else`
/// mirrors the parse-time "docker section required unless `runner: local`" rule.
#[derive(Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
#[schemars(
    title = "ci-tui config",
    description = "ci-tui.yaml: CI checks to run for changed files",
    extend(
        "if" = {"properties": {"runner": {"const": "local"}}, "required": ["runner"]},
        "else" = {"required": ["docker"], "properties": {"docker": {"type": "object"}}}
    )
)]
pub(crate) struct RawCiConfig {
    /// Config format version (must be 2)
    #[schemars(extend("const" = SUPPORTED_VERSION))]
    version: u32,
    /// Where checks run: `docker` (default) or `local` (on the host)
    #[serde(default)]
    runner: RunnerMode,
    /// Required when runner is docker (the default)
    #[serde(default)]
    docker: Option<RawDockerConfig>,
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
    #[schemars(schema_with = "timeout_schema", description = TIMEOUT_DESCRIPTION)]
    timeout: Option<Duration>,
    /// Max lines kept per stdout/stderr stream (default 10000)
    #[serde(default)]
    max_output_lines: Option<usize>,
    /// Max checks running at once in a `parallel: true` group (>= 1; default:
    /// CPU count; `--jobs` overrides)
    #[serde(default)]
    max_parallel: Option<NonZeroUsize>,
    /// TUI: ring the bell and send a desktop notification (OSC 9) when a full
    /// run finishes (default false; `--notify` also enables it)
    #[serde(default)]
    notify: bool,
}

impl TryFrom<RawCiConfig> for CiConfig {
    type Error = String;

    fn try_from(raw: RawCiConfig) -> std::result::Result<Self, Self::Error> {
        // Before the docker section: a wrong version explains its other errors
        check_version(raw.version)?;
        let runner = match (raw.runner, raw.docker) {
            (RunnerMode::Docker, Some(docker)) => ExecTarget::Docker(docker.try_into()?),
            (RunnerMode::Docker, None) => {
                return Err(
                    "`docker` section is required when runner is `docker` (the default). \
                     Set `runner: local` to run checks on the host instead."
                        .to_string(),
                )
            }
            // A `docker` section in local mode is ignored, but must be valid
            // (mode rules too, as the schema requires)
            (RunnerMode::Local, docker) => {
                docker.map(DockerConfig::try_from).transpose()?;
                ExecTarget::Local(raw.local)
            }
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
            max_parallel: raw.max_parallel,
            notify: raw.notify,
            source_hash: 0,
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

/// Deserialize a key that may be left out (`None`); a value, `~` included
/// (a string to serde_yaml), is `Some`
fn deserialize_some<'de, D>(deserializer: D) -> std::result::Result<Option<String>, D::Error>
where
    D: serde::Deserializer<'de>,
{
    String::deserialize(deserializer).map(Some)
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

/// Deserialize an optional `field:` regex, compiled at load so a bad pattern
/// fails startup (serde_yaml prefixes the check's path, as for `timeout`).
fn deserialize_regex<'de, D>(
    deserializer: D,
    field: &str,
) -> std::result::Result<Option<Regex>, D::Error>
where
    D: serde::Deserializer<'de>,
{
    Option::<String>::deserialize(deserializer)?
        .map(|raw| {
            Regex::new(&raw).map_err(|e| {
                serde::de::Error::custom(format!("`{field}`: invalid regex '{raw}': {e}"))
            })
        })
        .transpose()
}

/// [`deserialize_regex`] for a check's `error_pattern:`
fn deserialize_error_pattern<'de, D>(
    deserializer: D,
) -> std::result::Result<Option<Regex>, D::Error>
where
    D: serde::Deserializer<'de>,
{
    deserialize_regex(deserializer, "error_pattern")
}

/// [`deserialize_regex`] for `triggers.files_filter:`
fn deserialize_files_filter<'de, D>(deserializer: D) -> std::result::Result<Option<Regex>, D::Error>
where
    D: serde::Deserializer<'de>,
{
    deserialize_regex(deserializer, "files_filter")
}

/// Built-in error-line regex for checks without `error_pattern`. Matches error
/// indicators, not paths or counts: `error:` / `Error[E0308]:` (so `src/Error.php`
/// and `0 error(s)` stay unmarked), whole-word `ERROR` / `FAIL` / `FAILED` /
/// `panicked` / `Traceback`, and `*Exception` followed by `:`, space or end of
/// line (not `FooException.php`). Case-sensitive.
const DEFAULT_ERROR_PATTERN: &str =
    r"\b[Ee]rror(?:\[\w+\])?:|\b(?:ERROR|FAIL|FAILED|panicked|Traceback)\b|\w*Exception(?::|\s|$)";

/// Compiled [`DEFAULT_ERROR_PATTERN`] (built once)
pub fn default_error_regex() -> &'static Regex {
    static RE: OnceLock<Regex> = OnceLock::new();
    RE.get_or_init(|| Regex::new(DEFAULT_ERROR_PATTERN).expect("valid default error pattern"))
}

/// Schema description shared by every `timeout:` field.
const TIMEOUT_DESCRIPTION: &str =
    "Max runtime: <n>s, <n>m or <n>h (e.g. 30s, 10m); expired commands are killed. \
     Unset = no limit (top-level `timeout` is the default for checks and pre-commands)";

/// Schema for `timeout:` fields: what [`deserialize_timeout`] accepts
/// (`<n>s|m|h`, n > 0) or null.
fn timeout_schema(_: &mut schemars::SchemaGenerator) -> schemars::Schema {
    schemars::json_schema!({
        "type": ["string", "null"],
        "pattern": "^0*[1-9][0-9]*[smh]$"
    })
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
    /// Run inside Docker: compose services or a plain image (see [`DockerConfig`])
    Docker(DockerConfig),
    /// Run directly on the host via `local.shell -c` from the repo root
    Local(LocalConfig),
}

/// File pattern definition with optional color for UI display
#[derive(Debug, Clone, Deserialize, JsonSchema)]
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
            max_parallel: self.max_parallel,
            notify: self.notify,
            source_hash: self.source_hash,
            // Reset caches on clone — repopulated via load_config or lazy fallback
            compiled_ignore_patterns: OnceLock::new(),
            compiled_file_patterns: OnceLock::new(),
        }
    }
}

/// Raw `runner:` value; resolved into [`ExecTarget`] during parsing.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize, Default, JsonSchema)]
#[serde(rename_all = "lowercase")]
enum RunnerMode {
    #[default]
    Docker,
    Local,
}

/// Configuration for local (host) execution when `runner: local`.
#[derive(Debug, Clone, Deserialize, JsonSchema)]
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
/// Two modes, picked by `image`:
/// - compose (no `image`): a command runs in its compose service (`service`,
///   overridable per check / pre-command): `docker exec` into the service's
///   running container, else `docker compose run`
/// - plain image (`image` set): every command runs in `docker run <image>`
///   with `volume_mount` / `work_dir` / `user`; no compose calls
//
// On-disk shape (both modes' keys side by side); converted to the typed
// [`DockerConfig`] right after parsing, which rejects the other mode's keys.
#[derive(Deserialize, JsonSchema)]
#[serde(rename = "DockerConfig", deny_unknown_fields)]
#[schemars(extend(
    "if" = {"required": ["image"]},
    "then" = {
        "not": {"anyOf": [
            {"required": ["project_dir"]},
            {"required": ["service"]}
        ]}
    },
    "else" = {
        "required": ["project_dir"],
        "not": {"anyOf": [
            {"required": ["volume_mount"]},
            {"required": ["work_dir"]},
            {"required": ["user"]}
        ]}
    }
))]
pub(crate) struct RawDockerConfig {
    /// Compose mode (required without `image`): directory for `docker compose
    /// --project-directory`
    #[serde(default)]
    project_dir: Option<String>,
    /// Compose mode: default service commands run in (default "app")
    #[serde(default, deserialize_with = "deserialize_some")]
    #[schemars(with = "String", extend("default" = DEFAULT_SERVICE))]
    service: Option<String>,
    /// Plain-image mode: every command runs in `docker run` of this image
    /// (e.g. "rust:latest"); no compose
    #[serde(default)]
    image: Option<String>,
    /// Plain-image mode: repo mount for `docker run -v` (e.g. ".:/build";
    /// `${VAR}` expanded)
    #[serde(default)]
    volume_mount: Option<String>,
    /// Plain-image mode: working directory inside the container (default "/app")
    #[serde(default)]
    work_dir: Option<String>,
    /// Plain-image mode: `docker run --user` (e.g. "1000:1000"); "host" = uid:gid
    /// of the ci-tui process, so files checks write are owned like its own.
    /// Default: the image's user
    #[serde(default)]
    user: Option<String>,
    /// Shell to use inside container (e.g., "bash" or "/bin/sh" for Alpine)
    shell: String,
    /// Environment variables passed to every docker command
    #[serde(default)]
    env: HashMap<String, String>,
}

impl TryFrom<RawDockerConfig> for DockerConfig {
    type Error = String;

    /// Pick the mode by `image`; error on a key of the other mode (or a
    /// compose mode without `project_dir`)
    fn try_from(mut raw: RawDockerConfig) -> std::result::Result<Self, Self::Error> {
        let mode = match raw.image.take() {
            Some(image) => raw.image_mode(image)?,
            None => raw.compose_mode()?,
        };
        Ok(Self {
            mode,
            shell: raw.shell,
            env: raw.env,
        })
    }
}

impl RawDockerConfig {
    /// Plain-image mode of `image` (takes its keys); error on a compose key
    fn image_mode(&mut self, image: String) -> std::result::Result<DockerMode, String> {
        let compose_keys = [
            ("project_dir", &self.project_dir),
            ("service", &self.service),
        ];
        reject_set_keys(&compose_keys, |key| {
            format!(
                "`docker.{key}` is not allowed with `docker.image`: \
                 plain-image mode runs every command in that image, without compose"
            )
        })?;
        Ok(DockerMode::Image(ImageConfig {
            image,
            volume_mount: self.volume_mount.take(),
            work_dir: self.work_dir.take(),
            user: self.user.take(),
        }))
    }

    /// Compose mode (takes its keys); error on a plain-image key or without
    /// `project_dir`
    fn compose_mode(&mut self) -> std::result::Result<DockerMode, String> {
        let image_keys = [
            ("volume_mount", &self.volume_mount),
            ("work_dir", &self.work_dir),
            ("user", &self.user),
        ];
        reject_set_keys(&image_keys, |key| {
            format!(
                "`docker.{key}` needs `docker.image`: compose services take volumes, \
                 working_dir and user from the compose file"
            )
        })?;
        let project_dir = self.project_dir.take().ok_or(
            "`docker.project_dir` is required without `docker.image` (compose mode: \
             the directory `docker compose --project-directory` uses)",
        )?;
        Ok(DockerMode::Compose(ComposeConfig {
            project_dir,
            service: self
                .service
                .take()
                .unwrap_or_else(|| DEFAULT_SERVICE.to_string()),
        }))
    }
}

/// `error(key)` for the first set `docker.<key>` of `keys` (wrong-mode keys)
fn reject_set_keys(
    keys: &[(&str, &Option<String>)],
    error: impl Fn(&str) -> String,
) -> std::result::Result<(), String> {
    match keys.iter().find(|(_, value)| value.is_some()) {
        Some((key, _)) => Err(error(key)),
        None => Ok(()),
    }
}

/// Docker settings of `runner: docker` (from [`RawDockerConfig`]): the mode
/// plus what both modes share.
///
/// Design: the mode is an enum chosen once at parse time instead of
/// `Option` fields of both modes side by side, so wrong-mode keys are
/// rejected in one place ([`TryFrom<RawDockerConfig>`]) and every consumer
/// `match`es the mode (see [`crate::runner::resolve_service`]).
#[derive(Debug, Clone)]
pub struct DockerConfig {
    /// Compose services or a plain image
    pub mode: DockerMode,
    /// Shell to use inside container (e.g., "bash" or "/bin/sh" for Alpine)
    pub shell: String,
    /// Environment variables passed to every docker command
    pub env: HashMap<String, String>,
}

/// How docker-mode commands get a container
#[derive(Debug, Clone)]
pub enum DockerMode {
    /// Compose services (`docker exec` / `docker compose run`)
    Compose(ComposeConfig),
    /// `docker run` of one image (`docker.image`)
    Image(ImageConfig),
}

/// Compose mode (no `docker.image`)
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ComposeConfig {
    /// Directory for `docker compose --project-directory`
    pub project_dir: String,
    /// Service a command without its own `service` runs in (default "app");
    /// applied by [`crate::runner::resolve_service`]
    pub service: String,
}

#[cfg(test)]
impl DockerConfig {
    /// Compose mode of `project_dir` / `service`, `bash`, no env
    pub(crate) fn compose_for_test(project_dir: &str, service: &str) -> Self {
        Self {
            mode: DockerMode::Compose(ComposeConfig {
                project_dir: project_dir.to_string(),
                service: service.to_string(),
            }),
            shell: "bash".to_string(),
            env: HashMap::new(),
        }
    }
}

impl ComposeConfig {
    /// Service a command runs in: its own `service`, else `docker.service`
    pub fn service_for<'a>(&'a self, service: Option<&'a str>) -> &'a str {
        service.unwrap_or(&self.service)
    }
}

/// Plain-image mode (`docker.image`): every command is a `docker run`
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ImageConfig {
    /// Image every command runs in (e.g. "rust:latest")
    pub image: String,
    /// Repo mount for `docker run -v` (`${VAR}` expanded)
    pub volume_mount: Option<String>,
    /// Working directory inside the container (default "/app")
    pub work_dir: Option<String>,
    /// `docker run --user`; "host" = uid:gid of the ci-tui process
    pub user: Option<String>,
}

impl ImageConfig {
    /// Get volume mount arguments for docker run
    /// Returns "-v {volume_mount}" if volume_mount is set, None otherwise
    /// Expands environment variables in format ${VAR_NAME}
    pub fn volume_args(&self) -> Option<String> {
        self.volume_mount.as_ref().map(|mount| {
            let expanded = expand_env_vars(mount);
            format!("-v {}", expanded)
        })
    }

    /// `docker run --user` value: `user`, "host" resolved to this process's
    /// `uid:gid`; None if unset (image's user)
    pub fn run_user(&self) -> Option<String> {
        self.user.as_deref().map(|user| match user {
            "host" => crate::utils::docker::host_user(),
            spec => spec.to_owned(),
        })
    }

    /// Get the working directory inside container
    /// Returns work_dir if set, otherwise "/app"
    pub fn working_dir(&self) -> &str {
        self.work_dir.as_deref().unwrap_or("/app")
    }
}

/// Default compose service (`docker.service` unset)
const DEFAULT_SERVICE: &str = "app";

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
#[derive(Debug, Clone, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct GitConfig {
    pub base_branch: String,
    pub fallback_branch: String,
}

/// Configuration for an execution group
#[derive(Debug, Clone, Deserialize, JsonSchema)]
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
    /// Max checks running at once when `parallel: true` (>= 1; capped by the
    /// top-level `max_parallel` / `--jobs`)
    #[serde(default)]
    pub max_parallel: Option<NonZeroUsize>,
    /// Checks in this group (key is check ID)
    pub checks: IndexMap<String, CheckDefinition>,
}

/// Definition of a single CI check.
///
/// Contains the command to run, optional fix command, triggers for when
/// to run, and other configuration options.
#[derive(Debug, Clone, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct CheckDefinition {
    pub name: String,
    pub command: String,
    /// Compose service to run this check in (default `docker.service`; not
    /// allowed with `docker.image` or `runner: local`)
    #[serde(default)]
    pub service: Option<String>,
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
    #[schemars(schema_with = "timeout_schema", description = TIMEOUT_DESCRIPTION)]
    pub timeout: Option<Duration>,
    /// Regex marking error lines in a failed check's output (TUI highlights
    /// them and opens the check scrolled to the first). Unset = built-in
    /// default (error:, Error[code]:, ERROR, FAIL, FAILED, panicked,
    /// Traceback, *Exception followed by `:`/space/end of line).
    #[serde(default, deserialize_with = "deserialize_error_pattern")]
    #[schemars(with = "Option<String>")]
    pub error_pattern: Option<Regex>,
}

/// Triggers that determine when a check should run.
#[derive(Debug, Clone, Deserialize, Default, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct CheckTriggers {
    #[serde(default)]
    pub file_pattern: Option<String>,
    #[serde(default)]
    pub test_discovery: Option<TestDiscoveryConfig>,
    /// Inline regex every `{files}` path must match: file_pattern matches and
    /// discovered tests that don't are dropped (e.g. `Test\.php$` keeps test
    /// fixtures away from the test runner). All dropped: no-tests fallback
    /// (on_demand / full command without `{files}` / skip). Needs
    /// `file_pattern` or `test_discovery`.
    #[serde(default, deserialize_with = "deserialize_files_filter")]
    #[schemars(with = "Option<String>")]
    pub files_filter: Option<Regex>,
}

impl CheckTriggers {
    /// `files_filter` without `file_pattern` / `test_discovery` to filter
    fn files_filter_alone(&self) -> bool {
        self.files_filter.is_some() && self.file_pattern.is_none() && self.test_discovery.is_none()
    }
}

/// Inline test discovery configuration for a check
#[derive(Debug, Clone, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct TestDiscoveryConfig {
    /// Source file pattern key that triggers discovery (references file_patterns)
    pub source_pattern: String,
    /// Strategies to find related tests
    pub strategies: Vec<TestDiscoveryStrategy>,
}

/// Strategy for discovering related test files
#[derive(Debug, Clone, Deserialize, JsonSchema)]
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
#[derive(Debug, Clone, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct PathMappingRule {
    /// Source file pattern with {path} placeholder (e.g., "src/{path}.php")
    pub source: String,
    /// Test file patterns with {path} placeholder (e.g., "tests/{path}Test.php")
    pub tests: Vec<String>,
}

/// A command to run before checks in a group (e.g., database initialization).
#[derive(Debug, Clone, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct PreCommand {
    /// Display name for the command
    pub name: String,
    /// The command to execute
    pub command: String,
    /// Compose service to run in (default `docker.service`; not allowed with
    /// `docker.image` or `runner: local`)
    #[serde(default)]
    pub service: Option<String>,
    /// If true, run command directly on host instead of inside Docker
    #[serde(default)]
    pub host: bool,
    /// Additional environment variables for this command (merged with global env)
    #[serde(default)]
    pub env: std::collections::HashMap<String, String>,
    /// Max runtime; expiry fails the pre-command (and so the group). `None` = unbounded.
    #[serde(default, deserialize_with = "deserialize_timeout")]
    #[schemars(schema_with = "timeout_schema", description = TIMEOUT_DESCRIPTION)]
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

/// Error if a check or pre-command sets `service`, which `mode` (`runner:
/// local`, `docker.image`) does not allow: services only exist in compose mode.
fn reject_services(
    checks: &IndexMap<String, GroupConfig>,
    mode: &str,
) -> std::result::Result<(), String> {
    for (g, group) in checks {
        if let Some(i) = group.pre_commands.iter().position(|p| p.service.is_some()) {
            return Err(format!(
                "`{g}.pre_commands[{i}]` sets `service`, which is not allowed with {mode}"
            ));
        }
        if let Some((c, _)) = group
            .checks
            .iter()
            .find(|(_, check)| check.service.is_some())
        {
            return Err(format!(
                "`{g}.{c}` sets `service`, which is not allowed with {mode}"
            ));
        }
    }
    Ok(())
}

/// Error unless `version` is [`SUPPORTED_VERSION`]
fn check_version(version: u32) -> std::result::Result<(), String> {
    if version == SUPPORTED_VERSION {
        Ok(())
    } else {
        Err(format!(
            "unsupported config version {version} (expected {SUPPORTED_VERSION})"
        ))
    }
}

/// Personal overrides file, read from the config file's directory
/// (gitignored by the user); see [`LocalOverrides`]
pub const LOCAL_CONFIG_FILE: &str = "ci-tui.local.yaml";

/// Allowed shape of [`LOCAL_CONFIG_FILE`]: personal settings only, same
/// paths as in the config. Validation only: [`merge_yaml`] merges the file's
/// YAML into the config's before parsing, so mode rules still apply.
//
// Fields are never read: deserializing checks keys and types, naming the
// offending key. Alternative: apply typed fields onto the parsed config;
// rejected, it would repeat the docker mode rules of `RawDockerConfig`.
#[allow(dead_code)]
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct LocalOverrides {
    #[serde(default)]
    docker: Option<DockerOverrides>,
    #[serde(default)]
    local: Option<EnvOverrides>,
    #[serde(default)]
    max_parallel: Option<NonZeroUsize>,
    #[serde(default)]
    notify: Option<bool>,
}

/// `docker:` keys of [`LocalOverrides`]
#[allow(dead_code)]
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct DockerOverrides {
    #[serde(default)]
    service: Option<String>,
    #[serde(default)]
    env: Option<HashMap<String, String>>,
}

/// `local:` keys of [`LocalOverrides`]
#[allow(dead_code)]
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct EnvOverrides {
    #[serde(default)]
    env: Option<HashMap<String, String>>,
}

/// YAML of the [`LOCAL_CONFIG_FILE`] at `path`, validated as
/// [`LocalOverrides`]; `None` if missing or empty
fn read_overrides(path: &Path) -> Result<Option<serde_yaml::Value>> {
    let content = match std::fs::read_to_string(path) {
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(None),
        content => content
            .with_context(|| format!("Failed to read local config file: {}", path.display()))?,
    };
    let invalid = || {
        format!(
            "Invalid local config file: {} (allowed: docker.service, docker.env, \
             local.env, max_parallel, notify)",
            path.display()
        )
    };
    let value: serde_yaml::Value = serde_yaml::from_str(&content).with_context(invalid)?;
    if value.is_null() {
        return Ok(None);
    }
    serde_yaml::from_str::<LocalOverrides>(&content).with_context(invalid)?;
    Ok(Some(value))
}

/// Merge `local` into `base`: mappings key by key (`env` maps: local wins
/// per key), other values replaced; `~` keeps the base value
fn merge_yaml(base: &mut serde_yaml::Value, local: serde_yaml::Value) {
    use serde_yaml::Value;
    match (base, local) {
        (_, Value::Null) => {}
        (Value::Mapping(base), Value::Mapping(local)) => {
            for (key, value) in local.into_iter().filter(|(_, v)| !v.is_null()) {
                merge_yaml(base.entry(key).or_insert(Value::Null), value);
            }
        }
        (base, local) => *base = local,
    }
}

/// Load configuration from a YAML file, merged with the
/// [`LOCAL_CONFIG_FILE`] next to it if present.
///
/// # Errors
///
/// Returns an error if either file cannot be read, parsed, or contains invalid regexes.
pub fn load_config(path: &Path) -> Result<CiConfig> {
    let content = std::fs::read_to_string(path)
        .with_context(|| format!("Failed to read config file: {}", path.display()))
        .map_err(ConfigError)?;

    // Parsed alone first: its errors keep their line numbers
    let mut config: CiConfig = serde_yaml::from_str(&content)
        .with_context(|| format!("Failed to parse config file: {}", path.display()))
        .map_err(ConfigError)?;
    let local_path = path.with_file_name(LOCAL_CONFIG_FILE);
    let mut cached = content;
    if let Some(overrides) = read_overrides(&local_path).map_err(ConfigError)? {
        let parse_merged = || -> Result<(CiConfig, String)> {
            let mut merged: serde_yaml::Value = serde_yaml::from_str(&cached)?;
            merge_yaml(&mut merged, overrides);
            let text = serde_yaml::to_string(&merged)?;
            Ok((serde_yaml::from_value(merged)?, text))
        };
        (config, cached) = parse_merged()
            .with_context(|| {
                format!(
                    "Failed to parse config file: {} with overrides from {}",
                    path.display(),
                    local_path.display()
                )
            })
            .map_err(ConfigError)?;
    }
    config
        .validate_and_compile()
        .with_context(|| format!("Invalid config file: {}", path.display()))
        .map_err(ConfigError)?;
    config.source_hash = crate::cache::hash_bytes(cache_text(&cached).as_bytes());
    Ok(config)
}

/// Config content the result cache key covers: the YAML re-serialized
/// without the UI-only `notify` key (toggling it keeps cached passes;
/// comments and formatting don't count either). Raw text if not a mapping.
fn cache_text(content: &str) -> String {
    match serde_yaml::from_str::<serde_yaml::Value>(content) {
        Ok(serde_yaml::Value::Mapping(mut map)) => {
            map.remove("notify");
            serde_yaml::to_string(&map).unwrap_or_else(|_| content.to_string())
        }
        _ => content.to_string(),
    }
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
            max_parallel: None,
            notify: false,
            source_hash: 0,
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
        check_version(self.version).map_err(anyhow::Error::msg)?;
        // Docker section and mode keys are checked at parse time (RawCiConfig)
        self.check_services().map_err(anyhow::Error::msg)?;
        if let ExecTarget::Local(local) = &self.runner {
            ensure!(
                !local.shell.trim().is_empty(),
                "`local.shell` must not be empty when runner is `local`"
            );
        }
        self.check_pattern_references()?;
        self.check_files_filter_scope()?;
        self.check_unique_ids()?;
        let _ = self
            .compiled_file_patterns
            .set(compile_file_patterns(&self.file_patterns)?);
        let _ = self
            .compiled_ignore_patterns
            .set(compile_ignore_patterns(&self.ignore_patterns)?);
        Ok(())
    }

    /// Check / pre-command `service` only in compose mode (services exist
    /// there only)
    fn check_services(&self) -> std::result::Result<(), String> {
        let mode = match &self.runner {
            ExecTarget::Docker(DockerConfig {
                mode: DockerMode::Compose(_),
                ..
            }) => return Ok(()),
            ExecTarget::Docker(_) => "`docker.image` (every command runs in that image)",
            ExecTarget::Local(_) => "`runner: local`",
        };
        reject_services(&self.checks, mode)
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

    /// `files_filter` filters `file_pattern` / `test_discovery` files: alone it
    /// would silently drop the check (empty triggers block).
    fn check_files_filter_scope(&self) -> Result<()> {
        let orphan = self
            .checks
            .iter()
            .flat_map(|(g, group)| group.checks.iter().map(move |(c, check)| (g, c, check)))
            .find(|(_, _, check)| {
                (check.triggers.as_ref()).is_some_and(CheckTriggers::files_filter_alone)
            });
        match orphan {
            Some((g, c, _)) => bail!(
                "checks.{g}.checks.{c}.triggers.files_filter: needs `file_pattern` or `test_discovery` (it filters their files)"
            ),
            None => Ok(()),
        }
    }

    /// Check ids must be unique across groups: results, `--only` and the
    /// result cache are keyed by id alone.
    fn check_unique_ids(&self) -> Result<()> {
        let mut groups: HashMap<&str, &str> = HashMap::new();
        let duplicate = self
            .checks
            .iter()
            .flat_map(|(g, group)| group.checks.keys().map(move |c| (g, c)))
            .find_map(|(g, c)| Some((c, groups.insert(c, g)?, g)));
        match duplicate {
            Some((c, first, g)) => {
                bail!("check id `{c}` is defined in groups `{first}` and `{g}`: ids must be unique")
            }
            None => Ok(()),
        }
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

    /// Get group config by name
    pub fn get_group(&self, group_name: &str) -> Option<&GroupConfig> {
        self.checks.get(group_name)
    }

    /// Pre-commands of `group` (none for an unknown group)
    pub fn pre_commands(&self, group: &str) -> &[PreCommand] {
        self.get_group(group).map_or(&[], |g| &g.pre_commands)
    }

    /// Iterate over groups in execution order
    pub fn groups(&self) -> impl Iterator<Item = (&str, &GroupConfig)> {
        self.checks.iter().map(|(k, v)| (k.as_str(), v))
    }

    /// Max concurrent checks in parallel `group` (see [`parallel_limit`])
    pub fn group_parallel_limit(&self, group: Option<&GroupConfig>) -> usize {
        parallel_limit(self.max_parallel, group.and_then(|g| g.max_parallel)).get()
    }
}

/// Effective cap for a `parallel: true` group: `global` (`--jobs` / top-level
/// `max_parallel`, else CPU count, else 1), lowered by the group's own
/// `max_parallel` (a group cannot raise the global cap). Applies to checks
/// started by a group run only: TUI retries (`r`), on-demand triggers (`t`) and
/// run-all-files (`A`) are not limited. Clamped to the semaphore's
/// [`MAX_PERMITS`](tokio::sync::Semaphore::MAX_PERMITS) (larger values panic).
fn parallel_limit(global: Option<NonZeroUsize>, group: Option<NonZeroUsize>) -> NonZeroUsize {
    let global = global
        .or_else(|| std::thread::available_parallelism().ok())
        .unwrap_or(NonZeroUsize::MIN);
    let max = NonZeroUsize::new(tokio::sync::Semaphore::MAX_PERMITS).unwrap_or(NonZeroUsize::MIN);
    group.map_or(global, |group| group.min(global)).min(max)
}

impl GroupConfig {
    /// Get display name (uses provided name or falls back to key)
    pub fn display_name<'a>(&'a self, key: &'a str) -> &'a str {
        self.name.as_deref().unwrap_or(key)
    }
}

impl CheckDefinition {
    /// Check if this check should always run (no triggers defined)
    pub fn always_run(&self) -> bool {
        self.triggers.is_none()
    }

    /// Regex marking error lines: `error_pattern` or the built-in default
    pub fn error_regex(&self) -> &Regex {
        self.error_pattern
            .as_ref()
            .unwrap_or_else(|| default_error_regex())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use rstest::rstest;

    // Import builder types from the main config module
    use super::{
        CheckDefinition, CheckTriggers, CiConfig, ComposeConfig, DockerConfig, DockerMode,
        FilePattern, GitConfig, GroupConfig, DEFAULT_MAX_OUTPUT_LINES,
    };
    use indexmap::IndexMap;
    use std::collections::HashMap;
    use std::sync::OnceLock;

    /// Compose settings of a docker-mode `config` (panics otherwise)
    fn compose(config: &CiConfig) -> &ComposeConfig {
        match config.docker().map(|d| &d.mode) {
            Some(DockerMode::Compose(compose)) => compose,
            other => panic!("expected compose mode, got {other:?}"),
        }
    }

    /// Plain-image settings of a docker-mode `config` (panics otherwise)
    fn image(config: &CiConfig) -> &ImageConfig {
        match config.docker().map(|d| &d.mode) {
            Some(DockerMode::Image(image)) => image,
            other => panic!("expected plain-image mode, got {other:?}"),
        }
    }

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
                    max_parallel: None,
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
                    max_parallel: None,
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
                    max_parallel: None,
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
                runner: ExecTarget::Docker(DockerConfig::compose_for_test(
                    &self.docker_project_dir,
                    &self.docker_service,
                )),
                git: GitConfig {
                    base_branch: self.git_base,
                    fallback_branch: self.git_fallback,
                },
                file_patterns: self.file_patterns,
                checks: self.checks,
                ignore_patterns: self.ignore_patterns,
                max_output_lines: self.max_output_lines,
                max_parallel: None,
                notify: false,
                source_hash: 0,
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
                fix_command: self.fix_command,
                triggers: self.triggers,
                on_demand: false,
                env: HashMap::new(),
                timeout: None,
                error_pattern: None,
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
    fn test_compose_service() {
        let config = ConfigBuilder::new()
            .with_docker("./infrastructure", "php")
            .build();
        assert_eq!(compose(&config).service, "php");
        assert_eq!(compose(&config).service_for(None), "php");
        assert_eq!(compose(&config).service_for(Some("db")), "db");
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
    fn test_check_service() {
        let config = php_project_config();

        let fast = config.get_group("fast").unwrap();
        let php_lint = fast.checks.get("php-lint").unwrap();
        assert_eq!(php_lint.service, None);

        let tests = config.get_group("tests").unwrap();
        let phpunit = tests.checks.get("phpunit").unwrap();
        assert_eq!(phpunit.service.as_deref(), Some("custom-service"));
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

        /// Load-validated config with `docker` section body `docker` and
        /// group `g` body `group`; the validation error text on failure
        fn validated(docker: &str, group: &str) -> std::result::Result<CiConfig, String> {
            let yaml = format!(
                "version: 2\ndocker:\n{docker}git:\n  base_branch: main\n  fallback_branch: HEAD~1\nfile_patterns: {{}}\nchecks:\n  g:\n{group}"
            );
            let config: CiConfig = serde_yaml::from_str(&yaml).map_err(|e| e.to_string())?;
            config.validate_and_compile().map_err(|e| e.to_string())?;
            Ok(config)
        }

        const COMPOSE: &str = "  project_dir: .\n  shell: bash\n";
        const IMAGE: &str = "  image: img\n  shell: bash\n";
        const NO_CHECKS: &str = "    checks: {}\n";

        #[test]
        fn test_compose_mode_requires_project_dir() {
            let err = validated("  shell: bash\n", NO_CHECKS).unwrap_err();
            assert!(err.contains("`docker.project_dir` is required"), "{err}");
            assert!(err.contains("docker.image"), "{err}");
        }

        #[test]
        fn test_image_mode_needs_no_project_dir_or_service() {
            let config = validated(IMAGE, NO_CHECKS).unwrap();
            assert_eq!(image(&config).image, "img");
        }

        #[rstest]
        #[case::volume_mount("volume_mount: .:/app")]
        #[case::work_dir("work_dir: /app")]
        #[case::user("user: host")]
        fn test_compose_mode_rejects_image_only_keys(#[case] line: &str) {
            let key = line.split(':').next().unwrap();
            let err = validated(&format!("{COMPOSE}  {line}\n"), NO_CHECKS).unwrap_err();
            assert!(
                err.contains(&format!("`docker.{key}` needs `docker.image`")),
                "{err}"
            );
        }

        #[rstest]
        #[case::project_dir("project_dir: .")]
        #[case::service("service: app")]
        fn test_image_mode_rejects_compose_keys(#[case] line: &str) {
            let key = line.split(':').next().unwrap();
            let err = validated(&format!("{IMAGE}  {line}\n"), NO_CHECKS).unwrap_err();
            assert!(
                err.contains(&format!(
                    "`docker.{key}` is not allowed with `docker.image`"
                )),
                "{err}"
            );
        }

        // `service: ~` counts as set, like the schema's `required` (no
        // `null` → unset shortcut)
        #[test]
        fn test_image_mode_rejects_null_service() {
            let err = validated(&format!("{IMAGE}  service: ~\n"), NO_CHECKS).unwrap_err();
            assert!(err.contains("`docker.service` is not allowed"), "{err}");
        }

        #[rstest]
        #[case::volume_mount("volume_mount: .:/app")]
        #[case::work_dir("work_dir: /app")]
        #[case::user("user: host")]
        fn test_image_mode_allows_image_only_keys(#[case] line: &str) {
            validated(&format!("{IMAGE}  {line}\n"), NO_CHECKS).unwrap();
        }

        #[rstest]
        #[case::check(
            "    checks:\n      lint:\n        name: Lint\n        command: x\n        service: worker\n",
            "`g.lint` sets `service`"
        )]
        #[case::pre_command(
            "    pre_commands:\n      - name: init\n        command: x\n        service: db\n    checks: {}\n",
            "`g.pre_commands[0]` sets `service`"
        )]
        fn test_image_mode_rejects_service(#[case] group: &str, #[case] expected: &str) {
            let err = validated(IMAGE, group).unwrap_err();
            assert!(err.contains(expected), "{err}");
            assert!(err.contains("docker.image"), "{err}");
            validated(COMPOSE, group).expect("compose mode allows `service`");
        }

        // Configs built without YAML (`CiConfig::new`) get the check too
        #[test]
        fn test_validate_rejects_service_in_built_image_config() {
            let group = "    checks:\n      lint:\n        name: Lint\n        command: x\n        service: worker\n";
            let mut config = validated(COMPOSE, group).unwrap();
            config.runner = ExecTarget::Docker(DockerConfig {
                mode: DockerMode::Image(image(&validated(IMAGE, NO_CHECKS).unwrap()).clone()),
                shell: "bash".to_string(),
                env: HashMap::new(),
            });
            let err = config.validate_and_compile().unwrap_err().to_string();
            assert!(err.contains("`g.lint` sets `service`"), "{err}");
        }

        #[rstest]
        #[case::docker("  project_dir: .\n  shell: bash\n  container: c\n", NO_CHECKS)]
        #[case::check(
            COMPOSE,
            "    checks:\n      lint:\n        name: Lint\n        command: x\n        container: c\n"
        )]
        #[case::pre_command(
            COMPOSE,
            "    pre_commands:\n      - name: init\n        command: x\n        container: c\n    checks: {}\n"
        )]
        fn test_container_field_removed(#[case] docker: &str, #[case] group: &str) {
            let err = validated(docker, group).unwrap_err();
            assert!(err.contains("unknown field `container`"), "{err}");
        }

        #[test]
        fn test_pre_command_exec_field_removed() {
            let group = "    pre_commands:\n      - name: init\n        command: x\n        exec: true\n    checks: {}\n";
            let err = validated(COMPOSE, group).unwrap_err();
            assert!(err.contains("unknown field `exec`"), "{err}");
        }

        // Edge case: Tests Docker volume mount configuration - requires raw YAML to verify volume_mount field parsing and formatting
        #[test]
        fn test_volume_args() {
            let yaml = r#"
version: 2
docker:
  image: img
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
                image(&config).volume_args(),
                Some("-v .:/build".to_string())
            );
        }

        // Edge case: Tests default working directory when work_dir not specified - requires raw YAML to verify /app default
        #[test]
        fn test_working_dir_default() {
            let yaml = r#"
version: 2
docker:
  image: img
  shell: bash
git:
  base_branch: main
  fallback_branch: HEAD~1
file_patterns: {}
checks: {}
"#;
            let config: CiConfig = serde_yaml::from_str(yaml).unwrap();
            assert_eq!(image(&config).working_dir(), "/app");
        }

        // Edge case: Tests custom working directory configuration - requires raw YAML to verify work_dir field parsing
        #[test]
        fn test_working_dir_custom() {
            let yaml = r#"
version: 2
docker:
  image: img
  shell: bash
  work_dir: "/custom/path"
git:
  base_branch: main
  fallback_branch: HEAD~1
file_patterns: {}
checks: {}
"#;
            let config: CiConfig = serde_yaml::from_str(yaml).unwrap();
            assert_eq!(image(&config).working_dir(), "/custom/path");
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
  image: img
  shell: bash
git:
  base_branch: main
  fallback_branch: HEAD~1
file_patterns: {}
checks: {}
"#;
            let config: CiConfig = serde_yaml::from_str(yaml).unwrap();
            assert_eq!(image(&config).volume_args(), None);
        }

        // volume_args() expands env vars when present
        #[test]
        fn test_volume_args_expands_env_vars() {
            std::env::set_var("CI_TUI_TEST_VOL", "/tmp/ci-tui-vol");
            let yaml = r#"
version: 2
docker:
  image: img
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
                image(&config).volume_args(),
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
            let msg = serde_yaml::from_str::<CiConfig>(&yaml)
                .unwrap_err()
                .to_string();
            assert!(msg.contains("unsupported config version"), "got: {msg}");
        }

        // Version is checked before the docker section: its mode errors are
        // likely a schema mismatch, not the real cause
        #[test]
        fn test_version_error_before_docker_mode_error() {
            let yaml = "version: 1\ndocker:\n  image: img\n  service: app\n  shell: bash\ngit:\n  base_branch: main\n  fallback_branch: HEAD~1\nfile_patterns: {}\nchecks: {}\n";
            let msg = serde_yaml::from_str::<CiConfig>(yaml)
                .unwrap_err()
                .to_string();
            assert!(msg.contains("unsupported config version 1"), "got: {msg}");
        }

        // `CiConfig::new` (no YAML) still gets the version check
        #[test]
        fn test_validate_checks_version_of_built_config() {
            let mut config = ConfigBuilder::new().build();
            config.version = 1;
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

        // Edge case: the same check id in two groups is rejected at load
        #[test]
        fn test_validate_rejects_duplicate_check_id() {
            let yaml = "version: 2\ndocker:\n  project_dir: .\n  shell: bash\ngit:\n  base_branch: main\n  fallback_branch: HEAD~1\nfile_patterns: {}\nchecks:\n  php:\n    checks:\n      lint: {name: A, command: 'true'}\n  js:\n    checks:\n      lint: {name: B, command: 'true'}\n";
            let config: CiConfig = serde_yaml::from_str(yaml).unwrap();
            let msg = config.validate_and_compile().unwrap_err().to_string();
            assert!(
                msg.contains("`lint`") && msg.contains("`php` and `js`"),
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
            assert_eq!(compose(&config).project_dir, ".");
            assert_eq!(compose(&config).service, "app"); // Default value
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

        // Edge case: Tests a check's explicit service is parsed - requires raw YAML to verify service field override
        #[test]
        fn test_check_with_service() {
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
            assert_eq!(check.service.as_deref(), Some("custom"));
        }

        // Edge case: Tests a check without service stays unset (the compose default is applied by resolve_service) - requires raw YAML
        #[test]
        fn test_check_without_service() {
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
            assert_eq!(check.service, None);
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
                DockerConfig::compose_for_test(".", "app"),
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
            assert!(config.docker().is_none());
        }

        // Ignored `docker` section still follows its mode rules
        #[rstest]
        #[case::valid("docker:\n  image: img\n  shell: bash\n", None)]
        #[case::no_project_dir(
            "docker:\n  shell: bash\n",
            Some("`docker.project_dir` is required")
        )]
        #[case::wrong_mode_key(
            "docker:\n  image: img\n  service: app\n  shell: bash\n",
            Some("`docker.service` is not allowed with `docker.image`")
        )]
        fn test_runner_local_validates_docker_section(
            #[case] docker: &str,
            #[case] expected: Option<&str>,
        ) {
            let yaml = LOCAL_MIN.replace("runner: local\n", &format!("runner: local\n{docker}"));
            let result = serde_yaml::from_str::<CiConfig>(&yaml);
            match expected {
                None => assert!(result.unwrap().docker().is_none()),
                Some(msg) => {
                    let err = result.unwrap_err().to_string();
                    assert!(err.contains(msg), "got: {err}");
                }
            }
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
        fn test_local_mode_rejects_pre_command_service() {
            let yaml = LOCAL_MIN.replace(
                "checks: {}\n",
                "checks:\n  g:\n    pre_commands:\n      - name: init\n        command: echo\n        service: db\n    checks: {}\n",
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

    mod test_error_pattern {
        use super::*;
        use std::io::Write;

        /// Local-mode config with one check `c` in group `g`; `pattern` is an
        /// optional raw YAML `error_pattern:` scalar.
        fn yaml(pattern: Option<&str>) -> String {
            let extra = pattern
                .map(|p| format!("        error_pattern: {p}\n"))
                .unwrap_or_default();
            format!(
                "version: 2\nrunner: local\ngit:\n  base_branch: main\n  fallback_branch: HEAD~1\nfile_patterns: {{}}\nchecks:\n  g:\n    checks:\n      c:\n        name: C\n        command: 'true'\n{extra}"
            )
        }

        fn check(yaml: &str) -> CheckDefinition {
            let config: CiConfig = serde_yaml::from_str(yaml).unwrap();
            config.get_group("g").unwrap().checks["c"].clone()
        }

        #[rstest]
        #[case("error: mismatched types")]
        #[case("Error: cannot open file")]
        #[case("ERROR in ./src/app.ts")]
        #[case("FAIL src/app.test.ts")]
        #[case("test foo ... FAILED")]
        #[case("thread 'main' panicked at src/main.rs:3:5")]
        #[case("PHP Fatal: Uncaught Exception in Foo.php")]
        #[case("Traceback (most recent call last):")]
        #[case("RuntimeException: boom")]
        #[case("error[E0308]: mismatched types")]
        #[case("PHP Fatal error:  Uncaught Error: Call to undefined function")]
        #[case("Unhandled Exception")]
        fn default_pattern_matches_error_lines(#[case] line: &str) {
            assert!(default_error_regex().is_match(line), "{line}");
        }

        #[rstest]
        #[case("test result: ok. 12 passed")]
        #[case("Compiling ci-tui v0.4.0")]
        #[case("error_reporting(E_ALL);")]
        #[case("── stderr ──")]
        #[case("terrorism")]
        #[case("failures: none")]
        #[case("M src/Error.php")]
        #[case("Found 0 error(s)")]
        #[case("src/FooException.php")]
        fn default_pattern_skips_normal_lines(#[case] line: &str) {
            assert!(!default_error_regex().is_match(line), "{line}");
        }

        #[test]
        fn omitted_uses_default() {
            let c = check(&yaml(None));
            assert!(c.error_pattern.is_none());
            assert_eq!(c.error_regex().as_str(), default_error_regex().as_str());
        }

        #[test]
        fn custom_pattern_overrides_default() {
            let c = check(&yaml(Some("'^E\\d+'")));
            assert!(c.error_regex().is_match("E0308 mismatched"));
            assert!(!c.error_regex().is_match("error: mismatched"));
        }

        #[test]
        fn invalid_pattern_rejected_at_load() {
            let mut tmp = tempfile::NamedTempFile::new().unwrap();
            tmp.write_all(yaml(Some("'[invalid'")).as_bytes()).unwrap();
            let msg = format!("{:#}", load_config(tmp.path()).unwrap_err());
            assert!(msg.contains("checks.g.checks.c"), "names the check: {msg}");
            assert!(
                msg.contains("`error_pattern`") && msg.contains("[invalid"),
                "names field + pattern: {msg}"
            );
        }
    }

    mod test_files_filter {
        use super::*;
        use std::io::Write;

        /// Local-mode config with one `src`-triggered check `c` in group `g`;
        /// `filter` is an optional raw YAML `files_filter:` scalar.
        fn yaml(filter: Option<&str>) -> String {
            let extra = filter
                .map(|f| format!("          files_filter: {f}\n"))
                .unwrap_or_default();
            format!(
                "version: 2\nrunner: local\ngit:\n  base_branch: main\n  fallback_branch: HEAD~1\nfile_patterns:\n  src: {{ pattern: 'x' }}\nchecks:\n  g:\n    checks:\n      c:\n        name: C\n        command: 'true'\n        triggers:\n          file_pattern: src\n{extra}"
            )
        }

        fn triggers(yaml: &str) -> CheckTriggers {
            let config: CiConfig = serde_yaml::from_str(yaml).unwrap();
            let check = &config.get_group("g").unwrap().checks["c"];
            check.triggers.clone().unwrap()
        }

        #[test]
        fn omitted_is_none() {
            assert!(triggers(&yaml(None)).files_filter.is_none());
        }

        #[test]
        fn inline_regex_is_compiled() {
            let filter = triggers(&yaml(Some("'Test\\.php$'"))).files_filter.unwrap();
            assert!(filter.is_match("tests/FooTest.php"));
            assert!(!filter.is_match("tests/Fixtures/Foo/valid_case.php"));
        }

        #[test]
        fn invalid_regex_rejected_at_load() {
            let mut tmp = tempfile::NamedTempFile::new().unwrap();
            tmp.write_all(yaml(Some("'[invalid'")).as_bytes()).unwrap();
            let msg = format!("{:#}", load_config(tmp.path()).unwrap_err());
            assert!(msg.contains("checks.g.checks.c"), "names the check: {msg}");
            assert!(
                msg.contains("`files_filter`") && msg.contains("[invalid"),
                "names field + pattern: {msg}"
            );
        }

        #[test]
        fn without_file_pattern_or_test_discovery_rejected_at_load() {
            let yaml = yaml(Some("'Test'")).replace("          file_pattern: src\n", "");
            let mut tmp = tempfile::NamedTempFile::new().unwrap();
            tmp.write_all(yaml.as_bytes()).unwrap();
            let msg = format!("{:#}", load_config(tmp.path()).unwrap_err());
            assert!(
                msg.contains("checks.g.checks.c.triggers.files_filter: needs"),
                "got: {msg}"
            );
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

    mod test_notify {
        use super::*;

        const YAML: &str = "version: 2\nrunner: local\ngit:\n  base_branch: main\n  fallback_branch: HEAD~1\nfile_patterns: {}\nchecks: {}\n";

        #[test]
        fn off_when_omitted() {
            let config: CiConfig = serde_yaml::from_str(YAML).unwrap();
            assert!(!config.notify);
        }

        #[test]
        fn on_when_set() {
            let config: CiConfig = serde_yaml::from_str(&format!("{YAML}notify: true\n")).unwrap();
            assert!(config.notify);
        }

        #[test]
        fn not_in_cache_text() {
            let with_notify = format!("{YAML}notify: true\n");
            assert_eq!(cache_text(&with_notify), cache_text(YAML));
            let jobs = format!("{YAML}max_parallel: 2\n");
            assert_ne!(cache_text(&jobs), cache_text(YAML));
        }
    }

    mod test_max_parallel {
        use super::*;
        use std::io::Write;
        use std::num::NonZeroUsize;

        /// Local-mode config with parallel group `g`; optional top-level and
        /// group `max_parallel` lines.
        fn yaml(global: Option<&str>, group: Option<&str>) -> String {
            let global = global
                .map(|n| format!("max_parallel: {n}\n"))
                .unwrap_or_default();
            let group = group
                .map(|n| format!("    max_parallel: {n}\n"))
                .unwrap_or_default();
            format!(
                "version: 2\nrunner: local\n{global}git:\n  base_branch: main\n  fallback_branch: HEAD~1\nfile_patterns: {{}}\nchecks:\n  g:\n    parallel: true\n{group}    checks:\n      c:\n        name: C\n        command: 'true'\n"
            )
        }

        fn nz(n: usize) -> Option<NonZeroUsize> {
            NonZeroUsize::new(n)
        }

        #[test]
        fn parses_global_and_group() {
            let config: CiConfig = serde_yaml::from_str(&yaml(Some("4"), Some("2"))).unwrap();
            assert_eq!(config.max_parallel, nz(4));
            assert_eq!(config.get_group("g").unwrap().max_parallel, nz(2));
        }

        #[test]
        fn omitted_is_none() {
            let config: CiConfig = serde_yaml::from_str(&yaml(None, None)).unwrap();
            assert_eq!(config.max_parallel, None);
            assert_eq!(config.get_group("g").unwrap().max_parallel, None);
        }

        #[rstest]
        #[case::global_zero(Some("0"), None)]
        #[case::group_zero(None, Some("0"))]
        fn invalid_is_config_error(#[case] global: Option<&str>, #[case] group: Option<&str>) {
            let mut tmp = tempfile::NamedTempFile::new().unwrap();
            tmp.write_all(yaml(global, group).as_bytes()).unwrap();
            let err = load_config(tmp.path()).unwrap_err();
            assert_eq!(crate::exit::code_for(&err), crate::exit::CONFIG_ERROR);
            assert!(format!("{err:#}").contains("max_parallel"), "{err:#}");
        }

        #[rstest]
        #[case::global_only(nz(3), None, 3)]
        #[case::group_below_global(nz(4), nz(2), 2)]
        #[case::group_above_global(nz(2), nz(8), 2)]
        #[case::group_equal_global(nz(2), nz(2), 2)]
        fn resolves_limit(
            #[case] global: Option<NonZeroUsize>,
            #[case] group: Option<NonZeroUsize>,
            #[case] expected: usize,
        ) {
            assert_eq!(parallel_limit(global, group).get(), expected);
        }

        #[rstest]
        #[case::global(nz(usize::MAX), None)]
        #[case::group(None, nz(usize::MAX))]
        #[case::both(nz(usize::MAX), nz(usize::MAX))]
        fn huge_limit_clamped_to_semaphore_max(
            #[case] global: Option<NonZeroUsize>,
            #[case] group: Option<NonZeroUsize>,
        ) {
            let max = tokio::sync::Semaphore::MAX_PERMITS;
            let limit = parallel_limit(global, group).get();
            assert!(limit <= max, "{limit} > {max}");
            if global.is_some() {
                assert_eq!(limit, max);
            }
        }

        #[test]
        fn defaults_to_cpu_count() {
            let cpus = std::thread::available_parallelism().unwrap().get();
            assert_eq!(parallel_limit(None, None).get(), cpus);
            assert_eq!(parallel_limit(None, nz(cpus + 5)).get(), cpus);
        }

        #[test]
        fn group_limit_applies_under_global() {
            let config: CiConfig = serde_yaml::from_str(&yaml(Some("4"), Some("2"))).unwrap();
            assert_eq!(config.group_parallel_limit(config.get_group("g")), 2);
            assert_eq!(config.group_parallel_limit(None), 4);
        }
    }

    mod test_local_overrides {
        use super::*;

        const HEAD: &str = "version: 2\ngit:\n  base_branch: main\n  fallback_branch: HEAD~1\nfile_patterns: {}\nchecks: {}\n";

        /// Compose-mode base config (service `app`, env `A`, `B`)
        fn compose() -> String {
            format!("{HEAD}docker:\n  project_dir: .\n  shell: sh\n  env: {{A: base, B: base}}\n")
        }

        /// Local-mode base config (env `A`, `B`)
        fn local_runner() -> String {
            format!("{HEAD}runner: local\nlocal:\n  env: {{A: base, B: base}}\n")
        }

        /// Dir with `ci-tui.yaml` = `base` and, if given, `ci-tui.local.yaml` = `local`
        fn project(base: &str, local: Option<&str>) -> tempfile::TempDir {
            let dir = tempfile::tempdir().unwrap();
            std::fs::write(dir.path().join("ci-tui.yaml"), base).unwrap();
            if let Some(local) = local {
                std::fs::write(dir.path().join(LOCAL_CONFIG_FILE), local).unwrap();
            }
            dir
        }

        fn load(base: &str, local: Option<&str>) -> Result<CiConfig> {
            load_config(&project(base, local).path().join("ci-tui.yaml"))
        }

        fn env(map: &HashMap<String, String>) -> Vec<(&str, &str)> {
            let mut env: Vec<_> = map.iter().map(|(k, v)| (k.as_str(), v.as_str())).collect();
            env.sort();
            env
        }

        #[test]
        fn merges_docker_service_env_and_settings() {
            let local = "docker:\n  service: mine\n  env: {B: local, C: local}\nmax_parallel: 2\nnotify: true\n";
            let config = load(&compose(), Some(local)).unwrap();
            let docker = config.docker().unwrap();
            let DockerMode::Compose(compose) = &docker.mode else {
                panic!("compose mode expected");
            };
            assert_eq!(compose.service, "mine");
            assert_eq!(
                env(&docker.env),
                [("A", "base"), ("B", "local"), ("C", "local")]
            );
            assert_eq!(config.max_parallel, NonZeroUsize::new(2));
            assert!(config.notify);
        }

        #[test]
        fn merges_local_env() {
            let config = load(&local_runner(), Some("local:\n  env: {B: local}\n")).unwrap();
            let ExecTarget::Local(local) = &config.runner else {
                panic!("local runner expected");
            };
            assert_eq!(env(&local.env), [("A", "base"), ("B", "local")]);
            assert_eq!(local.shell, "bash", "base keys kept");
        }

        #[rstest]
        #[case::missing(None)]
        #[case::empty(Some(""))]
        #[case::comments_only(Some("# nothing yet\n"))]
        fn absent_or_empty_changes_nothing(#[case] local: Option<&str>) {
            let config = load(&compose(), local).unwrap();
            let base = load(&compose(), None).unwrap();
            assert_eq!(config.source_hash, base.source_hash);
            assert_eq!(config.max_parallel, None);
        }

        #[test]
        fn found_next_to_config_flag_path() {
            // `--config other/x.yaml`: the local file sits next to it, not in cwd
            let dir = project("", None);
            std::fs::create_dir(dir.path().join("other")).unwrap();
            std::fs::write(dir.path().join("other/x.yaml"), compose()).unwrap();
            std::fs::write(
                dir.path().join("other").join(LOCAL_CONFIG_FILE),
                "notify: true\n",
            )
            .unwrap();
            assert!(
                load_config(&dir.path().join("other/x.yaml"))
                    .unwrap()
                    .notify
            );
        }

        #[rstest]
        #[case::checks("checks: {}\n", "checks")]
        #[case::git("git:\n  base_branch: dev\n", "git")]
        #[case::runner("runner: local\n", "runner")]
        #[case::docker_shell("docker:\n  shell: bash\n", "shell")]
        #[case::docker_project_dir("docker:\n  project_dir: /x\n", "project_dir")]
        #[case::docker_image("docker:\n  image: img\n", "image")]
        #[case::local_shell("local:\n  shell: zsh\n", "shell")]
        #[case::wrong_type_max_parallel("max_parallel: lots\n", "max_parallel")]
        #[case::zero_max_parallel("max_parallel: 0\n", "max_parallel")]
        #[case::wrong_type_notify("notify: maybe\n", "notify")]
        #[case::wrong_type_env("docker:\n  env: [A]\n", "env")]
        #[case::invalid_yaml("docker: [unclosed\n", "")]
        #[case::not_a_map("- notify\n", "")]
        fn rejected_naming_local_file(#[case] local: &str, #[case] key: &str) {
            let err = load(&compose(), Some(local)).unwrap_err();
            assert_eq!(crate::exit::code_for(&err), crate::exit::CONFIG_ERROR);
            let msg = format!("{err:#}");
            assert!(
                msg.contains(LOCAL_CONFIG_FILE),
                "names the local file: {msg}"
            );
            assert!(msg.contains(key), "names `{key}`: {msg}");
        }

        #[test]
        fn wrong_mode_service_rejected_by_mode_rules() {
            let base = format!("{HEAD}docker:\n  image: img\n  shell: sh\n");
            let err = load(&base, Some("docker:\n  service: app\n")).unwrap_err();
            assert_eq!(crate::exit::code_for(&err), crate::exit::CONFIG_ERROR);
            let msg = format!("{err:#}");
            assert!(
                msg.contains("`docker.service` is not allowed with `docker.image`"),
                "{msg}"
            );
            assert!(msg.contains(LOCAL_CONFIG_FILE), "{msg}");
        }

        #[rstest]
        #[case::service("docker:\n  service: mine\n", false)]
        #[case::env("docker:\n  env: {A: local}\n", false)]
        #[case::same_env("docker:\n  env: {A: base}\n", true)]
        #[case::max_parallel("max_parallel: 2\n", false)]
        #[case::ui_only_notify("notify: true\n", true)]
        fn cache_hash_covers_overrides(#[case] local: &str, #[case] same: bool) {
            let base = load(&compose(), None).unwrap().source_hash;
            let merged = load(&compose(), Some(local)).unwrap().source_hash;
            assert_eq!(merged == base, same, "{local}");
        }
    }
}
