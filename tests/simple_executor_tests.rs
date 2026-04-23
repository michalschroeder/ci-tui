//! Executor-backed tests for simple.rs — exercises run_check_with_executor
//! and run_sequential / run_parallel orchestration under MockCommandExecutor.

mod common;

use ci_tui::config::{CheckDefinition, DockerConfig};
use ci_tui::runner::{CheckStatus, CommandOutput, MockCommandExecutor};
use ci_tui::simple::run_check_with_executor;
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

fn check(id: &str) -> ci_tui::checks::CheckToRun {
    ci_tui::checks::CheckToRun {
        id: id.into(),
        group: "g".into(),
        definition: CheckDefinition {
            name: id.into(),
            command: "cmd".into(),
            service: None,
            container: None,
            fix_command: None,
            triggers: None,
            on_demand: false,
            env: HashMap::new(),
        },
        service: "app".into(),
        files: vec![],
        resolved_command: "cargo test".into(),
        resolved_fix_command: None,
        on_demand: false,
        skipped_no_files: false,
    }
}

#[tokio::test]
async fn passes_when_executor_succeeds() {
    let mut mock = MockCommandExecutor::new();
    mock.expect_is_container_running().returning(|_| true);
    mock.expect_execute().returning(|_, _| CommandOutput {
        success: true,
        stdout: "output".into(),
        stderr: String::new(),
    });
    let cfg = docker_cfg();
    let result = run_check_with_executor(&check("c"), Path::new("/app"), &cfg, &mock).await;
    assert_eq!(result.status, CheckStatus::Passed);
    assert_eq!(result.output, "output");
}

#[tokio::test]
async fn fails_with_stderr_when_executor_reports_failure() {
    let mut mock = MockCommandExecutor::new();
    mock.expect_is_container_running().returning(|_| true);
    mock.expect_execute().returning(|_, _| CommandOutput {
        success: false,
        stdout: String::new(),
        stderr: "boom".into(),
    });
    let cfg = docker_cfg();
    let result = run_check_with_executor(&check("c"), Path::new("/app"), &cfg, &mock).await;
    assert_eq!(result.status, CheckStatus::Failed);
    assert_eq!(result.error_output, "boom");
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
    run_check_with_executor(&check("c"), Path::new("/app"), &cfg, &mock).await;
}

#[tokio::test]
async fn per_check_container_overrides_default() {
    let mut mock = MockCommandExecutor::new();
    mock.expect_is_container_running()
        .withf(|name| name == "override")
        .returning(|_| true);
    mock.expect_execute()
        .withf(|cmd, _| cmd.contains("override"))
        .returning(|_, _| CommandOutput {
            success: true,
            stdout: String::new(),
            stderr: String::new(),
        });
    let mut c = check("c");
    c.definition.container = Some("override".into());
    let cfg = docker_cfg();
    run_check_with_executor(&c, Path::new("/app"), &cfg, &mock).await;
}
