//! Docker container utilities.

/// Check if a Docker container is currently running.
///
/// Returns `false` if:
/// - Container doesn't exist
/// - Container exists but is not running
/// - Docker command fails
pub(crate) fn is_running(container_name: &str) -> bool {
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

/// `docker kill` a container, fire-and-forget (best effort: errors, e.g.
/// the container already gone, are ignored).
///
/// Not waited on: called from drop guards on tokio workers, which must not
/// block. The client is reaped on a helper thread; it outlives our exit, so
/// the kill still reaches the daemon when quitting.
pub(crate) fn kill(container_name: &str) {
    let child = std::process::Command::new("docker")
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
    fn test_non_existent_container_returns_false() {
        // This container name should never exist
        assert!(!is_running("ci-tui-test-nonexistent-container-12345"));
    }

    // Note: Testing with a real running container requires Docker setup
    // and is better suited for integration tests
}
