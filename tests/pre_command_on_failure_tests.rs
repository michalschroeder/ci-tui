//! Per-pre-command `on_failure` (#152): config parsing through the real
//! loader, runner event sequences for `abort` / `skip_group` / `continue`
//! (real local-mode commands), and `--fix` verification through the binary.
//! TUI state is unit-tested in `ui`.

use ci_tui::config::{CiConfig, OnFailure};
use ci_tui::exit;
use ci_tui::runner::{
    CheckRunner, CheckStatus, PreCommandOutcome, RealCommandExecutor, RunnerEvent,
};
use std::path::Path;
use std::sync::Arc;
use std::time::Duration;

mod common;
use common::configs::{CheckBuilder, ConfigBuilder, YAML_HEAD};
use common::{
    assert_code, collect_events, load, local_sh, make_exec_check, make_on_demand_check, run_config,
    stdout,
};

// Edge case: raw YAML, the loader is under test.
/// Local-mode config: group `g` with pre-command `p` (`extra` appended to
/// its entry) and check `c`
fn yaml_with_pre_command(extra: &str) -> String {
    format!(
        "{YAML_HEAD}runner: local
checks:
  g:
    pre_commands:
      - {{ name: p, command: 'true'{extra} }}
    checks:
      c: {{ name: C, command: 'true' }}
"
    )
}

#[rstest::rstest]
#[case::unset("", OnFailure::Abort)]
#[case::abort(", on_failure: abort", OnFailure::Abort)]
#[case::skip_group(", on_failure: skip_group", OnFailure::SkipGroup)]
#[case::continue_(", on_failure: continue", OnFailure::Continue)]
fn on_failure_parses(#[case] extra: &str, #[case] expected: OnFailure) {
    let config = load(&yaml_with_pre_command(extra)).expect("config loads");
    assert_eq!(config.pre_commands("g")[0].on_failure, expected);
}

#[test]
fn unknown_on_failure_value_is_a_config_error_naming_the_field() {
    let err = load(&yaml_with_pre_command(", on_failure: retry")).unwrap_err();
    let err = format!("{err:#}");
    assert!(err.contains("on_failure"), "{err}");
    assert!(err.contains("skip_group"), "lists valid values: {err}");
}

/// Two local-mode groups: `g1` runs pre-commands `p1` (`p1_cmd`, with
/// `policy`) and `p2`, then check `c1` (plus on-demand `od`); `g2` runs
/// pre-command `p3`, then check `c2`
fn two_groups(p1_cmd: &str, policy: OnFailure) -> CiConfig {
    let check = || CheckBuilder::new("C", "true").build();
    let mut config = ConfigBuilder::new()
        .with_pre_command("g1", "p1", p1_cmd)
        .with_pre_command("g1", "p2", "true")
        .with_check("g1", "c1", check())
        .with_check(
            "g1",
            "od",
            CheckBuilder::new("Od", "true").on_demand().build(),
        )
        .with_pre_command("g2", "p3", "true")
        .with_check("g2", "c2", check())
        .build();
    config.checks["g1"].pre_commands[0].on_failure = policy;
    config.runner = local_sh();
    config
}

/// Events of a full run of [`two_groups`]' checks
async fn run_two_groups(config: CiConfig) -> Vec<RunnerEvent> {
    let mut c1 = make_exec_check("c1", "true", None);
    c1.group = "g1".into();
    let od = make_on_demand_check("od", "g1", "Od");
    let mut c2 = make_exec_check("c2", "true", None);
    c2.group = "g2".into();
    let runner =
        CheckRunner::with_executor(config, Path::new("/tmp"), Arc::new(RealCommandExecutor));
    let (tx, rx) = tokio::sync::mpsc::channel(100);
    runner.run_checks(vec![c1, od, c2], tx).await.unwrap();
    collect_events(rx).await
}

/// `(status, not_run, output)` of each `CheckFinished` for `id`
fn finished(events: &[RunnerEvent], id: &str) -> Vec<(CheckStatus, bool, String)> {
    events
        .iter()
        .filter_map(|e| match e {
            RunnerEvent::CheckFinished { result } if result.check_id == id => {
                Some((result.status.clone(), result.not_run, result.output.clone()))
            }
            _ => None,
        })
        .collect()
}

fn started(events: &[RunnerEvent], id: &str) -> bool {
    events
        .iter()
        .any(|e| matches!(e, RunnerEvent::CheckStarted { check_id } if check_id == id))
}

fn pre_started(events: &[RunnerEvent], pre: &str) -> bool {
    events
        .iter()
        .any(|e| matches!(e, RunnerEvent::PreCommandStarted { name, .. } if name == pre))
}

/// Reason of the `PreCommandSkipped` event for `pre`, if any
fn skipped_reason(events: &[RunnerEvent], pre: &str) -> Option<String> {
    events.iter().find_map(|e| match e {
        RunnerEvent::PreCommandSkipped { name, reason, .. } if name == pre => Some(reason.clone()),
        _ => None,
    })
}

const G1_FAILED: &str = "Not run: pre-command `p1` of group `g1` failed";
const ABORTED: &str = "Not run: run aborted by failed pre-command `p1` of group `g1`";

