//! Group `stop_on_failure` (#245): a check of such a group that ran and
//! failed as an error stops the run; later groups are reported not run.
//! Full runs through `CheckRunner::run_checks` and `simple::run_with_executor`
//! (real local-mode commands logging their names), `--simple` and `--fix`
//! through the binary. TUI state is unit-tested in `ui`.

use ci_tui::config::{CiConfig, OnFailure};
use ci_tui::exit;
use ci_tui::runner::{CancelRegistry, CheckRunner, CheckStatus, RealCommandExecutor, RunnerEvent};
use std::sync::Arc;
use std::time::Duration;

mod common;
use common::configs::{
    logged, on_failure_groups, stop_on_failure_groups, CheckBuilder, G1_ABORTED, G1_FAILED,
    G1_STOPPED, YAML_HEAD,
};
use common::{
    assert_code, collect_events, finished, log_lines, outcomes, run_config, run_simple,
    select_for_a_txt, skipped_reason, stdout,
};

/// [`stop_on_failure_groups`] with every command logging its name
fn two_groups(c1_cmd: &str) -> CiConfig {
    stop_on_failure_groups(c1_cmd, true)
}

/// `config` plus check `c3` in `g1` after `c1` (passing, slower: with
/// `parallel` still running when `c1` fails)
fn with_sibling(mut config: CiConfig, parallel: bool) -> CiConfig {
    let g1 = &mut config.checks["g1"];
    let c3 = CheckBuilder::new("c3", &logged("c3", "sleep 0.2", true)).build();
    g1.checks.insert("c3".into(), c3);
    g1.parallel = parallel;
    config
}

/// `log` sorted (parallel checks log in any order)
fn sorted(mut log: Vec<String>) -> Vec<String> {
    log.sort();
    log
}

/// Events of a full runner run of `config` in a fresh dir, and the dir's
/// `log` lines
async fn run_full(config: CiConfig) -> (Vec<RunnerEvent>, Vec<String>) {
    let tmp = tempfile::TempDir::new().unwrap();
    let checks = select_for_a_txt(&config, tmp.path());
    let runner = CheckRunner::with_executor(config, tmp.path(), Arc::new(RealCommandExecutor));
    let (tx, rx) = tokio::sync::mpsc::channel(100);
    runner.run_checks(checks, tx).await.unwrap();
    (collect_events(rx).await, log_lines(tmp.path()))
}

fn ran(status: CheckStatus) -> (CheckStatus, bool, String) {
    (status, false, String::new())
}

fn blocked(reason: &str) -> (CheckStatus, bool, String) {
    (CheckStatus::Failed, true, reason.to_string())
}

/// `outcome` (see [`common::outcome`]) of check `id`, as in
/// [`common::outcomes`]
fn id(id: &str, outcome: (CheckStatus, bool, String)) -> (String, CheckStatus, bool, String) {
    let (status, not_run, output) = outcome;
    (id.to_string(), status, not_run, output)
}

// --- runner (TUI full run, `--fix` verify) ---

#[rstest::rstest]
#[case::sequential(false)]
#[case::parallel(true)]
#[tokio::test]
async fn failed_check_stops_later_groups_after_its_group_finishes(#[case] parallel: bool) {
    let config = with_sibling(two_groups("false"), parallel);

    let (events, log) = run_full(config).await;

    assert_eq!(
        sorted(log),
        ["c1", "c3", "p1", "p2"],
        "later group never ran"
    );
    assert_eq!(finished(&events, "c1"), [ran(CheckStatus::Failed)]);
    assert_eq!(finished(&events, "c3"), [ran(CheckStatus::Passed)]);
    assert_eq!(finished(&events, "c2"), [blocked(G1_STOPPED)]);
    assert_eq!(skipped_reason(&events, "p3").as_deref(), Some(G1_STOPPED));
    assert!(events.iter().any(|e| matches!(e,
        RunnerEvent::RunStopped { group, check_id } if group == "g1" && check_id == "c1")));
    assert!(finished(&events, "od").is_empty(), "on-demand not reported");
    assert!(matches!(events.last(), Some(RunnerEvent::AllFinished)));
}

