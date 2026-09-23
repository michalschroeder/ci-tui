//! Tests for runner.rs - command building and execution functions
//!
//! This test file covers:
//! - Pure command-building functions (no mocks needed)
//! - Docker command execution (using mocks)
//! - CheckResult factory methods
//! - CheckRunner orchestration

use ci_tui::runner::{
    build_docker_exec_command, build_docker_run_command, execute_command_with_executor,
    filter_docker_warnings, CheckResult, CheckStatus,
};
use rstest::rstest;
use std::collections::HashMap;

mod common;
use common::{mock_executor_success, CommandOutput, MockCommandExecutor};

mod build_docker_exec_command_tests {
    use super::*;
    use pretty_assertions::assert_eq;

    #[test]
    fn no_env_vars() {
        let env = HashMap::new();
        let cmd = build_docker_exec_command("my-container", &env, "cargo test", "bash");
        assert_eq!(cmd, "docker exec my-container bash -c 'cargo test'");
    }

    #[rstest]
    #[case("FOO", "bar", "-e FOO='bar'")]
    #[case("PATH", "/usr/bin", "-e PATH='/usr/bin'")]
    fn single_env_var(#[case] key: &str, #[case] val: &str, #[case] expected_flag: &str) {
        let mut env = HashMap::new();
        env.insert(key.to_string(), val.to_string());
        let cmd = build_docker_exec_command("container", &env, "test", "bash");
        assert!(
            cmd.contains(expected_flag),
            "Command '{}' should contain '{}'",
            cmd,
            expected_flag
        );
    }

    #[test]
    fn multiple_env_vars() {
        let mut env = HashMap::new();
        env.insert("FOO".to_string(), "bar".to_string());
        env.insert("BAZ".to_string(), "qux".to_string());
        let cmd = build_docker_exec_command("container", &env, "test", "bash");

        // Both env vars should be present
        assert!(
            cmd.contains("-e FOO='bar'"),
            "Command '{}' missing FOO",
            cmd
        );
        assert!(
            cmd.contains("-e BAZ='qux'"),
            "Command '{}' missing BAZ",
            cmd
        );
    }

    #[test]
    fn special_characters_in_url() {
        let mut env = HashMap::new();
        env.insert("URL".to_string(), "http://test?a=1&b=2".to_string());
        let cmd = build_docker_exec_command("container", &env, "curl $URL", "bash");

        // URL with special characters should be properly quoted
        assert!(cmd.contains("-e URL='http://test?a=1&b=2'"));
    }

    #[test]
    fn single_quote_in_env_value() {
        let mut env = HashMap::new();
        env.insert("MSG".to_string(), "it's working".to_string());
        let cmd = build_docker_exec_command("container", &env, "echo $MSG", "bash");

        // Single quotes in values should be escaped
        assert!(cmd.contains("it'\\''s working"));
    }

    #[test]
    fn single_quote_in_command() {
        let env = HashMap::new();
        let cmd = build_docker_exec_command("container", &env, "echo 'hello world'", "bash");

        // Single quotes in command should be escaped
        assert!(cmd.contains("echo '\\''hello world'\\''"));
    }

    #[test]
    fn command_with_env_flag_placement() {
        let mut env = HashMap::new();
        env.insert("FOO".to_string(), "bar".to_string());
        let cmd = build_docker_exec_command("my-container", &env, "cargo test", "bash");

        // Env flags should come before container name and bash -c
        assert!(cmd.starts_with("docker exec -e FOO='bar' my-container bash -c"));
    }

    #[test]
    fn custom_shell_sh() {
        let env = HashMap::new();
        let cmd = build_docker_exec_command("my-container", &env, "ls -la", "/bin/sh");
        assert_eq!(cmd, "docker exec my-container /bin/sh -c 'ls -la'");
    }

    #[test]
    fn custom_shell_with_env_vars() {
        let mut env = HashMap::new();
        env.insert("FOO".to_string(), "bar".to_string());
        let cmd = build_docker_exec_command("container", &env, "test", "/bin/sh");

        // Should use custom shell
        assert!(cmd.contains("/bin/sh -c"));
        assert!(!cmd.contains("bash"));
    }

    #[test]
    fn invalid_env_key_is_skipped() {
        let mut env = HashMap::new();
        env.insert("GOOD_KEY".to_string(), "v1".to_string());
        env.insert("bad key'; rm -rf /".to_string(), "v2".to_string());
        let cmd = build_docker_exec_command("container", &env, "test", "bash");

        assert!(cmd.contains("-e GOOD_KEY='v1'"));
        assert!(
            !cmd.contains("rm -rf"),
            "invalid key must not reach the command: {cmd}"
        );
        assert!(!cmd.contains("bad key"));
    }

