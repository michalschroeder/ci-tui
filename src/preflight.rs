//! Startup sanity probe for docker mode.
//!
//! Checks get repo-relative paths via `{files}`, which only resolve if the
//! repo is visible where the command runs in the container (the container's
//! WORKDIR for `docker exec`, `docker.work_dir` for `docker run`). A mismatch would
//! otherwise surface only as "file not found" inside checks, so warn up front.
//!
//! Warnings are advisory: callers print/show them and run normally.

use crate::runner::{build_docker_exec_command, CommandExecutor, ExecTarget, OutputSink};
use crate::utils::shell::quote;
use std::collections::HashMap;
use std::path::Path;

/// Warning when changed files likely won't resolve inside the default docker
/// container; `None` when fine or not applicable.
///
/// - local mode or no changed files: `None`
/// - container running: `test -e <file>` via `docker exec` on the first changed
///   file present on the host; warn if missing
/// - container not running and no `docker.volume_mount`: warn
pub async fn docker_warning(
    target: &ExecTarget,
    files: &[String],
    root: &Path,
    executor: &dyn CommandExecutor,
) -> Option<String> {
    let ExecTarget::Docker(docker) = target else {
        return None;
    };
    if files.is_empty() {
        return None;
    }
    let container = docker.container_name();
    if !executor.is_container_running(&container) {
        let work_dir = docker.working_dir().trim_end_matches('/');
        return docker.volume_mount.is_none().then(|| {
            format!(
                "`{container}` not running, `docker run` fallback has no repo mount: \
                 set `docker.volume_mount` (e.g. `.:{work_dir}`, matching `docker.work_dir`)"
            )
        });
    }

    let file = files.iter().find(|f| root.join(f).exists())?;
    let probe = format!("test -e {}", quote(file));
    let command = build_docker_exec_command(&container, &HashMap::new(), &probe, &docker.shell);
    let output = executor.execute(&command, root, &OutputSink::none()).await;
    (!output.success).then(|| {
        format!(
            "`{file}` not found in `{container}` working directory: mount the repo at the \
             container's WORKDIR"
        )
    })
}
