//! Executor-backed tests for fix.rs — exercises run_fix_command_with_executor
//! error-message branches that were unreachable without an injected executor.

mod common;

use ci_tui::fix::run_fix_command_with_executor;
use ci_tui::runner::{CommandOutput, MockCommandExecutor};
use std::path::Path;

#[tokio::test]
async fn returns_ok_on_success() {
    let mut mock = MockCommandExecutor::new();
    mock.expect_is_container_running().returning(|_| true);
    mock.expect_execute().returning(|_, _, _| CommandOutput {
        success: true,
        stdout: String::new(),
        stderr: String::new(),
    });
    let cfg = common::test_docker_target("img:latest");
    let result =
        run_fix_command_with_executor("cargo fmt", "fmt", Path::new("/app"), &cfg, None, &mock)
            .await;
    assert!(result.is_ok());
}

#[tokio::test]
async fn reports_stderr_when_present() {
    let mut mock = MockCommandExecutor::new();
    mock.expect_is_container_running().returning(|_| true);
    mock.expect_execute().returning(|_, _, _| CommandOutput {
        success: false,
        stdout: "stdout-noise".into(),
        stderr: "stderr-wins".into(),
    });
    let cfg = common::test_docker_target("img:latest");
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
    mock.expect_execute().returning(|_, _, _| CommandOutput {
        success: false,
        stdout: "stdout-only".into(),
        stderr: String::new(),
    });
    let cfg = common::test_docker_target("img:latest");
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
    mock.expect_execute().returning(|_, _, _| CommandOutput {
        success: false,
        stdout: String::new(),
        stderr: String::new(),
    });
    let cfg = common::test_docker_target("img:latest");
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
        .withf(|cmd, _, _| cmd.starts_with("docker run"))
        .returning(|_, _, _| CommandOutput {
            success: true,
            stdout: String::new(),
            stderr: String::new(),
        });
    let cfg = common::test_docker_target("img:latest");
    run_fix_command_with_executor("cargo fmt", "fmt", Path::new("/app"), &cfg, None, &mock)
        .await
        .unwrap();
}

use ci_tui::checks::{determine_checks, CheckFiles, CheckToRun};
use ci_tui::fix::{run_with_executor, FixSummary};
use ci_tui::git::ChangedFiles;
use std::sync::{Arc, Mutex};

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
    mock.expect_execute().returning(|_, _, _| CommandOutput {
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
        run_with_executor(config, changed, "/app".into(), None, Arc::new(mock))
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
    mock.expect_execute().returning(|_, _, _| CommandOutput {
        success: false,
        stdout: String::new(),
        stderr: "diff found, rejecting".into(),
    });

    let config = rust_fmt_with_fix_config();
    let changed = ChangedFiles {
        files: vec!["src/main.rs".into()],
        base_ref: "main".into(),
    };

    let summary = run_with_executor(config, changed, "/app".into(), None, Arc::new(mock))
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

    let summary = run_with_executor(config, changed, "/app".into(), None, Arc::new(mock))
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

    let summary = run_with_executor(config, changed, "/app".into(), None, Arc::new(mock))
        .await
        .unwrap();

    assert_eq!(summary.fix_count, 0);
}

/// `src/main.rs` changed
fn main_rs_changed() -> ChangedFiles {
    ChangedFiles {
        files: vec!["src/main.rs".into()],
        base_ref: "main".into(),
    }
}

/// Kind of a command run by the fixtures here: group setup (`init-db`),
/// the check (`--check`) or its fix
fn kind(command: &str) -> &'static str {
    if command.contains("init-db") {
        "setup"
    } else if command.contains("--check") {
        "check"
    } else {
        "fix"
    }
}

/// Mock executor logging the [`kind`] of every command; commands of the
/// kinds in `failing` fail with `diff found` on stdout
fn logging_mock(
    failing: &'static [&'static str],
) -> (MockCommandExecutor, Arc<Mutex<Vec<String>>>) {
    let log = Arc::new(Mutex::new(Vec::new()));
    let seen = Arc::clone(&log);
    let mut mock = MockCommandExecutor::new();
    mock.expect_is_container_running().returning(|_| true);
    mock.expect_execute().returning(move |cmd, _, _| {
        seen.lock().unwrap().push(kind(cmd).to_string());
        let success = !failing.contains(&kind(cmd));
        CommandOutput {
            success,
            stdout: if success {
                String::new()
            } else {
                "diff found".into()
            },
            stderr: String::new(),
        }
    });
    (mock, log)
}

/// Fix run of `config` with `src/main.rs` changed, verifying against
/// `checks` (`None`: --no-verify); the summary and the command kinds run
async fn fix_run(
    config: ci_tui::config::CiConfig,
    checks: Option<Vec<CheckToRun>>,
    failing: &'static [&'static str],
) -> (FixSummary, Vec<String>) {
    let (mock, log) = logging_mock(failing);
    let summary = run_with_executor(
        config,
        main_rs_changed(),
        "/app".into(),
        checks,
        Arc::new(mock),
    )
    .await
    .unwrap();
    let log = log.lock().unwrap().clone();
    (summary, log)
}

