//! Check execution (Docker or local host) with event streaming.
//!
//! This module handles the actual execution of CI checks. [`ExecTarget`] picks
//! the mode: Docker (`docker exec` for running containers, `docker run` for
//! standalone execution) or local host (`shell -c`). Lifecycle events
//! ([`RunnerEvent`]) go through async channels; while a check runs its output
//! is streamed as batched [`RunnerEvent::CheckOutput`] chunks (see
//! [`OutputSink`]). The final [`RunnerEvent::CheckFinished`] result stays
//! authoritative (truncated, Docker warnings filtered).
//!
//! # Key Types
//!
//! - [`CheckRunner`]: Orchestrates check execution with parallel/sequential support
//! - [`CheckResult`]: Result of executing a check including output and timing
//! - [`CheckStatus`]: Current status of a check (Pending, Running, Passed, TimedOut, etc.)
//! - [`RunnerEvent`]: Events emitted during execution for UI updates
//! - [`CancelRegistry`]: Per-check cancel switches (TUI `s` key)
//!
//! # Process cleanup
//!
//! A command that does not finish normally (cancel, timeout, or its task
//! aborted/dropped on quit or retry-all) is cleaned up by drop guards:
//! [`RealCommandExecutor`] kills the command's whole process group, and a
//! `docker run` fallback (always started with a unique `--name`) gets a
//! `docker kill`. LIMITATION: for `docker exec` only the local client dies;
//! the process inside the container keeps running until it exits.
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
use std::future::Future;
use std::path::Path;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Duration;
use tokio::sync::{mpsc, oneshot};

/// Output from executing a command
#[derive(Debug, Clone)]
pub struct CommandOutput {
    pub success: bool,
    pub stdout: String,
    pub stderr: String,
}

/// Max delay between streamed [`RunnerEvent::CheckOutput`] chunks: output is
/// batched per interval (and flushed on exit), not sent per line
pub const OUTPUT_FLUSH_INTERVAL: Duration = Duration::from_millis(100);

/// Where a running command's live output chunks go: a
/// [`RunnerEvent::CheckOutput`] for one check, or nowhere ([`Self::none`]:
/// pre-commands, fixes, simple mode).
#[derive(Clone, Default, Debug)]
pub struct OutputSink(Option<(String, mpsc::Sender<RunnerEvent>)>);

impl OutputSink {
    /// Discard live output
    pub fn none() -> Self {
        Self(None)
    }

    /// Stream live output of `check_id` to `tx`
    pub fn new(check_id: impl Into<String>, tx: mpsc::Sender<RunnerEvent>) -> Self {
        Self(Some((check_id.into(), tx)))
    }

    /// True if live output goes somewhere
    pub fn is_some(&self) -> bool {
        self.0.is_some()
    }

    /// Send one chunk (Docker warning lines dropped from `stderr`). Skipped
    /// when empty; send errors (receiver gone) are ignored.
    pub async fn send(&self, stdout: String, stderr: String) {
        let Some((check_id, tx)) = &self.0 else {
            return;
        };
        let stderr: String = stderr
            .split_inclusive('\n')
            .filter(|line| !is_docker_warning(line))
            .collect();
        if stdout.is_empty() && stderr.is_empty() {
            return;
        }
        let _ = tx
            .send(RunnerEvent::CheckOutput {
                check_id: check_id.clone(),
                stdout,
                stderr,
            })
            .await;
    }
}

/// Trait for executing commands, allowing mock implementations in tests
#[cfg_attr(any(test, feature = "test"), mockall::automock)]
#[async_trait]
pub trait CommandExecutor: Send + Sync {
    /// Execute a shell command and return the output. Implementations may
    /// stream output chunks to `sink` while it runs; the returned output is
    /// complete either way.
    async fn execute(&self, command: &str, working_dir: &Path, sink: &OutputSink) -> CommandOutput;

    /// Check if a Docker container is running
    fn is_container_running(&self, container_name: &str) -> bool;

    /// Kill a Docker container (best effort; errors ignored). Called when a
    /// `docker run` command is cancelled, times out or is dropped.
    fn kill_container(&self, name: &str);
}

/// Production implementation of CommandExecutor
pub struct RealCommandExecutor;

