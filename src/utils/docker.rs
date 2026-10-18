//! Docker container utilities.

/// Id of the first running container of compose `service`: `docker compose
/// --project-directory <project_dir> ps -q --status running <service>`, run
/// in `working_dir` (relative `project_dir` resolves like the check commands).
///
/// Returns `None` if no container of the service runs, or the command fails
/// (no docker / compose, unknown service, invalid project).
pub(crate) fn compose_running(
    project_dir: &str,
    service: &str,
    working_dir: &std::path::Path,
) -> Option<String> {
    let output = super::own_process_group(&mut std::process::Command::new("docker"))
        .args(["compose", "--project-directory", project_dir])
        .args(["ps", "-q", "--status", "running", service])
        .current_dir(working_dir)
        .stdin(std::process::Stdio::null())
        .output()
        .ok()?;
    if !output.status.success() {
        return None;
    }
    first_id(&String::from_utf8_lossy(&output.stdout))
}

/// First container id (non-empty line) of `docker compose ps -q` output;
/// scaled services list several, the first is used
fn first_id(stdout: &str) -> Option<String> {
    stdout
        .lines()
        .map(str::trim)
        .find(|line| !line.is_empty())
        .map(str::to_owned)
}

/// `docker kill` a container, fire-and-forget (best effort: errors, e.g.
/// the container already gone, are ignored).
///
/// Not waited on: called from drop guards on tokio workers, which must not
/// block. The client is reaped on a helper thread; it outlives our exit, so
/// the kill still reaches the daemon when quitting.
pub(crate) fn kill(container_name: &str) {
    let child = super::own_process_group(&mut std::process::Command::new("docker"))
        .args(["kill", container_name])
        .stdin(std::process::Stdio::null())
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .spawn();
    if let Ok(mut child) = child {
        std::thread::spawn(move || child.wait());
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_first_id_takes_first_non_empty_line() {
        assert_eq!(first_id("\n abc123 \ndef456\n"), Some("abc123".into()));
        assert_eq!(first_id(""), None);
        assert_eq!(first_id(" \n"), None);
    }

    #[test]
    fn test_unknown_project_is_not_running() {
        // No compose project here (or no docker at all): treated as stopped
        let dir = std::env::temp_dir().join("ci-tui-test-no-compose-project-12345");
        let running = compose_running(dir.to_str().unwrap(), "app", &std::env::temp_dir());
        assert_eq!(running, None);
    }
}