    #[test]
    fn all_env_keys_invalid_behaves_like_no_env() {
        let mut env = HashMap::new();
        env.insert("has space".to_string(), "v".to_string());
        let cmd = build_docker_exec_command("my-container", &env, "cargo test", "bash");
        assert_eq!(cmd, "docker exec my-container bash -c 'cargo test'");
    }
}

mod build_docker_run_command_tests {
    use super::*;
    use pretty_assertions::assert_eq;

    #[test]
    fn basic_command() {
        let config = super::common::test_docker_config("test-image:latest");
        let env = HashMap::new();
        let cmd = build_docker_run_command(&config, &env, "cargo test");

        assert!(cmd.starts_with("docker run --rm"));
        assert!(cmd.contains("test-image:latest"));
        assert!(cmd.contains("bash -c 'cargo test'"));
    }

    #[test]
    fn with_custom_shell() {
        let mut config = super::common::test_docker_config("test-image:latest");
        config.shell = "/bin/sh".to_string();
        let env = HashMap::new();
        let cmd = build_docker_run_command(&config, &env, "ls");

        assert!(cmd.contains("/bin/sh -c 'ls'"));
        assert!(!cmd.contains("bash"));
    }

    #[test]
    fn with_custom_working_directory() {
        let mut config = super::common::test_docker_config("test-image:latest");
        config.work_dir = Some("/custom".to_string());
        let env = HashMap::new();
        let cmd = build_docker_run_command(&config, &env, "ls");

        assert!(cmd.contains("-w /custom"));
    }

    #[test]
    fn with_env_vars() {
        let config = super::common::test_docker_config("test-image:latest");
        let mut env = HashMap::new();
        env.insert("FOO".to_string(), "bar".to_string());
        let cmd = build_docker_run_command(&config, &env, "test");

        assert!(cmd.contains("-e FOO='bar'"));
    }

    #[test]
    fn with_volume_mount() {
        let mut config = super::common::test_docker_config("test-image:latest");
        config.volume_mount = Some("/host/path:/container/path".to_string());
        let env = HashMap::new();
        let cmd = build_docker_run_command(&config, &env, "test");

        // Volume mount should be included
        assert!(cmd.contains("-v /host/path:/container/path"));
    }

    #[test]
    fn rm_flag_always_present() {
        let config = super::common::test_docker_config("test-image:latest");
        let env = HashMap::new();
        let cmd = build_docker_run_command(&config, &env, "test");

        // --rm flag should be present for auto-cleanup
        assert!(cmd.contains("--rm"));
    }

    #[test]
    fn command_structure_order() {
        let config = super::common::test_docker_config("test-image:latest");
        let env = HashMap::new();
        let cmd = build_docker_run_command(&config, &env, "test");

        // Check general structure: docker run --rm [volumes] [env] -w dir image bash -c 'cmd'
        let parts: Vec<&str> = cmd.split_whitespace().collect();
        assert_eq!(parts[0], "docker");
        assert_eq!(parts[1], "run");
        assert_eq!(parts[2], "--rm");
    }
}

mod filter_docker_warnings_tests {
    use super::*;
    use pretty_assertions::assert_eq;

    #[test]
    fn filters_variable_not_set_warnings() {
        let stderr =
            "WARN[0000] variable is not set. Defaulting to a blank string\nActual error here\n";
        let filtered = filter_docker_warnings(stderr);

        // Note: filter_docker_warnings joins with \n but doesn't preserve trailing newlines
        assert_eq!(filtered, "Actual error here");
    }

    #[test]
    fn preserves_non_warning_lines() {
        let stderr = "Error: something failed\nAnother error line\n";
        let filtered = filter_docker_warnings(stderr);

        // Note: filter_docker_warnings joins with \n but doesn't preserve trailing newlines
        assert_eq!(filtered, "Error: something failed\nAnother error line");
    }

    #[test]
    fn handles_empty_input() {
        let stderr = "";
        let filtered = filter_docker_warnings(stderr);

        assert_eq!(filtered, "");
    }

    #[test]
    fn handles_only_warnings() {
        let stderr = "WARN[0000] variable is not set. Defaulting to a blank string\nWARN[0001] another variable is not set. Defaulting to a blank string\n";
        let filtered = filter_docker_warnings(stderr);

        // When all lines are filtered, the result should be empty or just empty strings joined
        assert!(
            filtered.is_empty() || filtered.trim().is_empty(),
            "filtered: {:?}",
            filtered
        );
    }

    #[test]
    fn mixed_warnings_and_errors() {
        let stderr = "WARN[0000] variable is not set. Defaulting to a blank string
Error: connection failed
WARN[0001] another variable is not set. Defaulting to a blank string
Fatal: cannot continue";

        let filtered = filter_docker_warnings(stderr);

        // Should contain errors but not warnings
        assert!(filtered.contains("Error: connection failed"));
        assert!(filtered.contains("Fatal: cannot continue"));
        assert!(!filtered.contains("variable is not set"));
    }