#[async_trait]
impl CommandExecutor for RealCommandExecutor {
    async fn execute(&self, command: &str, working_dir: &Path, sink: &OutputSink) -> CommandOutput {
        let mut cmd = tokio::process::Command::new("sh");
        cmd.arg("-c")
            .arg(command)
            .current_dir(working_dir)
            // Null stdin: a child outside the terminal's foreground group
            // must not read the TTY (SIGTTIN)
            .stdin(std::process::Stdio::null())
            .stdout(std::process::Stdio::piped())
            .stderr(std::process::Stdio::piped())
            .kill_on_drop(true);
        // Own process group, so cancel/timeout/drop can kill the whole tree
        // (kill_on_drop alone only kills `sh`)
        #[cfg(unix)]
        cmd.process_group(0);

        let output = match cmd.spawn() {
            Ok(mut child) => {
                let mut group = ProcessGroupGuard(child.id());
                let output = read_streaming(&mut child, sink).await;
                // Finished normally: leave anything it backgrounded alone
                group.0 = None;
                output
            }
            Err(e) => Err(e),
        };

        match output {
            Ok((status, stdout, stderr)) => CommandOutput {
                success: status.success(),
                stdout: bytes_to_string(stdout),
                stderr: bytes_to_string(stderr),
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

    fn kill_container(&self, name: &str) {
        crate::utils::docker::kill(name);
    }
}

/// Read `child`'s piped stdout/stderr to EOF, streaming complete lines to
/// `sink` every [`OUTPUT_FLUSH_INTERVAL`] (rest flushed at EOF), then wait
/// for it. Returns the exit status and the full raw output.
///
/// Raw bytes are buffered and decoded only per chunk, so the returned
/// output is byte-identical to `wait_with_output`. Chunks end on `\n`, so
/// they never split a UTF-8 character.
async fn read_streaming(
    child: &mut tokio::process::Child,
    sink: &OutputSink,
) -> std::io::Result<(std::process::ExitStatus, Vec<u8>, Vec<u8>)> {
    let mut out_pipe = child.stdout.take();
    let mut err_pipe = child.stderr.take();
    let (mut stdout, mut stderr) = (Vec::new(), Vec::new());
    let (mut out_sent, mut err_sent) = (0, 0);
    let mut flush = tokio::time::interval(OUTPUT_FLUSH_INTERVAL);
    flush.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Delay);

    while out_pipe.is_some() || err_pipe.is_some() {
        stdout.reserve(READ_CHUNK_BYTES);
        stderr.reserve(READ_CHUNK_BYTES);
        // `read_buf` is cancel safe: a losing branch loses no bytes
        tokio::select! {
            n = read_some(&mut out_pipe, &mut stdout) => {
                if n? == 0 {
                    out_pipe = None;
                }
            }
            n = read_some(&mut err_pipe, &mut stderr) => {
                if n? == 0 {
                    err_pipe = None;
                }
            }
            _ = flush.tick(), if sink.is_some() => {
                let out = unsent(&stdout, &mut out_sent, false);
                let err = unsent(&stderr, &mut err_sent, false);
                sink.send(out, err).await;
            }
        }
    }
    let out = unsent(&stdout, &mut out_sent, true);
    let err = unsent(&stderr, &mut err_sent, true);
    sink.send(out, err).await;

    let status = child.wait().await?;
    Ok((status, stdout, stderr))
}

/// Decode owned bytes, reusing the buffer when valid UTF-8 (lossy copy only
/// otherwise)
fn bytes_to_string(bytes: Vec<u8>) -> String {
    String::from_utf8(bytes).unwrap_or_else(|e| String::from_utf8_lossy(e.as_bytes()).into_owned())
}

/// Buffer growth step for [`read_streaming`]
const READ_CHUNK_BYTES: usize = 8 * 1024;

/// Read available bytes of `pipe` into `buf`; 0 on EOF. Pending forever
/// once `pipe` is closed (`None`), so `select!` skips it.
async fn read_some<R: tokio::io::AsyncRead + Unpin>(
    pipe: &mut Option<R>,
    buf: &mut Vec<u8>,
) -> std::io::Result<usize> {
    use tokio::io::AsyncReadExt;
    match pipe {
        Some(pipe) => pipe.read_buf(buf).await,
        None => std::future::pending().await,
    }
}

/// Decoded bytes of `buf` after `*sent`, up to the last complete line (or
/// everything when `all`); advances `*sent`
fn unsent(buf: &[u8], sent: &mut usize, all: bool) -> String {
    let rest = &buf[*sent..];
    let len = if all {
        rest.len()
    } else {
        rest.iter().rposition(|&b| b == b'\n').map_or(0, |i| i + 1)
    };
    *sent += len;
    String::from_utf8_lossy(&rest[..len]).into_owned()
}

/// SIGKILLs process group `.0` on drop, i.e. when [`RealCommandExecutor`]'s
/// future is dropped before the command finished. Forgotten on normal exit.
struct ProcessGroupGuard(Option<u32>);

impl Drop for ProcessGroupGuard {
    fn drop(&mut self) {
        if let Some(pgid) = self.0 {
            // Shelling out to kill(1) instead of adding a libc dependency.
            // Blocking but brief; must finish before a quitting process exits.
            let _ = std::process::Command::new("kill")
                .args(["-9", "--", &format!("-{pgid}")])
                .stdout(std::process::Stdio::null())
                .stderr(std::process::Stdio::null())
                .status();
        }
    }
}

/// `docker kill`s a `docker run` container on drop unless disarmed
/// (`name` cleared)
struct ContainerGuard<'a> {
    executor: &'a dyn CommandExecutor,
    name: Option<&'a str>,
}

impl Drop for ContainerGuard<'_> {
    fn drop(&mut self) {
        if let Some(name) = self.name {
            self.executor.kill_container(name);
        }
    }
}

