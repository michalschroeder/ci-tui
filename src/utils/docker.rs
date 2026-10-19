//! Docker container utilities.

/// Id of the first running service container of compose `service`: `docker
/// [`compose_ps_args`]`, run in `working_dir` (relative `project_dir`
/// resolves like the check commands).
///
/// Returns `None` if no container of the service runs, or the command fails
/// (no docker / compose, unknown service, invalid project).
pub(crate) fn compose_running(
    project_dir: &str,
    service: &str,
    working_dir: &std::path::Path,
) -> Option<String> {
    let output = super::own_process_group(&mut std::process::Command::new("docker"))
        .args(compose_ps_args(project_dir, service))
        .current_dir(working_dir)
        .stdin(std::process::Stdio::null())
        .output()
        .ok()?;
    if !output.status.success() {
        return None;
    }
    first_service_id(&String::from_utf8_lossy(&output.stdout))
}

/// `docker` args listing running containers of `service`, one `<id>
/// <oneoff label>` line each. `--status running` skips paused / restarting
/// ones (`docker exec` fails on them) but also lists one-off `docker
/// compose run` containers, filtered by [`first_service_id`].
fn compose_ps_args<'a>(project_dir: &'a str, service: &'a str) -> [&'a str; 10] {
    [
        "compose",
        "--project-directory",
        project_dir,
        "ps",
        "--status",
        "running",
        "--no-trunc",
        "--format",
        r#"{{.ID}} {{.Label "com.docker.compose.oneoff"}}"#,
        service,
    ]
}

/// First id of [`compose_ps_args`] output that is not a one-off (`True`);
/// scaled services list several, the first is used. One-off `docker compose
/// run` containers (another ci-tui's `--rm` check, or a user's shell) are
/// never exec targets: a `--rm` one is removed when its command ends,
/// killing a check exec'd into it (#235).
fn first_service_id(stdout: &str) -> Option<String> {
    stdout.lines().find_map(|line| {
        let mut fields = line.split_whitespace();
        let id = fields.next()?;
        (fields.next() != Some("True")).then(|| id.to_owned())
    })
}

/// `<uid>:<gid>` of this process (real ids), for `docker.user: host`.
/// Declared directly (std links libc but exposes no getuid): no `libc`
/// crate dependency.
pub(crate) fn host_user() -> String {
    extern "C" {
        fn getuid() -> u32;
        fn getgid() -> u32;
    }
    // SAFETY: POSIX getuid / getgid take no arguments, always succeed and
    // touch no memory; uid_t / gid_t are u32 on Linux and macOS.
    let (uid, gid) = unsafe { (getuid(), getgid()) };
    format!("{uid}:{gid}")
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

    // Dropping `--status running` would pick paused / restarting containers;
    // dropping the label column would exec into one-off ones (#235)
    #[test]
    fn test_compose_ps_args_filter_running_and_print_one_off_label() {
        assert_eq!(
            compose_ps_args("/proj", "app").join(" "),
            "compose --project-directory /proj ps --status running --no-trunc \
             --format {{.ID}} {{.Label \"com.docker.compose.oneoff\"}} app"
        );
    }

    /// Real ids, as `id -ru` / `id -rg` report them for this process
    #[test]
    fn test_host_user_is_process_uid_gid() {
        let id = |flag: &str| {
            let out = std::process::Command::new("id").arg(flag).output().unwrap();
            String::from_utf8(out.stdout).unwrap().trim().to_owned()
        };
        assert_eq!(host_user(), format!("{}:{}", id("-ru"), id("-rg")));
    }

    #[test]
    fn test_unknown_project_is_not_running() {
        // No compose project here (or no docker at all): treated as stopped
        let dir = std::env::temp_dir().join("ci-tui-test-no-compose-project-12345");
        let running = compose_running(dir.to_str().unwrap(), "app", &std::env::temp_dir());
        assert_eq!(running, None);
    }
}
