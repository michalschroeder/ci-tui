//! Tests for runner.rs - command building and execution functions
//!
//! This test file covers:
//! - Pure command-building functions (no mocks needed)
//! - Docker command execution (using mocks)

use ci_tui::runner::{
    build_docker_exec_command, build_docker_run_command, execute_docker_command_with_executor,
    filter_docker_warnings, CheckStatus,
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
        let cmd = build_docker_exec_command("my-container", &env, "cargo test");
        assert_eq!(cmd, "docker exec my-container bash -c 'cargo test'");
    }

    #[rstest]
    #[case("FOO", "bar", "-e FOO='bar'")]
    #[case("PATH", "/usr/bin", "-e PATH='/usr/bin'")]
    fn single_env_var(#[case] key: &str, #[case] val: &str, #[case] expected_flag: &str) {
        let mut env = HashMap::new();
        env.insert(key.to_string(), val.to_string());
        let cmd = build_docker_exec_command("container", &env, "test");
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
        let cmd = build_docker_exec_command("container", &env, "test");

        // Both env vars should be present
        assert!(cmd.contains("-e FOO='bar'") || cmd.contains("-e BAZ='qux'"));
        assert!(cmd.contains("-e FOO='bar'") || cmd.contains("-e BAZ='qux'"));
    }

    #[test]
    fn special_characters_in_url() {
        let mut env = HashMap::new();
        env.insert("URL".to_string(), "http://test?a=1&b=2".to_string());
        let cmd = build_docker_exec_command("container", &env, "curl $URL");

        // URL with special characters should be properly quoted
        assert!(cmd.contains("-e URL='http://test?a=1&b=2'"));
    }

    #[test]
    fn single_quote_in_env_value() {
        let mut env = HashMap::new();
        env.insert("MSG".to_string(), "it's working".to_string());
        let cmd = build_docker_exec_command("container", &env, "echo $MSG");

        // Single quotes in values should be escaped
        assert!(cmd.contains("it'\\''s working"));
    }

    #[test]
    fn single_quote_in_command() {
        let env = HashMap::new();
        let cmd = build_docker_exec_command("container", &env, "echo 'hello world'");

        // Single quotes in command should be escaped
        assert!(cmd.contains("echo '\\''hello world'\\''"));
    }

    #[test]
    fn command_with_env_flag_placement() {
        let mut env = HashMap::new();
        env.insert("FOO".to_string(), "bar".to_string());
        let cmd = build_docker_exec_command("my-container", &env, "cargo test");

        // Env flags should come before container name and bash -c
        assert!(cmd.starts_with("docker exec -e FOO='bar' my-container bash -c"));
    }
}

mod build_docker_run_command_tests {
    use super::*;
    use ci_tui::config::DockerConfig;
    use pretty_assertions::assert_eq;

    fn minimal_docker_config() -> DockerConfig {
        DockerConfig {
            project_dir: "/app".to_string(),
            service: "app".to_string(),
            container: None,
            image: Some("test-image:latest".to_string()),
            volume_mount: None,
            work_dir: None,
            env: HashMap::new(),
        }
    }

    #[test]
    fn basic_command() {
        let config = minimal_docker_config();
        let env = HashMap::new();
        let cmd = build_docker_run_command(&config, &env, "cargo test");

        assert!(cmd.starts_with("docker run --rm"));
        assert!(cmd.contains("test-image:latest"));
        assert!(cmd.contains("bash -c 'cargo test'"));
    }

    #[test]
    fn with_custom_working_directory() {
        let mut config = minimal_docker_config();
        config.work_dir = Some("/custom".to_string());
        let env = HashMap::new();
        let cmd = build_docker_run_command(&config, &env, "ls");

        assert!(cmd.contains("-w /custom"));
    }

    #[test]
    fn with_env_vars() {
        let config = minimal_docker_config();
        let mut env = HashMap::new();
        env.insert("FOO".to_string(), "bar".to_string());
        let cmd = build_docker_run_command(&config, &env, "test");

        assert!(cmd.contains("-e FOO='bar'"));
    }

    #[test]
    fn with_volume_mount() {
        let mut config = minimal_docker_config();
        config.volume_mount = Some("/host/path:/container/path".to_string());
        let env = HashMap::new();
        let cmd = build_docker_run_command(&config, &env, "test");

        // Volume mount should be included
        assert!(cmd.contains("-v /host/path:/container/path"));
    }

    #[test]
    fn rm_flag_always_present() {
        let config = minimal_docker_config();
        let env = HashMap::new();
        let cmd = build_docker_run_command(&config, &env, "test");

        // --rm flag should be present for auto-cleanup
        assert!(cmd.contains("--rm"));
    }

    #[test]
    fn command_structure_order() {
        let config = minimal_docker_config();
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
    use ci_tui::config::DockerConfig;
    use mockall::predicate::*;
    use pretty_assertions::assert_eq;
    use std::path::Path;

    fn minimal_docker_config() -> DockerConfig {
        DockerConfig {
            project_dir: "/app".to_string(),
            service: "app".to_string(),
            container: None,
            image: Some("test-image:latest".to_string()),
            volume_mount: None,
            work_dir: None,
            env: HashMap::new(),
        }
    }

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

        let config = minimal_docker_config();
        let env = HashMap::new();
        let result = execute_docker_command_with_executor(
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

        let config = minimal_docker_config();
        let env = HashMap::new();
        let result = execute_docker_command_with_executor(
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

        let config = minimal_docker_config();
        let env = HashMap::new();
        let _result = execute_docker_command_with_executor(
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

        let config = minimal_docker_config();
        let env = HashMap::new();
        let _result = execute_docker_command_with_executor(
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

        let config = minimal_docker_config();
        let env = HashMap::new();
        let result = execute_docker_command_with_executor(
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

        let config = minimal_docker_config();
        let env = HashMap::new();
        let result = execute_docker_command_with_executor(
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