    #[test]
    fn preserves_line_breaks() {
        let stderr = "Line 1\nLine 2\nLine 3\n";
        let filtered = filter_docker_warnings(stderr);

        // Line structure should be preserved
        let lines: Vec<&str> = filtered.lines().collect();
        assert_eq!(lines.len(), 3);
    }
}

mod execute_docker_command_tests {
    use super::*;
    use mockall::predicate::*;
    use pretty_assertions::assert_eq;
    use std::path::Path;

    #[tokio::test]
    async fn execute_successful_check() {
        let mut mock = MockCommandExecutor::new();
        mock.expect_is_container_running()
            .with(eq("test-container"))
            .returning(|_| true);
        mock.expect_execute()
            .withf(|cmd: &str, _| cmd.contains("cargo test"))
            .returning(|_, _| CommandOutput {
                success: true,
                stdout: "test result: ok. 5 passed\n".to_string(),
                stderr: String::new(),
            });

        let config = super::common::test_docker_target("test-image:latest");
        let env = HashMap::new();
        let result = execute_command_with_executor(
            "test-check".to_string(),
            "cargo test",
            Path::new("/app"),
            "test-container",
            &config,
            &env,
            &mock,
        )
        .await;

        assert_eq!(result.status, CheckStatus::Passed);
        assert!(result.output.contains("test result: ok"));
        assert!(result.error_output.is_empty());
    }

    #[tokio::test]
    async fn execute_failed_check() {
        let mut mock = MockCommandExecutor::new();
        mock.expect_is_container_running()
            .with(eq("test-container"))
            .returning(|_| true);
        mock.expect_execute()
            .withf(|cmd: &str, _| cmd.contains("cargo test"))
            .returning(|_, _| CommandOutput {
                success: false,
                stdout: String::new(),
                stderr: "error: test failed\n".to_string(),
            });

        let config = super::common::test_docker_target("test-image:latest");
        let env = HashMap::new();
        let result = execute_command_with_executor(
            "test-check".to_string(),
            "cargo test",
            Path::new("/app"),
            "test-container",
            &config,
            &env,
            &mock,
        )
        .await;

        assert_eq!(result.status, CheckStatus::Failed);
        assert!(result.error_output.contains("test failed"));
    }

    #[tokio::test]
    async fn uses_docker_exec_when_container_running() {
        let mut mock = MockCommandExecutor::new();
        mock.expect_is_container_running()
            .with(eq("test-container"))
            .returning(|_| true);
        mock.expect_execute()
            .withf(|cmd: &str, _| {
                // Should use docker exec when container is running
                cmd.starts_with("docker exec")
            })
            .returning(|_, _| CommandOutput {
                success: true,
                stdout: String::new(),
                stderr: String::new(),
            });

        let config = super::common::test_docker_target("test-image:latest");
        let env = HashMap::new();
        let _result = execute_command_with_executor(
            "test-check".to_string(),
            "test",
            Path::new("/app"),
            "test-container",
            &config,
            &env,
            &mock,
        )
        .await;
    }

    #[tokio::test]
    async fn uses_docker_run_when_container_not_running() {
        let mut mock = MockCommandExecutor::new();
        mock.expect_is_container_running()
            .with(eq("test-container"))
            .returning(|_| false);
        mock.expect_execute()
            .withf(|cmd: &str, _| {
                // Should use docker run when container is not running
                cmd.starts_with("docker run")
            })
            .returning(|_, _| CommandOutput {
                success: true,
                stdout: String::new(),
                stderr: String::new(),
            });

        let config = super::common::test_docker_target("test-image:latest");
        let env = HashMap::new();
        let _result = execute_command_with_executor(
            "test-check".to_string(),
            "test",
            Path::new("/app"),
            "test-container",
            &config,
            &env,
            &mock,
        )
        .await;
    }

    #[tokio::test]
    async fn captures_duration() {
        let mock = mock_executor_success("output");

        let config = super::common::test_docker_target("test-image:latest");
        let env = HashMap::new();
        let result = execute_command_with_executor(
            "test-check".to_string(),
            "test",
            Path::new("/app"),
            "test-container",
            &config,
            &env,
            &mock,
        )
        .await;

        // Duration should be captured
        assert!(result.started_at.is_some());
        assert!(result.finished_at.is_some());
    }

