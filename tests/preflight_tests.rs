//! Tests for the docker startup probe (`preflight::docker_warnings`): warn when
//! changed files (relative to the project dir) won't resolve inside the
//! containers the selected checks use (project dir not at a running service
//! container's WORKDIR, or a plain-image `docker run` without a matching
//! project dir mount); and the fatal setup checks (`preflight::run_with`):
//! Docker unreachable, unset / empty `docker.volume_mount` variables, no
//! usable compose file, unknown compose services → exit 3.

use ci_tui::checks::{CheckFiles, CheckToRun};
use ci_tui::config::CiConfig;
use ci_tui::exit;
use ci_tui::preflight::{docker_reachable, docker_warnings, run_with};
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

/// Plain-image config of `img` with `volume_mount` (if any)
fn image_config(mount: Option<&str>) -> CiConfig {
    let builder = ConfigBuilder::new().with_image("img");
    match mount {
        Some(mount) => builder.with_volume_mount(mount),
        None => builder,
    }
    .build()
}

/// Plain-image mode target with `volume_mount` (if any)
fn image(mount: Option<&str>) -> ExecTarget {
    image_config(mount).runner
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
    assert!(
        warning.contains("mount the project dir at the container's WORKDIR"),
        "{warning}"
    );
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
    assert!(warning.contains("has no project dir mount"), "{warning}");
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
    assert!(
        warnings[0].contains("mounts the project dir at `/build`"),
        "{warnings:?}"
    );
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

/// Executor whose probes never answer (daemon / compose hanging), except
/// `docker version` when `answers_version`
struct HangingExecutor {
    answers_version: bool,
}

#[async_trait::async_trait]
impl CommandExecutor for HangingExecutor {
    async fn execute(&self, cmd: &str, _: &Path, _: &OutputSink) -> CommandOutput {
        if !(self.answers_version && cmd.starts_with("docker version")) {
            std::future::pending::<()>().await;
        }
        ok_output("")
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
        &HangingExecutor {
            answers_version: false,
        },
        Duration::from_millis(10),
    )
    .await
    .unwrap_err();

    assert_eq!(exit::code_for(&err), exit::ENV_ERROR);
    assert!(format!("{err:#}").contains("did not answer"), "{err:#}");
}

// ---------------------------------------------------------------------------
// preflight::run_with: docker setup errors (exit 3 with the cause)
// ---------------------------------------------------------------------------

/// Commands the mock ran, in order
type Seen = Arc<Mutex<Vec<String>>>;

/// Mock: `docker version` answers; the compose services probe answers
/// `services` (stdout) or fails with `Err(stderr)`. A profiled `worker`
/// service is listed only when the probe enables every profile.
fn setup_mock(services: Result<&str, &str>, seen: Seen) -> MockCommandExecutor {
    let (success, stdout, stderr) = match services {
        Ok(out) => (true, out.to_string(), String::new()),
        Err(err) => (false, String::new(), err.to_string()),
    };
    let mut mock = MockCommandExecutor::new();
    mock.expect_running_container().returning(|_, _, _| None);
    mock.expect_execute().returning(move |cmd, at, _| {
        assert_eq!(at, root(), "probe must run in the exec root");
        seen.lock().unwrap().push(cmd.to_string());
        if cmd.starts_with("docker version") {
            return ok_output("");
        }
        let profiled = if cmd.contains("--profile '*'") {
            "worker\n"
        } else {
            ""
        };
        CommandOutput {
            success,
            stdout: format!("{stdout}{profiled}"),
            stderr: stderr.clone(),
        }
    });
    mock
}

/// Successful command output with `stdout`
fn ok_output(stdout: &str) -> CommandOutput {
    CommandOutput {
        success: true,
        stdout: stdout.to_string(),
        stderr: String::new(),
    }
}

/// [`run_with`] needed, no changed files (no path warnings), [`TIMEOUT`]
async fn setup(
    config: &CiConfig,
    checks: &[CheckToRun],
    mock: &dyn CommandExecutor,
) -> anyhow::Result<Vec<String>> {
    run_with(config, true, root(), checks, &[], mock, TIMEOUT).await
}

/// Error of a setup expected to fail: exit 3, message text
fn setup_error(result: anyhow::Result<Vec<String>>) -> String {
    let err = result.expect_err("setup error expected");
    assert_eq!(exit::code_for(&err), exit::ENV_ERROR);
    format!("{err:#}")
}

/// Compose services probes among `seen`
fn compose_probes(seen: &Seen) -> Vec<String> {
    let seen = seen.lock().unwrap();
    seen.iter()
        .filter(|c| c.contains("compose"))
        .cloned()
        .collect()
}

#[tokio::test]
async fn compose_probe_lists_services_with_every_profile() {
    let seen = Seen::default();
    let mock = setup_mock(Ok("app\n"), Arc::clone(&seen));
    let config = ConfigBuilder::new().with_docker("my dir", "app").build();

    setup(&config, &default_check(), &mock).await.unwrap();

    let probes = compose_probes(&seen);
    assert_eq!(probes.len(), 1, "{probes:?}");
    let cmd = &probes[0];
    assert!(
        cmd.starts_with("docker compose --project-directory 'my dir' "),
        "{cmd}"
    );
    assert!(cmd.contains("--profile '*'"), "{cmd}");
    assert!(cmd.ends_with("config --services"), "{cmd}");
}

#[rstest::rstest]
#[case::no_compose_file("\n  no configuration file provided: not found  \nmore\n")]
#[case::invalid_compose_file("yaml: line 1: did not find expected ',' or ']'\nmore\n")]
#[tokio::test]
async fn unusable_compose_project_is_env_error(#[case] stderr: &str) {
    let mock = setup_mock(Err(stderr), Seen::default());
    let config = ConfigBuilder::new().build();

    let msg = setup_error(setup(&config, &default_check(), &mock).await);

    assert!(msg.contains("docker.project_dir `./test`"), "{msg}");
    let cause = stderr.trim_start().lines().next().unwrap().trim();
    assert!(msg.contains(cause), "{msg}");
    assert!(!msg.contains("more"), "{msg}");
}

#[tokio::test]
async fn failed_compose_probe_does_not_blame_the_compose_file() {
    // compose missing / daemon refusing: not the compose file's fault
    let mock = setup_mock(
        Err("docker: 'compose' is not a docker command.\n"),
        Seen::default(),
    );
    let config = ConfigBuilder::new().build();

    let msg = setup_error(setup(&config, &default_check(), &mock).await);

    assert!(
        msg.contains(
            "docker.project_dir `./test`: `docker compose config` failed: \
             docker: 'compose' is not a docker command."
        ),
        "{msg}"
    );
    assert!(!msg.contains("no usable compose file"), "{msg}");
}

#[tokio::test]
async fn unknown_check_service_is_env_error() {
    let mock = setup_mock(Ok("app\ndb\n"), Seen::default());
    let config = ConfigBuilder::new().build();
    let checks = vec![
        make_exec_check("a", "true", Some("db")),
        make_exec_check("lint", "true", Some("nope")),
    ];

    let msg = setup_error(setup(&config, &checks, &mock).await);

    assert!(msg.contains("`nope`"), "{msg}");
    assert!(msg.contains("check `lint`"), "{msg}");
    assert!(msg.contains("`./test`"), "{msg}");
    assert!(
        !msg.contains("check `a`"),
        "only the missing service: {msg}"
    );
}

#[tokio::test]
async fn unknown_default_service_names_docker_service() {
    let mock = setup_mock(Ok("db\n"), Seen::default());
    let config = ConfigBuilder::new().build();

    let msg = setup_error(setup(&config, &default_check(), &mock).await);

    assert!(msg.contains("`app`"), "{msg}");
    assert!(msg.contains("`docker.service`"), "{msg}");
}

#[tokio::test]
async fn unknown_on_demand_check_service_is_env_error() {
    // `t` can run it later
    let mock = setup_mock(Ok("app\n"), Seen::default());
    let config = ConfigBuilder::new().build();
    let mut check = make_exec_check("slow", "true", Some("nope"));
    check.files = CheckFiles::OnDemand;

    let msg = setup_error(setup(&config, &[check], &mock).await);

    assert!(msg.contains("`nope`"), "{msg}");
}

/// Config with pre-command `init` (compose `service`, `host`) in `group`
fn pre_command_config(group: &str, service: &str, host: bool) -> CiConfig {
    let builder = ConfigBuilder::new();
    match host {
        true => builder.with_host_pre_command(group, "init", "true"),
        false => builder.with_pre_command(group, "init", "true"),
    }
    .with_pre_command_service(group, service)
    .build()
}

#[tokio::test]
async fn unknown_pre_command_service_is_env_error() {
    let mock = setup_mock(Ok("app\n"), Seen::default());
    let config = pre_command_config("g", "db", false);

    let msg = setup_error(setup(&config, &default_check(), &mock).await);

    assert!(msg.contains("`db`"), "{msg}");
    assert!(msg.contains("pre-command `init`"), "{msg}");
}

#[rstest::rstest]
#[case::host_pre_command("g", true)]
#[case::other_group("other", false)]
#[tokio::test]
async fn unused_pre_command_service_is_not_checked(#[case] group: &str, #[case] host: bool) {
    let seen = Seen::default();
    let mock = setup_mock(Ok("app\n"), Arc::clone(&seen));
    let config = pre_command_config(group, "db", host);

    setup(&config, &default_check(), &mock).await.unwrap();

    assert_eq!(compose_probes(&seen).len(), 1);
}

#[tokio::test]
async fn profiled_service_counts_as_present() {
    let seen = Seen::default();
    let mock = setup_mock(Ok("app\n"), Arc::clone(&seen));
    let checks = vec![make_exec_check("w", "true", Some("worker"))];

    setup(&ConfigBuilder::new().build(), &checks, &mock)
        .await
        .unwrap();

    assert_eq!(compose_probes(&seen).len(), 1);
}

#[tokio::test]
async fn working_compose_setup_probes_once_then_warns_as_before() {
    let seen = Seen::default();
    let mock = setup_mock(Ok("app\ndb\n"), Arc::clone(&seen));
    let checks = vec![
        make_exec_check("a", "true", None),
        make_exec_check("b", "true", Some("db")),
        make_exec_check("c", "true", Some("db")),
    ];
    let files = files(&["Cargo.toml"]);
    let config = ConfigBuilder::new().build();

    let warnings = run_with(&config, true, root(), &checks, &files, &mock, TIMEOUT)
        .await
        .unwrap();

    // stopped services: `docker compose run`, nothing to warn about
    assert!(warnings.is_empty(), "{warnings:?}");
    let seen = seen.lock().unwrap();
    assert_eq!(seen.len(), 2, "{seen:?}");
    assert!(seen[0].starts_with("docker version"), "{seen:?}");
}

#[tokio::test]
async fn unreachable_docker_is_reported_before_the_compose_probe() {
    let seen = Seen::default();
    let log = Arc::clone(&seen);
    let mut mock = MockCommandExecutor::new();
    mock.expect_execute().returning(move |cmd, _, _| {
        log.lock().unwrap().push(cmd.to_string());
        CommandOutput {
            success: false,
            stdout: String::new(),
            stderr: "Cannot connect to the Docker daemon".to_string(),
        }
    });

    let result = setup(&ConfigBuilder::new().build(), &default_check(), &mock).await;

    let msg = setup_error(result);
    assert!(msg.contains("Docker is not reachable"), "{msg}");
    assert_eq!(seen.lock().unwrap().len(), 1, "no compose probe");
}

#[tokio::test]
async fn compose_probe_timeout_is_env_error() {
    let config = ConfigBuilder::new().build();
    let executor = HangingExecutor {
        answers_version: true,
    };
    let timeout = Duration::from_millis(10);

    let result = run_with(
        &config,
        true,
        root(),
        &default_check(),
        &[],
        &executor,
        timeout,
    )
    .await;

    let msg = setup_error(result);
    assert!(msg.contains("config --services"), "{msg}");
    assert!(msg.contains("did not answer"), "{msg}");
    assert!(msg.contains("`./test`"), "{msg}");
}

/// Mock answering every command, recording it into `seen`
fn recording_mock(seen: &Seen) -> MockCommandExecutor {
    let log = Arc::clone(seen);
    let mut mock = MockCommandExecutor::new();
    mock.expect_running_container().never();
    mock.expect_execute().returning(move |cmd, _, _| {
        log.lock().unwrap().push(cmd.to_string());
        ok_output("")
    });
    mock
}

#[tokio::test]
async fn unset_volume_mount_var_fails_before_any_docker_command() {
    std::env::remove_var("CI_TUI_TEST_PF_UNSET");
    let seen = Seen::default();
    let config = image_config(Some("${CI_TUI_TEST_PF_UNSET}:/app"));

    let result = setup(&config, &default_check(), &recording_mock(&seen)).await;

    let msg = setup_error(result);
    assert!(
        msg.contains(
            "docker.volume_mount uses `${CI_TUI_TEST_PF_UNSET}`, which is not set \
             (try `CI_TUI_TEST_PF_UNSET=$PWD ci-tui`)"
        ),
        "{msg}"
    );
    let seen = seen.lock().unwrap();
    assert!(seen.is_empty(), "{seen:?}");
}

#[tokio::test]
async fn empty_volume_mount_var_is_env_error() {
    std::env::set_var("CI_TUI_TEST_PF_EMPTY", "");
    let config = image_config(Some("${CI_TUI_TEST_PF_EMPTY}:/app"));
    let mock = recording_mock(&Seen::default());

    let msg = setup_error(setup(&config, &default_check(), &mock).await);

    assert!(
        msg.contains("docker.volume_mount uses `${CI_TUI_TEST_PF_EMPTY}`, which is empty"),
        "{msg}"
    );
    assert!(msg.contains("`CI_TUI_TEST_PF_EMPTY=$PWD ci-tui`"), "{msg}");
}

#[tokio::test]
async fn every_blank_volume_mount_var_is_named() {
    std::env::remove_var("CI_TUI_TEST_PF_MULTI_UNSET");
    std::env::set_var("CI_TUI_TEST_PF_MULTI_EMPTY", "");
    std::env::set_var("CI_TUI_TEST_PF_MULTI_SET", "/src");
    let config = image_config(Some(
        "${CI_TUI_TEST_PF_MULTI_UNSET}:${CI_TUI_TEST_PF_MULTI_EMPTY}:ro${CI_TUI_TEST_PF_MULTI_SET}",
    ));
    let mock = recording_mock(&Seen::default());

    let msg = setup_error(setup(&config, &default_check(), &mock).await);

    let unset = "`${CI_TUI_TEST_PF_MULTI_UNSET}`, which is not set";
    let empty = "`${CI_TUI_TEST_PF_MULTI_EMPTY}`, which is empty";
    assert!(msg.contains(unset) && msg.contains(empty), "{msg}");
    // `$PWD` only for the whole source section
    assert!(
        msg.contains("try `CI_TUI_TEST_PF_MULTI_UNSET=$PWD ci-tui`"),
        "{msg}"
    );
    assert!(
        msg.contains("set `CI_TUI_TEST_PF_MULTI_EMPTY` before running ci-tui"),
        "{msg}"
    );
    assert!(!msg.contains("MULTI_EMPTY=$PWD"), "{msg}");
    assert!(!msg.contains("MULTI_SET"), "{msg}");
}

/// Blank vars that are not the whole mount source: named, no `$PWD` guess
#[rstest::rstest]
#[case::destination(
    "/src:${CI_TUI_TEST_PF_DST}",
    "docker.volume_mount uses `${CI_TUI_TEST_PF_DST}`, which is not set \
     (set `CI_TUI_TEST_PF_DST` before running ci-tui)"
)]
#[case::destination_and_options(
    "/src:${CI_TUI_TEST_PF_DST}:${CI_TUI_TEST_PF_OPTS}",
    "(set `CI_TUI_TEST_PF_DST`, `CI_TUI_TEST_PF_OPTS` before running ci-tui)"
)]
#[tokio::test]
async fn blank_non_source_vars_are_named_without_pwd_hint(
    #[case] mount: &str,
    #[case] expected: &str,
) {
    std::env::remove_var("CI_TUI_TEST_PF_DST");
    std::env::remove_var("CI_TUI_TEST_PF_OPTS");
    let mock = recording_mock(&Seen::default());

    let msg = setup_error(setup(&image_config(Some(mount)), &default_check(), &mock).await);

    assert!(msg.contains(expected), "{msg}");
    assert!(!msg.contains("$PWD"), "{msg}");
}

