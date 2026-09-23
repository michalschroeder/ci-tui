//! Check execution (Docker or local host) with event streaming.
//!
//! This module handles the actual execution of CI checks. [`ExecTarget`] picks
//! the mode: Docker (`docker exec` for running containers, `docker run` for
//! standalone execution) or local host (`shell -c`). Output is captured when a command completes and delivered as
//! lifecycle events ([`RunnerEvent`]) through async channels.
//!
//! # Key Types
//!
//! - [`CheckRunner`]: Orchestrates check execution with parallel/sequential support
//! - [`CheckResult`]: Result of executing a check including output and timing
//! - [`CheckStatus`]: Current status of a check (Pending, Running, Passed, etc.)
//! - [`RunnerEvent`]: Events emitted during execution for UI updates
//!
//! # Key Functions
//!
//! - [`run_single_check`]: Execute a single check (for retry operations)
//! - [`run_fix_command`]: Execute a fix command for a check

use crate::checks::CheckToRun;
use crate::config::CiConfig;
use anyhow::Result;
use async_trait::async_trait;
use chrono::{DateTime, Local};
use std::collections::HashMap;
use std::path::Path;
use std::sync::Arc;
use tokio::sync::mpsc;

/// Output from executing a command
#[derive(Debug, Clone)]
pub struct CommandOutput {
    pub success: bool,
    pub stdout: String,
    pub stderr: String,
}

/// Trait for executing commands, allowing mock implementations in tests
#[cfg_attr(any(test, feature = "test"), mockall::automock)]
#[async_trait]
pub trait CommandExecutor: Send + Sync {
    /// Execute a shell command and return the output
    async fn execute(&self, command: &str, working_dir: &Path) -> CommandOutput;

    /// Check if a Docker container is running
    fn is_container_running(&self, container_name: &str) -> bool;
}

/// Production implementation of CommandExecutor
pub struct RealCommandExecutor;

#[async_trait]
impl CommandExecutor for RealCommandExecutor {
    async fn execute(&self, command: &str, working_dir: &Path) -> CommandOutput {
        let output = tokio::process::Command::new("sh")
            .arg("-c")
            .arg(command)
            .current_dir(working_dir)
            // Aborted tasks (e.g. on retry-all) must not leave docker running
            .kill_on_drop(true)
            .output()
            .await;

        match output {
            Ok(output) => CommandOutput {
                success: output.status.success(),
                stdout: String::from_utf8_lossy(&output.stdout).to_string(),
                stderr: String::from_utf8_lossy(&output.stderr).to_string(),
            },
            Err(e) => CommandOutput {
                success: false,
                stdout: String::new(),
                stderr: format!("Failed to execute: {}", e),
            },
        }
    }

    fn is_container_running(&self, container_name: &str) -> bool {
        crate::utils::docker::is_running(container_name)
    }
}

/// Filter out Docker Compose warning messages from stderr
pub fn filter_docker_warnings(stderr: &str) -> String {
    stderr
        .lines()
        .filter(|line| {
            // Filter out common Docker Compose warnings that are noise
            !line.contains("variable is not set. Defaulting to a blank string")
        })
        .collect::<Vec<_>>()
        .join("\n")
}

/// True if `key` is a valid environment variable identifier: `[A-Za-z_][A-Za-z0-9_]*`.
fn is_valid_env_key(key: &str) -> bool {
    let mut chars = key.chars();
    matches!(chars.next(), Some(c) if c.is_ascii_alphabetic() || c == '_')
        && chars.all(|c| c.is_ascii_alphanumeric() || c == '_')
}

/// Build a docker exec command with environment variables.
///
/// Values are single-quoted (with `'` escaped) to survive the outer shell.
/// Keys cannot be quoted in `-e KEY=...`, so keys that are not valid env
/// identifiers are skipped entirely to prevent shell injection.
pub fn build_docker_exec_command(
    container_name: &str,
    env: &HashMap<String, String>,
    command: &str,
    shell: &str,
) -> String {
    let env_flags: String = env
        .iter()
        .filter(|(k, _)| is_valid_env_key(k))
        .map(|(k, v)| format!("-e {}='{}' ", k, v.replace('\'', "'\\''")))
        .collect();

    // Single format path: env_flags is either empty or ends with a trailing space.
    format!(
        "docker exec {}{} {} -c '{}'",
        env_flags,
        container_name,
        shell,
        command.replace('\'', "'\\''")
    )
}