/// Checks `select_checks` picks for `config` with `src/main.rs` changed
fn selected(config: &ci_tui::config::CiConfig) -> Vec<CheckToRun> {
    determine_checks(config, &main_rs_changed(), std::path::Path::new("/app"))
}

#[tokio::test]
async fn verify_reruns_check_after_fix() {
    let config = rust_fmt_with_fix_config();
    let checks = selected(&config);

    let (summary, log) = fix_run(config, Some(checks), &[]).await;

    assert_eq!(log, ["fix", "check"]);
    assert_eq!((summary.verify_count, summary.verify_pass_count), (1, 1));
    assert!(!summary.has_failures);
}

#[tokio::test]
async fn verify_still_failing_fails_the_run() {
    let config = rust_fmt_with_fix_config();
    let checks = selected(&config);

    let (summary, log) = fix_run(config, Some(checks), &["check"]).await;

    assert_eq!(log, ["fix", "check"]);
    assert_eq!((summary.pass_count, summary.fail_count), (1, 0));
    assert_eq!((summary.verify_count, summary.verify_fail_count), (1, 1));
    assert!(summary.has_failures, "a failed verification fails the run");
}

#[tokio::test]
async fn failed_fix_is_not_verified() {
    let config = rust_fmt_with_fix_config();
    let checks = selected(&config);

    let (summary, log) = fix_run(config, Some(checks), &["fix"]).await;

    assert_eq!(log, ["fix"]);
    assert_eq!(summary.fail_count, 1);
    assert_eq!((summary.verify_count, summary.unverified_count), (0, 0));
    assert!(summary.has_failures);
}

#[tokio::test]
async fn unselected_or_on_demand_check_is_not_verified() {
    let config = rust_fmt_with_fix_config();
    let mut on_demand = selected(&config);
    on_demand[0].files = CheckFiles::OnDemand;

    for checks in [vec![], on_demand] {
        let (summary, log) = fix_run(config.clone(), Some(checks), &["check"]).await;

        assert_eq!(log, ["fix"]);
        assert_eq!((summary.verify_count, summary.unverified_count), (0, 1));
        assert!(!summary.has_failures);
    }
}

#[tokio::test]
async fn no_verify_runs_fixes_only() {
    let (summary, log) = fix_run(rust_fmt_with_fix_config(), None, &["check"]).await;

    assert_eq!(log, ["fix"]);
    assert_eq!((summary.verify_count, summary.unverified_count), (0, 0));
    assert!(!summary.verify && !summary.has_failures);
}

/// [`rust_fmt_with_fix_config`] plus a second fixable check `fmt2` in
/// group `lint`, which has pre-command `init-db`
fn fix_config_with_setup() -> ci_tui::config::CiConfig {
    common::configs::ConfigBuilder::new()
        .with_file_pattern("rust", r"\.rs$", None)
        .with_pre_command("lint", "db", "init-db")
        .with_check(
            "lint",
            "fmt",
            common::configs::CheckBuilder::new("Format", "cargo fmt --check {files}")
                .with_fix_command("cargo fmt {files}")
                .with_file_pattern_trigger("rust")
                .build(),
        )
        .with_check(
            "lint",
            "fmt2",
            common::configs::CheckBuilder::new("Format 2", "rustfmt --check {files}")
                .with_fix_command("rustfmt {files}")
                .with_file_pattern_trigger("rust")
                .build(),
        )
        .build()
}

#[tokio::test]
async fn group_setup_runs_once_before_first_verification() {
    let config = fix_config_with_setup();
    let checks = selected(&config);

    let (summary, log) = fix_run(config, Some(checks), &[]).await;

    assert_eq!(log, ["fix", "setup", "check", "fix", "check"]);
    assert_eq!((summary.verify_count, summary.verify_pass_count), (2, 2));
}

#[tokio::test]
async fn failed_group_setup_fails_its_verifications() {
    let config = fix_config_with_setup();
    let checks = selected(&config);

    let (summary, log) = fix_run(config, Some(checks), &["setup"]).await;

    assert_eq!(
        log,
        ["fix", "setup", "fix"],
        "setup not retried, checks not run"
    );
    assert_eq!((summary.verify_count, summary.verify_fail_count), (2, 2));
    assert_eq!(summary.fail_count, 0, "the fixes passed");
    assert!(summary.has_failures);
}

#[tokio::test]
async fn no_setup_without_verification() {
    let config = fix_config_with_setup();

    let (_, log) = fix_run(config, None, &[]).await;

    assert_eq!(log, ["fix", "fix"]);
}
