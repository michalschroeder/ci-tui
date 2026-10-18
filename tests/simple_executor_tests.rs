//! Executor-backed tests for simple.rs — exercises run_check_with_executor
//! and run_sequential / run_parallel orchestration under MockCommandExecutor.

mod common;

use ci_tui::config::DEFAULT_MAX_OUTPUT_LINES;
use ci_tui::runner::{CheckStatus, CommandOutput, MockCommandExecutor};
use ci_tui::simple::run_check_with_executor;
use std::path::Path;

#[tokio::test]
async fn passes_when_executor_succeeds() {
    let mut mock = MockCommandExecutor::new();
    mock.expect_execute().returning(|_, _, _| CommandOutput {
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
        DEFAULT_MAX_OUTPUT_LINES,
    )
    .await;
    assert_eq!(result.status, CheckStatus::Passed);
    assert_eq!(result.output, "output");
}

#[tokio::test]
async fn fails_with_stderr_when_executor_reports_failure() {
    let mut mock = MockCommandExecutor::new();
    mock.expect_execute().returning(|_, _, _| CommandOutput {
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
        DEFAULT_MAX_OUTPUT_LINES,
    )
    .await;
    assert_eq!(result.status, CheckStatus::Failed);
    assert_eq!(result.error_output, "boom");
}

#[tokio::test]
async fn image_mode_uses_docker_run_without_compose() {
    let mut mock = MockCommandExecutor::new();
    mock.expect_running_container().never();
    mock.expect_execute()
        .withf(|cmd, _, _| cmd.starts_with("docker run") && cmd.contains(" img:latest "))
        .returning(|_, _, _| CommandOutput {
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
        DEFAULT_MAX_OUTPUT_LINES,
    )
    .await;
}

#[tokio::test]
async fn check_service_execs_into_its_resolved_container() {
    let mut mock = MockCommandExecutor::new();
    mock.expect_running_container()
        .withf(|_, service, _| service == "worker")
        .returning(|_, _, _| Some("w0rk3r".into()));
    mock.expect_execute()
        .withf(|cmd, _, _| cmd == "docker exec w0rk3r bash -c 'cargo test'")
        .times(1)
        .returning(|_, _, _| CommandOutput {
            success: true,
            stdout: String::new(),
            stderr: String::new(),
        });
    let cfg = common::configs::ConfigBuilder::new().build().runner;
    let c = common::make_exec_check("c", "cargo test", Some("worker"));
    run_check_with_executor(&c, Path::new("/app"), &cfg, &mock, DEFAULT_MAX_OUTPUT_LINES).await;
}

#[tokio::test]
async fn simple_run_resolves_each_service_once() {
    let mut mock = MockCommandExecutor::new();
    mock.expect_running_container()
        .times(1)
        .returning(|_, _, _| Some(common::RUNNING_ID.into()));
    mock.expect_execute()
        .times(3)
        .returning(|_, _, _| CommandOutput {
            success: true,
            stdout: String::new(),
            stderr: String::new(),
        });
    let config = common::configs::ConfigBuilder::new()
        .with_parallel_group("g2")
        .build();
    let checks = vec![
        common::make_widget_check("a", "g1", "A", false),
        common::make_widget_check("b", "g2", "B", false),
        common::make_widget_check("c", "g2", "C", false),
    ];
    let results = run_with_executor(config, checks, "/app".into(), Arc::new(mock))
        .await
        .unwrap();
    assert_eq!(results.len(), 3);
}

use ci_tui::simple::run_with_executor;
use std::sync::Arc;

#[tokio::test]
async fn sequential_group_runs_checks_in_order() {
    let mut mock = MockCommandExecutor::new();
    mock.expect_running_container()
        .returning(|_, _, _| Some(common::RUNNING_ID.into()));
    mock.expect_execute().returning(|_, _, _| CommandOutput {
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
    mock.expect_running_container()
        .returning(|_, _, _| Some(common::RUNNING_ID.into()));
    mock.expect_execute().returning(|_, _, _| CommandOutput {
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
    mock.expect_running_container()
        .returning(|_, _, _| Some(common::RUNNING_ID.into()));
    mock.expect_execute()
        .times(1)
        .returning(|_, _, _| CommandOutput {
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
    mock.expect_running_container()
        .returning(|_, _, _| Some(common::RUNNING_ID.into()));
    mock.expect_execute().returning(|_, _, _| CommandOutput {
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

#[tokio::test]
async fn hung_check_times_out_with_real_executor() {
    let mut config = common::configs::ConfigBuilder::new().build();
    config.runner = ci_tui::config::ExecTarget::Local(ci_tui::config::LocalConfig {
        shell: "sh".into(),
        ..Default::default()
    });
    let mut check = common::make_exec_check("hang", "sleep 999", None);
    check.definition.timeout = Some(std::time::Duration::from_millis(100));
    let executor = Arc::new(ci_tui::runner::RealCommandExecutor);
    let results = run_with_executor(config, vec![check], "/tmp".into(), executor)
        .await
        .unwrap();
    assert_eq!(results[0].status, CheckStatus::TimedOut);
}

#[rstest::rstest]
#[case::global_cap(common::configs::ConfigBuilder::new().with_parallel_group("g").with_max_parallel(2), 2)]
#[case::group_cap(
    common::configs::ConfigBuilder::new().with_max_parallel(3).with_group_max_parallel("g", 1),
    1
)]
#[tokio::test]
async fn parallel_group_respects_max_parallel(
    #[case] config: common::configs::ConfigBuilder,
    #[case] expected: usize,
) {
    let executor = Arc::new(common::ConcurrencyExecutor::default());
    let checks = (0..5)
        .map(|i| common::make_widget_check(&format!("c{i}"), "g", "C", false))
        .collect();
    let results = run_with_executor(
        config.build(),
        checks,
        std::path::PathBuf::from("/app"),
        executor.clone(),
    )
    .await
    .unwrap();
    assert_eq!(results.len(), 5, "every check still runs");
    assert_eq!(executor.peak(), expected);
}

/// Daemon error of a `docker run` / `docker compose run` whose `--name` is taken
const NAME_CONFLICT: &str = "Error response from daemon: Conflict. The container name \
                             \"/ci-tui-0123456789abcdef-0\" is already in use by container \"35127c\".";

// A container that never starts fails the check with docker's error in its
// output (stderr), not an empty failure
#[rstest::rstest]
#[case::compose_run(common::configs::ConfigBuilder::new().build().runner, "docker compose ")]
#[case::docker_run(common::test_docker_target("img:latest"), "docker run ")]
#[tokio::test]
async fn failed_container_start_shows_docker_error(
    #[case] target: ci_tui::runner::ExecTarget,
    #[case] prefix: &'static str,
) {
    let mut mock = MockCommandExecutor::new();
    mock.expect_running_container().returning(|_, _, _| None);
    mock.expect_execute()
        .withf(move |cmd, _, _| cmd.starts_with(prefix) && cmd.contains(" --name ci-tui-"))
        .returning(|_, _, _| CommandOutput {
            success: false,
            stdout: String::new(),
            stderr: format!(" Container ci-tui-0123456789abcdef-0  Creating\n{NAME_CONFLICT}\n"),
        });
    let check = common::make_exec_check("c", "cargo test", None);
    let result = run_check_with_executor(
        &check,
        Path::new("/app"),
        &target,
        &mock,
        DEFAULT_MAX_OUTPUT_LINES,
    )
    .await;
    assert_eq!(result.status, CheckStatus::Failed);
    assert!(
        result.error_output.contains(NAME_CONFLICT),
        "{}",
        result.error_output
    );
    let shown = ci_tui::simple::format_failed_check(&result, None);
    assert!(shown.contains(NAME_CONFLICT), "{shown}");
}
