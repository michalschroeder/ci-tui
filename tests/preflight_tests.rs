//! Tests for the docker startup probe (`preflight::docker_warnings`): warn when
//! changed files won't resolve inside the containers the selected checks use
//! (repo not at the exec container's WORKDIR, or `docker run` fallback
//! without a matching repo mount).

use ci_tui::checks::CheckToRun;
use ci_tui::preflight::docker_warnings;
use ci_tui::runner::ExecTarget;
use std::path::Path;
use std::sync::{Arc, Mutex};

mod common;
use common::configs::ConfigBuilder;
use common::{local_sh, make_exec_check, CommandOutput, MockCommandExecutor};

/// Crate root: `Cargo.toml` exists there on the host
fn root() -> &'static Path {
    Path::new(env!("CARGO_MANIFEST_DIR"))
}

fn files(paths: &[&str]) -> Vec<String> {
    paths.iter().map(|p| p.to_string()).collect()
}

/// One selected check on the default container
fn default_check() -> Vec<CheckToRun> {
    vec![make_exec_check("a", "true", None)]
}

/// Probe with one default-container check and `Cargo.toml` changed
async fn warnings_for(target: &ExecTarget, mock: &MockCommandExecutor) -> Vec<String> {
    docker_warnings(
        target,
        &default_check(),
        &files(&["Cargo.toml"]),
        root(),
        mock,
    )
    .await
}

fn container(target: &ExecTarget) -> String {
    match target {
        ExecTarget::Docker(docker) => docker.container_name(),
        ExecTarget::Local(_) => unreachable!("docker target expected"),
    }
}

/// Mock: container running; probe command recorded, answers `found`
fn running_mock(found: bool, seen: Arc<Mutex<Vec<String>>>) -> MockCommandExecutor {
    let mut mock = MockCommandExecutor::new();
    mock.expect_is_container_running().returning(|_| true);
    mock.expect_execute().times(1).returning(move |cmd, _, _| {
        seen.lock().unwrap().push(cmd.to_string());
        CommandOutput {
            success: found,
            stdout: String::new(),
            stderr: String::new(),
        }
    });
    mock
}

/// Mock: container not running; no probe may run
fn stopped_mock() -> MockCommandExecutor {
    let mut mock = MockCommandExecutor::new();
    mock.expect_is_container_running().returning(|_| false);
    mock.expect_execute().never();
    mock
}

#[tokio::test]
async fn exec_probe_found_gives_no_warning() {
    // exec checks resolve {files} against the container WORKDIR, not work_dir
    let target = ConfigBuilder::new().with_work_dir("/build/").build().runner;
    let seen = Default::default();
    let mock = running_mock(true, Arc::clone(&seen));

    let warnings = warnings_for(&target, &mock).await;

    assert!(warnings.is_empty(), "{warnings:?}");
    let cmd = seen.lock().unwrap()[0].clone();
    assert!(
        cmd.starts_with(&format!("docker exec {} ", container(&target))),
        "{cmd}"
    );
    assert!(cmd.contains("test -e"), "{cmd}");
    assert!(cmd.contains("Cargo.toml"), "{cmd}");
    assert!(
        !cmd.contains("/Cargo.toml"),
        "path must stay repo-relative: {cmd}"
    );
    assert!(!cmd.contains("/build"), "{cmd}");
    assert!(!cmd.contains(" -w "), "exec keeps container WORKDIR: {cmd}");
}

#[tokio::test]
async fn exec_probe_missing_warns_with_file_and_container() {
    let target = ConfigBuilder::new().build().runner;
    let mock = running_mock(false, Default::default());

    let warnings = warnings_for(&target, &mock).await;
    assert_eq!(
        warnings.len(),
        1,
        "missing file must warn once: {warnings:?}"
    );
    let warning = &warnings[0];

    assert!(warning.contains("Cargo.toml"), "{warning}");
    assert!(warning.contains(&container(&target)), "{warning}");
    assert!(warning.contains("working directory"), "{warning}");
    // docker.work_dir does not apply to `docker exec`: must not be suggested
    assert!(!warning.contains("docker.work_dir"), "{warning}");
}

#[tokio::test]
async fn exec_probe_skips_files_deleted_on_host() {
    let target = ConfigBuilder::new().build().runner;
    let seen = Default::default();
    let mock = running_mock(true, Arc::clone(&seen));

    docker_warnings(
        &target,
        &default_check(),
        &files(&["gone/deleted_file.rs", "Cargo.toml"]),
        root(),
        &mock,
    )
    .await;

    let cmd = seen.lock().unwrap()[0].clone();
    assert!(cmd.contains("Cargo.toml"), "{cmd}");
    assert!(!cmd.contains("deleted_file"), "{cmd}");
}

#[tokio::test]
async fn exec_no_probe_when_all_changed_files_deleted() {
    let target = ConfigBuilder::new().build().runner;
    let mut mock = MockCommandExecutor::new();
    mock.expect_is_container_running().returning(|_| true);
    mock.expect_execute().never();

    let warnings = docker_warnings(
        &target,
        &default_check(),
        &files(&["gone/deleted_file.rs"]),
        root(),
        &mock,
    )
    .await;

    assert!(warnings.is_empty(), "{warnings:?}");
}

#[tokio::test]
async fn run_fallback_without_volume_mount_warns() {
    let target = ConfigBuilder::new().build().runner;
    let mock = stopped_mock();

    let warnings = warnings_for(&target, &mock).await;
    assert_eq!(warnings.len(), 1, "{warnings:?}");
    let warning = &warnings[0];

    assert!(warning.contains("docker.volume_mount"), "{warning}");
    assert!(warning.contains("docker.work_dir"), "{warning}");
    assert!(warning.contains("docker run"), "{warning}");
    assert!(warning.contains(&container(&target)), "{warning}");
}