/// Build a docker run command with environment variables
pub fn build_docker_run_command(
    docker_config: &crate::config::DockerConfig,
    env: &HashMap<String, String>,
    command: &str,
) -> String {
    // Build env flags for docker run (-e KEY='VALUE' for each)
    let env_flags: String = env
        .iter()
        .map(|(k, v)| format!("-e {}='{}'", k, v.replace('\'', "'\\''")))
        .collect::<Vec<_>>()
        .join(" ");

    // Get image name from config (explicit or derived from container name)
    let image_name = docker_config.image_name();

    // Get working directory from config (default: /app)
    let work_dir = docker_config.working_dir();

    // Get shell from config (default: bash)
    let shell = &docker_config.shell;

    // Get volume mount args if configured
    let volume_args = docker_config.volume_args().unwrap_or_default();

    // Build docker run command with --rm flag
    let mut parts = vec!["docker run --rm".to_string()];

    if !volume_args.is_empty() {
        parts.push(volume_args);
    }

    if !env_flags.is_empty() {
        parts.push(env_flags);
    }

    parts.push(format!("-w {}", work_dir));
    parts.push(image_name);
    parts.push(format!("{} -c '{}'", shell, command.replace('\'', "'\\''")));

    parts.join(" ")
}

/// Where check commands execute, derived from the config's runner mode.
#[derive(Debug, Clone)]
pub enum ExecTarget {
    Docker(crate::config::DockerConfig),
    Local(crate::config::LocalConfig),
}

impl ExecTarget {
    /// Target for `config.runner`. Docker presence is validated at load.
    pub fn from_config(config: &CiConfig) -> Self {
        match config.runner {
            crate::config::RunnerMode::Docker => Self::Docker(config.docker().clone()),
            crate::config::RunnerMode::Local => Self::Local(config.local.clone()),
        }
    }

    /// Global env: `docker.env` or `local.env`.
    pub fn env(&self) -> &HashMap<String, String> {
        match self {
            Self::Docker(docker) => &docker.env,
            Self::Local(local) => &local.env,
        }
    }

    /// Default container for docker mode; empty in local mode (never used there).
    pub fn default_container(&self) -> String {
        match self {
            Self::Docker(docker) => docker.container_name(),
            Self::Local(_) => String::new(),
        }
    }

    /// Full shell command for `command`. Queries container state ONLY in docker mode.
    pub fn build_command(
        &self,
        container_name: &str,
        env: &HashMap<String, String>,
        command: &str,
        executor: &dyn CommandExecutor,
    ) -> String {
        match self {
            Self::Local(local) => build_local_command(local, env, command),
            Self::Docker(docker) if executor.is_container_running(container_name) => {
                build_docker_exec_command(container_name, env, command, &docker.shell)
            }
            Self::Docker(docker) => build_docker_run_command(docker, env, command),
        }
    }
}

/// Status of a CI check during execution
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum CheckStatus {
    /// Check has not started yet
    Pending,
    /// Check is currently executing
    Running,
    /// Check completed successfully (exit code 0)
    Passed,
    /// Check failed (non-zero exit code)
    Failed,
    /// Check was skipped (no matching files)
    Skipped,
    /// Check is available but requires manual trigger (press 't')
    OnDemand,
}

/// Result of executing a CI check, including output and timing information
#[derive(Debug, Clone)]
pub struct CheckResult {
    /// Unique identifier for this check
    pub check_id: String,
    /// Current status of the check
    pub status: CheckStatus,
    /// Standard output from the check command
    pub output: String,
    /// Standard error from the check command (filtered for Docker noise)
    pub error_output: String,
    /// Execution time in milliseconds
    pub duration_ms: u64,
    /// When the check started executing
    pub started_at: Option<DateTime<Local>>,
    /// When the check finished executing
    pub finished_at: Option<DateTime<Local>>,
}

