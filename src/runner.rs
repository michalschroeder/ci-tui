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
//! - [`format_duration`]: Format milliseconds as human-readable duration

use crate::checks::CheckToRun;
use crate::config::CiConfig;
use anyhow::Result;
use chrono::{DateTime, Local};
use std::path::Path;
use std::sync::Arc;
use tokio::sync::mpsc;

/// Filter out Docker Compose warning messages from stderr
fn filter_docker_warnings(stderr: &str) -> String {
    stderr
        .lines()
        .filter(|line| {
            // Filter out common Docker Compose warnings that are noise
            !line.contains("variable is not set. Defaulting to a blank string")
        })
        .collect::<Vec<_>>()
        .join("\n")
}

/// Check if a Docker container is currently running
fn is_container_running(container_name: &str) -> bool {
    let output = std::process::Command::new("docker")
        .args(["inspect", "-f", "{{.State.Running}}", container_name])
        .output();

    match output {
        Ok(output) => {
            let result = String::from_utf8_lossy(&output.stdout);
            result.trim() == "true"
        }
        Err(_) => false,
    }
}

/// Build a docker exec command with environment variables
fn build_docker_exec_command(
    container_name: &str,
    env: &std::collections::HashMap<String, String>,
    command: &str,
) -> String {
    // Build env flags for docker exec (-e KEY='VALUE' for each)
    // Values are quoted to handle special characters like & ? in URLs
    let env_flags: String = env
        .iter()
        .map(|(k, v)| format!("-e {}='{}'", k, v.replace('\'', "'\\''")))
        .collect::<Vec<_>>()
        .join(" ");

    // Build docker exec command
    // Use bash with single quotes to prevent outer shell from expanding variables
    if env_flags.is_empty() {
        format!(
            "docker exec {} bash -c '{}'",
            container_name,
            command.replace('\'', "'\\''")
        )
    } else {
        format!(
            "docker exec {} {} bash -c '{}'",
            env_flags,
            container_name,
            command.replace('\'', "'\\''")
        )
    }
}

/// Build a docker run command with environment variables
fn build_docker_run_command(
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
    parts.push(format!("bash -c '{}'", command.replace('\'', "'\\''")));

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
}

