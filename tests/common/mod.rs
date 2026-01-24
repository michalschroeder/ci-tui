//! Shared test utilities and mock helpers

use ci_tui::checks::CheckToRun;
use ci_tui::config::{CheckDefinition, CiConfig};
use ci_tui::git::ChangedFiles;
use ci_tui::runner::CheckStatus;
use ci_tui::ui::app::App;

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
#[allow(dead_code)]
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

/// Minimal config YAML for widget tests
#[allow(dead_code)]
pub fn widget_test_config_yaml() -> &'static str {
    r#"
version: 2

docker:
  project_dir: ./test
  service: app

git:
  base_branch: main
  fallback_branch: HEAD~1

file_patterns:
  rust:
    pattern: '\.rs$'
    color: yellow
  toml:
    pattern: '\.toml$'
    color: cyan

checks:
  lint:
    name: Lint
    parallel: true
    checks:
      clippy:
        name: Clippy
        command: cargo clippy
        triggers:
          file_pattern: rust
      fmt:
        name: Format Check
        command: cargo fmt --check
        fix_command: cargo fmt
        triggers:
          file_pattern: rust

  test:
    name: Tests
    checks:
      unit:
        name: Unit Tests
        command: cargo test
        triggers:
          file_pattern: rust
"#
}

/// Parse the widget test config
#[allow(dead_code)]
pub fn parse_widget_config() -> CiConfig {
    serde_yaml::from_str(widget_test_config_yaml()).expect("Failed to parse widget test config")
}

/// Create a CheckToRun for testing
#[allow(dead_code)]
pub fn make_widget_check(id: &str, group: &str, name: &str, has_fix: bool) -> CheckToRun {
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
            on_demand: false,
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
