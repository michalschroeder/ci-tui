//! Shared test utilities and mock helpers

pub mod configs;

use ci_tui::checks::CheckToRun;
use ci_tui::config::{CheckDefinition, CiConfig, DockerConfig};
use ci_tui::git::ChangedFiles;
use ci_tui::runner::CheckStatus;
use ci_tui::ui::app::App;

// Re-export types used by runner tests
#[allow(unused_imports)]
pub use ci_tui::runner::{CommandOutput, MockCommandExecutor};

/// Create a mock that returns the given output for any command
#[allow(dead_code)]
pub fn mock_git_with_output(output: &str) -> ci_tui::git::MockGitExecutor {
    use ci_tui::git::MockGitExecutor;
    let output = output.to_string();
    let mut mock = MockGitExecutor::new();
    mock.expect_run_command()
        .returning(move |_, _| Ok(output.clone()));
    mock
}

/// Create a mock that returns an error
#[allow(dead_code)]
pub fn mock_git_error(error_msg: &str) -> ci_tui::git::MockGitExecutor {
    use ci_tui::git::MockGitExecutor;
    let error_msg = error_msg.to_string();
    let mut mock = MockGitExecutor::new();
    mock.expect_run_command()
        .returning(move |_, _| Err(anyhow::anyhow!("{}", error_msg)));
    mock
}

/// Create a mock executor that returns success
#[allow(dead_code)]
pub fn mock_executor_success(stdout: &str) -> ci_tui::runner::MockCommandExecutor {
    use ci_tui::runner::{CommandOutput, MockCommandExecutor};
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
pub fn mock_executor_failure(stderr: &str) -> ci_tui::runner::MockCommandExecutor {
    use ci_tui::runner::{CommandOutput, MockCommandExecutor};
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

/// Parse the widget test config (uses shared fixture from configs module)
#[allow(dead_code)]
pub fn parse_widget_config() -> CiConfig {
    configs::widget_test_config()
}

/// Create a CheckToRun for testing
#[allow(dead_code)]
pub fn make_widget_check(id: &str, group: &str, name: &str, has_fix: bool) -> CheckToRun {
    make_check_with_options(id, group, name, has_fix, false)
}

/// Create a CheckToRun with on_demand option for testing
#[allow(dead_code)]
pub fn make_on_demand_check(id: &str, group: &str, name: &str) -> CheckToRun {
    make_check_with_options(id, group, name, false, true)
}

/// Create a CheckToRun with all options for testing
#[allow(dead_code)]
pub fn make_check_with_options(
    id: &str,
    group: &str,
    name: &str,
    has_fix: bool,
    on_demand: bool,
) -> CheckToRun {
    CheckToRun {
        id: id.to_string(),
        group: group.to_string(),
        definition: CheckDefinition {
            name: name.to_string(),
            command: format!("{} --check", id),
            service: None,
            container: None,
            fix_command: if has_fix {
                Some(format!("{} --fix", id))
            } else {
                None
            },
            triggers: None,
            on_demand,
            env: std::collections::HashMap::new(),
        },
        service: "app".to_string(),
        files: vec!["src/main.rs".to_string(), "src/lib.rs".to_string()],
        resolved_command: format!("{} --check src/main.rs src/lib.rs", id),
        resolved_fix_command: if has_fix {
            Some(format!("{} --fix src/main.rs src/lib.rs", id))
        } else {
            None
        },
        on_demand,
        skipped_no_files: false,
    }
}

/// Minimal DockerConfig for mocked-executor tests. Field values are immaterial
/// beyond being stable strings that mocks can match against.
#[allow(dead_code)]
pub fn test_docker_config(image: &str) -> DockerConfig {
    DockerConfig {
        project_dir: "/app".to_string(),
        service: "app".to_string(),
        container: None,
        image: Some(image.to_string()),
        volume_mount: None,
        work_dir: None,
        shell: "bash".to_string(),
        env: std::collections::HashMap::new(),
    }
}

/// CheckToRun whose resolved_command equals `command`, for executor tests.
/// `container` sets the per-check container override.
#[allow(dead_code)]
pub fn make_exec_check(id: &str, command: &str, container: Option<&str>) -> CheckToRun {
    CheckToRun {
        id: id.to_string(),
        group: "g".to_string(),
        definition: CheckDefinition {
            name: id.to_string(),
            command: command.to_string(),
            service: None,
            container: container.map(String::from),
            fix_command: None,
            triggers: None,
            on_demand: false,
            env: std::collections::HashMap::new(),
        },
        service: "app".to_string(),
        files: vec![],
        resolved_command: command.to_string(),
        resolved_fix_command: None,
        on_demand: false,
        skipped_no_files: false,
    }
}

/// Create an App with known state for widget tests
///
/// Returns App with:
/// - 3 checks: clippy (pending), fmt (passed), unit (failed)
/// - 2 changed files
/// - Branch: "feature/test"
/// - Some CPU/memory history
#[allow(dead_code)]
pub fn make_test_app() -> App {
    let config = parse_widget_config();
    let changed_files = ChangedFiles {
        files: vec!["src/main.rs".to_string(), "src/lib.rs".to_string()],
        base_ref: "main".to_string(),
    };
    let checks = vec![
        make_widget_check("clippy", "lint", "Clippy", false),
        make_widget_check("fmt", "lint", "Format Check", true),
        make_widget_check("unit", "test", "Unit Tests", false),
    ];

    let mut app = App::new(config, changed_files, checks, "feature/test".to_string());

    // Set known states for tests
    app.results.get_mut("clippy").unwrap().status = CheckStatus::Pending;
    app.results.get_mut("fmt").unwrap().status = CheckStatus::Passed;
    app.results.get_mut("fmt").unwrap().duration_ms = 1500;
    app.results.get_mut("fmt").unwrap().output = "All files formatted correctly\n".to_string();

    app.results.get_mut("unit").unwrap().status = CheckStatus::Failed;
    app.results.get_mut("unit").unwrap().duration_ms = 5200;
    app.results.get_mut("unit").unwrap().output =
        "running 10 tests\ntest foo::bar ... ok\n".to_string();
    app.results.get_mut("unit").unwrap().error_output =
        "test baz::qux ... FAILED\n\nfailures:\n    baz::qux\n".to_string();

    // Add some CPU/memory history for sparkline tests
    for i in 0..30 {
        app.update_stats(
            (20.0 + i as f32 * 2.0).min(80.0),
            8_000_000_000,
            16_000_000_000,
        );
    }

    app
}

/// Create App with all checks passed (for success state tests)
#[allow(dead_code)]
pub fn make_test_app_all_passed() -> App {
    let mut app = make_test_app();
    app.all_finished = true;
    for result in app.results.values_mut() {
        result.status = CheckStatus::Passed;
        result.duration_ms = 1000;
    }
    app
}

/// Create App with running checks (for progress state tests)
#[allow(dead_code)]
pub fn make_test_app_running() -> App {
    let mut app = make_test_app();
    app.current_group = Some("lint".to_string());
    app.results.get_mut("clippy").unwrap().status = CheckStatus::Running;
    app
}
