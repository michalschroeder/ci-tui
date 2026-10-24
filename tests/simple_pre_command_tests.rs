//! Group `pre_commands` in `--simple` (#244): setup runs before a group's
//! checks with the runner's `on_failure` semantics, through
//! `simple::run_with_executor` (real local-mode commands) and the binary
//! (text, `--format json|junit`, result cache).

use ci_tui::checks::{determine_checks, CheckToRun};
use ci_tui::config::{CiConfig, OnFailure};
use ci_tui::exit;
use ci_tui::git::ChangedFiles;
use ci_tui::runner::{CheckResult, CheckStatus, RealCommandExecutor};
use ci_tui::simple::run_with_executor;
use std::path::Path;
use std::sync::Arc;

mod common;
use common::configs::{
    logged, on_failure_groups, CheckBuilder, ConfigBuilder, G1_ABORTED, G1_FAILED, NEVER_PATTERN,
    YAML_HEAD,
};
use common::{assert_code, local_sh, project, run_config, simple_run, stderr, stdout, write};

/// [`on_failure_groups`] with every command logging its name
fn two_groups(p1_cmd: &str, policy: OnFailure) -> CiConfig {
    on_failure_groups(p1_cmd, policy, true)
}

/// `run_with_executor` of every check `config` selects for `a.txt`, in a
/// fresh project dir; the results and the dir's `log` lines
async fn run_simple(config: CiConfig) -> (Vec<CheckResult>, Vec<String>) {
    let tmp = tempfile::TempDir::new().unwrap();
    let changed = ChangedFiles {
        files: vec!["a.txt".into()],
        base_ref: "main".into(),
    };
    let checks: Vec<CheckToRun> = determine_checks(&config, &changed, tmp.path());
    let executor = Arc::new(RealCommandExecutor);
    let results = run_with_executor(config, checks, tmp.path().to_path_buf(), executor)
        .await
        .unwrap();
    let log = log(tmp.path());
    (results, log.lines().map(str::to_owned).collect())
}

/// Lines commands appended to `log` in `dir`
fn log(dir: &Path) -> String {
    std::fs::read_to_string(dir.join("log")).unwrap_or_default()
}

/// Byte offset of `s` in `text`, panicking with `text` when missing
fn pos(text: &str, s: &str) -> usize {
    text.find(s)
        .unwrap_or_else(|| panic!("{s:?} missing: {text}"))
}

/// `(id, status, not_run, output)` of each result
fn outcomes(results: &[CheckResult]) -> Vec<(String, CheckStatus, bool, String)> {
    results
        .iter()
        .map(|r| {
            let output = if r.not_run {
                r.output.clone()
            } else {
                String::new()
            };
            (r.check_id.clone(), r.status.clone(), r.not_run, output)
        })
        .collect()
}

fn passed(id: &str) -> (String, CheckStatus, bool, String) {
    (id.into(), CheckStatus::Passed, false, String::new())
}

fn blocked(id: &str, reason: &str) -> (String, CheckStatus, bool, String) {
    (id.into(), CheckStatus::Failed, true, reason.into())
}

#[rstest::rstest]
#[case::sequential(false)]
#[case::parallel(true)]
#[tokio::test]
async fn passing_pre_commands_run_once_before_their_group(#[case] parallel: bool) {
    let mut config = two_groups("true", OnFailure::Abort);
    config.checks["g1"].parallel = parallel;

    let (results, log) = run_simple(config).await;

    assert_eq!(log, ["p1", "p2", "c1", "p3", "c2"]);
    assert_eq!(outcomes(&results), [passed("c1"), passed("c2")]);
}

