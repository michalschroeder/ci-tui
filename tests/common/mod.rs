//! Shared test utilities and mock helpers

// Re-export for convenience
pub use ci_tui::git::MockGitExecutor;
pub use ci_tui::runner::{CommandOutput, MockCommandExecutor};

/// Create a mock that returns the given output for any command
#[allow(dead_code)]
pub fn mock_git_with_output(output: &str) -> MockGitExecutor {
    let output = output.to_string();
    let mut mock = MockGitExecutor::new();
    mock.expect_run_command()
        .returning(move |_, _| Ok(output.clone()));
    mock
}

/// Create a mock that returns an error
#[allow(dead_code)]
pub fn mock_git_error(error_msg: &str) -> MockGitExecutor {
    let error_msg = error_msg.to_string();
    let mut mock = MockGitExecutor::new();
    mock.expect_run_command()
        .returning(move |_, _| Err(anyhow::anyhow!("{}", error_msg)));
    mock
}

/// Create a mock executor that returns success
pub fn mock_executor_success(stdout: &str) -> MockCommandExecutor {
    let stdout = stdout.to_string();
    let mut mock = MockCommandExecutor::new();
    mock.expect_execute().returning(move |_, _| {
        let out = stdout.clone();
        CommandOutput {
            success: true,
            stdout: out,
            stderr: String::new(),
        }
    });
    mock.expect_is_container_running().returning(|_| true);
    mock
}

/// Create a mock executor that returns failure
#[allow(dead_code)]
pub fn mock_executor_failure(stderr: &str) -> MockCommandExecutor {
    let stderr = stderr.to_string();
    let mut mock = MockCommandExecutor::new();
    mock.expect_execute().returning(move |_, _| {
        let err = stderr.clone();
        CommandOutput {
            success: false,
            stdout: String::new(),
            stderr: err,
        }
    });
    mock.expect_is_container_running().returning(|_| true);
    mock
}