/// [`with_sibling`] (parallel) where slow `c1` and fast `c3` both fail
fn two_failing_in_parallel() -> CiConfig {
    let mut config = with_sibling(two_groups("sleep 0.3; false"), true);
    config.checks["g1"].checks["c3"].command = logged("c3", "false", true);
    config
}

#[tokio::test]
async fn parallel_failures_name_the_first_check_in_input_order() {
    let (events, _) = run_full(two_failing_in_parallel()).await;

    assert_eq!(finished(&events, "c2"), [blocked(G1_STOPPED)]);
    assert!(events.iter().any(|e| matches!(e,
        RunnerEvent::RunStopped { check_id, .. } if check_id == "c1")));
}

#[tokio::test]
async fn timed_out_check_stops_later_groups() {
    let mut config = two_groups("sleep 5");
    config.checks["g1"].checks["c1"].timeout = Some(Duration::from_millis(100));

    let (events, log) = run_full(config).await;

    assert_eq!(finished(&events, "c1"), [ran(CheckStatus::TimedOut)]);
    assert_eq!(finished(&events, "c2"), [blocked(G1_STOPPED)]);
    assert!(!log.contains(&"c2".to_string()), "{log:?}");
}

#[tokio::test]
async fn allowed_failure_does_not_stop_the_run() {
    // Guard: a warning is not an error
    let mut config = two_groups("false");
    config.checks["g1"].checks["c1"].allow_failure = true;

    let (events, log) = run_full(config).await;

    assert_eq!(finished(&events, "c1"), [ran(CheckStatus::Failed)]);
    assert_eq!(finished(&events, "c2"), [ran(CheckStatus::Passed)]);
    assert_eq!(log, ["p1", "p2", "c1", "p3", "c2"]);
}

#[tokio::test]
async fn cancelled_check_does_not_stop_the_run() {
    // Guard: cancelled is not a failure
    let config = two_groups("sleep 30");
    let tmp = tempfile::TempDir::new().unwrap();
    let checks = select_for_a_txt(&config, tmp.path());
    let cancels = CancelRegistry::default();
    let executor = Arc::new(RealCommandExecutor);
    let runner =
        CheckRunner::with_executor(config, tmp.path(), executor).with_cancels(cancels.clone());
    let (tx, mut rx) = tokio::sync::mpsc::channel(100);
    let run = tokio::spawn(async move { runner.run_checks(checks, tx).await });
    let mut events = Vec::new();
    while let Some(event) = rx.recv().await {
        if matches!(&event, RunnerEvent::CheckStarted { check_id } if check_id == "c1") {
            cancels.cancel("c1");
        }
        events.push(event);
    }
    run.await.unwrap().unwrap();

    assert_eq!(finished(&events, "c1"), [ran(CheckStatus::Cancelled)]);
    assert_eq!(finished(&events, "c2"), [ran(CheckStatus::Passed)]);
}

#[tokio::test]
async fn checks_blocked_by_skip_group_do_not_stop_the_run() {
    // Guard: only checks that ran count
    let mut config = two_groups("true");
    config.checks["g1"].pre_commands[0].command = "false".into();
    config.checks["g1"].pre_commands[0].on_failure = OnFailure::SkipGroup;

    let (events, _) = run_full(config).await;

    assert_eq!(finished(&events, "c1"), [blocked(G1_FAILED)]);
    assert_eq!(finished(&events, "c2"), [ran(CheckStatus::Passed)]);
}

#[tokio::test]
async fn aborting_pre_command_keeps_its_reason() {
    // Guard: `on_failure: abort` in a stop_on_failure group, unchanged
    let mut config = two_groups("true");
    config.checks["g1"].pre_commands[0].command = "false".into();

    let (events, _) = run_full(config).await;

    assert_eq!(finished(&events, "c2"), [blocked(G1_ABORTED)]);
    assert_eq!(skipped_reason(&events, "p3").as_deref(), Some(G1_ABORTED));
}

#[tokio::test]
async fn without_stop_on_failure_later_groups_run() {
    // Guard: default `stop_on_failure: false`
    let mut config = two_groups("false");
    config.checks["g1"].stop_on_failure = false;

    let (events, log) = run_full(config).await;

    assert_eq!(finished(&events, "c2"), [ran(CheckStatus::Passed)]);
    assert_eq!(log, ["p1", "p2", "c1", "p3", "c2"]);
}