/// Run `command` via `executor`, cleaning up if it does not finish.
///
/// Cleanup when this future is dropped before the command finished (timeout
/// expiry, a cancel, or its task aborted on quit, Ctrl-C, retry-all):
/// dropping the executor future makes [`RealCommandExecutor`] SIGKILL the
/// command's whole process group, and a `docker run` command
/// ([`BuiltCommand::container`]) gets `docker kill <name>`. LIMITATION: a
/// `docker exec` command keeps running inside the container (only the local
/// client is killed).
pub async fn execute_built(
    executor: &dyn CommandExecutor,
    command: &BuiltCommand,
    working_dir: &Path,
    sink: &OutputSink,
) -> CommandOutput {
    let mut container = ContainerGuard {
        executor,
        name: command.container.as_deref(),
    };
    let output = executor.execute(&command.command, working_dir, sink).await;
    container.name = None; // finished: `--rm` already removed it
    output
}

/// [`execute_built`] bounded by `timeout` (`None` = unbounded).
///
/// Returns `Err(message)` on expiry; expiry drops the [`execute_built`]
/// future, which kills the command (see there). The timeout wraps the
/// executor future rather than living in [`CommandExecutor`] so mocks stay
/// unchanged.
pub async fn execute_with_timeout(
    executor: &dyn CommandExecutor,
    command: &BuiltCommand,
    working_dir: &Path,
    timeout: Option<Duration>,
    sink: &OutputSink,
) -> std::result::Result<CommandOutput, String> {
    let run = execute_built(executor, command, working_dir, sink);
    match timeout {
        Some(limit) => tokio::time::timeout(limit, run).await.map_err(|_| {
            format!(
                "timed out after {}",
                crate::utils::time::format_from_duration(limit)
            )
        }),
        None => Ok(run.await),
    }
}

/// True for a common Docker Compose warning line that is noise
fn is_docker_warning(line: &str) -> bool {
    line.contains("variable is not set. Defaulting to a blank string")
}

/// Filter out Docker Compose warning messages from stderr
pub fn filter_docker_warnings(stderr: &str) -> String {
    stderr
        .lines()
        .filter(|line| !is_docker_warning(line))
        .collect::<Vec<_>>()
        .join("\n")
}

/// Truncation marker line is `TRUNCATED_PREFIX` + count + `TRUNCATED_SUFFIX`
const TRUNCATED_PREFIX: &str = "… ";
const TRUNCATED_SUFFIX: &str = " lines truncated";

/// Marker line for `n` dropped lines (`"… N lines truncated"`)
fn truncation_marker(n: usize) -> String {
    format!("{TRUNCATED_PREFIX}{n}{TRUNCATED_SUFFIX}")
}

/// Cap `text` at `max_lines` lines, keeping the *last* `max_lines` (a runaway
/// command's most recent output is usually the relevant part). When lines are
/// dropped, a `"... N lines truncated"` marker is prepended. Returns `text`
/// unchanged (no copy) when it's already within the cap.
pub fn truncate_output(text: String, max_lines: usize) -> String {
    let total = text.lines().count();
    if total <= max_lines {
        return text;
    }
    let truncated = total - max_lines;
    let marker = truncation_marker(truncated);
    std::iter::once(marker.as_str())
        .chain(text.lines().skip(truncated))
        .collect::<Vec<_>>()
        .join("\n")
}

/// Line count of a [`truncate_output`] marker line (`"… N lines truncated"`)
fn truncated_marker_count(line: &str) -> Option<usize> {
    line.strip_prefix(TRUNCATED_PREFIX)?
        .strip_suffix(TRUNCATED_SUFFIX)?
        .parse()
        .ok()
}

/// Append streamed `chunk` to `buf`, keeping only the last `max_lines` lines
/// (rolling tail, like [`truncate_output`]). The `"… N lines truncated"`
/// marker is kept as the first line, its count accumulating across calls.
pub fn append_output(buf: &mut String, chunk: &str, max_lines: usize) {
    if chunk.is_empty() {
        return;
    }
    buf.push_str(chunk);
    let first = buf.lines().next().unwrap_or_default();
    let (dropped, body_start) = match truncated_marker_count(first) {
        Some(n) => (n, (first.len() + 1).min(buf.len())),
        None => (0, 0),
    };
    let body = &buf[body_start..];
    let total = body.lines().count();
    if total <= max_lines {
        return;
    }
    let drop = total - max_lines;
    let cut = body
        .match_indices('\n')
        .nth(drop - 1)
        .map_or(body.len(), |(i, _)| i + 1);
    *buf = format!("{}\n{}", truncation_marker(dropped + drop), &body[cut..]);
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
    let env_flags: String = env_assignments(env).map(|a| format!("-e {a} ")).collect();

    // Single format path: env_flags is either empty or ends with a trailing space.
    format!(
        "docker exec {}{} {} -c {}",
        env_flags,
        container_name,
        shell,
        single_quote(command)
    )
}