    #[tokio::test]
    async fn filters_docker_warnings_in_stderr() {
        let mut mock = MockCommandExecutor::new();
        mock.expect_is_container_running().returning(|_| true);
        mock.expect_execute().returning(|_, _| CommandOutput {
            success: true,
            stdout: String::new(),
            stderr: "WARN[0000] variable is not set. Defaulting to a blank string\nActual error\n"
                .to_string(),
        });

        let config = super::common::test_docker_target("test-image:latest");
        let env = HashMap::new();
        let result = execute_command_with_executor(
            "test-check".to_string(),
            "test",
            Path::new("/app"),
            "test-container",
            &config,
            &env,
            &mock,
        )
        .await;

        // Docker warnings should be filtered from error output
        assert!(!result.error_output.contains("variable is not set"));
        assert!(result.error_output.contains("Actual error"));
    }
}

mod check_result_tests {
    use super::*;
    use pretty_assertions::assert_eq;

    #[test]
    fn pending_returns_correct_check_id() {
        let result = CheckResult::pending("my-check");
        assert_eq!(result.check_id, "my-check");
    }

    #[test]
    fn pending_has_pending_status() {
        let result = CheckResult::pending("test");
        assert_eq!(result.status, CheckStatus::Pending);
    }

    #[test]
    fn pending_has_empty_output() {
        let result = CheckResult::pending("test");
        assert!(result.output.is_empty());
        assert!(result.error_output.is_empty());
    }

    #[test]
    fn pending_has_zero_duration() {
        let result = CheckResult::pending("test");
        assert_eq!(result.duration_ms, 0);
    }

    #[test]
    fn pending_has_no_timestamps() {
        let result = CheckResult::pending("test");
        assert!(result.started_at.is_none());
        assert!(result.finished_at.is_none());
    }

    #[test]
    fn skipped_returns_correct_check_id() {
        let result = CheckResult::skipped("skip-check");
        assert_eq!(result.check_id, "skip-check");
    }

    #[test]
    fn skipped_has_skipped_status() {
        let result = CheckResult::skipped("test");
        assert_eq!(result.status, CheckStatus::Skipped);
    }

    #[test]
    fn skipped_output_contains_no_changes() {
        let result = CheckResult::skipped("test");
        assert!(
            result.output.contains("No changes detected"),
            "Expected output to contain 'No changes detected', got: {}",
            result.output
        );
    }

    #[test]
    fn skipped_has_zero_duration() {
        let result = CheckResult::skipped("test");
        assert_eq!(result.duration_ms, 0);
    }

    #[test]
    fn on_demand_returns_correct_check_id() {
        let result = CheckResult::on_demand("demand-check");
        assert_eq!(result.check_id, "demand-check");
    }

    #[test]
    fn on_demand_has_on_demand_status() {
        let result = CheckResult::on_demand("test");
        assert_eq!(result.status, CheckStatus::OnDemand);
    }

    #[test]
    fn on_demand_output_contains_press_t() {
        let result = CheckResult::on_demand("test");
        assert!(
            result.output.contains("Press 't'"),
            "Expected output to contain 'Press 't'', got: {}",
            result.output
        );
    }

    #[test]
    fn on_demand_has_zero_duration() {
        let result = CheckResult::on_demand("test");
        assert_eq!(result.duration_ms, 0);
    }

    #[rstest]
    #[case("check-1")]
    #[case("my-clippy-check")]
    #[case("unit-tests")]
    fn factory_methods_preserve_check_id(#[case] check_id: &str) {
        assert_eq!(CheckResult::pending(check_id).check_id, check_id);
        assert_eq!(CheckResult::skipped(check_id).check_id, check_id);
        assert_eq!(CheckResult::on_demand(check_id).check_id, check_id);
    }
}

mod check_runner_tests {
    use super::*;
    use ci_tui::runner::{CheckRunner, CommandOutput, MockCommandExecutor, RunnerEvent};
    use common::configs::ConfigBuilder;
    use common::{make_on_demand_check, make_widget_check};
    use std::path::Path;
    use std::sync::Arc;

    /// Helper to collect all events from a channel
    async fn collect_events(mut rx: tokio::sync::mpsc::Receiver<RunnerEvent>) -> Vec<RunnerEvent> {
        let mut events = Vec::new();
        while let Some(event) = rx.recv().await {
            events.push(event);
        }
        events
    }

    #[test]
    fn new_creates_runner_with_real_executor() {
        let config = ConfigBuilder::new().build();
        let _runner = CheckRunner::new(config, Path::new("/tmp"));
        // If we get here without panic, the constructor works
    }

    #[test]
    fn with_executor_accepts_custom_executor() {
        let config = ConfigBuilder::new().build();
        let mock = MockCommandExecutor::new();
        let _runner = CheckRunner::with_executor(config, Path::new("/tmp"), Arc::new(mock));
        // If we get here without panic, the constructor works
    }

