//! Check execution in Docker containers with event streaming.
//!
//! This module handles the actual execution of CI checks via Docker. It supports
//! both `docker exec` (for running containers) and `docker run` (for standalone
//! execution), with real-time output streaming through async channels.
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

/// Build a docker exec command with environment variables
pub fn build_docker_exec_command(
    container_name: &str,
    env: &std::collections::HashMap<String, String>,
    command: &str,
    shell: &str,
) -> String {
    // Build env flags for docker exec (-e KEY='VALUE' for each)
    // Values are quoted to handle special characters like & ? in URLs
    let env_flags: String = env
        .iter()
        .map(|(k, v)| format!("-e {}='{}'", k, v.replace('\'', "'\\''")))
        .collect::<Vec<_>>()
        .join(" ");

    // Build docker exec command
    // Use configured shell with single quotes to prevent outer shell from expanding variables
    if env_flags.is_empty() {
        format!(
            "docker exec {} {} -c '{}'",
            container_name,
            shell,
            command.replace('\'', "'\\''")
        )
    } else {
        format!(
            "docker exec {} {} {} -c '{}'",
            env_flags,
            container_name,
            shell,
            command.replace('\'', "'\\''")
        )
    }
}

/// Build a docker run command with environment variables
pub fn build_docker_run_command(
    docker_config: &crate::config::DockerConfig,
    env: &std::collections::HashMap<String, String>,
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
    let shell = docker_config.shell();

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
    /// Output line received from a running check (for streaming)
    CheckOutput { check_id: String, line: String },
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
        let container_name: Arc<str> = config.docker.container_name().into();
        Self {
            config: Arc::new(config),
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

        let has_runnable = group_checks.iter().any(|c| !c.on_demand);
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
        let runnable: Vec<_> = checks.into_iter().filter(|c| !c.on_demand).collect();
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
        let runnable: Vec<_> = checks.into_iter().filter(|c| !c.on_demand).collect();
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
        let docker_config = self.config.docker.clone();
        let global_env = self.config.docker.env.clone();
        let executor = self.executor.clone();

        tokio::spawn(async move {
            let result = run_docker_check_with_executor(
                &check,
                &project_root,
                &container_name,
                &docker_config,
                &global_env,
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
        run_docker_check_with_executor(
            check,
            &self.project_root,
            container_name,
            &self.config.docker,
            &self.config.docker.env,
            event_tx,
            self.executor.as_ref(),
        )
        .await
    }

    /// Run a pre-command for a group (e.g., DB initialization)
    /// Returns (success, output, duration_ms)
    async fn run_pre_command(&self, pre_cmd: &crate::config::PreCommand) -> (bool, String, u64) {
        let service = pre_cmd
            .service
            .as_deref()
            .unwrap_or_else(|| self.config.default_service());
        let start = std::time::Instant::now();

        // Merge global env with pre-command-specific env (command env takes precedence)
        let mut env = self.config.docker.env.clone();
        env.extend(pre_cmd.env.clone());

        // Get container name: explicit container > service-based derivation > default
        let container_name = if let Some(ref container) = pre_cmd.container {
            // Explicit container name specified
            container.clone()
        } else if service != self.config.default_service() {
            // Different service, derive container name
            let project_name = std::path::Path::new(&self.config.docker.project_dir)
                .file_name()
                .and_then(|n| n.to_str())
                .unwrap_or("project");
            format!("{}-{}-1", project_name, service)
        } else {
            self.container_name.to_string()
        };

        // Check if container is running, use exec if yes, run if no
        let docker_cmd = if self.executor.is_container_running(&container_name) {
            build_docker_exec_command(
                &container_name,
                &env,
                &pre_cmd.command,
                self.config.docker.shell(),
            )
        } else {
            build_docker_run_command(&self.config.docker, &env, &pre_cmd.command)
        };

        let output = self.executor.execute(&docker_cmd, &self.project_root).await;

        let duration_ms = start.elapsed().as_millis() as u64;

        let stderr = filter_docker_warnings(&output.stderr);
        let combined = if stderr.is_empty() {
            output.stdout.clone()
        } else {
            format!("{}\n{}", output.stdout, stderr)
        };
        (output.success, combined, duration_ms)
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

async fn run_docker_check_with_executor(
    check: &CheckToRun,
    project_root: &Path,
    container_name: &str,
    docker_config: &crate::config::DockerConfig,
    global_env: &std::collections::HashMap<String, String>,
    event_tx: &mpsc::Sender<RunnerEvent>,
    executor: &dyn CommandExecutor,
) -> CheckResult {
    let check_id = check.id().to_string();

    // Merge global env with check-specific env (check env takes precedence)
    let mut env = global_env.clone();
    env.extend(check.definition.env.clone());

    let _ = event_tx
        .send(RunnerEvent::CheckStarted {
            check_id: check_id.clone(),
        })
        .await;

    execute_docker_command_with_executor(
        check_id,
        &check.resolved_command,
        project_root,
        container_name,
        docker_config,
        &env,
        executor,
    )
    .await
}

/// Execute a command in Docker and return the result (for testing with executor)
pub async fn execute_docker_command_with_executor(
    check_id: String,
    command: &str,
    project_root: &std::path::Path,
    container_name: &str,
    docker_config: &crate::config::DockerConfig,
    env: &std::collections::HashMap<String, String>,
    executor: &dyn CommandExecutor,
) -> CheckResult {
    let started_at = chrono::Local::now();
    let start = std::time::Instant::now();

    // Check if container is running, use exec if yes, run if no
    let docker_cmd = if executor.is_container_running(container_name) {
        build_docker_exec_command(container_name, env, command, docker_config.shell())
    } else {
        build_docker_run_command(docker_config, env, command)
    };

    let output = executor.execute(&docker_cmd, project_root).await;

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

/// Execute a command in Docker and return the result (backward-compatible wrapper)
async fn execute_docker_command(
    check_id: String,
    command: &str,
    project_root: &std::path::Path,
    container_name: &str,
    docker_config: &crate::config::DockerConfig,
    env: &std::collections::HashMap<String, String>,
) -> CheckResult {
    execute_docker_command_with_executor(
        check_id,
        command,
        project_root,
        container_name,
        docker_config,
        env,
        &RealCommandExecutor,
    )
    .await
}

/// Run a single check (for retry single)
pub async fn run_single_check(
    check: &CheckToRun,
    project_root: &std::path::Path,
    default_container: &str,
    docker_config: &crate::config::DockerConfig,
    env: &std::collections::HashMap<String, String>,
) -> CheckResult {
    // Use per-check container if specified, otherwise use default
    let container_name = check
        .definition
        .container
        .as_deref()
        .unwrap_or(default_container);
    // Merge global env with check-specific env (check env takes precedence)
    let mut merged_env = env.clone();
    merged_env.extend(check.definition.env.clone());
    execute_docker_command(
        check.id().to_string(),
        &check.resolved_command,
        project_root,
        container_name,
        docker_config,
        &merged_env,
    )
    .await
}

/// Run a fix command for a check
pub async fn run_fix_command(
    fix_command: &str,
    project_root: &std::path::Path,
    container_name: &str,
    docker_config: &crate::config::DockerConfig,
    env: &std::collections::HashMap<String, String>,
) -> CheckResult {
    execute_docker_command(
        "fix".to_string(),
        fix_command,
        project_root,
        container_name,
        docker_config,
        env,
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
    docker_config: &crate::config::DockerConfig,
    env: &std::collections::HashMap<String, String>,
) -> CheckResult {
    // Use per-check container if specified, otherwise use default
    let container_name = check
        .definition
        .container
        .as_deref()
        .unwrap_or(default_container);
    // Merge global env with check-specific env (check env takes precedence)
    let mut merged_env = env.clone();
    merged_env.extend(check.definition.env.clone());
    execute_docker_command(
        check.id().to_string(),
        command,
        project_root,
        container_name,
        docker_config,
        &merged_env,
    )
    .await
}