#[tokio::test]
async fn group_with_only_on_demand_checks_runs_no_pre_commands() {
    let check = |id: &str| CheckBuilder::new(id, &logged(id, "true", true));
    let mut config = ConfigBuilder::new()
        .with_file_pattern("never", NEVER_PATTERN, None)
        .with_pre_command("g1", "p1", &logged("p1", "true", true))
        .with_check(
            "g1",
            "od",
            check("od").with_file_pattern_trigger("never").build(),
        )
        .with_pre_command("g2", "p2", &logged("p2", "true", true))
        .with_check("g2", "c", check("c").build())
        .build();
    config.runner = local_sh();

    let (results, log) = run_simple(config).await;

    assert_eq!(log, ["p2", "c"]);
    assert_eq!(outcomes(&results), [passed("c")]);
}

#[tokio::test]
async fn skip_group_blocks_its_group_and_runs_later_groups() {
    let (results, log) = run_simple(two_groups("false", OnFailure::SkipGroup)).await;

    assert_eq!(log, ["p1", "p3", "c2"]);
    assert_eq!(outcomes(&results), [blocked("c1", G1_FAILED), passed("c2")]);
}

#[tokio::test]
async fn abort_blocks_the_rest_of_the_run() {
    let (results, log) = run_simple(two_groups("false", OnFailure::Abort)).await;

    assert_eq!(log, ["p1"]);
    assert_eq!(
        outcomes(&results),
        [blocked("c1", G1_FAILED), blocked("c2", G1_ABORTED)],
        "on-demand `od` not reported"
    );
}

#[tokio::test]
async fn continue_runs_the_group_as_if_setup_passed() {
    let (results, log) = run_simple(two_groups("false", OnFailure::Continue)).await;

    assert_eq!(log, ["p1", "p2", "c1", "p3", "c2"]);
    assert_eq!(outcomes(&results), [passed("c1"), passed("c2")]);
}

#[tokio::test]
async fn timed_out_pre_command_follows_its_on_failure() {
    let mut config = two_groups("sleep 5", OnFailure::SkipGroup);
    config.checks["g1"].pre_commands[0].timeout = Some(std::time::Duration::from_millis(100));

    let (results, _) = run_simple(config).await;

    assert_eq!(outcomes(&results), [blocked("c1", G1_FAILED), passed("c2")]);
}

// Edge case: raw YAML, the binary reads it from disk.
/// Issue #244 repro: local-mode group `main` whose pre-command `setup`
/// prints `db down` and fails (`extra` appended to its entry), check `ok`
fn repro(extra: &str, check_extra: &str) -> String {
    format!(
        "{YAML_HEAD}runner: local
checks:
  main:
    pre_commands:
      - {{ name: setup, command: 'echo db down; false'{extra} }}
    checks:
      ok: {{ name: OK, command: 'true'{check_extra} }}
"
    )
}

const SETUP_FAILED: &str = "Not run: pre-command `setup` of group `main` failed";

#[test]
fn failed_pre_command_blocks_the_group_and_exits_1() {
    let (_tmp, out) = run_config(&repro("", ""), &["--simple", "--no-color", "-f", "a.txt"]);

    assert_code(&out, exit::CHECKS_FAILED);
    let text = stdout(&out);
    let failed = pos(&text, "  ✗ pre-command setup failed\n    db down\n");
    assert!(pos(&text, "── MAIN ──") < failed);
    assert!(failed < pos(&text, "    db down\n  ✗ ok not run\n"));
    assert!(pos(&text, "Failed checks:") < pos(&text, SETUP_FAILED));
    assert!(text.contains("0/1 checks passed, 1 failed"), "{text}");
}

#[test]
fn ignored_pre_command_failure_is_a_warning_and_exits_0() {
    let config = repro(", on_failure: continue", "");
    let (_tmp, out) = run_config(&config, &["--simple", "--no-color", "-f", "a.txt"]);

    assert_code(&out, exit::SUCCESS);
    let text = stdout(&out);
    assert!(
        text.contains(
            "  ⚠ pre-command setup failed (ignored: on_failure: continue)\n    db down\n"
        ),
        "{text}"
    );
    assert!(text.contains("All 1 checks passed"), "{text}");
}

