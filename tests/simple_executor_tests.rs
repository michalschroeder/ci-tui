//! Executor-backed tests for simple.rs — exercises run_check_with_executor
//! and run_sequential / run_parallel orchestration under MockCommandExecutor.

mod common;

use ci_tui::runner::{CheckStatus, CommandOutput, MockCommandExecutor};
use ci_tui::simple::run_check_with_executor;
use std::path::Path;

#[tokio::test]
async fn passes_when_executor_succeeds() {
    let mut mock = MockCommandExecutor::new();
    mock.expect_is_container_running().returning(|_| true);
    mock.expect_execute().returning(|_, _| CommandOutput {
        success: true,
        stdout: "output".into(),
        stderr: String::new(),
    });
    let cfg = common::test_docker_target("img:latest");
    let result = run_check_with_executor(
        &common::make_exec_check("c", "cargo test", None),
        Path::new("/app"),
        &cfg,
        &mock,
    )
    .await;
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
    let cfg = common::test_docker_target("img:latest");
    let result = run_check_with_executor(
        &common::make_exec_check("c", "cargo test", None),
        Path::new("/app"),
        &cfg,
        &mock,
    )
    .await;
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
    let cfg = common::test_docker_target("img:latest");
    run_check_with_executor(
        &common::make_exec_check("c", "cargo test", None),
        Path::new("/app"),
        &cfg,
        &mock,
    )
    .await;
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
    let mut c = common::make_exec_check("c", "cargo test", None);
    c.definition.container = Some("override".into());
    let cfg = common::test_docker_target("img:latest");
    run_check_with_executor(&c, Path::new("/app"), &cfg, &mock).await;
}

use ci_tui::simple::run_with_executor;
use std::sync::Arc;

#[tokio::test]
async fn sequential_group_runs_checks_in_order() {
    let mut mock = MockCommandExecutor::new();
    mock.expect_is_container_running().returning(|_| true);
    mock.expect_execute().returning(|_, _| CommandOutput {
        success: true,
        stdout: String::new(),
        stderr: String::new(),
    });

    let config = common::configs::ConfigBuilder::new().build();
    let checks = vec![
        common::make_widget_check("a", "g1", "A", false),
        common::make_widget_check("b", "g1", "B", false),
    ];
    let results = run_with_executor(
        config,
        checks,
        std::path::PathBuf::from("/app"),
        Arc::new(mock),
    )
    .await
    .unwrap();
    assert_eq!(results.len(), 2);
    assert_eq!(results[0].check_id, "a");
    assert_eq!(results[1].check_id, "b");
}

#[tokio::test]
async fn parallel_group_runs_all_checks() {
    let mut mock = MockCommandExecutor::new();
    mock.expect_is_container_running().returning(|_| true);
    mock.expect_execute().returning(|_, _| CommandOutput {
        success: true,
        stdout: String::new(),
        stderr: String::new(),
    });

    let config = common::configs::ConfigBuilder::new()
        .with_parallel_group("g1")
        .build();
    let checks = vec![
        common::make_widget_check("a", "g1", "A", false),
        common::make_widget_check("b", "g1", "B", false),
    ];
    let results = run_with_executor(
        config,
        checks,
        std::path::PathBuf::from("/app"),
        Arc::new(mock),
    )
    .await
    .unwrap();
    assert_eq!(results.len(), 2);
    let mut ids: Vec<_> = results.iter().map(|r| r.check_id.clone()).collect();
    ids.sort();
    assert_eq!(ids, vec!["a".to_string(), "b".to_string()]);
}

#[tokio::test]
async fn on_demand_checks_skipped_in_both_modes() {
    let mut mock = MockCommandExecutor::new();
    mock.expect_is_container_running().returning(|_| true);
    mock.expect_execute()
        .times(1)
        .returning(|_, _| CommandOutput {
            success: true,
            stdout: String::new(),
            stderr: String::new(),
        });

    let config = common::configs::ConfigBuilder::new().build();
    let checks = vec![
        common::make_widget_check("a", "g1", "A", false),
        common::make_on_demand_check("b-slow", "g1", "Slow"),
    ];
    let results = run_with_executor(
        config,
        checks,
        std::path::PathBuf::from("/app"),
        Arc::new(mock),
    )
    .await
    .unwrap();
    assert_eq!(results.len(), 1);
    assert_eq!(results[0].check_id, "a");
}

#[tokio::test]
async fn failure_recorded_in_results() {
    let mut mock = MockCommandExecutor::new();
    mock.expect_is_container_running().returning(|_| true);
    mock.expect_execute().returning(|_, _| CommandOutput {
        success: false,
        stdout: String::new(),
        stderr: "failed".into(),
    });

    let config = common::configs::ConfigBuilder::new().build();
    let checks = vec![common::make_widget_check("a", "g1", "A", false)];
    let results = run_with_executor(
        config,
        checks,
        std::path::PathBuf::from("/app"),
        Arc::new(mock),
    )
    .await
    .unwrap();
    assert_eq!(results.len(), 1);
    assert_eq!(results[0].status, CheckStatus::Failed);
    assert_eq!(results[0].error_output, "failed");
}