    #[tokio::test]
    async fn run_checks_sequential_sends_events_in_order() {
        // Create mock that returns success for any command
        let mut mock = MockCommandExecutor::new();
        mock.expect_is_container_running().returning(|_| true);
        mock.expect_execute().returning(|_, _| CommandOutput {
            success: true,
            stdout: "ok".to_string(),
            stderr: String::new(),
        });

        let config = ConfigBuilder::new().build();
        let runner = CheckRunner::with_executor(config, Path::new("/tmp"), Arc::new(mock));

        let (tx, rx) = tokio::sync::mpsc::channel(100);
        let checks = vec![make_widget_check("check1", "group1", "Check 1", false)];

        runner.run_checks(checks, tx).await.unwrap();

        let events = collect_events(rx).await;

        // Verify event order for sequential execution:
        // GroupStarted -> CheckStarted -> CheckFinished -> GroupFinished -> AllFinished
        assert!(
            events.len() >= 5,
            "Expected at least 5 events, got {}",
            events.len()
        );

        // Check first event is GroupStarted
        assert!(
            matches!(&events[0], RunnerEvent::GroupStarted { group } if group == "group1"),
            "First event should be GroupStarted, got {:?}",
            events[0]
        );

        // Check CheckStarted comes before CheckFinished
        let check_started_idx = events.iter().position(
            |e| matches!(e, RunnerEvent::CheckStarted { check_id } if check_id == "check1"),
        );
        let check_finished_idx = events.iter().position(
            |e| matches!(e, RunnerEvent::CheckFinished { result } if result.check_id == "check1"),
        );
        assert!(
            check_started_idx.is_some() && check_finished_idx.is_some(),
            "Should have CheckStarted and CheckFinished events"
        );
        assert!(
            check_started_idx.unwrap() < check_finished_idx.unwrap(),
            "CheckStarted should come before CheckFinished"
        );

        // Check GroupFinished comes before AllFinished
        let group_finished_idx = events
            .iter()
            .position(|e| matches!(e, RunnerEvent::GroupFinished { group } if group == "group1"));
        let all_finished_idx = events
            .iter()
            .position(|e| matches!(e, RunnerEvent::AllFinished));
        assert!(
            group_finished_idx.is_some() && all_finished_idx.is_some(),
            "Should have GroupFinished and AllFinished events"
        );
        assert!(
            group_finished_idx.unwrap() < all_finished_idx.unwrap(),
            "GroupFinished should come before AllFinished"
        );
    }

    #[tokio::test]
    async fn run_checks_skips_on_demand_checks() {
        // Create mock that should NOT be called for on-demand checks
        let mut mock = MockCommandExecutor::new();
        mock.expect_is_container_running().returning(|_| true);
        // No execute expectation for on-demand checks

        let config = ConfigBuilder::new().build();
        let runner = CheckRunner::with_executor(config, Path::new("/tmp"), Arc::new(mock));

        let (tx, rx) = tokio::sync::mpsc::channel(100);
        let checks = vec![make_on_demand_check(
            "on-demand-check",
            "group1",
            "On Demand",
        )];

        runner.run_checks(checks, tx).await.unwrap();

        let events = collect_events(rx).await;

        // Verify no CheckStarted or CheckFinished events for on-demand check
        let has_check_started = events.iter().any(|e| {
            matches!(e, RunnerEvent::CheckStarted { check_id } if check_id == "on-demand-check")
        });
        let has_check_finished = events.iter().any(|e| {
            matches!(e, RunnerEvent::CheckFinished { result } if result.check_id == "on-demand-check")
        });

        assert!(
            !has_check_started,
            "On-demand check should not have CheckStarted event"
        );
        assert!(
            !has_check_finished,
            "On-demand check should not have CheckFinished event"
        );

        // Should still have GroupStarted, GroupFinished, and AllFinished
        assert!(events
            .iter()
            .any(|e| matches!(e, RunnerEvent::GroupStarted { .. })));
        assert!(events
            .iter()
            .any(|e| matches!(e, RunnerEvent::GroupFinished { .. })));
        assert!(events.iter().any(|e| matches!(e, RunnerEvent::AllFinished)));
    }