#[test]
fn ignored_pre_command_failure_line_is_yellow_with_color() {
    let config = repro(", on_failure: continue", "");
    let (_tmp, out) = run_config(&config, &["--simple", "--color", "always", "-f", "a.txt"]);

    let text = stdout(&out);
    assert!(
        text.contains("  \x1b[33m⚠\x1b[0m pre-command setup"),
        "{text:?}"
    );
}

// Edge case: raw YAML, the binary reads it from disk.
/// Local-mode config: `g1`'s pre-command `p1` fails (`extra` appended to
/// its entry), then `p2`; `g2`'s pre-command `p3` fails (default `abort`)
fn failing_groups(extra: &str) -> String {
    format!(
        "{YAML_HEAD}runner: local
checks:
  g1:
    pre_commands:
      - {{ name: p1, command: 'false'{extra} }}
      - {{ name: p2, command: 'true' }}
    checks:
      c1: {{ name: C1, command: 'true' }}
  g2:
    pre_commands:
      - {{ name: p3, command: 'false' }}
    checks:
      c2: {{ name: C2, command: 'true' }}
"
    )
}

#[test]
fn skip_group_prints_remaining_pre_commands_skipped() {
    let config = failing_groups(", on_failure: skip_group");
    let (_tmp, out) = run_config(&config, &["--simple", "--no-color", "-f", "a.txt"]);

    assert_code(&out, exit::CHECKS_FAILED);
    let text = stdout(&out);
    assert!(
        text.contains(&format!("  ⊘ pre-command p2 skipped ({G1_FAILED})\n")),
        "{text}"
    );
    // g2 still runs its setup: its own (aborting) failure blocks c2
    assert!(text.contains("  ✗ pre-command p3 failed\n"), "{text}");
}

#[test]
fn abort_prints_later_pre_commands_skipped() {
    let (_tmp, out) = run_config(
        &failing_groups(""),
        &["--simple", "--no-color", "-f", "a.txt"],
    );

    let text = stdout(&out);
    let skipped = format!("  ⊘ pre-command p3 skipped ({G1_ABORTED})");
    assert!(pos(&text, "── G2 ──") < pos(&text, &skipped));
    assert!(!text.contains("✗ pre-command p3"), "{text}");
}

#[test]
fn json_report_counts_blocked_check_as_failed() {
    let (_tmp, out) = run_config(&repro("", ""), &["--format", "json", "-f", "a.txt"]);

    assert_code(&out, exit::CHECKS_FAILED);
    let doc: serde_json::Value = serde_json::from_str(&stdout(&out)).expect("only the report");
    assert_eq!(doc["checks"][0]["id"], "ok");
    assert_eq!(doc["checks"][0]["status"], "failed");
    assert_eq!(doc["checks"][0]["output"], SETUP_FAILED);
    assert_eq!(doc["summary"]["failed"], 1);
    let err = stderr(&out);
    assert!(
        err.contains("✗ pre-command setup failed\n    db down\n"),
        "{err}"
    );
}

#[test]
fn junit_report_shows_blocked_check_as_failure() {
    let (_tmp, out) = run_config(&repro("", ""), &["--format", "junit", "-f", "a.txt"]);

    assert_code(&out, exit::CHECKS_FAILED);
    let doc = stdout(&out);
    assert!(doc.starts_with("<?xml"), "{doc}");
    assert!(
        doc.contains("<failure message=\"failed\">Not run: pre-command `setup`"),
        "{doc}"
    );
    assert!(stderr(&out).contains("✗ pre-command setup failed"));
}

#[test]
fn report_file_keeps_pre_command_lines_on_stdout() {
    let args = [
        "--format",
        "json",
        "--output",
        "r.json",
        "--no-color",
        "-f",
        "a.txt",
    ];
    let (_tmp, out) = run_config(&repro("", ""), &args);

    assert_code(&out, exit::CHECKS_FAILED);
    assert!(stdout(&out).contains("✗ pre-command setup failed"));
    assert!(!stderr(&out).contains("pre-command"), "{}", stderr(&out));
}