impl CheckResult {
    /// Create a pending check result (not yet started)
    pub fn pending(check_id: &str) -> Self {
        Self {
            check_id: check_id.to_string(),
            status: CheckStatus::Pending,
            output: String::new(),
            error_output: String::new(),
            duration_ms: 0,
            started_at: None,
            finished_at: None,
        }
    }

    /// Create a skipped check result (no matching files)
    pub fn skipped(check_id: &str) -> Self {
        Self {
            check_id: check_id.to_string(),
            status: CheckStatus::Skipped,
            output: "No changes detected".to_string(),
            error_output: String::new(),
            duration_ms: 0,
            started_at: None,
            finished_at: None,
        }
    }

    /// Create an on-demand check result (requires manual trigger)
    pub fn on_demand(check_id: &str) -> Self {
        Self {
            check_id: check_id.to_string(),
            status: CheckStatus::OnDemand,
            output: "Press 't' to run this test".to_string(),
            error_output: String::new(),
            duration_ms: 0,
            started_at: None,
            finished_at: None,
        }
    }
}

/// Events emitted by the check runner during execution
#[derive(Debug, Clone)]
pub enum RunnerEvent {
    /// A check has started executing
    CheckStarted { check_id: String },
    /// A check has finished executing
    CheckFinished { result: CheckResult },
    /// A group of checks has started
    GroupStarted { group: String },
    /// A group of checks has finished
    GroupFinished { group: String },
    /// A pre-command has started
    PreCommandStarted { group: String, name: String },
    /// A pre-command has finished
    PreCommandFinished {
        group: String,
        name: String,
        success: bool,
        output: String,
        duration_ms: u64,
    },
    /// All checks have completed
    AllFinished,
}

/// Executes CI checks in Docker containers
///
/// Manages check execution with support for:
/// - Sequential execution within groups
/// - Parallel execution across groups (when configured)
/// - Event streaming for UI updates
pub struct CheckRunner {
    config: Arc<CiConfig>,
    target: ExecTarget,
    project_root: Arc<Path>,
    container_name: Arc<str>,
    executor: Arc<dyn CommandExecutor>,
}

impl CheckRunner {
    /// Create a new check runner with the given configuration
    pub fn new(config: CiConfig, project_root: &Path) -> Self {
        Self::with_executor(config, project_root, Arc::new(RealCommandExecutor))
    }

    /// Create a new check runner with a custom executor (for testing)
    pub fn with_executor(
        config: CiConfig,
        project_root: &Path,
        executor: Arc<dyn CommandExecutor>,
    ) -> Self {
        let target = ExecTarget::from_config(&config);
        let container_name: Arc<str> = target.default_container().into();
        Self {
            config: Arc::new(config),
            target,
            project_root: Arc::from(project_root),
            container_name,
            executor,
        }
    }

    /// Run all checks, grouped by execution group, sending events via the channel
    ///
    /// Note: Channel send errors are silently ignored because they indicate the
    /// receiver (UI) has been dropped, which is expected during shutdown.
    pub async fn run_checks(
        &self,
        checks: Vec<CheckToRun>,
        event_tx: mpsc::Sender<RunnerEvent>,
    ) -> Result<()> {
        use crate::checks::group_checks;
        let grouped = group_checks(&checks);
        run_check_groups(self, grouped, &event_tx).await
    }

