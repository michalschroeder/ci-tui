//! Tests for the docker startup probe (`preflight::docker_warning`): warn when
//! changed files won't resolve inside the container (repo not at the exec
//! container's WORKDIR, or `docker run` fallback without a repo mount).

use ci_tui::preflight::docker_warning;
use ci_tui::runner::ExecTarget;
use std::path::Path;
use std::sync::{Arc, Mutex};

mod common;
use common::configs::ConfigBuilder;
use common::{local_sh, CommandOutput, MockCommandExecutor};

/// Crate root: `Cargo.toml` exists there on the host
fn root() -> &'static Path {
    Path::new(env!("CARGO_MANIFEST_DIR"))
}

fn files(paths: &[&str]) -> Vec<String> {
    paths.iter().map(|p| p.to_string()).collect()
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

    let warning = docker_warning(&target, &files(&["Cargo.toml"]), root(), &mock).await;

    assert_eq!(warning, None);
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

    let warning = docker_warning(&target, &files(&["Cargo.toml"]), root(), &mock)
        .await
        .expect("missing file must warn");

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

    docker_warning(
        &target,
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

    let warning = docker_warning(&target, &files(&["gone/deleted_file.rs"]), root(), &mock).await;

    assert_eq!(warning, None);
}

#[tokio::test]
async fn run_fallback_without_volume_mount_warns() {
    let target = ConfigBuilder::new().build().runner;
    let mock = stopped_mock();

    let warning = docker_warning(&target, &files(&["Cargo.toml"]), root(), &mock)
        .await
        .expect("docker run without mount must warn");

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

    let warning = docker_warning(&target, &files(&["Cargo.toml"]), root(), &mock).await;

    assert_eq!(warning, None);
}

#[tokio::test]
async fn local_mode_never_probes() {
    let mut mock = MockCommandExecutor::new();
    mock.expect_is_container_running().never();
    mock.expect_execute().never();

    let warning = docker_warning(&local_sh(), &files(&["Cargo.toml"]), root(), &mock).await;

    assert_eq!(warning, None);
}

#[tokio::test]
async fn no_changed_files_never_probes() {
    let target = ConfigBuilder::new().build().runner;
    let mut mock = MockCommandExecutor::new();
    mock.expect_is_container_running().never();
    mock.expect_execute().never();

    let warning = docker_warning(&target, &[], root(), &mock).await;

    assert_eq!(warning, None);
}
