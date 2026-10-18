//! Docker container utilities.

/// Id of the first running service container of compose `service`: `docker
/// compose --project-directory <project_dir> ps --status running --no-trunc
/// --format <id + oneoff label> <service>`, run in `working_dir` (relative
/// `project_dir` resolves like the check commands). One-off `docker compose
/// run` containers are skipped (see [`first_service_id`]).
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
        .args(["ps", "--status", "running", "--no-trunc", "--format"])
        .arg(format!("{{{{.ID}}}} {{{{.Label \"{ONE_OFF_LABEL}\"}}}}"))
        .arg(service)
        .current_dir(working_dir)
        .stdin(std::process::Stdio::null())
        .output()
        .ok()?;
    if !output.status.success() {
        return None;
    }
    first_service_id(&String::from_utf8_lossy(&output.stdout))
}

/// Compose label, `True` on `docker compose run` containers
const ONE_OFF_LABEL: &str = "com.docker.compose.oneoff";

/// First container id of [`compose_running`]'s `<id> <oneoff>` lines that
/// is not a one-off (`True`); scaled services list several, the first is
/// used. One-off containers are skipped because `docker compose ps` lists
/// them too: another ci-tui's (or our own) `docker compose run --rm`
/// container, removed when its command ends, killing any check exec'd into
/// it mid-run (#235).
fn first_service_id(stdout: &str) -> Option<String> {
    stdout
        .lines()
        .filter_map(|line| {
            let mut fields = line.split_whitespace();
            Some((fields.next()?, fields.next()))
        })
        .find(|&(_, one_off)| one_off != Some("True"))
        .map(|(id, _)| id.to_owned())
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
    fn test_first_service_id_takes_first_non_empty_line() {
        assert_eq!(
            first_service_id("\n abc123 False\ndef456 False\n"),
            Some("abc123".into())
        );
        assert_eq!(first_service_id(""), None);
        assert_eq!(first_service_id(" \n"), None);
    }

    // A `docker compose run` container (another ci-tui's check, or ours)
    // is removed when its command ends: exec'ing into it fails mid-check
    #[test]
    fn test_first_service_id_skips_one_off_containers() {
        assert_eq!(
            first_service_id("oneoff1 True\nsvc2 False\n"),
            Some("svc2".into())
        );
        assert_eq!(first_service_id("oneoff1 True\n"), None);
        // No label (container not created by compose's service path)
        assert_eq!(first_service_id("svc3\n"), Some("svc3".into()));
    }

    #[test]
    fn test_unknown_project_is_not_running() {
        // No compose project here (or no docker at all): treated as stopped
        let dir = std::env::temp_dir().join("ci-tui-test-no-compose-project-12345");
        let running = compose_running(dir.to_str().unwrap(), "app", &std::env::temp_dir());
        assert_eq!(running, None);
    }
}
