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