#[tokio::test]
async fn run_fallback_with_volume_mount_gives_no_warning() {
    let target = ConfigBuilder::new()
        .with_volume_mount(".:/app")
        .build()
        .runner;
    let mock = stopped_mock();

    let warnings = warnings_for(&target, &mock).await;

    assert!(warnings.is_empty(), "{warnings:?}");
}

#[tokio::test]
async fn local_mode_never_probes() {
    let mut mock = MockCommandExecutor::new();
    mock.expect_is_container_running().never();
    mock.expect_execute().never();

    let warnings = warnings_for(&local_sh(), &mock).await;

    assert!(warnings.is_empty(), "{warnings:?}");
}

#[tokio::test]
async fn no_changed_files_never_probes() {
    let target = ConfigBuilder::new().build().runner;
    let mut mock = MockCommandExecutor::new();
    mock.expect_is_container_running().never();
    mock.expect_execute().never();

    let warnings = docker_warnings(&target, &default_check(), &[], root(), &mock).await;

    assert!(warnings.is_empty(), "{warnings:?}");
}

#[tokio::test]
async fn no_selected_checks_never_probes() {
    let target = ConfigBuilder::new().build().runner;
    let mut mock = MockCommandExecutor::new();
    mock.expect_is_container_running().never();
    mock.expect_execute().never();

    let warnings = docker_warnings(&target, &[], &files(&["Cargo.toml"]), root(), &mock).await;

    assert!(warnings.is_empty(), "{warnings:?}");
}

#[tokio::test]
async fn check_container_override_is_probed_instead_of_default() {
    let target = ConfigBuilder::new().build().runner;
    let checked = Arc::new(Mutex::new(Vec::new()));
    let seen = Arc::new(Mutex::new(Vec::new()));
    let mut mock = MockCommandExecutor::new();
    let log = Arc::clone(&checked);
    mock.expect_is_container_running().returning(move |name| {
        log.lock().unwrap().push(name.to_string());
        true
    });
    let log = Arc::clone(&seen);
    mock.expect_execute().times(1).returning(move |cmd, _, _| {
        log.lock().unwrap().push(cmd.to_string());
        CommandOutput {
            success: true,
            stdout: String::new(),
            stderr: String::new(),
        }
    });
    let checks = vec![make_exec_check("a", "true", Some("custom-c"))];

    docker_warnings(&target, &checks, &files(&["Cargo.toml"]), root(), &mock).await;

    assert_eq!(*checked.lock().unwrap(), vec!["custom-c".to_string()]);
    let cmd = seen.lock().unwrap()[0].clone();
    assert!(cmd.starts_with("docker exec custom-c "), "{cmd}");
}

#[tokio::test]
async fn checks_sharing_a_container_probe_it_once() {
    let target = ConfigBuilder::new().build().runner;
    let mock = running_mock(true, Default::default()); // execute: times(1)
    let checks = vec![
        make_exec_check("a", "true", None),
        make_exec_check("b", "true", None),
    ];

    let warnings = docker_warnings(&target, &checks, &files(&["Cargo.toml"]), root(), &mock).await;

    assert!(warnings.is_empty(), "{warnings:?}");
}

#[tokio::test]
async fn stopped_containers_report_run_fallback_once() {
    let target = ConfigBuilder::new().build().runner;
    let mock = stopped_mock();
    let checks = vec![
        make_exec_check("a", "true", None),
        make_exec_check("b", "true", Some("other-c")),
    ];

    let warnings = docker_warnings(&target, &checks, &files(&["Cargo.toml"]), root(), &mock).await;

    assert_eq!(warnings.len(), 1, "{warnings:?}");
}

#[tokio::test]
async fn run_fallback_mount_target_mismatch_warns() {
    let target = ConfigBuilder::new()
        .with_volume_mount(".:/build")
        .build()
        .runner;
    let mock = stopped_mock();

    let warnings = warnings_for(&target, &mock).await;

    assert_eq!(warnings.len(), 1, "{warnings:?}");
    assert!(warnings[0].contains("/build"), "{warnings:?}");
    assert!(warnings[0].contains("/app"), "{warnings:?}");
}

#[tokio::test]
async fn run_fallback_mount_target_trailing_slash_gives_no_warning() {
    let target = ConfigBuilder::new()
        .with_volume_mount(".:/app/")
        .build()
        .runner;
    let mock = stopped_mock();

    let warnings = warnings_for(&target, &mock).await;

    assert!(warnings.is_empty(), "{warnings:?}");
}

#[tokio::test]
async fn run_fallback_mount_target_with_options_matching_gives_no_warning() {
    let target = ConfigBuilder::new()
        .with_volume_mount(".:/app:ro")
        .build()
        .runner;
    let mock = stopped_mock();

    let warnings = warnings_for(&target, &mock).await;

    assert!(warnings.is_empty(), "{warnings:?}");
}

#[tokio::test]
async fn exec_probe_failure_includes_first_stderr_line() {
    let target = ConfigBuilder::new().build().runner;
    let mut mock = MockCommandExecutor::new();
    mock.expect_is_container_running().returning(|_| true);
    mock.expect_execute().returning(|_, _, _| CommandOutput {
        success: false,
        stdout: String::new(),
        stderr: "  OCI runtime exec failed: bash not found  \nsecond line\n".to_string(),
    });

    let warnings = warnings_for(&target, &mock).await;

    assert_eq!(warnings.len(), 1, "{warnings:?}");
    assert!(
        warnings[0].contains("OCI runtime exec failed: bash not found"),
        "{warnings:?}"
    );
    assert!(!warnings[0].contains("second line"), "{warnings:?}");
}
