//! Startup sanity probe for docker mode.
//!
//! Checks get repo-relative paths via `{files}`, which only resolve if the
//! repo is visible where the command runs in the container (the container's
//! WORKDIR for `docker exec`, `docker.work_dir` for `docker run`). A mismatch would
//! otherwise surface only as "file not found" inside checks, so warn up front.
//!
//! Warnings are advisory: callers print/show them and run normally. An
//! unreachable Docker daemon is not: [`docker_reachable`] fails the run
//! (exit 3) before any check starts.

use crate::checks::CheckToRun;
use crate::config::DockerConfig;
use crate::runner::{build_docker_exec_command, CommandExecutor, ExecTarget, OutputSink};
use crate::utils::shell::quote;
use std::collections::HashMap;
use std::path::Path;

/// Probe run by [`docker_reachable`]: needs the daemon, answers fast when it is up
const DOCKER_PROBE: &str = "docker version --format '{{.Server.Version}}'";

/// Err when checks would run in docker mode but the Docker CLI / daemon is
/// unreachable (first stderr line of the probe as cause). No probe in local
/// mode or with no selected checks.
pub async fn docker_reachable(
    target: &ExecTarget,
    checks: &[CheckToRun],
    root: &Path,
    executor: &dyn CommandExecutor,
) -> anyhow::Result<()> {
    if !matches!(target, ExecTarget::Docker(_)) || checks.is_empty() {
        return Ok(());
    }
    let output = executor
        .execute(DOCKER_PROBE, root, &OutputSink::none())
        .await;
    if output.success {
        return Ok(());
    }
    let cause = output
        .stderr
        .lines()
        .map(str::trim)
        .find(|l| !l.is_empty())
        .unwrap_or("no output");
    anyhow::bail!(
        "Docker is not reachable (`docker version` failed: {cause}). \
         Start Docker, or set `runner: local` to run checks on the host."
    )
}

/// Warnings for containers where changed files likely won't resolve; empty
/// when fine or not applicable.
///
/// - local mode, no selected checks or no changed files: no probe
/// - each distinct container the checks use (`container:` override, else the
///   default) is probed once:
///   - running: `test -e <file>` via `docker exec` on the first changed file
///     present on the host; warn if missing (first stderr line appended)
///   - not running: checks use the `docker run` fallback, whose mount is
///     checked once (see [`run_fallback_warning`])
pub async fn docker_warnings(
    target: &ExecTarget,
    checks: &[CheckToRun],
    files: &[String],
    root: &Path,
    executor: &dyn CommandExecutor,
) -> Vec<String> {
    let ExecTarget::Docker(docker) = target else {
        return Vec::new();
    };
    if checks.is_empty() || files.is_empty() {
        return Vec::new();
    }
    let mut containers: Vec<String> = Vec::new();
    for check in checks {
        let name = check
            .definition
            .container
            .clone()
            .unwrap_or_else(|| docker.container_name());
        if !containers.contains(&name) {
            containers.push(name);
        }
    }

    let mut warnings = Vec::new();
    let mut stopped = Vec::new();
    for container in &containers {
        if executor.is_container_running(container) {
            warnings.extend(exec_probe(docker, container, files, root, executor).await);
        } else {
            stopped.push(container.as_str());
        }
    }
    if !stopped.is_empty() {
        warnings.extend(run_fallback_warning(docker, &stopped.join("`, `")));
    }
    warnings
}

/// `test -e <file>` in running `container`; warning if the file is missing
async fn exec_probe(
    docker: &DockerConfig,
    container: &str,
    files: &[String],
    root: &Path,
    executor: &dyn CommandExecutor,
) -> Option<String> {
    let file = files.iter().find(|f| root.join(f).exists())?;
    let probe = format!("test -e {}", quote(file));
    let command = build_docker_exec_command(container, &HashMap::new(), &probe, &docker.shell);
    let output = executor.execute(&command, root, &OutputSink::none()).await;
    if output.success {
        return None;
    }
    let mut warning = format!(
        "`{file}` not found in `{container}` working directory: mount the repo at the \
         container's WORKDIR"
    );
    // Surface the real cause when exec itself failed (missing shell, container gone)
    if let Some(cause) = output
        .stderr
        .lines()
        .next()
        .map(str::trim)
        .filter(|l| !l.is_empty())
    {
        warning.push_str(&format!(" ({cause})"));
    }
    Some(warning)
}

/// Warning for the `docker run` fallback of stopped `containers`: no
/// `docker.volume_mount`, or its destination (`src:dst[:opts]`) is not
/// `docker.work_dir`, so `{files}` would not resolve.
fn run_fallback_warning(docker: &DockerConfig, containers: &str) -> Option<String> {
    let work_dir = docker.working_dir().trim_end_matches('/');
    let Some(mount) = &docker.volume_mount else {
        return Some(format!(
            "`{containers}` not running, `docker run` fallback has no repo mount: \
             set `docker.volume_mount` (e.g. `.:{work_dir}`, matching `docker.work_dir`)"
        ));
    };
    let dst = mount.split(':').nth(1)?.trim_end_matches('/');
    (dst != work_dir).then(|| {
        format!(
            "`{containers}` not running, `docker run` fallback mounts the repo at `{dst}` \
             but runs in `docker.work_dir` `{work_dir}`: make them match"
        )
    })
}