#[tokio::test]
async fn passing_stop_on_failure_group_runs_later_groups() {
    // Guard
    let (events, _) = run_full(two_groups("true")).await;

    assert_eq!(finished(&events, "c2"), [ran(CheckStatus::Passed)]);
    assert_eq!(skipped_reason(&events, "p3"), None);
}

// --- simple ---

#[rstest::rstest]
#[case::sequential(false)]
#[case::parallel(true)]
#[tokio::test]
async fn simple_failed_check_stops_later_groups(#[case] parallel: bool) {
    let config = with_sibling(two_groups("false"), parallel);

    let (results, log) = run_simple(config).await;

    assert_eq!(
        sorted(log),
        ["c1", "c3", "p1", "p2"],
        "later group never ran"
    );
    assert_eq!(
        outcomes(&results),
        [
            id("c1", ran(CheckStatus::Failed)),
            id("c3", ran(CheckStatus::Passed)),
            id("c2", blocked(G1_STOPPED)),
        ]
    );
}

#[tokio::test]
async fn simple_parallel_failures_name_the_first_check_in_input_order() {
    // Guard: simple already picks by input order
    let (results, _) = run_simple(two_failing_in_parallel()).await;

    assert_eq!(outcomes(&results)[2], id("c2", blocked(G1_STOPPED)));
}

#[tokio::test]
async fn simple_allowed_failure_does_not_stop_the_run() {
    // Guard
    let mut config = two_groups("false");
    config.checks["g1"].checks["c1"].allow_failure = true;

    let (results, log) = run_simple(config).await;

    assert_eq!(outcomes(&results)[1], id("c2", ran(CheckStatus::Passed)));
    assert_eq!(log, ["p1", "p2", "c1", "p3", "c2"]);
}

#[tokio::test]
async fn simple_checks_blocked_by_skip_group_do_not_stop_the_run() {
    // Guard
    let mut config = on_failure_groups("false", OnFailure::SkipGroup, true);
    config.checks["g1"].stop_on_failure = true;

    let (results, _) = run_simple(config).await;

    assert_eq!(
        outcomes(&results),
        [
            id("c1", blocked(G1_FAILED)),
            id("c2", ran(CheckStatus::Passed))
        ]
    );
}

// Edge case: raw YAML, the binary reads it from disk.
/// Local-mode config: `stop_on_failure` group `ga` (check `a` failing,
/// fix `true`), then `gb` (pre-command `prep`; check `b` passing once its
/// fix touched a marker file)
const STOP_TWO_GROUPS: &str = "runner: local
checks:
  ga:
    stop_on_failure: true
    checks:
      a: { name: A, command: 'false', fix_command: 'true' }
  gb:
    pre_commands:
      - { name: prep, command: 'true' }
    checks:
      b: { name: B, command: 'test -e fixed_b', fix_command: 'touch fixed_b' }
";

const GA_STOPPED: &str = "Not run: run stopped by failed check `a` of group `ga` (stop_on_failure)";

#[test]
fn simple_binary_prints_later_groups_not_run_and_exits_1() {
    let (_tmp, out) = run_config(
        &format!("{YAML_HEAD}{STOP_TWO_GROUPS}"),
        &["--simple", "--no-color", "-f", "a.rs"],
    );

    assert_code(&out, exit::CHECKS_FAILED);
    let text = stdout(&out);
    assert!(text.contains("✗ b not run"), "{text}");
    assert!(
        text.contains(&format!("⊘ pre-command prep skipped ({GA_STOPPED})")),
        "{text}"
    );
}

#[test]
fn fix_verify_reports_later_groups_not_run() {
    let (_tmp, out) = run_config(
        &format!("{YAML_HEAD}{STOP_TWO_GROUPS}"),
        &["--fix", "--no-color", "-f", "a.rs"],
    );

    assert_code(&out, exit::CHECKS_FAILED);
    let text = stdout(&out);
    assert!(!text.contains("✓ b verified"), "{text}");
    assert!(text.contains(GA_STOPPED), "{text}");
}