#[test]
fn blocked_allow_failure_check_still_fails_the_run() {
    let config = repro("", ", allow_failure: true");
    let (_tmp, out) = run_config(&config, &["--format", "json", "-f", "a.txt"]);

    assert_code(&out, exit::CHECKS_FAILED);
    let doc: serde_json::Value = serde_json::from_str(&stdout(&out)).unwrap();
    assert_eq!(doc["summary"]["failed"], 1);
    assert_eq!(doc["summary"]["warnings"], 0);
}

// Edge case: raw YAML, the binary reads it from disk; the result cache
// needs a git repo and file-triggered checks.
/// Local-mode config: groups `g0` (pre-command `s0`, always-run `first`),
/// `g1` (pre-command `s1`, `lint` triggered by `.rs` files: cacheable) and
/// `g2` (pre-command `s2`, always-run `always`). Pre-command `sN` fails
/// while a `failN` file exists; every command logs its name to `log`.
const CACHED: &str = r#"version: 2
git:
  base_branch: main
  fallback_branch: HEAD~1
runner: local
local:
  shell: sh
file_patterns:
  rust:
    pattern: '\.rs$'
checks:
  g0:
    pre_commands:
      - { name: s0, command: 'echo s0 >> log; test ! -e fail0' }
    checks:
      first: { name: First, command: 'echo first >> log' }
  g1:
    pre_commands:
      - { name: s1, command: 'echo s1 >> log; test ! -e fail1' }
    checks:
      lint: { name: Lint, command: 'echo lint >> log', triggers: { file_pattern: rust } }
  g2:
    pre_commands:
      - { name: s2, command: 'echo s2 >> log; test ! -e fail2' }
    checks:
      always: { name: Always, command: 'echo always >> log' }
"#;

/// Git repo with [`CACHED`] and `a.rs`, after one passing run (`lint`
/// cached from then on); `log` cleared
fn cached_project() -> tempfile::TempDir {
    let tmp = project(CACHED, true);
    assert_code(&simple(tmp.path(), &[]), exit::SUCCESS);
    std::fs::remove_file(tmp.path().join("log")).unwrap();
    tmp
}

/// `--simple --no-color` run on `a.rs` plus `extra` args
fn simple(dir: &Path, extra: &[&str]) -> std::process::Output {
    simple_run(dir, &[&["--no-color"], extra].concat())
}

#[test]
fn all_cached_group_runs_no_pre_commands() {
    let tmp = cached_project();

    let out = simple(tmp.path(), &[]);

    assert_code(&out, exit::SUCCESS);
    assert_eq!(
        log(tmp.path()),
        "s0\nfirst\ns2\nalways\n",
        "{}",
        stdout(&out)
    );
    assert!(stdout(&out).contains("✓ lint cached"), "{}", stdout(&out));
}

#[test]
fn abort_keeps_later_cached_checks_cached() {
    let tmp = cached_project();
    write(tmp.path(), "fail0", "");

    let out = simple(tmp.path(), &[]);

    assert_code(&out, exit::CHECKS_FAILED);
    let text = stdout(&out);
    assert_eq!(log(tmp.path()), "s0\n", "{text}");
    assert!(text.contains("✓ lint cached"), "{text}");
    let aborted = "Not run: run aborted by failed pre-command `s0` of group `g0`";
    assert!(
        text.contains(&format!("┌─ always ─┐\n  {aborted}")),
        "{text}"
    );
    assert!(text.contains("1/3 checks passed, 2 failed"), "{text}");
}

#[test]
fn blocked_check_keeps_its_cached_pass() {
    // Guard: --no-cache runs `lint`; its blocked result must not drop the pass
    let tmp = cached_project();
    write(tmp.path(), "fail1", "");
    assert_code(&simple(tmp.path(), &["--no-cache"]), exit::CHECKS_FAILED);
    std::fs::remove_file(tmp.path().join("fail1")).unwrap();

    let out = simple(tmp.path(), &[]);

    assert!(stdout(&out).contains("✓ lint cached"), "{}", stdout(&out));
}
