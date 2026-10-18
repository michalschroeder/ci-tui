//! Tests for the docker startup probe (`preflight::docker_warnings`): warn when
//! changed files won't resolve inside the containers the selected checks use
//! (repo not at a running service container's WORKDIR, or a plain-image
//! `docker run` without a matching repo mount).

use ci_tui::checks::CheckToRun;
use ci_tui::exit;
use ci_tui::preflight::{docker_reachable, docker_warnings};
use ci_tui::runner::{CommandExecutor, ExecTarget, OutputSink};
use std::path::Path;
use std::sync::{Arc, Mutex};
use std::time::Duration;

mod common;
use common::configs::ConfigBuilder;
use common::{local_sh, make_exec_check, CommandOutput, MockCommandExecutor, RUNNING_ID};

/// Crate root: `Cargo.toml` exists there on the host
fn root() -> &'static Path {
    Path::new(env!("CARGO_MANIFEST_DIR"))
}

fn files(paths: &[&str]) -> Vec<String> {
    paths.iter().map(|p| p.to_string()).collect()
}

/// Compose mode target (`project_dir: ./test`, default service `app`)
fn compose() -> ExecTarget {
    ConfigBuilder::new().build().runner
}

/// Plain-image mode target with `volume_mount` (if any)
fn image(mount: Option<&str>) -> ExecTarget {
    let builder = ConfigBuilder::new().with_image("img");
    match mount {
        Some(mount) => builder.with_volume_mount(mount),
        None => builder,
    }
    .build()
    .runner
}

/// One selected check on the default service
fn default_check() -> Vec<CheckToRun> {
    vec![make_exec_check("a", "true", None)]
}

/// Probe with one default-service check and `Cargo.toml` changed
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

