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

/// `docker kill` a container, waiting for the client (best effort: errors,
/// e.g. the container already gone, are ignored).
///
/// Blocking on purpose: called from drop guards, possibly while the process
/// is quitting, so the kill must be sent before we exit.
pub(crate) fn kill(container_name: &str) {
    let _ = std::process::Command::new("docker")
        .args(["kill", container_name])
        .stdin(std::process::Stdio::null())
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .status();
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