/// `KEY='VALUE'` for each valid env key; invalid keys are skipped (unquotable
/// → shell injection). Values are single-quoted with `'` escaped.
fn env_assignments(env: &HashMap<String, String>) -> impl Iterator<Item = String> + '_ {
    env.iter()
        .filter(|(k, _)| is_valid_env_key(k))
        .map(|(k, v)| format!("{k}={}", single_quote(v)))
}

/// Always single-quote `s` for POSIX shells (`'` escaped as `'\''`).
fn single_quote(s: &str) -> String {
    format!("'{}'", s.replace('\'', "'\\''"))
}

/// Unique `docker run --name` for this process: `ci-tui-<pid>-<n>`
fn unique_container_name() -> String {
    static NEXT: AtomicU64 = AtomicU64::new(0);
    let n = NEXT.fetch_add(1, Ordering::Relaxed);
    format!("ci-tui-{}-{n}", std::process::id())
}

/// Build a docker run command with environment variables. `name` becomes
/// `--name`, so cancel/timeout can `docker kill` the container.
pub fn build_docker_run_command(
    docker_config: &crate::config::DockerConfig,
    env: &HashMap<String, String>,
    command: &str,
    name: &str,
) -> String {
    // Build env flags for docker run (-e KEY='VALUE' for each)
    let env_flags: String = env_assignments(env)
        .map(|a| format!("-e {a}"))
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
    let mut parts = vec![format!("docker run --rm --name {name}")];

    if !volume_args.is_empty() {
        parts.push(volume_args);
    }

    if !env_flags.is_empty() {
        parts.push(env_flags);
    }

    parts.push(format!("-w {}", work_dir));
    parts.push(image_name);
    parts.push(format!("{} -c {}", shell, single_quote(command)));

    parts.join(" ")
}

pub use crate::config::ExecTarget;

/// Shell command built for an [`ExecTarget`]
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BuiltCommand {
    /// Full shell command line
    pub command: String,
    /// `--name` of the container a `docker run` command starts (killed on
    /// cancel/timeout/drop); `None` for `docker exec` and host commands
    pub container: Option<String>,
}

impl BuiltCommand {
    /// Command with no container to clean up (host, local or `docker exec`)
    pub fn plain(command: impl Into<String>) -> Self {
        Self {
            command: command.into(),
            container: None,
        }
    }
}

impl ExecTarget {
    /// Global env: `docker.env` or `local.env`.
    pub fn env(&self) -> &HashMap<String, String> {
        match self {
            Self::Docker(docker) => &docker.env,
            Self::Local(local) => &local.env,
        }
    }

    /// Full shell command for `command`. `container` overrides the docker
    /// default container (`docker.container_name()`); ignored in local mode.
    /// Queries container state ONLY in docker mode.
    pub fn build_command(
        &self,
        container: Option<&str>,
        env: &HashMap<String, String>,
        command: &str,
        executor: &dyn CommandExecutor,
    ) -> BuiltCommand {
        match self {
            Self::Local(local) => BuiltCommand::plain(build_local_command(local, env, command)),
            Self::Docker(docker) => build_docker_command(docker, container, env, command, executor),
        }
    }
}

/// `docker exec` into `container` (default: `docker.container_name()`) when it
/// is running, else standalone `docker run`.
fn build_docker_command(
    docker: &crate::config::DockerConfig,
    container: Option<&str>,
    env: &HashMap<String, String>,
    command: &str,
    executor: &dyn CommandExecutor,
) -> BuiltCommand {
    let name = container.map_or_else(|| docker.container_name(), str::to_owned);
    if executor.is_container_running(&name) {
        BuiltCommand::plain(build_docker_exec_command(
            &name,
            env,
            command,
            &docker.shell,
        ))
    } else {
        let run_name = unique_container_name();
        BuiltCommand {
            command: build_docker_run_command(docker, env, command, &run_name),
            container: Some(run_name),
        }
    }
}

/// Env for a command: target (global) env, then `extra` on top (extra wins).
fn merged_env(target: &ExecTarget, extra: &HashMap<String, String>) -> HashMap<String, String> {
    let mut env = target.env().clone();
    env.extend(extra.clone());
    env
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
    /// Check exceeded its `timeout` and was killed
    TimedOut,
    /// Check was cancelled by the user (press 's') and killed; not a failure
    Cancelled,
}

impl CheckStatus {
    /// True for outcomes that count as a failure (exit code, summaries,
    /// failed filter, retry/fix eligibility).
    pub fn is_failure(&self) -> bool {
        matches!(self, Self::Failed | Self::TimedOut)
    }

    /// True once the check ran to an outcome: passed, cancelled or a failure
    pub fn is_finished(&self) -> bool {
        matches!(self, Self::Passed | Self::Cancelled) || self.is_failure()
    }
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