impl CheckRunner {
    /// Create a new check runner with the given configuration
    pub fn new(config: CiConfig, project_root: &Path) -> Self {
        let container_name: Arc<str> = config.docker.container_name().into();
        Self {
            config: Arc::new(config),
            project_root: Arc::from(project_root),
            container_name,
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

        for (group_name, group_checks) in grouped {
            let _ = event_tx
                .send(RunnerEvent::GroupStarted {
                    group: group_name.to_string(),
                })
                .await;

            // Get group config
            let group_config = self.config.get_group(group_name);

            // Only run pre-commands if there are actual checks to run (not all on-demand)
            let has_runnable_checks = group_checks.iter().any(|c| !c.on_demand);

            // Run pre-commands for this group (only if there are runnable checks)
            if has_runnable_checks {
                if let Some(config) = group_config {
                    for pre_cmd in &config.pre_commands {
                        let _ = event_tx
                            .send(RunnerEvent::PreCommandStarted {
                                group: group_name.to_string(),
                                name: pre_cmd.name.clone(),
                            })
                            .await;

                        let (success, output, duration_ms) = self.run_pre_command(pre_cmd).await;

                        let _ = event_tx
                            .send(RunnerEvent::PreCommandFinished {
                                group: group_name.to_string(),
                                name: pre_cmd.name.clone(),
                                success,
                                output,
                                duration_ms,
                            })
                            .await;

                        // Stop if pre-command failed
                        if !success {
                            let _ = event_tx
                                .send(RunnerEvent::GroupFinished {
                                    group: group_name.to_string(),
                                })
                                .await;
                            let _ = event_tx.send(RunnerEvent::AllFinished).await;
                            return Ok(());
                        }
                    }
                }
            }

            let parallel = group_config.map(|g| g.parallel).unwrap_or(false);

            if parallel {
                self.run_parallel(group_checks, &event_tx).await?;
            } else {
                self.run_sequential(group_checks, &event_tx).await?;
            }

            let _ = event_tx
                .send(RunnerEvent::GroupFinished {
                    group: group_name.to_string(),
                })
                .await;
        }

        let _ = event_tx.send(RunnerEvent::AllFinished).await;
        Ok(())
    }

    async fn run_sequential(
        &self,
        checks: Vec<&CheckToRun>,
        event_tx: &mpsc::Sender<RunnerEvent>,
    ) -> Result<()> {
        for check in checks {
            // Skip on-demand checks - they require manual trigger
            if check.on_demand {
                continue;
            }
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
        let mut handles = Vec::new();

        for check in checks {
            // Skip on-demand checks - they require manual trigger
            if check.on_demand {
                continue;
            }
            let check = check.clone();
            let event_tx = event_tx.clone();
            let project_root = self.project_root.clone();
            // Use per-check container if specified, otherwise use default
            let container_name: Arc<str> = check
                .definition
                .container
                .clone()
                .unwrap_or_else(|| self.container_name.to_string())
                .into();
            let docker_config = self.config.docker.clone();
            let global_env = self.config.docker.env.clone();

            let handle = tokio::spawn(async move {
                let result = run_docker_check(
                    &check,
                    &project_root,
                    &container_name,
                    &docker_config,
                    &global_env,
                    &event_tx,
                )
                .await;
                let _ = event_tx.send(RunnerEvent::CheckFinished { result }).await;
            });

            handles.push(handle);
        }

        for handle in handles {
            let _ = handle.await;
        }

        Ok(())
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
        run_docker_check(
            check,
            &self.project_root,
            container_name,
            &self.config.docker,
            &self.config.docker.env,
            event_tx,
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
        let docker_cmd = if is_container_running(&container_name) {
            build_docker_exec_command(&container_name, &env, &pre_cmd.command)
        } else {
            build_docker_run_command(&self.config.docker, &env, &pre_cmd.command)
        };

        let output = tokio::process::Command::new("sh")
            .arg("-c")
            .arg(&docker_cmd)
            .current_dir(&self.project_root)
            .output()
            .await;

        let duration_ms = start.elapsed().as_millis() as u64;

        match output {
            Ok(output) => {
                let stdout = String::from_utf8_lossy(&output.stdout).to_string();
                let stderr = filter_docker_warnings(&String::from_utf8_lossy(&output.stderr));
                let combined = if stderr.is_empty() {
                    stdout
                } else {
                    format!("{}\n{}", stdout, stderr)
                };
                (output.status.success(), combined, duration_ms)
            }
            Err(e) => (false, format!("Failed to execute: {}", e), duration_ms),
        }
    }
}

async fn run_docker_check(
    check: &CheckToRun,
    project_root: &Path,
    container_name: &str,
    docker_config: &crate::config::DockerConfig,
    global_env: &std::collections::HashMap<String, String>,
    event_tx: &mpsc::Sender<RunnerEvent>,
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

    execute_docker_command(
        check_id,
        &check.resolved_command,
        project_root,
        container_name,
        docker_config,
        &env,
    )
    .await
}

/// Format duration in human readable format
pub fn format_duration(ms: u64) -> String {
    if ms < 1000 {
        format!("{}ms", ms)
    } else if ms < 60_000 {
        format!("{:.1}s", ms as f64 / 1000.0)
    } else {
        let mins = ms / 60_000;
        let secs = (ms % 60_000) / 1000;
        format!("{}m {}s", mins, secs)
    }
}

/// Execute a command in Docker and return the result
async fn execute_docker_command(
    check_id: String,
    command: &str,
    project_root: &std::path::Path,
    container_name: &str,
    docker_config: &crate::config::DockerConfig,
    env: &std::collections::HashMap<String, String>,
) -> CheckResult {
    let started_at = chrono::Local::now();
    let start = std::time::Instant::now();

    // Check if container is running, use exec if yes, run if no
    let docker_cmd = if is_container_running(container_name) {
        build_docker_exec_command(container_name, env, command)
    } else {
        build_docker_run_command(docker_config, env, command)
    };

    let output = tokio::process::Command::new("sh")
        .arg("-c")
        .arg(&docker_cmd)
        .current_dir(project_root)
        .output()
        .await;

    let duration_ms = start.elapsed().as_millis() as u64;
    let finished_at = chrono::Local::now();

    match output {
        Ok(output) => {
            let stdout = String::from_utf8_lossy(&output.stdout).to_string();
            let stderr = filter_docker_warnings(&String::from_utf8_lossy(&output.stderr));
            let status = if output.status.success() {
                CheckStatus::Passed
            } else {
                CheckStatus::Failed
            };

            CheckResult {
                check_id,
                status,
                output: stdout,
                error_output: stderr,
                duration_ms,
                started_at: Some(started_at),
                finished_at: Some(finished_at),
            }
        }
        Err(e) => CheckResult {
            check_id,
            status: CheckStatus::Failed,
            output: String::new(),
            error_output: format!("Failed to execute: {}", e),
            duration_ms,
            started_at: Some(started_at),
            finished_at: Some(finished_at),
        },
    }
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