/// Mock: every service runs as [`RUNNING_ID`] (queried in `./test` from the
/// crate root); one `execute` (the path probe) recorded, answering `found`
fn running_mock(found: bool, seen: Arc<Mutex<Vec<String>>>) -> MockCommandExecutor {
    let mut mock = MockCommandExecutor::new();
    mock.expect_running_container()
        .withf(|dir, _, at| dir == "./test" && at == root())
        .returning(|_, _, _| Some(RUNNING_ID.into()));
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

/// Mock: no service container runs; no probe may run
fn stopped_mock() -> MockCommandExecutor {
    let mut mock = MockCommandExecutor::new();
    mock.expect_running_container().returning(|_, _, _| None);
    mock.expect_execute().never();
    mock
}

/// Mock for plain-image mode: compose never asked, no probe runs
fn image_mock() -> MockCommandExecutor {
    let mut mock = MockCommandExecutor::new();
    mock.expect_running_container().never();
    mock.expect_execute().never();
    mock
}

#[tokio::test]
async fn exec_probe_found_gives_no_warning() {
    let seen = Default::default();
    let mock = running_mock(true, Arc::clone(&seen));

    let warnings = warnings_for(&compose(), &mock).await;

    assert!(warnings.is_empty(), "{warnings:?}");
    let cmd = seen.lock().unwrap()[0].clone();
    assert!(
        cmd.starts_with(&format!("docker exec {RUNNING_ID} ")),
        "{cmd}"
    );
    assert!(cmd.contains("test -e"), "{cmd}");
    assert!(cmd.contains("Cargo.toml"), "{cmd}");
    assert!(
        !cmd.contains("/Cargo.toml"),
        "path must stay repo-relative: {cmd}"
    );
    assert!(!cmd.contains(" -w "), "exec keeps container WORKDIR: {cmd}");
}

#[tokio::test]
async fn exec_probe_missing_warns_with_file_and_service() {
    let mock = running_mock(false, Default::default());

    let warnings = warnings_for(&compose(), &mock).await;
    assert_eq!(
        warnings.len(),
        1,
        "missing file must warn once: {warnings:?}"
    );
    let warning = &warnings[0];

    assert!(warning.contains("Cargo.toml"), "{warning}");
    assert!(warning.contains("service `app`"), "{warning}");
    assert!(warning.contains(RUNNING_ID), "{warning}");
    assert!(warning.contains("working directory"), "{warning}");
    // docker.work_dir does not apply to `docker exec`: must not be suggested
    assert!(!warning.contains("docker.work_dir"), "{warning}");
}

#[tokio::test]
async fn exec_probe_skips_files_deleted_on_host() {
    let seen = Default::default();
    let mock = running_mock(true, Arc::clone(&seen));

    docker_warnings(
        &compose(),
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
    let mut mock = MockCommandExecutor::new();
    mock.expect_running_container()
        .returning(|_, _, _| Some(RUNNING_ID.into()));
    mock.expect_execute().never();

    let warnings = docker_warnings(
        &compose(),
        &default_check(),
        &files(&["gone/deleted_file.rs"]),
        root(),
        &mock,
    )
    .await;

    assert!(warnings.is_empty(), "{warnings:?}");
}

#[tokio::test]
async fn stopped_compose_service_gives_no_warning() {
    // `docker compose run` takes volumes / working_dir from the compose file
    let warnings = warnings_for(&compose(), &stopped_mock()).await;

    assert!(warnings.is_empty(), "{warnings:?}");
}

#[tokio::test]
async fn custom_named_service_container_is_probed_by_resolved_id() {
    // `container_name: legacy_db` / Compose v1 names: `compose ps` finds them
    let checked = Arc::new(Mutex::new(Vec::new()));
    let seen: Arc<Mutex<Vec<String>>> = Arc::default();
    let log = Arc::clone(&checked);
    let mut mock = MockCommandExecutor::new();
    mock.expect_running_container()
        .returning(move |_, service, _| {
            log.lock().unwrap().push(service.to_string());
            (service == "db").then(|| "legacy_db".to_string())
        });
    let probes = Arc::clone(&seen);
    mock.expect_execute().times(1).returning(move |cmd, _, _| {
        probes.lock().unwrap().push(cmd.to_string());
        CommandOutput {
            success: false,
            stdout: String::new(),
            stderr: String::new(),
        }
    });
    let checks = vec![
        make_exec_check("a", "true", Some("db")),
        make_exec_check("b", "true", Some("worker")),
    ];

    let warnings =
        docker_warnings(&compose(), &checks, &files(&["Cargo.toml"]), root(), &mock).await;

    assert_eq!(*checked.lock().unwrap(), ["db", "worker"]);
    let cmd = seen.lock().unwrap()[0].clone();
    assert!(cmd.starts_with("docker exec legacy_db "), "{cmd}");
    // stopped `worker`: compose run, no warning
    assert_eq!(warnings.len(), 1, "{warnings:?}");
    assert!(warnings[0].contains("service `db`"), "{warnings:?}");
}

#[tokio::test]
async fn checks_sharing_a_service_resolve_and_probe_it_once() {
    let mut mock = MockCommandExecutor::new();
    mock.expect_running_container()
        .times(1)
        .returning(|_, _, _| Some(RUNNING_ID.into()));
    mock.expect_execute()
        .times(1)
        .returning(|_, _, _| CommandOutput {
            success: true,
            stdout: String::new(),
            stderr: String::new(),
        });
    let checks = vec![
        make_exec_check("a", "true", None),
        make_exec_check("b", "true", None),
    ];

    let warnings =
        docker_warnings(&compose(), &checks, &files(&["Cargo.toml"]), root(), &mock).await;

    assert!(warnings.is_empty(), "{warnings:?}");
}

#[tokio::test]
async fn image_without_volume_mount_warns() {
    let warnings = warnings_for(&image(None), &image_mock()).await;
    assert_eq!(warnings.len(), 1, "{warnings:?}");
    let warning = &warnings[0];

    assert!(warning.contains("docker.volume_mount"), "{warning}");
    assert!(warning.contains("docker.work_dir"), "{warning}");
    assert!(warning.contains("docker run"), "{warning}");
    assert!(warning.contains("`img`"), "{warning}");
}

#[tokio::test]
async fn image_with_matching_volume_mount_gives_no_warning() {
    for mount in [".:/app", ".:/app/", ".:/app:ro"] {
        let warnings = warnings_for(&image(Some(mount)), &image_mock()).await;

        assert!(warnings.is_empty(), "{mount}: {warnings:?}");
    }
}

#[tokio::test]
async fn image_mount_target_mismatch_warns() {
    let warnings = warnings_for(&image(Some(".:/build")), &image_mock()).await;

    assert_eq!(warnings.len(), 1, "{warnings:?}");
    assert!(warnings[0].contains("/build"), "{warnings:?}");
    assert!(warnings[0].contains("/app"), "{warnings:?}");
}

#[tokio::test]
async fn image_mode_warns_once_for_many_checks() {
    let checks = vec![
        make_exec_check("a", "true", None),
        make_exec_check("b", "true", Some("other")),
    ];

    let warnings = docker_warnings(
        &image(None),
        &checks,
        &files(&["Cargo.toml"]),
        root(),
        &image_mock(),
    )
    .await;

    assert_eq!(warnings.len(), 1, "{warnings:?}");
}

#[tokio::test]
async fn local_mode_never_probes() {
    let mut mock = MockCommandExecutor::new();
    mock.expect_running_container().never();
    mock.expect_execute().never();

    let warnings = warnings_for(&local_sh(), &mock).await;

    assert!(warnings.is_empty(), "{warnings:?}");
}

#[tokio::test]
async fn no_changed_files_never_probes() {
    let mut mock = MockCommandExecutor::new();
    mock.expect_running_container().never();
    mock.expect_execute().never();

    let warnings = docker_warnings(&compose(), &default_check(), &[], root(), &mock).await;

    assert!(warnings.is_empty(), "{warnings:?}");
}

#[tokio::test]
async fn no_selected_checks_never_probes() {
    let mut mock = MockCommandExecutor::new();
    mock.expect_running_container().never();
    mock.expect_execute().never();

    let warnings = docker_warnings(&compose(), &[], &files(&["Cargo.toml"]), root(), &mock).await;

    assert!(warnings.is_empty(), "{warnings:?}");
}

#[tokio::test]
async fn exec_probe_failure_includes_first_stderr_line() {
    let mut mock = MockCommandExecutor::new();
    mock.expect_running_container()
        .returning(|_, _, _| Some(RUNNING_ID.into()));
    mock.expect_execute().returning(|_, _, _| CommandOutput {
        success: false,
        stdout: String::new(),
        stderr: "  OCI runtime exec failed: bash not found  \nsecond line\n".to_string(),
    });

    let warnings = warnings_for(&compose(), &mock).await;

    assert_eq!(warnings.len(), 1, "{warnings:?}");
    assert!(
        warnings[0].contains("OCI runtime exec failed: bash not found"),
        "{warnings:?}"
    );
    assert!(!warnings[0].contains("second line"), "{warnings:?}");
}

// ---------------------------------------------------------------------------
// preflight::docker_reachable: exit 3 when docker is down
// ---------------------------------------------------------------------------

const TIMEOUT: Duration = Duration::from_secs(5);

/// Mock answering the `docker version` probe with `success` / `stderr`
fn probe_mock(success: bool, stderr: &str) -> MockCommandExecutor {
    let stderr = stderr.to_string();
    let mut mock = MockCommandExecutor::new();
    mock.expect_execute()
        .withf(|cmd, _, _| cmd.starts_with("docker version"))
        .times(1)
        .returning(move |_, _, _| CommandOutput {
            success,
            stdout: String::new(),
            stderr: stderr.clone(),
        });
    mock
}

#[tokio::test]
async fn docker_probe_up_is_ok() {
    let target = ConfigBuilder::new().build().runner;
    docker_reachable(&target, true, root(), &probe_mock(true, ""), TIMEOUT)
        .await
        .unwrap();
}

#[tokio::test]
async fn docker_probe_failure_is_env_error() {
    let target = ConfigBuilder::new().build().runner;
    let mock = probe_mock(
        false,
        "Cannot connect to the Docker daemon at unix:///var/run/docker.sock\nmore\n",
    );

    let err = docker_reachable(&target, true, root(), &mock, TIMEOUT)
        .await
        .unwrap_err();

    assert_eq!(exit::code_for(&err), exit::ENV_ERROR);
    let msg = format!("{err:#}");
    assert!(msg.contains("Docker is not reachable"), "{msg}");
    assert!(msg.contains("Cannot connect to the Docker daemon"), "{msg}");
    assert!(!msg.contains("more"), "{msg}");
}

#[tokio::test]
async fn docker_probe_skipped_in_local_mode_or_when_not_needed() {
    let docker = ConfigBuilder::new().build().runner;
    let mut mock = MockCommandExecutor::new();
    mock.expect_execute().never();
    docker_reachable(&local_sh(), true, root(), &mock, TIMEOUT)
        .await
        .unwrap();
    docker_reachable(&docker, false, root(), &mock, TIMEOUT)
        .await
        .unwrap();
}

/// Executor whose probe never answers (daemon hanging)
struct HangingExecutor;

#[async_trait::async_trait]
impl CommandExecutor for HangingExecutor {
    async fn execute(&self, _: &str, _: &Path, _: &OutputSink) -> CommandOutput {
        std::future::pending().await
    }

    fn running_container(&self, _: &str, _: &str, _: &Path) -> Option<String> {
        None
    }

    fn kill_container(&self, _: &str) {}
}

#[tokio::test]
async fn docker_probe_timeout_is_env_error() {
    let target = ConfigBuilder::new().build().runner;
    let err = docker_reachable(
        &target,
        true,
        root(),
        &HangingExecutor,
        Duration::from_millis(10),
    )
    .await
    .unwrap_err();

    assert_eq!(exit::code_for(&err), exit::ENV_ERROR);
    assert!(format!("{err:#}").contains("did not answer"), "{err:#}");
}
