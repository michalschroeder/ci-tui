//! Shared test utilities and mock helpers

// Re-export for convenience
pub use ci_tui::git::MockGitExecutor;

/// Create a mock that returns the given output for any command
pub fn mock_git_with_output(output: &str) -> MockGitExecutor {
    let output = output.to_string();
    let mut mock = MockGitExecutor::new();
    mock.expect_run_command()
        .returning(move |_, _| Ok(output.clone()));
    mock
}

/// Create a mock that returns an error
pub fn mock_git_error(error_msg: &str) -> MockGitExecutor {
    let error_msg = error_msg.to_string();
    let mut mock = MockGitExecutor::new();
    mock.expect_run_command()
        .returning(move |_, _| Err(anyhow::anyhow!("{}", error_msg)));
    mock
}
