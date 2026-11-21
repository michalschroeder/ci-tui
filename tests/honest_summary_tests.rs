//! Honest `--simple` summaries (#309): a run that executed nothing says so
//! and every summary counts skipped / on-demand checks; groups with nothing
//! to run print no header.

use ci_tui::exit;

mod common;
use common::{assert_code, run_config_env, stdout};

// Edge case: the binary reads a YAML file from disk, so configs stay raw YAML.
/// Local-mode config: `lint` (`fmt` takes `{files}`, `pedantic` is on-demand)
/// and `slow` (`bench` triggers on rust files, no `{files}`: on-demand when
/// nothing matches). `extra` is appended (more groups).
fn config(extra: &str) -> String {
    format!(
        "version: 2
runner: local
git:
  base_branch: main
  fallback_branch: HEAD~1
file_patterns:
  rust:
    pattern: '\\.rs$'
checks:
  lint:
    checks:
      fmt:
        name: Fmt
        command: 'true {{files}}'
        triggers:
          file_pattern: rust
      pedantic:
        name: Pedantic
        command: 'true'
        on_demand: true
  slow:
    checks:
      bench:
        name: Bench
        command: 'true'
        triggers:
          file_pattern: rust
{extra}"
    )
}

const NOTHING_RAN: &str = "0 of 3 checks ran (1 skipped, 2 on-demand) — see --list";

#[test]
fn docs_only_change_prints_nothing_ran_and_exits_0() {
    let (_tmp, out) = run_config_env(&config(""), &["-s", "-f", "README.md"], &[]);
    assert_code(&out, exit::SUCCESS);
    let text = stdout(&out);
    assert!(text.contains(NOTHING_RAN), "{text}");
    assert!(!text.contains("checks passed"), "{text}");
}

#[test]
fn nothing_to_run_groups_print_no_header_or_blank_line() {
    let (_tmp, out) = run_config_env(&config(""), &["-s", "-f", "README.md"], &[]);
    let text = stdout(&out);
    assert!(!text.contains("── LINT ──"), "{text}");
    assert!(!text.contains("── SLOW ──"), "{text}");
    assert!(
        text.contains("(manual)\n\n── Summary ──\n"),
        "no blank line left by dropped groups: {text:?}"
    );
}

#[test]
fn github_actions_emits_no_group_pair_for_nothing_to_run() {
    let env = [("GITHUB_ACTIONS", "true")];
    let (_tmp, out) = run_config_env(&config(""), &["-s", "-f", "README.md"], &env);
    let text = stdout(&out);
    assert!(!text.contains("::group::"), "{text}");
    assert!(!text.contains("::endgroup::"), "{text}");
}

#[test]
fn run_groups_keep_header_and_summary_counts_the_rest() {
    let env = [("GITHUB_ACTIONS", "true")];
    let (_tmp, out) = run_config_env(&config(""), &["-s", "-f", "a.rs"], &env);
    assert_code(&out, exit::SUCCESS);
    let text = stdout(&out);
    assert!(text.contains("::group::LINT"), "{text}");
    assert!(text.contains("::group::SLOW"), "{text}");
    assert_eq!(text.matches("::endgroup::").count(), 2, "{text}");
    assert!(text.contains("✓ All 2 checks passed in"), "{text}");
    assert!(text.contains("s (1 on-demand)\n"), "{text}");
}

#[test]
fn group_with_only_on_demand_checks_drops_its_header() {
    // `--only` keeps `pedantic` (on-demand) and `bench` (runs): `lint` has
    // nothing to run, `slow` does
    let args = ["-s", "--only", "pedantic,bench", "-f", "a.rs"];
    let (_tmp, out) = run_config_env(&config(""), &args, &[]);
    assert_code(&out, exit::SUCCESS);
    let text = stdout(&out);
    assert!(!text.contains("── LINT ──"), "{text}");
    assert!(text.contains("── SLOW ──"), "{text}");
    assert!(text.contains("(1 on-demand)"), "{text}");
}

#[test]
fn only_on_demand_check_is_a_message_not_an_error() {
    let args = ["-s", "--only", "pedantic", "-f", "a.rs"];
    let (_tmp, out) = run_config_env(&config(""), &args, &[]);
    assert_code(&out, exit::SUCCESS);
    let text = stdout(&out);
    assert!(
        text.contains("0 of 1 checks ran (1 on-demand) — see --list"),
        "{text}"
    );
    assert!(!text.contains("── LINT ──"), "{text}");
}

#[test]
fn json_summary_counts_skipped_and_on_demand() {
    let args = ["--format", "json", "-f", "README.md"];
    let (_tmp, out) = run_config_env(&config(""), &args, &[]);
    assert_code(&out, exit::SUCCESS);
    let doc: serde_json::Value = serde_json::from_str(&stdout(&out)).unwrap();
    assert_eq!(doc["version"], 1);
    assert_eq!(doc["summary"]["total"], 0);
    assert_eq!(doc["summary"]["skipped"], 1);
    assert_eq!(doc["summary"]["on_demand"], 2);
    assert_eq!(doc["checks"].as_array().unwrap().len(), 0);
}

#[test]
fn stopped_group_keeps_its_header_and_skipped_pre_commands() {
    // guard: a group a failed pre-command stopped the run before still has a
    // runnable check, so it keeps its header with the skipped pre-command
    // line under it
    let early = "  early:
    pre_commands:
      - name: boom
        command: 'false'
    checks:
      first:
        name: First
        command: 'true'
        triggers:
          file_pattern: rust
  late:
    pre_commands:
      - name: warm
        command: 'true'
    checks:
      after:
        name: After
        command: 'true'
        triggers:
          file_pattern: rust
";
    let (_tmp, out) = run_config_env(&config(early), &["-s", "-f", "a.rs"], &[]);
    assert_code(&out, exit::CHECKS_FAILED);
    let text = stdout(&out);
    let late = text.find("── LATE ──").unwrap_or_else(|| panic!("{text}"));
    let skipped = text
        .find("pre-command warm skipped")
        .unwrap_or_else(|| panic!("{text}"));
    assert!(late < skipped, "{text}");
}