    /// Execute a single group: run pre-commands, then checks (parallel or sequential)
    /// Returns Ok(true) to continue, Ok(false) to stop execution
    async fn execute_group(
        &self,
        group_name: &str,
        group_checks: Vec<&CheckToRun>,
        event_tx: &mpsc::Sender<RunnerEvent>,
    ) -> Result<bool> {
        let group_config = self.config.get_group(group_name);

        let has_runnable = group_checks.iter().any(|c| !c.is_on_demand());
        let pre_commands = if has_runnable {
            group_config
                .map(|g| g.pre_commands.as_slice())
                .unwrap_or(&[])
        } else {
            &[]
        };

        if !run_all_pre_commands(self, group_name, pre_commands, event_tx).await {
            return Ok(false);
        }

        let parallel = group_config.map(|g| g.parallel).unwrap_or(false);
        if parallel {
            self.run_parallel(group_checks, event_tx).await?;
        } else {
            self.run_sequential(group_checks, event_tx).await?;
        }

        Ok(true)
    }

    async fn run_sequential(
        &self,
        checks: Vec<&CheckToRun>,
        event_tx: &mpsc::Sender<RunnerEvent>,
    ) -> Result<()> {
        let runnable: Vec<_> = checks.into_iter().filter(|c| !c.is_on_demand()).collect();
        for check in runnable {
            let result = self.run_single_check(check, event_tx).await;
            let _ = event_tx.send(RunnerEvent::CheckFinished { result }).await;
        }
        Ok(())
    }

    async fn run_parallel(
        &self,
        checks: Vec<&CheckToRun>,
        event_tx: &mpsc::Sender<RunnerEvent>,
    ) -> Result<()> {
        let runnable: Vec<_> = checks.into_iter().filter(|c| !c.is_on_demand()).collect();
        let mut handles = Vec::new();
        for check in runnable {
            handles.push(self.spawn_check(check, event_tx));
        }
        for handle in handles {
            let _ = handle.await;
        }
        Ok(())
    }

    fn spawn_check(
        &self,
        check: &CheckToRun,
        event_tx: &mpsc::Sender<RunnerEvent>,
    ) -> tokio::task::JoinHandle<()> {
        let check = check.clone();
        let event_tx = event_tx.clone();
        let project_root = self.project_root.clone();
        let container_name: Arc<str> = check
            .definition
            .container
            .clone()
            .unwrap_or_else(|| self.container_name.to_string())
            .into();
        let target = self.target.clone();
        let executor = self.executor.clone();

        tokio::spawn(async move {
            let result = run_check_with_target(
                &check,
                &project_root,
                &container_name,
                &target,
                &event_tx,
                executor.as_ref(),
            )
            .await;
            let _ = event_tx.send(RunnerEvent::CheckFinished { result }).await;
        })
    }

    async fn run_single_check(
        &self,
        check: &CheckToRun,
        event_tx: &mpsc::Sender<RunnerEvent>,
    ) -> CheckResult {
        // Use per-check container if specified, otherwise use default
        let container_name = check
            .definition
            .container
            .as_deref()
            .unwrap_or(&self.container_name);
        run_check_with_target(
            check,
            &self.project_root,
            container_name,
            &self.target,
            event_tx,
            self.executor.as_ref(),
        )
        .await
    }

    /// Run a pre-command for a group (e.g., DB initialization)
    /// Returns (success, output, duration_ms)
    async fn run_pre_command(&self, pre_cmd: &crate::config::PreCommand) -> (bool, String, u64) {
        let start = std::time::Instant::now();

        let cmd = if pre_cmd.host {
            pre_cmd.command.clone()
        } else {
            self.build_pre_command_cmd(pre_cmd)
        };

        let output = self.executor.execute(&cmd, &self.project_root).await;

        let duration_ms = start.elapsed().as_millis() as u64;

        let stderr = filter_docker_warnings(&output.stderr);
        let combined = if stderr.is_empty() {
            output.stdout.clone()
        } else {
            format!("{}\n{}", output.stdout, stderr)
        };
        (output.success, combined, duration_ms)
    }

