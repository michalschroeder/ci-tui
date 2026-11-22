//! Startup sanity probe for docker mode.
//!
//! Checks get paths relative to the project dir (the execution root) via
//! `{files}`, which only resolve if the project is visible where the command
//! runs in the container (the running container's WORKDIR for `docker exec`,
//! `docker.work_dir` for plain-image `docker run`; `docker compose run` takes
//! volumes / working_dir from the compose file, not checked). A mismatch
//! would otherwise surface only as "file not found" inside checks, so warn up
//! front.
//!
//! Warnings are advisory: callers print/show them and run normally. A setup
//! that cannot start any container is not: [`run_with`] fails the run (exit
//! 3, with the cause) before any check starts when an unset or empty
//! variable leaves a `docker.volume_mount` section empty, the Docker daemon
//! is unreachable ([`docker_reachable`]), `docker compose config` fails for
//! `docker.project_dir` (no / invalid compose file, compose missing, ...), or
//! its compose file lacks a service the selected checks or their groups'
//! pre-commands use.

use crate::checks::CheckToRun;
use crate::config::{BlankVar, CiConfig, ComposeConfig, DockerConfig, DockerMode, ImageConfig};
use crate::runner::{
    build_docker_exec_command, resolve_service, CommandExecutor, CommandOutput, DockerPlacement,
    ExecTarget, OutputSink, RealCommandExecutor,
};
use crate::utils::shell::quote;
use std::collections::HashMap;
use std::path::Path;
use std::time::Duration;
use tokio::time::error::Elapsed;

/// Probe run by [`docker_reachable`]: fails when the daemon is down. No
/// `--format`: podman's docker shim may not fill the server fields.
const DOCKER_PROBE: &str = "docker version";

/// How long [`docker_reachable`] waits for the probe (daemon starting,
/// unreachable remote `DOCKER_HOST`)
const DOCKER_PROBE_TIMEOUT: Duration = Duration::from_secs(10);

/// Startup Docker checks with the real executor (see [`run_with`])
pub async fn run(
    config: &CiConfig,
    needed: bool,
    root: &Path,
    checks: &[CheckToRun],
    changed_files: &[String],
) -> anyhow::Result<Vec<String>> {
    let executor = &RealCommandExecutor;
    run_with(
        config,
        needed,
        root,
        checks,
        changed_files,
        executor,
        DOCKER_PROBE_TIMEOUT,
    )
    .await
}

/// Startup Docker checks via `executor`, fatal ones only when commands would
/// run in docker mode (`needed`): plain image [`mount_vars_set`] (no docker
/// command), [`docker_reachable`], compose [`compose_services`] (probes wait
/// `timeout`); then the [`docker_warnings`] for `checks` and `changed_files`.
pub async fn run_with(
    config: &CiConfig,
    needed: bool,
    root: &Path,
    checks: &[CheckToRun],
    changed_files: &[String],
    executor: &dyn CommandExecutor,
    timeout: Duration,
) -> anyhow::Result<Vec<String>> {
    let target = &config.runner;
    let mode = match target {
        ExecTarget::Docker(docker) if needed => Some(&docker.mode),
        _ => None,
    };
    if let Some(DockerMode::Image(image)) = mode {
        mount_vars_set(image)?;
    }
    docker_reachable(target, needed, root, executor, timeout).await?;
    if let Some(DockerMode::Compose(compose)) = mode {
        let uses = service_uses(config, compose, checks);
        compose_services(compose, &uses, root, executor, timeout).await?;
    }
    Ok(docker_warnings(target, checks, changed_files, root, executor).await)
}

/// Err naming every unset or empty `${VAR}` that leaves a
/// `docker.volume_mount` section empty ([`ImageConfig::blank_mount_vars`]):
/// `docker run` refuses to start (`invalid spec: :/app`), failing every
/// check the same way.
fn mount_vars_set(image: &ImageConfig) -> anyhow::Result<()> {
    let blank = image.blank_mount_vars();
    if blank.is_empty() {
        return Ok(());
    }
    let uses: Vec<String> = blank
        .iter()
        .map(|var| format!("`${{{}}}`, which is {}", var.name(), var.problem()))
        .collect();
    let mount = image.volume_mount.as_deref().unwrap_or_default();
    let source = mount.split(':').next().unwrap_or_default();
    anyhow::bail!(
        "docker.volume_mount uses {} ({})",
        uses.join(", and "),
        mount_hint(&blank, source)
    )
}

/// How to fix `blank`: `NAME=$PWD` only for the var that is the whole mount
/// `source` (the project dir, as in `${HOST_PWD}:/app`); the others are
/// named and asked to be set, their values are not guessable.
fn mount_hint(blank: &[BlankVar], source: &str) -> String {
    let whole = blank
        .iter()
        .find(|var| source == format!("${{{}}}", var.name()));
    let mut hints = Vec::new();
    if let Some(var) = whole {
        hints.push(format!("try `{}=$PWD ci-tui`", var.name()));
    }
    let others: Vec<String> = blank
        .iter()
        .filter(|var| Some(*var) != whole)
        .map(|var| format!("`{}`", var.name()))
        .collect();
    if !others.is_empty() {
        hints.push(format!("set {} before running ci-tui", others.join(", ")));
    }
    hints.join("; ")
}