    #[tokio::test]
    async fn run_checks_handles_pre_commands_success() {
        let mut mock = MockCommandExecutor::new();
        mock.expect_is_container_running().returning(|_| true);
        mock.expect_execute().returning(|_, _| CommandOutput {
            success: true,
            stdout: "ok".to_string(),
            stderr: String::new(),
        });

        // Create config with a pre-command
        let config = ConfigBuilder::new()
            .with_pre_command("lint", "warmup", "echo warmup")
            .with_check(
                "lint",
                "clippy",
                common::configs::CheckBuilder::new("Clippy", "cargo clippy").build(),
            )
            .build();

        let runner = CheckRunner::with_executor(config, Path::new("/tmp"), Arc::new(mock));

        let (tx, rx) = tokio::sync::mpsc::channel(100);
        let checks = vec![make_widget_check("clippy", "lint", "Clippy", false)];

        runner.run_checks(checks, tx).await.unwrap();

        let events = collect_events(rx).await;

        // Verify PreCommandStarted and PreCommandFinished events are sent
        let has_pre_started = events.iter().any(|e| {
            matches!(e, RunnerEvent::PreCommandStarted { group, name }
                if group == "lint" && name == "warmup")
        });
        let has_pre_finished = events.iter().any(|e| {
            matches!(e, RunnerEvent::PreCommandFinished { group, name, success, .. }
                if group == "lint" && name == "warmup" && *success)
        });

        assert!(has_pre_started, "Should have PreCommandStarted event");
        assert!(
            has_pre_finished,
            "Should have PreCommandFinished event with success=true"
        );
    }

    #[tokio::test]
    async fn local_mode_pre_command_runs_on_host_shell() {
        let mut mock = MockCommandExecutor::new();
        mock.expect_is_container_running().times(0);
        mock.expect_execute()
            .withf(|cmd, _| cmd == "bash -c 'echo warmup'")
            .times(1)
            .returning(|_, _| CommandOutput {
                success: true,
                stdout: "ok".to_string(),
                stderr: String::new(),
            });
        mock.expect_execute()
            .withf(|cmd, _| cmd != "bash -c 'echo warmup'")
            .returning(|_, _| CommandOutput {
                success: true,
                stdout: "ok".to_string(),
                stderr: String::new(),
            });

        let mut config = ConfigBuilder::new()
            .with_pre_command("lint", "warmup", "echo warmup")
            .with_check(
                "lint",
                "clippy",
                common::configs::CheckBuilder::new("Clippy", "cargo clippy").build(),
            )
            .build();
        config.runner = ci_tui::config::RunnerMode::Local;

        let runner = CheckRunner::with_executor(config, Path::new("/tmp"), Arc::new(mock));
        let (tx, rx) = tokio::sync::mpsc::channel(100);
        let checks = vec![make_widget_check("clippy", "lint", "Clippy", false)];
        runner.run_checks(checks, tx).await.unwrap();

        let events = collect_events(rx).await;
        assert!(
            events.iter().any(|e| matches!(e,
            RunnerEvent::PreCommandFinished { name, success, .. } if name == "warmup" && *success))
        );
    }

    #[tokio::test]
    async fn run_checks_pre_command_failure_stops_execution() {
        let mut mock = MockCommandExecutor::new();
        mock.expect_is_container_running().returning(|_| true);
        mock.expect_execute().returning(|_, _| CommandOutput {
            success: false, // Pre-command fails
            stdout: String::new(),
            stderr: "pre-command failed".to_string(),
        });

        // Create config with a failing pre-command
        let config = ConfigBuilder::new()
            .with_pre_command("lint", "setup", "failing-command")
            .with_check(
                "lint",
                "clippy",
                common::configs::CheckBuilder::new("Clippy", "cargo clippy").build(),
            )
            .build();

        let runner = CheckRunner::with_executor(config, Path::new("/tmp"), Arc::new(mock));

        let (tx, rx) = tokio::sync::mpsc::channel(100);
        let checks = vec![make_widget_check("clippy", "lint", "Clippy", false)];

        runner.run_checks(checks, tx).await.unwrap();

        let events = collect_events(rx).await;

        // Verify pre-command failure stops execution
        let has_pre_failed = events
            .iter()
            .any(|e| matches!(e, RunnerEvent::PreCommandFinished { success, .. } if !*success));
        assert!(
            has_pre_failed,
            "Should have PreCommandFinished with success=false"
        );

        // No CheckStarted for the check (execution stopped)
        let has_check_started = events
            .iter()
            .any(|e| matches!(e, RunnerEvent::CheckStarted { .. }));
        assert!(
            !has_check_started,
            "Check should not start after pre-command failure"
        );

        // Should still have GroupFinished and AllFinished (cleanup events)
        assert!(events
            .iter()
            .any(|e| matches!(e, RunnerEvent::GroupFinished { .. })));
        assert!(events.iter().any(|e| matches!(e, RunnerEvent::AllFinished)));
    }