    /// Resolve the container for a check/pre-command.
    ///
    /// Precedence: explicit `container:` > compose-convention name for a
    /// non-default service > cached default container.
    ///
    /// LIMITATION: assumes the Docker Compose v2 naming convention
    /// `{project}-{service}-1`. `COMPOSE_PROJECT_NAME` is honored; a `name:`
    /// override inside the compose file is not. Non-UTF8 project paths fall
    /// back to the literal project name "project". Future work: resolve via
    /// `docker compose ps -q <service>` instead of string construction.
    fn resolve_container_name(&self, explicit: Option<&str>, service: &str) -> String {
        if let Some(container) = explicit {
            container.to_string()
        } else if service != self.config.default_service() {
            format!(
                "{}-{}-1",
                self.config.docker().compose_project_name(),
                service
            )
        } else {
            self.container_name.to_string()
        }
    }

    /// Shell command for a non-host pre-command. Local mode ignores
    /// `service`/`container` (rejected at load) and runs on the host.
    fn build_pre_command_cmd(&self, pre_cmd: &crate::config::PreCommand) -> String {
        let mut env = self.target.env().clone();
        env.extend(pre_cmd.env.clone());

        if let ExecTarget::Local(local) = &self.target {
            return build_local_command(local, &env, &pre_cmd.command);
        }

        let service = pre_cmd
            .service
            .as_deref()
            .unwrap_or_else(|| self.config.default_service());
        let container_name = self.resolve_container_name(pre_cmd.container.as_deref(), service);
        self.target.build_command(
            &container_name,
            &env,
            &pre_cmd.command,
            self.executor.as_ref(),
        )
    }
}

/// Run check groups sequentially, sending events via the channel
async fn run_check_groups<'a>(
    runner: &CheckRunner,
    grouped: Vec<(&'a str, Vec<&'a CheckToRun>)>,
    event_tx: &mpsc::Sender<RunnerEvent>,
) -> Result<()> {
    for (group_name, group_checks) in grouped {
        let _ = event_tx
            .send(RunnerEvent::GroupStarted {
                group: group_name.to_string(),
            })
            .await;

        let should_continue = runner
            .execute_group(group_name, group_checks, event_tx)
            .await?;

        let _ = event_tx
            .send(RunnerEvent::GroupFinished {
                group: group_name.to_string(),
            })
            .await;

        if !should_continue {
            break;
        }
    }
    let _ = event_tx.send(RunnerEvent::AllFinished).await;
    Ok(())
}

/// Run pre-commands for a group, returns true if all succeeded (or none to run)
async fn run_all_pre_commands(
    runner: &CheckRunner,
    group_name: &str,
    pre_commands: &[crate::config::PreCommand],
    event_tx: &mpsc::Sender<RunnerEvent>,
) -> bool {
    for pre_cmd in pre_commands {
        let _ = event_tx
            .send(RunnerEvent::PreCommandStarted {
                group: group_name.to_string(),
                name: pre_cmd.name.clone(),
            })
            .await;

        let (success, output, duration_ms) = runner.run_pre_command(pre_cmd).await;

        let _ = event_tx
            .send(RunnerEvent::PreCommandFinished {
                group: group_name.to_string(),
                name: pre_cmd.name.clone(),
                success,
                output,
                duration_ms,
            })
            .await;

        if !success {
            return false;
        }
    }
    true
}

async fn run_check_with_target(
    check: &CheckToRun,
    project_root: &Path,
    container_name: &str,
    target: &ExecTarget,
    event_tx: &mpsc::Sender<RunnerEvent>,
    executor: &dyn CommandExecutor,
) -> CheckResult {
    let check_id = check.id().to_string();

    // Merge global env with check-specific env (check env takes precedence)
    let mut env = target.env().clone();
    env.extend(check.definition.env.clone());

    let _ = event_tx
        .send(RunnerEvent::CheckStarted {
            check_id: check_id.clone(),
        })
        .await;

    execute_command_with_executor(
        check_id,
        &check.resolved_command,
        project_root,
        container_name,
        target,
        &env,
        executor,
    )
    .await
}