/// A compose service a selected command runs in, and where it is set
struct ServiceUse<'a> {
    service: &'a str,
    /// e.g. "`service` of check `lint`", "`docker.service`"
    origin: String,
}

impl<'a> ServiceUse<'a> {
    /// Use of `own` service (else `docker.service`) by `command`
    fn new(compose: &'a ComposeConfig, own: Option<&'a str>, command: String) -> Self {
        let origin = match own {
            Some(_) => format!("`service` of {command}"),
            None => "`docker.service`".to_string(),
        };
        let service = compose.service_for(own);
        Self { service, origin }
    }
}

/// Services `checks` (on-demand too: `t` runs them) and the non-`host`
/// pre-commands of their groups run in
fn service_uses<'a>(
    config: &'a CiConfig,
    compose: &'a ComposeConfig,
    checks: &'a [CheckToRun],
) -> Vec<ServiceUse<'a>> {
    let mut uses = Vec::new();
    let mut groups: Vec<&str> = Vec::new();
    for check in checks {
        let command = format!("check `{}`", check.id);
        uses.push(ServiceUse::new(compose, check.service(), command));
        if !groups.contains(&check.group.as_str()) {
            groups.push(&check.group);
        }
    }
    for group in groups {
        for pre in config.pre_commands(group).iter().filter(|p| !p.host) {
            let command = format!("pre-command `{}` of group `{group}`", pre.name);
            uses.push(ServiceUse::new(compose, pre.service.as_deref(), command));
        }
    }
    uses
}

/// Compose probe listing the project's services. Every profile enabled
/// (`'*'` quoted against the shell's glob): `docker compose run` starts a
/// service behind an inactive profile, so it counts as defined.
fn compose_services_probe(project_dir: &str) -> String {
    format!(
        "docker compose --project-directory {} --profile '*' config --services",
        quote(project_dir)
    )
}

/// Err when the compose project in `docker.project_dir` has no usable compose
/// file (missing dir / file, invalid file: the probe's first stderr line as
/// cause), does not answer within `timeout`, or lacks a service in `uses`.
async fn compose_services(
    compose: &ComposeConfig,
    uses: &[ServiceUse<'_>],
    root: &Path,
    executor: &dyn CommandExecutor,
    timeout: Duration,
) -> anyhow::Result<()> {
    let dir = &compose.project_dir;
    let command = compose_services_probe(dir);
    let Ok(output) = probe(&command, root, executor, timeout).await else {
        anyhow::bail!(
            "docker.project_dir `{dir}`: `{command}` did not answer within {}s",
            timeout.as_secs()
        );
    };
    // Neutral: no / invalid compose file, compose not installed, daemon
    // refusing all fail here; compose's own line tells which
    if !output.success {
        anyhow::bail!(
            "docker.project_dir `{dir}`: `docker compose config` failed: {}",
            first_line(&output.stderr)
        );
    }
    let defined: Vec<&str> = output
        .stdout
        .lines()
        .map(str::trim)
        .filter(|l| !l.is_empty())
        .collect();
    let mut missing: Vec<&ServiceUse> = Vec::new();
    for used in uses {
        let known = defined.contains(&used.service);
        if !known && !missing.iter().any(|m| m.service == used.service) {
            missing.push(used);
        }
    }
    if missing.is_empty() {
        return Ok(());
    }
    let missing: Vec<String> = missing
        .iter()
        .map(|m| format!("`{}` ({})", m.service, m.origin))
        .collect();
    anyhow::bail!(
        "the compose file of docker.project_dir `{dir}` has no service {}; it defines: {}",
        missing.join(", "),
        if defined.is_empty() {
            "none".to_string()
        } else {
            defined.join(", ")
        }
    )
}

/// Run probe `command` in `root` (output not streamed), Err after `timeout`
async fn probe(
    command: &str,
    root: &Path,
    executor: &dyn CommandExecutor,
    timeout: Duration,
) -> Result<CommandOutput, Elapsed> {
    // On timeout the probe future is dropped, which kills its process group.
    let sink = OutputSink::none();
    tokio::time::timeout(timeout, executor.execute(command, root, &sink)).await
}

/// First non-empty line of a probe's stderr, trimmed
fn first_line(stderr: &str) -> &str {
    stderr
        .lines()
        .map(str::trim)
        .find(|l| !l.is_empty())
        .unwrap_or("no output")
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
    let Ok(output) = probe(DOCKER_PROBE, root, executor, timeout).await else {
        anyhow::bail!(
            "Docker is not reachable (`docker version` did not answer within {}s). \
             Start Docker, or set `runner: local` to run checks on the host.",
            timeout.as_secs()
        );
    };
    if output.success {
        return Ok(());
    }
    let cause = first_line(&output.stderr);
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
///   ([`resolve_service`]):
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
        if let DockerPlacement::Exec(id) = resolve_service(docker, Some(service), root, executor) {
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
         mount the project dir at the container's WORKDIR"
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
            "`docker run` of `{image}` has no project dir mount: set `docker.volume_mount` \
             (e.g. `.:{work_dir}`, matching `docker.work_dir`)"
        ));
    };
    let dst = mount.split(':').nth(1)?.trim_end_matches('/');
    (dst != work_dir).then(|| {
        format!(
            "`docker run` of `{image}` mounts the project dir at `{dst}` but runs in \
             `docker.work_dir` `{work_dir}`: make them match"
        )
    })
}
