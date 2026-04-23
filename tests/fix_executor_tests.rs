//! Executor-backed tests for fix.rs — exercises run_fix_command_with_executor
//! error-message branches that were unreachable without an injected executor.

mod common;

use ci_tui::config::DockerConfig;
use ci_tui::fix::run_fix_command_with_executor;
use ci_tui::runner::{CommandOutput, MockCommandExecutor};
use std::collections::HashMap;
use std::path::Path;

fn docker_cfg() -> DockerConfig {
    DockerConfig {
        project_dir: "/app".into(),
        service: "app".into(),
        container: None,
        image: Some("img:latest".into()),
        volume_mount: None,
        work_dir: None,
        shell: "bash".into(),
        env: HashMap::new(),
    }
}

#[tokio::test]
async fn returns_ok_on_success() {
    let mut mock = MockCommandExecutor::new();
    mock.expect_is_container_running().returning(|_| true);
    mock.expect_execute().returning(|_, _| CommandOutput {
        success: true,
        stdout: String::new(),
        stderr: String::new(),
    });
    let cfg = docker_cfg();
    let result =
        run_fix_command_with_executor("cargo fmt", "fmt", Path::new("/app"), &cfg, None, &mock)
            .await;
    assert!(result.is_ok());
}

#[tokio::test]
async fn reports_stderr_when_present() {
    let mut mock = MockCommandExecutor::new();
    mock.expect_is_container_running().returning(|_| true);
    mock.expect_execute().returning(|_, _| CommandOutput {
        success: false,
        stdout: "stdout-noise".into(),
        stderr: "stderr-wins".into(),
    });
    let cfg = docker_cfg();
    let err =
        run_fix_command_with_executor("cargo fmt", "fmt", Path::new("/app"), &cfg, None, &mock)
            .await
            .unwrap_err();
    assert!(err.to_string().contains("stderr-wins"));
}

#[tokio::test]
async fn falls_back_to_stdout_when_stderr_empty() {
    let mut mock = MockCommandExecutor::new();
    mock.expect_is_container_running().returning(|_| true);
    mock.expect_execute().returning(|_, _| CommandOutput {
        success: false,
        stdout: "stdout-only".into(),
        stderr: String::new(),
    });
    let cfg = docker_cfg();
    let err =
        run_fix_command_with_executor("cargo fmt", "fmt", Path::new("/app"), &cfg, None, &mock)
            .await
            .unwrap_err();
    assert!(err.to_string().contains("stdout-only"));
}

#[tokio::test]
async fn falls_back_to_formatted_message_when_both_empty() {
    let mut mock = MockCommandExecutor::new();
    mock.expect_is_container_running().returning(|_| true);
    mock.expect_execute().returning(|_, _| CommandOutput {
        success: false,
        stdout: String::new(),
        stderr: String::new(),
    });
    let cfg = docker_cfg();
    let err =
        run_fix_command_with_executor("cargo fmt", "fmt", Path::new("/app"), &cfg, None, &mock)
            .await
            .unwrap_err();
    let msg = err.to_string();
    assert!(msg.contains("Fix command 'fmt' failed"));
    assert!(msg.contains("exit code"));
}

#[tokio::test]
async fn uses_docker_run_when_container_not_running() {
    let mut mock = MockCommandExecutor::new();
    mock.expect_is_container_running().returning(|_| false);
    mock.expect_execute()
        .withf(|cmd, _| cmd.starts_with("docker run"))
        .returning(|_, _| CommandOutput {
            success: true,
            stdout: String::new(),
            stderr: String::new(),
        });
    let cfg = docker_cfg();
    run_fix_command_with_executor("cargo fmt", "fmt", Path::new("/app"), &cfg, None, &mock)
        .await
        .unwrap();
}

use ci_tui::fix::{run_with_executor, FixSummary};
use ci_tui::git::ChangedFiles;

fn rust_fmt_with_fix_config() -> ci_tui::config::CiConfig {
    common::configs::ConfigBuilder::new()
        .with_file_pattern("rust", r"\.rs$", None)
        .with_check(
            "lint",
            "fmt",
            common::configs::CheckBuilder::new("Format", "cargo fmt --check {files}")
                .with_fix_command("cargo fmt {files}")
                .with_file_pattern_trigger("rust")
                .build(),
        )
        .build()
}

#[tokio::test]
async fn all_fixes_pass_summary_is_clean() {
    let mut mock = MockCommandExecutor::new();
    mock.expect_is_container_running().returning(|_| true);
    mock.expect_execute().returning(|_, _| CommandOutput {
        success: true,
        stdout: String::new(),
        stderr: String::new(),
    });

    let config = rust_fmt_with_fix_config();
    let changed = ChangedFiles {
        files: vec!["src/main.rs".into()],
        base_ref: "main".into(),
    };

    let summary: FixSummary =
        run_with_executor(config, changed, std::path::PathBuf::from("/app"), &mock)
            .await
            .unwrap();

    assert_eq!(summary.fix_count, 1);
    assert_eq!(summary.pass_count, 1);
    assert_eq!(summary.fail_count, 0);
    assert!(!summary.has_failures);
}

#[tokio::test]
async fn failing_fix_recorded_in_summary() {
    let mut mock = MockCommandExecutor::new();
    mock.expect_is_container_running().returning(|_| true);
    mock.expect_execute().returning(|_, _| CommandOutput {
        success: false,
        stdout: String::new(),
        stderr: "diff found, rejecting".into(),
    });

    let config = rust_fmt_with_fix_config();
    let changed = ChangedFiles {
        files: vec!["src/main.rs".into()],
        base_ref: "main".into(),
    };

    let summary = run_with_executor(config, changed, std::path::PathBuf::from("/app"), &mock)
        .await
        .unwrap();

    assert_eq!(summary.fix_count, 1);
    assert_eq!(summary.pass_count, 0);
    assert_eq!(summary.fail_count, 1);
    assert!(summary.has_failures);
}

#[tokio::test]
async fn skips_check_without_fix_command() {
    // Executor should never be invoked — no fix_command means nothing to run
    let mut mock = MockCommandExecutor::new();
    mock.expect_execute().times(0);

    let config = common::configs::ConfigBuilder::new()
        .with_file_pattern("rust", r"\.rs$", None)
        .with_check(
            "lint",
            "clippy",
            common::configs::CheckBuilder::new("Clippy", "cargo clippy {files}")
                .with_file_pattern_trigger("rust")
                .build(),
        )
        .build();

    let changed = ChangedFiles {
        files: vec!["src/main.rs".into()],
        base_ref: "main".into(),
    };

    let summary = run_with_executor(config, changed, std::path::PathBuf::from("/app"), &mock)
        .await
        .unwrap();

    assert_eq!(summary.fix_count, 0);
}

#[tokio::test]
async fn skips_fix_when_files_placeholder_has_no_matches() {
    let mut mock = MockCommandExecutor::new();
    mock.expect_execute().times(0);

    let config = rust_fmt_with_fix_config();
    let changed = ChangedFiles {
        files: vec!["README.md".into()],
        base_ref: "main".into(),
    };

    let summary = run_with_executor(config, changed, std::path::PathBuf::from("/app"), &mock)
        .await
        .unwrap();

    assert_eq!(summary.fix_count, 0);
}