/// Execute a command on `target` and return the result (for testing with executor)
pub async fn execute_command_with_executor(
    check_id: String,
    command: &str,
    project_root: &std::path::Path,
    container_name: &str,
    target: &ExecTarget,
    env: &HashMap<String, String>,
    executor: &dyn CommandExecutor,
) -> CheckResult {
    let started_at = chrono::Local::now();
    let start = std::time::Instant::now();

    let full_cmd = target.build_command(container_name, env, command, executor);
    let output = executor.execute(&full_cmd, project_root).await;

    let duration_ms = start.elapsed().as_millis() as u64;
    let finished_at = chrono::Local::now();

    let stderr = filter_docker_warnings(&output.stderr);
    let status = if output.success {
        CheckStatus::Passed
    } else {
        CheckStatus::Failed
    };

    CheckResult {
        check_id,
        status,
        output: output.stdout,
        error_output: stderr,
        duration_ms,
        started_at: Some(started_at),
        finished_at: Some(finished_at),
    }
}

/// Run a single check (for retry single)
pub async fn run_single_check(
    check: &CheckToRun,
    project_root: &std::path::Path,
    default_container: &str,
    target: &ExecTarget,
    env: &HashMap<String, String>,
) -> CheckResult {
    run_single_check_with_executor(
        check,
        project_root,
        default_container,
        target,
        env,
        &RealCommandExecutor,
    )
    .await
}

/// Run a single check with a custom executor (test-facing).
///
/// Behaviour matches [`run_single_check`] except the caller supplies the executor.
pub async fn run_single_check_with_executor(
    check: &CheckToRun,
    project_root: &std::path::Path,
    default_container: &str,
    target: &ExecTarget,
    env: &HashMap<String, String>,
    executor: &dyn CommandExecutor,
) -> CheckResult {
    let container_name = check
        .definition
        .container
        .as_deref()
        .unwrap_or(default_container);
    let mut merged_env = env.clone();
    merged_env.extend(check.definition.env.clone());
    execute_command_with_executor(
        check.id().to_string(),
        &check.resolved_command,
        project_root,
        container_name,
        target,
        &merged_env,
        executor,
    )
    .await
}

/// Run a fix command for a check
pub async fn run_fix_command(
    fix_command: &str,
    project_root: &std::path::Path,
    container_name: &str,
    target: &ExecTarget,
    env: &HashMap<String, String>,
) -> CheckResult {
    run_fix_command_with_executor(
        fix_command,
        project_root,
        container_name,
        target,
        env,
        &RealCommandExecutor,
    )
    .await
}

/// Run a fix command with a custom executor (test-facing).
pub async fn run_fix_command_with_executor(
    fix_command: &str,
    project_root: &std::path::Path,
    container_name: &str,
    target: &ExecTarget,
    env: &HashMap<String, String>,
    executor: &dyn CommandExecutor,
) -> CheckResult {
    execute_command_with_executor(
        "fix".to_string(),
        fix_command,
        project_root,
        container_name,
        target,
        env,
        executor,
    )
    .await
}

/// Run a check with a custom command (e.g., for running without file filtering).
///
/// This is used when running a check for "all files" by removing the {files}
/// placeholder from the command.
pub async fn run_check_with_command(
    check: &CheckToRun,
    command: &str,
    project_root: &std::path::Path,
    default_container: &str,
    target: &ExecTarget,
    env: &HashMap<String, String>,
) -> CheckResult {
    run_check_with_command_with_executor(
        check,
        command,
        project_root,
        default_container,
        target,
        env,
        &RealCommandExecutor,
    )
    .await
}

/// Run a check with a custom command and custom executor (test-facing).
pub async fn run_check_with_command_with_executor(
    check: &CheckToRun,
    command: &str,
    project_root: &std::path::Path,
    default_container: &str,
    target: &ExecTarget,
    env: &HashMap<String, String>,
    executor: &dyn CommandExecutor,
) -> CheckResult {
    let container_name = check
        .definition
        .container
        .as_deref()
        .unwrap_or(default_container);
    let mut merged_env = env.clone();
    merged_env.extend(check.definition.env.clone());
    execute_command_with_executor(
        check.id().to_string(),
        command,
        project_root,
        container_name,
        target,
        &merged_env,
        executor,
    )
    .await
}