    #[tokio::test]
    async fn run_checks_host_pre_command_runs_raw_command() {
        let mut mock = MockCommandExecutor::new();
        // Host pre-commands skip the Docker path entirely.
        mock.expect_is_container_running().times(0);
        // Executor must receive the raw command, NOT a docker exec/run wrapped version.
        // Make it fail so the check never runs and we only need one execute expectation.
        mock.expect_execute()
            .withf(|cmd, _| cmd == "echo hello")
            .times(1)
            .returning(|_, _| CommandOutput {
                success: false,
                stdout: "hello".to_string(),
                stderr: String::new(),
            });

        let config = ConfigBuilder::new()
            .with_host_pre_command("lint", "host-warmup", "echo hello")
            .with_check(
                "lint",
                "clippy",
                common::configs::CheckBuilder::new("Clippy", "cargo clippy").build(),
            )
            .build();

        let runner = CheckRunner::with_executor(config, Path::new("/tmp"), Arc::new(mock));

        let (tx, rx) = tokio::sync::mpsc::channel(100);
        let checks = vec![make_widget_check("clippy", "lint", "Clippy", false)];

        runner.run_checks(checks, tx).await.unwrap();

        let events = collect_events(rx).await;

        // Pre-command should have been started and finished with success=false
        let has_pre_started = events.iter().any(|e| {
            matches!(e, RunnerEvent::PreCommandStarted { group, name }
                if group == "lint" && name == "host-warmup")
        });
        let has_pre_failed = events.iter().any(|e| {
            matches!(e, RunnerEvent::PreCommandFinished { group, name, success, .. }
                if group == "lint" && name == "host-warmup" && !*success)
        });
        assert!(has_pre_started, "Should have PreCommandStarted event");
        assert!(
            has_pre_failed,
            "Should have PreCommandFinished with success=false"
        );

        // Check must NOT have started (pre-command failure stops execution)
        let has_check_started = events
            .iter()
            .any(|e| matches!(e, RunnerEvent::CheckStarted { .. }));
        assert!(
            !has_check_started,
            "Check should not start after host pre-command failure"
        );
    }

    #[tokio::test]
    async fn all_on_demand_group_skips_pre_commands() {
        let mut mock = MockCommandExecutor::new();
        // has_runnable=false → pre_commands never run → executor never called for execute/is_running
        mock.expect_is_container_running().times(0);
        mock.expect_execute().times(0);

        let config = ConfigBuilder::new()
            .with_pre_command("lint", "warmup", "echo warmup")
            .with_check(
                "lint",
                "expensive",
                common::configs::CheckBuilder::new("Expensive", "slow-test")
                    .on_demand()
                    .build(),
            )
            .build();

        let runner = CheckRunner::with_executor(config, Path::new("/tmp"), Arc::new(mock));

        let (tx, rx) = tokio::sync::mpsc::channel(100);
        let checks = vec![make_on_demand_check("expensive", "lint", "Expensive")];

        runner.run_checks(checks, tx).await.unwrap();

        let events = collect_events(rx).await;

        // No PreCommand events — has_runnable=false
        assert!(
            !events
                .iter()
                .any(|e| matches!(e, RunnerEvent::PreCommandStarted { .. })),
            "pre-commands must be skipped when group has no runnable checks"
        );
        // Group lifecycle still fires
        assert!(events
            .iter()
            .any(|e| matches!(e, RunnerEvent::GroupStarted { .. })));
        assert!(events
            .iter()
            .any(|e| matches!(e, RunnerEvent::GroupFinished { .. })));
        assert!(events.iter().any(|e| matches!(e, RunnerEvent::AllFinished)));
    }
}

mod run_single_check_with_executor_tests {
    use super::*;
    use ci_tui::runner::run_single_check_with_executor;
    use mockall::predicate::*;
    use std::path::Path;

    #[tokio::test]
    async fn uses_default_container_when_check_has_none() {
        let mut mock = MockCommandExecutor::new();
        mock.expect_is_container_running()
            .with(eq("default-container"))
            .returning(|_| true);
        mock.expect_execute()
            .withf(|cmd: &str, _| cmd.contains("default-container") && cmd.contains("echo hi"))
            .returning(|_, _| CommandOutput {
                success: true,
                stdout: "hi".into(),
                stderr: String::new(),
            });

        let check = super::common::make_exec_check("c1", "echo hi", None);
        let cfg = super::common::test_docker_target("img:latest");
        let env = HashMap::new();
        let result = run_single_check_with_executor(
            &check,
            Path::new("/app"),
            "default-container",
            &cfg,
            &env,
            &mock,
        )
        .await;

        assert_eq!(result.status, CheckStatus::Passed);
        assert_eq!(result.check_id, "c1");
        assert_eq!(result.output, "hi");
    }