    /// Create a cancelled check result (killed by the user after `started_at`)
    pub fn cancelled(check_id: &str, started_at: DateTime<Local>) -> Self {
        let finished_at = Local::now();
        Self {
            check_id: check_id.to_string(),
            status: CheckStatus::Cancelled,
            output: "cancelled by user".to_string(),
            error_output: String::new(),
            duration_ms: (finished_at - started_at).num_milliseconds().max(0) as u64,
            started_at: Some(started_at),
            finished_at: Some(finished_at),
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

/// Per-check cancel switches for running commands, keyed by check id.
///
/// Cloning shares the registry. A run registers via [`run_check_cancellable`];
/// [`CancelRegistry::cancel`] drops that run's future, whose drop guards kill
/// the process group / container (see [`execute_built`]).
#[derive(Clone, Default)]
pub struct CancelRegistry {
    inner: Arc<Mutex<HashMap<String, oneshot::Sender<()>>>>,
}

impl CancelRegistry {
    /// Cancel the registered run of `check_id`. False if none is running.
    pub fn cancel(&self, check_id: &str) -> bool {
        let tx = self.lock().remove(check_id);
        tx.is_some_and(|tx| tx.send(()).is_ok())
    }

    /// Register a run of `check_id`, replacing any previous one (whose
    /// receiver then sees `Err`). Finished runs' entries are pruned here.
    fn register(&self, check_id: &str) -> oneshot::Receiver<()> {
        let (tx, rx) = oneshot::channel();
        let mut map = self.lock();
        map.retain(|_, tx| !tx.is_closed());
        map.insert(check_id.to_string(), tx);
        rx
    }

    fn lock(&self) -> std::sync::MutexGuard<'_, HashMap<String, oneshot::Sender<()>>> {
        // A panic while holding the lock leaves the map consistent
        self.inner.lock().unwrap_or_else(|e| e.into_inner())
    }
}

/// Run `run` as `check_id`, cancellable through `cancels`.
///
/// If cancelled, `run` is dropped (which kills its command, see
/// [`execute_built`]) and a [`CheckResult::cancelled`] is returned instead;
/// `T` is `CheckResult` or `Option<CheckResult>`.
pub async fn run_check_cancellable<T: From<CheckResult>>(
    cancels: &CancelRegistry,
    check_id: &str,
    run: impl Future<Output = T>,
) -> T {
    let started_at = Local::now();
    let cancelled = cancels.register(check_id);
    tokio::select! {
        output = run => output,
        // Err = replaced by a newer run of the same id: keep running
        Ok(()) = cancelled => CheckResult::cancelled(check_id, started_at).into(),
    }
}

/// Events emitted by the check runner during execution
#[derive(Debug, Clone)]
pub enum RunnerEvent {
    /// A check has started executing
    CheckStarted { check_id: String },
    /// Live output chunk of a running check (batched, see
    /// [`OUTPUT_FLUSH_INTERVAL`]); Docker warnings already dropped from
    /// `stderr`. The final [`Self::CheckFinished`] result supersedes it.
    CheckOutput {
        check_id: String,
        stdout: String,
        stderr: String,
    },
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
    /// Config; `config.runner` is the single source of truth for the target
    config: Arc<CiConfig>,
    project_root: Arc<Path>,
    executor: Arc<dyn CommandExecutor>,
    cancels: CancelRegistry,
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
        Self {
            config: Arc::new(config),
            project_root: Arc::from(project_root),
            executor,
            cancels: CancelRegistry::default(),
        }
    }

    /// Register running checks in `cancels` so they can be cancelled
    pub fn with_cancels(mut self, cancels: CancelRegistry) -> Self {
        self.cancels = cancels;
        self
    }

    /// Where commands execute (`config.runner`)
    fn target(&self) -> &ExecTarget {
        &self.config.runner
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
        // JoinSet, not detached spawns: aborting the runner (quit, retry-all)
        // drops the set, which aborts the checks and kills their commands
        let mut set = tokio::task::JoinSet::new();
        for check in runnable {
            set.spawn(self.check_task(check, event_tx));
        }
        while set.join_next().await.is_some() {}
        Ok(())
    }

    /// Owned future running `check` and reporting its result
    fn check_task(
        &self,
        check: &CheckToRun,
        event_tx: &mpsc::Sender<RunnerEvent>,
    ) -> impl Future<Output = ()> + Send + 'static {
        let check = check.clone();
        let event_tx = event_tx.clone();
        let project_root = self.project_root.clone();
        let config = Arc::clone(&self.config);
        let executor = self.executor.clone();
        let cancels = self.cancels.clone();

        async move {
            let result = run_check_with_target(
                &check,
                &project_root,
                &config.runner,
                &event_tx,
                executor.as_ref(),
                &cancels,
                config.max_output_lines,
            )
            .await;
            let _ = event_tx.send(RunnerEvent::CheckFinished { result }).await;
        }
    }

    async fn run_single_check(
        &self,
        check: &CheckToRun,
        event_tx: &mpsc::Sender<RunnerEvent>,
    ) -> CheckResult {
        run_check_with_target(
            check,
            &self.project_root,
            self.target(),
            event_tx,
            self.executor.as_ref(),
            &self.cancels,
            self.config.max_output_lines,
        )
        .await
    }