#[tokio::test]
async fn abort_reports_the_rest_of_the_run_as_not_run() {
    let events = run_two_groups(two_groups("false", OnFailure::Abort)).await;

    assert!(!pre_started(&events, "p2") && !pre_started(&events, "p3"));
    assert!(!started(&events, "c1") && !started(&events, "c2"));
    assert_eq!(skipped_reason(&events, "p2").as_deref(), Some(G1_FAILED));
    assert_eq!(skipped_reason(&events, "p3").as_deref(), Some(ABORTED));
    assert_eq!(
        finished(&events, "c1"),
        [(CheckStatus::Failed, true, G1_FAILED.to_string())]
    );
    assert_eq!(
        finished(&events, "c2"),
        [(CheckStatus::Failed, true, ABORTED.to_string())]
    );
    assert!(
        finished(&events, "od").is_empty(),
        "on-demand stays on-demand"
    );
    assert!(matches!(events.last(), Some(RunnerEvent::AllFinished)));
}

#[tokio::test]
async fn skip_group_skips_its_group_and_runs_later_groups() {
    let events = run_two_groups(two_groups("false", OnFailure::SkipGroup)).await;

    assert!(!pre_started(&events, "p2") && !started(&events, "c1"));
    assert_eq!(skipped_reason(&events, "p2").as_deref(), Some(G1_FAILED));
    assert_eq!(
        finished(&events, "c1"),
        [(CheckStatus::Failed, true, G1_FAILED.to_string())]
    );
    assert!(
        finished(&events, "od").is_empty(),
        "on-demand stays on-demand"
    );
    assert!(pre_started(&events, "p3"));
    assert_eq!(finished(&events, "c2")[0].0, CheckStatus::Passed);
}

#[tokio::test]
async fn continue_runs_the_group_as_if_it_passed() {
    let events = run_two_groups(two_groups("false", OnFailure::Continue)).await;

    assert!(events.iter().any(|e| matches!(e,
        RunnerEvent::PreCommandFinished { name, outcome: PreCommandOutcome::Ignored, .. } if name == "p1")));
    assert!(pre_started(&events, "p2"));
    assert_eq!(skipped_reason(&events, "p2"), None);
    assert_eq!(finished(&events, "c1")[0].0, CheckStatus::Passed);
    assert_eq!(finished(&events, "c2")[0].0, CheckStatus::Passed);
}

#[tokio::test]
async fn timed_out_pre_command_follows_its_on_failure() {
    let mut config = two_groups("sleep 5", OnFailure::SkipGroup);
    config.checks["g1"].pre_commands[0].timeout = Some(Duration::from_millis(100));

    let events = run_two_groups(config).await;

    assert_eq!(
        finished(&events, "c1"),
        [(CheckStatus::Failed, true, G1_FAILED.to_string())]
    );
    assert_eq!(finished(&events, "c2")[0].0, CheckStatus::Passed);
}

#[tokio::test]
async fn passing_pre_commands_report_nothing_skipped() {
    // Guard: no failure, no skip events or not-run results
    let events = run_two_groups(two_groups("true", OnFailure::Abort)).await;

    assert!(!events
        .iter()
        .any(|e| matches!(e, RunnerEvent::PreCommandSkipped { .. })));
    assert_eq!(finished(&events, "c1")[0].0, CheckStatus::Passed);
    assert_eq!(finished(&events, "c2")[0].0, CheckStatus::Passed);
}

// Edge case: raw YAML, the binary reads it from disk.
/// Local-mode `--fix` config: groups `ga` (pre-command `seed` failing with
/// `skip_group`) and `gb`, each with a check that passes once its fix
/// touched a marker file
const FIX_TWO_GROUPS: &str = "runner: local
checks:
  ga:
    pre_commands:
      - { name: seed, command: 'false', on_failure: skip_group }
    checks:
      a: { name: A, command: 'test -e fixed_a', fix_command: 'touch fixed_a' }
  gb:
    checks:
      b: { name: B, command: 'test -e fixed_b', fix_command: 'touch fixed_b' }
";

#[test]
fn fix_verify_with_skipped_group_still_verifies_later_group() {
    let (_tmp, out) = run_config(
        &format!("{YAML_HEAD}{FIX_TWO_GROUPS}"),
        &["--fix", "--no-color", "-f", "a.rs"],
    );

    assert_code(&out, exit::CHECKS_FAILED);
    let text = stdout(&out);
    assert!(text.contains("✓ b verified"), "{text}");
    assert!(
        text.contains("Not run: pre-command `seed` of group `ga` failed"),
        "{text}"
    );
}

#[test]
fn fix_verify_with_ignored_pre_command_failure_exits_0() {
    let config = FIX_TWO_GROUPS.replace("on_failure: skip_group", "on_failure: continue");
    let (_tmp, out) = run_config(
        &format!("{YAML_HEAD}{config}"),
        &["--fix", "--no-color", "-f", "a.rs"],
    );

    assert_code(&out, exit::SUCCESS);
    let text = stdout(&out);
    assert!(text.contains("✓ a verified"), "{text}");
}

#[test]
fn fix_verify_prints_ignored_pre_command_failure_as_warning() {
    let config = FIX_TWO_GROUPS.replace("on_failure: skip_group", "on_failure: continue");
    let (_tmp, out) = run_config(
        &format!("{YAML_HEAD}{config}"),
        &["--fix", "--no-color", "-f", "a.rs"],
    );

    let text = stdout(&out);
    assert!(
        text.contains("⚠ pre-command seed failed (ignored: on_failure: continue)"),
        "{text}"
    );
    assert!(!text.contains("✗ pre-command seed"), "{text}");
}