    #[tokio::test]
    async fn per_check_container_overrides_default() {
        let mut mock = MockCommandExecutor::new();
        mock.expect_is_container_running()
            .with(eq("override-container"))
            .returning(|_| true);
        mock.expect_execute()
            .withf(|cmd: &str, _| cmd.contains("override-container"))
            .returning(|_, _| CommandOutput {
                success: true,
                stdout: String::new(),
                stderr: String::new(),
            });

        let check = super::common::make_exec_check("c1", "ls", Some("override-container"));
        let cfg = super::common::test_docker_target("img:latest");
        let env = HashMap::new();
        let _ = run_single_check_with_executor(
            &check,
            Path::new("/app"),
            "default-container",
            &cfg,
            &env,
            &mock,
        )
        .await;
    }

    #[tokio::test]
    async fn merges_global_and_check_env_with_check_taking_precedence() {
        let mut mock = MockCommandExecutor::new();
        mock.expect_is_container_running().returning(|_| true);
        mock.expect_execute()
            .withf(|cmd: &str, _| {
                // global FOO=global overridden to FOO=check
                cmd.contains("-e FOO='check'") && cmd.contains("-e BAR='global'")
            })
            .returning(|_, _| CommandOutput {
                success: true,
                stdout: String::new(),
                stderr: String::new(),
            });

        let mut check = super::common::make_exec_check("c1", "env", None);
        check.definition.env.insert("FOO".into(), "check".into());

        let cfg = super::common::test_docker_target("img:latest");
        let mut env = HashMap::new();
        env.insert("FOO".into(), "global".into());
        env.insert("BAR".into(), "global".into());

        let _ =
            run_single_check_with_executor(&check, Path::new("/app"), "c", &cfg, &env, &mock).await;
    }

    #[tokio::test]
    async fn reports_failure_when_exec_fails() {
        let mut mock = MockCommandExecutor::new();
        mock.expect_is_container_running().returning(|_| true);
        mock.expect_execute().returning(|_, _| CommandOutput {
            success: false,
            stdout: String::new(),
            stderr: "oops".into(),
        });

        let check = super::common::make_exec_check("c1", "false", None);
        let cfg = super::common::test_docker_target("img:latest");
        let env = HashMap::new();
        let result =
            run_single_check_with_executor(&check, Path::new("/app"), "c", &cfg, &env, &mock).await;
        assert_eq!(result.status, CheckStatus::Failed);
        assert!(result.error_output.contains("oops"));
    }
}

mod run_check_with_command_with_executor_tests {
    use super::*;
    use ci_tui::runner::run_check_with_command_with_executor;
    use std::path::Path;

    // The supplied `command` arg — NOT check.resolved_command — is what gets executed
    #[tokio::test]
    async fn uses_supplied_command_not_resolved_command() {
        let mut mock = MockCommandExecutor::new();
        mock.expect_is_container_running().returning(|_| true);
        mock.expect_execute()
            .withf(|cmd: &str, _| cmd.contains("phpunit-all") && !cmd.contains("phpunit a.rs"))
            .returning(|_, _| CommandOutput {
                success: true,
                stdout: String::new(),
                stderr: String::new(),
            });

        let c = super::common::make_exec_check("phpunit", "phpunit a.rs", None);
        let cfg = super::common::test_docker_target("img:latest");
        let env = HashMap::new();
        let _ = run_check_with_command_with_executor(
            &c,
            "phpunit-all",
            Path::new("/app"),
            "default",
            &cfg,
            &env,
            &mock,
        )
        .await;
    }
}

mod run_fix_command_with_executor_tests {
    use super::*;
    use ci_tui::runner::run_fix_command_with_executor;
    use std::path::Path;

    #[tokio::test]
    async fn check_id_is_literal_fix() {
        let mut mock = MockCommandExecutor::new();
        mock.expect_is_container_running().returning(|_| true);
        mock.expect_execute().returning(|_, _| CommandOutput {
            success: true,
            stdout: String::new(),
            stderr: String::new(),
        });

        let cfg = super::common::test_docker_target("img:latest");
        let env = HashMap::new();
        let result = run_fix_command_with_executor(
            "cargo fmt",
            Path::new("/app"),
            "container",
            &cfg,
            &env,
            &mock,
        )
        .await;
        assert_eq!(result.check_id, "fix");
        assert_eq!(result.status, CheckStatus::Passed);
    }

    #[tokio::test]
    async fn uses_docker_run_when_container_not_running() {
        let mut mock = MockCommandExecutor::new();
        mock.expect_is_container_running().returning(|_| false);
        mock.expect_execute()
            .withf(|cmd: &str, _| cmd.starts_with("docker run") && cmd.contains("cargo fmt"))
            .returning(|_, _| CommandOutput {
                success: true,
                stdout: String::new(),
                stderr: String::new(),
            });

        let cfg = super::common::test_docker_target("img:latest");
        let env = HashMap::new();
        let _ = run_fix_command_with_executor(
            "cargo fmt",
            Path::new("/app"),
            "container",
            &cfg,
            &env,
            &mock,
        )
        .await;
    }
}