    /// Run a pre-command for a group (e.g., DB initialization)
    /// Returns (success, output, duration_ms)
    async fn run_pre_command(&self, pre_cmd: &crate::config::PreCommand) -> (bool, String, u64) {
        let start = std::time::Instant::now();

        let cmd = if pre_cmd.host {
            BuiltCommand::plain(pre_cmd.command.clone())
        } else {
            self.build_pre_command_cmd(pre_cmd)
        };

        let output = execute_with_timeout(
            self.executor.as_ref(),
            &cmd,
            &self.project_root,
            pre_cmd.timeout,
            &OutputSink::none(),
        )
        .await;
        let duration_ms = start.elapsed().as_millis() as u64;

        // Timeout → failure, same as a non-zero exit (group aborted)
        let output = match output {
            Ok(output) => output,
            Err(message) => return (false, message, duration_ms),
        };

        let stderr = filter_docker_warnings(&output.stderr);
        let combined = if stderr.is_empty() {
            output.stdout
        } else {
            format!("{}\n{}", output.stdout, stderr)
        };
        let combined = truncate_output(combined, self.config.max_output_lines);
        (output.success, combined, duration_ms)
    }

    /// Shell command for a non-host pre-command. Local mode ignores
    /// `service`/`container` (rejected at load) and runs on the host.
    fn build_pre_command_cmd(&self, pre_cmd: &crate::config::PreCommand) -> BuiltCommand {
        let env = merged_env(self.target(), &pre_cmd.env);
        let container = match self.target() {
            ExecTarget::Docker(docker) => Some(pre_command_container(docker, pre_cmd)),
            ExecTarget::Local(_) => None,
        };
        self.target().build_command(
            container.as_deref(),
            &env,
            &pre_cmd.command,
            self.executor.as_ref(),
        )
    }
}

