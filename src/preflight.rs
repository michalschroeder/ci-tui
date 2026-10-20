//! Startup sanity probe for docker mode.
//!
//! Checks get repo-relative paths via `{files}`, which only resolve if the
//! repo is visible where the command runs in the container (the running
//! container's WORKDIR for `docker exec`, `docker.work_dir` for plain-image
//! `docker run`; `docker compose run` takes volumes / working_dir from the
//! compose file, not checked). A mismatch would otherwise surface only as
//! "file not found" inside checks, so warn up front.
//!
//! Warnings are advisory: callers print/show them and run normally. An
//! unreachable Docker daemon is not: [`docker_reachable`] fails the run
//! (exit 3) before any check starts.

use crate::checks::CheckToRun;
use crate::config::{DockerConfig, DockerMode, ImageConfig};
use crate::runner::{
    build_docker_exec_command, compose_placement, CommandExecutor, DockerPlacement, ExecTarget,
    OutputSink, RealCommandExecutor,
};
use crate::utils::shell::quote;
use std::collections::HashMap;
use std::path::Path;
use std::time::Duration;

/// Probe run by [`docker_reachable`]: fails when the daemon is down. No
/// `--format`: podman's docker shim may not fill the server fields.
const DOCKER_PROBE: &str = "docker version";

/// How long [`docker_reachable`] waits for the probe (daemon starting,
/// unreachable remote `DOCKER_HOST`)
const DOCKER_PROBE_TIMEOUT: Duration = Duration::from_secs(10);

/// Startup Docker checks with the real executor: [`docker_reachable`]
/// (fatal), then the [`docker_warnings`] for `checks` and `changed_files`.
pub async fn run(
    target: &ExecTarget,
    needed: bool,
    root: &Path,
    checks: &[CheckToRun],
    changed_files: &[String],
) -> anyhow::Result<Vec<String>> {
    let executor = RealCommandExecutor;
    docker_reachable(target, needed, root, &executor, DOCKER_PROBE_TIMEOUT).await?;
    Ok(docker_warnings(target, checks, changed_files, root, &executor).await)
}

/// Err when commands would run in docker mode (`needed`) but the Docker CLI /
/// daemon is unreachable or does not answer within `timeout` (first stderr
/// line of the probe as cause). No probe in local mode or when not `needed`.
pub async fn docker_reachable(
    target: &ExecTarget,
    needed: bool,
    root: &Path,
    executor: &dyn CommandExecutor,
    timeout: Duration,
) -> anyhow::Result<()> {
    if !matches!(target, ExecTarget::Docker(_)) || !needed {
        return Ok(());
    }
    // On timeout the probe future is dropped, which kills its process group.
    let sink = OutputSink::none();
    let probe = executor.execute(DOCKER_PROBE, root, &sink);
    let Ok(output) = tokio::time::timeout(timeout, probe).await else {
        anyhow::bail!(
            "Docker is not reachable (`docker version` did not answer within {}s). \
             Start Docker, or set `runner: local` to run checks on the host.",
            timeout.as_secs()
        );
    };
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
/// - plain image: its `docker run` mount is checked once (see
///   [`image_mount_warning`])
/// - compose: each distinct service the checks use is resolved once
///   ([`compose_placement`]):
///   - running container: `test -e <file>` via `docker exec` on the first
///     changed file present on the host; warn if missing (first stderr line
///     appended)
///   - stopped (`docker compose run`): nothing to check, the compose file
///     supplies volumes and working_dir
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
    let compose = match &docker.mode {
        // One placement for every check: no per-service lookup
        DockerMode::Image(image) => return image_mount_warning(image).into_iter().collect(),
        DockerMode::Compose(compose) => compose,
    };
    let mut services: Vec<&str> = Vec::new();
    for check in checks {
        let service = compose.service_for(check.service());
        if !services.contains(&service) {
            services.push(service);
        }
    }

    let mut warnings = Vec::new();
    for service in services {
        if let DockerPlacement::Exec(id) = compose_placement(compose, service, root, executor) {
            warnings.extend(exec_probe(docker, service, &id, files, root, executor).await);
        }
    }
    warnings
}

/// `test -e <file>` in running container `id` of `service`; warning if the
/// file is missing
async fn exec_probe(
    docker: &DockerConfig,
    service: &str,
    id: &str,
    files: &[String],
    root: &Path,
    executor: &dyn CommandExecutor,
) -> Option<String> {
    let file = files.iter().find(|f| root.join(f).exists())?;
    let probe = format!("test -e {}", quote(file));
    let command = build_docker_exec_command(id, &HashMap::new(), &probe, &docker.shell);
    let output = executor.execute(&command, root, &OutputSink::none()).await;
    if output.success {
        return None;
    }
    let mut warning = format!(
        "`{file}` not found in service `{service}` working directory (container `{id}`): \
         mount the repo at the container's WORKDIR"
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

/// Warning for plain-image `docker run` of `config.image`: no
/// `docker.volume_mount`, or its destination (`src:dst[:opts]`) is not
/// `docker.work_dir`, so `{files}` would not resolve.
fn image_mount_warning(config: &ImageConfig) -> Option<String> {
    let work_dir = config.working_dir().trim_end_matches('/');
    let image = &config.image;
    let Some(mount) = &config.volume_mount else {
        return Some(format!(
            "`docker run` of `{image}` has no repo mount: set `docker.volume_mount` \
             (e.g. `.:{work_dir}`, matching `docker.work_dir`)"
        ));
    };
    let dst = mount.split(':').nth(1)?.trim_end_matches('/');
    (dst != work_dir).then(|| {
        format!(
            "`docker run` of `{image}` mounts the repo at `{dst}` but runs in \
             `docker.work_dir` `{work_dir}`: make them match"
        )
    })
}