/// A blank var next to text leaves its mount section non-empty: no error
/// (section rules: config.rs `test_blank_mount_vars_only_for_emptied_sections`)
#[tokio::test]
async fn blank_var_in_non_empty_section_is_fine() {
    std::env::set_var("CI_TUI_TEST_PF_OK_SET", "/src");
    std::env::remove_var("CI_TUI_TEST_PF_OK_OPTS");
    let mount = "${CI_TUI_TEST_PF_OK_SET}:/app${CI_TUI_TEST_PF_OK_OPTS}";
    let seen = Seen::default();

    let result = setup(
        &image_config(Some(mount)),
        &default_check(),
        &recording_mock(&seen),
    )
    .await;

    assert!(result.is_ok(), "{:#}", result.unwrap_err());
    assert_eq!(*seen.lock().unwrap(), ["docker version"]);
}

#[tokio::test]
async fn image_mode_with_set_vars_never_runs_the_compose_probe() {
    // guard: plain image probes only `docker version`
    std::env::set_var("CI_TUI_TEST_PF_SET", "/src");
    let seen = Seen::default();
    let config = image_config(Some("${CI_TUI_TEST_PF_SET}:/app"));

    setup(&config, &default_check(), &recording_mock(&seen))
        .await
        .unwrap();

    assert_eq!(*seen.lock().unwrap(), ["docker version"]);
}

#[tokio::test]
async fn setup_not_checked_in_local_mode_or_when_not_needed() {
    // guard: no docker command would run, so a bad setup cannot hurt
    std::env::remove_var("CI_TUI_TEST_PF_UNNEEDED");
    let mut mock = MockCommandExecutor::new();
    mock.expect_running_container().never();
    mock.expect_execute().never();
    let image = image_config(Some("${CI_TUI_TEST_PF_UNNEEDED}:/app"));
    let compose = ConfigBuilder::new().build();
    let mut local = ConfigBuilder::new().build();
    local.runner = local_sh();

    for (config, needed) in [(&image, false), (&compose, false), (&local, true)] {
        run_with(
            config,
            needed,
            root(),
            &default_check(),
            &[],
            &mock,
            TIMEOUT,
        )
        .await
        .unwrap();
    }
}