/// Build a host-local command: optional `env K='V'...` prefix + `shell -c` wrapper.
///
/// Invalid env keys are skipped (unquotable → shell injection), as in
/// [`build_docker_exec_command`].
pub fn build_local_command(
    local: &crate::config::LocalConfig,
    env: &HashMap<String, String>,
    command: &str,
) -> String {
    let env_flags: String = env
        .iter()
        .filter(|(k, _)| is_valid_env_key(k))
        .map(|(k, v)| format!("{}='{}' ", k, v.replace('\'', "'\\''")))
        .collect();
    let prefix = if env_flags.is_empty() {
        String::new()
    } else {
        format!("env {env_flags}")
    };
    format!(
        "{}{} -c '{}'",
        prefix,
        local.shell,
        command.replace('\'', "'\\''")
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_build_local_command_no_env() {
        let local = crate::config::LocalConfig::default();
        let cmd = build_local_command(&local, &HashMap::new(), "cargo test");
        assert_eq!(cmd, "bash -c 'cargo test'");
    }

    #[test]
    fn test_build_local_command_with_env_and_quotes() {
        let local = crate::config::LocalConfig::default();
        let env = HashMap::from([("APP_ENV".to_string(), "it's".to_string())]);
        let cmd = build_local_command(&local, &env, "echo 'hi'");
        assert_eq!(cmd, "env APP_ENV='it'\\''s' bash -c 'echo '\\''hi'\\'''");
    }

    #[test]
    fn test_build_local_command_skips_invalid_env_keys() {
        let local = crate::config::LocalConfig::default();
        let env = HashMap::from([("BAD;rm -rf /".to_string(), "x".to_string())]);
        let cmd = build_local_command(&local, &env, "ls");
        assert_eq!(cmd, "bash -c 'ls'");
    }

    fn local_target() -> ExecTarget {
        ExecTarget::Local(crate::config::LocalConfig::default())
    }

    fn docker_target() -> ExecTarget {
        let yaml = "project_dir: .\nservice: app\nshell: bash\nimage: test-img\n";
        ExecTarget::Docker(serde_yaml::from_str(yaml).unwrap())
    }

    #[test]
    fn test_build_command_local_never_queries_docker() {
        let mut mock = MockCommandExecutor::new();
        mock.expect_is_container_running().times(0);
        let cmd = local_target().build_command("", &HashMap::new(), "cargo test", &mock);
        assert_eq!(cmd, "bash -c 'cargo test'");
    }

    #[test]
    fn test_build_command_docker_running_uses_exec() {
        let mut mock = MockCommandExecutor::new();
        mock.expect_is_container_running().returning(|_| true);
        let cmd = docker_target().build_command("proj-app-1", &HashMap::new(), "ls", &mock);
        assert!(cmd.starts_with("docker exec proj-app-1"), "got: {cmd}");
    }

    #[test]
    fn test_build_command_docker_stopped_uses_run() {
        let mut mock = MockCommandExecutor::new();
        mock.expect_is_container_running().returning(|_| false);
        let cmd = docker_target().build_command("proj-app-1", &HashMap::new(), "ls", &mock);
        assert!(cmd.starts_with("docker run --rm"), "got: {cmd}");
    }

    #[test]
    fn test_from_config_local_env() {
        let yaml = "version: 2\nrunner: local\nlocal:\n  env:\n    A: b\ngit:\n  base_branch: main\n  fallback_branch: HEAD~1\nfile_patterns: {}\nchecks: {}\n";
        let config: crate::config::CiConfig = serde_yaml::from_str(yaml).unwrap();
        let target = ExecTarget::from_config(&config);
        assert!(matches!(target, ExecTarget::Local(_)));
        assert_eq!(target.env().get("A").unwrap(), "b");
        assert_eq!(target.default_container(), "");
    }

    #[test]
    fn test_build_local_command_custom_shell() {
        let local: crate::config::LocalConfig = serde_yaml::from_str("shell: /bin/sh").unwrap();
        let cmd = build_local_command(&local, &HashMap::new(), "ls");
        assert_eq!(cmd, "/bin/sh -c 'ls'");
    }
}