/// Resolve the container for a docker-mode pre-command.
///
/// Precedence: explicit `container:` > compose-convention name for a
/// non-default service > default container.
///
/// LIMITATION: assumes the Docker Compose v2 naming convention
/// `{project}-{service}-1`. `COMPOSE_PROJECT_NAME` and a top-level `name:`
/// key in the compose file are both honored (see `DockerConfig::compose_project_name`).
/// Non-UTF8 project paths fall back to the literal project name "project".
/// Future work: resolve via `docker compose ps -q <service>` instead of
/// string construction.
fn pre_command_container(
    docker: &crate::config::DockerConfig,
    pre_cmd: &crate::config::PreCommand,
) -> String {
    let service = pre_cmd.service.as_deref().unwrap_or(&docker.service);
    match pre_cmd.container.as_deref() {
        Some(container) => container.to_string(),
        None if service != docker.service => {
            format!("{}-{}-1", docker.compose_project_name(), service)
        }
        None => docker.container_name(),
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

/// Run `check`, cancellable via `cancels`. `CheckStarted` is sent once
/// registered, so a check shown as running can always be cancelled.
async fn run_check_with_target(
    check: &CheckToRun,
    project_root: &Path,
    target: &ExecTarget,
    event_tx: &mpsc::Sender<RunnerEvent>,
    executor: &dyn CommandExecutor,
    cancels: &CancelRegistry,
    max_output_lines: usize,
) -> CheckResult {
    let run = async {
        let _ = event_tx
            .send(RunnerEvent::CheckStarted {
                check_id: check.id().to_string(),
            })
            .await;
        let sink = OutputSink::new(check.id(), event_tx.clone());
        run_check_with_command_with_executor(
            check,
            &check.resolved_command,
            project_root,
            target,
            executor,
            max_output_lines,
            &sink,
        )
        .await
    };
    run_check_cancellable(cancels, check.id(), run).await
}

/// Execute a command on `target` and return the result (for testing with executor).
///
/// `container` overrides the docker default container (ignored in local mode).
/// `check_env` is merged over the target's global env (check env wins).
/// `timeout` bounds the run (`None` = unbounded); expiry yields
/// [`CheckStatus::TimedOut`] with the reason in `error_output`.
/// `max_output_lines` caps stored stdout/stderr to their last N lines each
/// (see [`truncate_output`]), bounding *retained* memory for a runaway
/// command. Peak memory during capture is unbounded until the command exits
/// or times out — output is buffered in full before truncation.
/// Live output chunks stream to `sink` while the command runs.
#[allow(clippy::too_many_arguments)]
pub async fn execute_command_with_executor(
    check_id: String,
    command: &str,
    project_root: &std::path::Path,
    container: Option<&str>,
    target: &ExecTarget,
    check_env: &HashMap<String, String>,
    executor: &dyn CommandExecutor,
    timeout: Option<Duration>,
    max_output_lines: usize,
    sink: &OutputSink,
) -> CheckResult {
    let started_at = chrono::Local::now();
    let start = std::time::Instant::now();

    let env = merged_env(target, check_env);
    let full_cmd = target.build_command(container, &env, command, executor);
    let output = execute_with_timeout(executor, &full_cmd, project_root, timeout, sink).await;

    let duration_ms = start.elapsed().as_millis() as u64;
    let finished_at = chrono::Local::now();

    let (status, stdout, stderr) = match output {
        Ok(out) if out.success => (CheckStatus::Passed, out.stdout, out.stderr),
        Ok(out) => (CheckStatus::Failed, out.stdout, out.stderr),
        Err(message) => (CheckStatus::TimedOut, String::new(), message),
    };

    CheckResult {
        check_id,
        status,
        output: truncate_output(stdout, max_output_lines),
        error_output: truncate_output(filter_docker_warnings(&stderr), max_output_lines),
        duration_ms,
        started_at: Some(started_at),
        finished_at: Some(finished_at),
    }
}

/// Run a single check (for retry single). Target env + check env apply.
pub async fn run_single_check(
    check: &CheckToRun,
    project_root: &std::path::Path,
    target: &ExecTarget,
    max_output_lines: usize,
) -> CheckResult {
    run_single_check_with_executor(
        check,
        project_root,
        target,
        &RealCommandExecutor,
        max_output_lines,
    )
    .await
}

/// Run a single check with a custom executor (test-facing).
///
/// Behaviour matches [`run_single_check`] except the caller supplies the executor.
pub async fn run_single_check_with_executor(
    check: &CheckToRun,
    project_root: &std::path::Path,
    target: &ExecTarget,
    executor: &dyn CommandExecutor,
    max_output_lines: usize,
) -> CheckResult {
    run_check_with_command_with_executor(
        check,
        &check.resolved_command,
        project_root,
        target,
        executor,
        max_output_lines,
        &OutputSink::none(),
    )
    .await
}

/// Run a fix command. `container` overrides the docker default container;
/// target env applies.
pub async fn run_fix_command(
    fix_command: &str,
    project_root: &std::path::Path,
    container: Option<&str>,
    target: &ExecTarget,
    max_output_lines: usize,
) -> CheckResult {
    run_fix_command_with_executor(
        fix_command,
        project_root,
        container,
        target,
        &RealCommandExecutor,
        max_output_lines,
    )
    .await
}

/// Run a fix command with a custom executor (test-facing).
pub async fn run_fix_command_with_executor(
    fix_command: &str,
    project_root: &std::path::Path,
    container: Option<&str>,
    target: &ExecTarget,
    executor: &dyn CommandExecutor,
    max_output_lines: usize,
) -> CheckResult {
    execute_command_with_executor(
        "fix".to_string(),
        fix_command,
        project_root,
        container,
        target,
        &HashMap::new(),
        executor,
        None,
        max_output_lines,
        &OutputSink::none(),
    )
    .await
}

/// Run a check with a custom command, streaming live output to `sink` as
/// [`RunnerEvent::CheckOutput`] chunks while the command runs.
///
/// Used e.g. when running a check for "all files" by removing the {files}
/// placeholder from the command. Check container override and env apply.
pub async fn run_check_with_command_with_executor(
    check: &CheckToRun,
    command: &str,
    project_root: &std::path::Path,
    target: &ExecTarget,
    executor: &dyn CommandExecutor,
    max_output_lines: usize,
    sink: &OutputSink,
) -> CheckResult {
    execute_command_with_executor(
        check.id().to_string(),
        command,
        project_root,
        check.definition.container.as_deref(),
        target,
        &check.definition.env,
        executor,
        check.definition.timeout,
        max_output_lines,
        sink,
    )
    .await
}

/// Build a host-local command: optional `env K='V'...` prefix + `shell -c` wrapper.
///
/// Invalid env keys are skipped (unquotable → shell injection), as in
/// [`build_docker_exec_command`].
fn build_local_command(
    local: &crate::config::LocalConfig,
    env: &HashMap<String, String>,
    command: &str,
) -> String {
    let env_flags: String = env_assignments(env).map(|a| format!("{a} ")).collect();
    let prefix = if env_flags.is_empty() {
        String::new()
    } else {
        format!("env {env_flags}")
    };
    format!("{}{} -c {}", prefix, local.shell, single_quote(command))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn test_read_some_propagates_pipe_error() {
        let pipe = tokio_test::io::Builder::new()
            .read_error(std::io::Error::other("pipe broke"))
            .build();
        let mut buf = Vec::new();

        let err = read_some(&mut Some(pipe), &mut buf).await.unwrap_err();

        assert_eq!(err.to_string(), "pipe broke");
    }

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

    #[test]
    fn test_truncate_output_under_cap_is_unchanged() {
        let text = "a\nb\nc";
        assert_eq!(truncate_output(text.to_string(), 10), text);
    }

    #[test]
    fn test_truncate_output_at_cap_is_unchanged() {
        let text = "a\nb\nc";
        assert_eq!(truncate_output(text.to_string(), 3), text);
    }

    #[test]
    fn test_truncate_output_over_cap_keeps_last_n_with_marker() {
        let text = "1\n2\n3\n4\n5";
        let result = truncate_output(text.to_string(), 2);
        assert_eq!(result, "… 3 lines truncated\n4\n5");
    }

    #[test]
    fn test_truncate_output_empty_text_is_unchanged() {
        assert_eq!(truncate_output(String::new(), 5), "");
    }

    #[test]
    fn test_append_output_under_cap_appends() {
        let mut buf = "a\n".to_string();
        append_output(&mut buf, "b\nc\n", 3);
        assert_eq!(buf, "a\nb\nc\n");
    }

    #[test]
    fn test_append_output_over_cap_keeps_rolling_tail() {
        let mut buf = "1\n2\n".to_string();
        append_output(&mut buf, "3\n4\n5\n", 2);
        assert_eq!(buf, "… 3 lines truncated\n4\n5\n");
    }

    #[test]
    fn test_append_output_marker_count_accumulates() {
        let mut buf = String::new();
        append_output(&mut buf, "1\n2\n3\n", 2);
        append_output(&mut buf, "4\n5\n", 2);
        assert_eq!(buf, "… 3 lines truncated\n4\n5\n");
    }

    #[test]
    fn test_filter_docker_warnings_unchanged() {
        let stderr = "WARN: X variable is not set. Defaulting to a blank string.\nreal\n";
        assert_eq!(filter_docker_warnings(stderr), "real");
    }

    #[test]
    fn test_build_docker_run_command_skips_invalid_env_keys() {
        let docker: crate::config::DockerConfig =
            serde_yaml::from_str("project_dir: .\nservice: app\nshell: bash\nimage: img\n")
                .unwrap();
        let env = HashMap::from([("BAD;rm -rf /".to_string(), "x".to_string())]);
        let cmd = build_docker_run_command(&docker, &env, "ls", "ci-tui-test-1");
        assert!(!cmd.contains("BAD"), "got: {cmd}");
        assert!(!cmd.contains(" -e "), "got: {cmd}");
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
        let cmd =
            local_target().build_command(Some("ignored"), &HashMap::new(), "cargo test", &mock);
        assert_eq!(cmd, BuiltCommand::plain("bash -c 'cargo test'"));
    }

    #[test]
    fn test_build_command_docker_running_uses_exec() {
        let mut mock = MockCommandExecutor::new();
        mock.expect_is_container_running().returning(|_| true);
        let cmd = docker_target().build_command(Some("proj-app-1"), &HashMap::new(), "ls", &mock);
        assert!(
            cmd.command.starts_with("docker exec proj-app-1"),
            "got: {cmd:?}"
        );
        assert_eq!(cmd.container, None, "exec starts no container");
    }

    #[test]
    fn test_build_command_docker_stopped_uses_run() {
        let mut mock = MockCommandExecutor::new();
        mock.expect_is_container_running().returning(|_| false);
        let cmd = docker_target().build_command(Some("proj-app-1"), &HashMap::new(), "ls", &mock);
        assert!(cmd.command.starts_with("docker run --rm"), "got: {cmd:?}");
    }

    #[test]
    fn test_build_command_docker_run_gets_unique_name() {
        let mut mock = MockCommandExecutor::new();
        mock.expect_is_container_running().returning(|_| false);
        let build = || docker_target().build_command(None, &HashMap::new(), "ls", &mock);
        let (first, second) = (build(), build());

        let name = first.container.clone().expect("docker run is named");
        assert!(name.starts_with("ci-tui-"), "got: {name}");
        assert!(
            first.command.contains(&format!("--name {name} ")),
            "got: {first:?}"
        );
        assert_ne!(first.container, second.container, "names must be unique");
    }

    #[test]
    fn test_cancelled_is_not_failure() {
        assert!(!CheckStatus::Cancelled.is_failure());
    }

    #[test]
    fn test_build_command_docker_defaults_to_config_container() {
        let mut mock = MockCommandExecutor::new();
        mock.expect_is_container_running()
            .withf(|name| name == "explicit-c")
            .returning(|_| true);
        let yaml = "project_dir: .\nservice: app\nshell: bash\ncontainer: explicit-c\n";
        let target = ExecTarget::Docker(serde_yaml::from_str(yaml).unwrap());
        let cmd = target.build_command(None, &HashMap::new(), "ls", &mock);
        assert!(
            cmd.command.starts_with("docker exec explicit-c"),
            "got: {cmd:?}"
        );
    }

    #[test]
    fn test_config_runner_local_env() {
        let yaml = "version: 2\nrunner: local\nlocal:\n  env:\n    A: b\ngit:\n  base_branch: main\n  fallback_branch: HEAD~1\nfile_patterns: {}\nchecks: {}\n";
        let config: crate::config::CiConfig = serde_yaml::from_str(yaml).unwrap();
        assert!(matches!(config.runner, ExecTarget::Local(_)));
        assert_eq!(config.runner.env().get("A").unwrap(), "b");
    }

    #[test]
    fn test_build_local_command_custom_shell() {
        let local: crate::config::LocalConfig = serde_yaml::from_str("shell: /bin/sh").unwrap();
        let cmd = build_local_command(&local, &HashMap::new(), "ls");
        assert_eq!(cmd, "/bin/sh -c 'ls'");
    }
}
