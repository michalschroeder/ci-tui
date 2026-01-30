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
