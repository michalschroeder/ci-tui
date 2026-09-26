//! Tests for `--only` / `--group`: config filtering, error on unknown ids,
//! and end-to-end binary runs in `--simple`, `--fix` and `--list` modes.

use ci_tui::checks::{determine_checks, Decision};
use ci_tui::filter;
use ci_tui::git::ChangedFiles;
use std::path::Path;

mod common;
use common::configs::checks_test_config;

fn ids(list: &[&str]) -> Vec<String> {
    list.iter().map(|s| s.to_string()).collect()
}

/// `(group, check)` pairs left in the config after filtering.
fn filtered(only: &[&str], groups: &[&str]) -> Vec<(String, String)> {
    let mut config = checks_test_config();
    filter::apply(&mut config, &ids(only), &ids(groups)).unwrap();
    config
        .groups()
        .flat_map(|(g, gc)| gc.checks.keys().map(move |id| (g.to_string(), id.clone())))
        .collect()
}

fn pairs(list: &[(&str, &str)]) -> Vec<(String, String)> {
    list.iter()
        .map(|(g, c)| (g.to_string(), c.to_string()))
        .collect()
}

fn filter_err(only: &[&str], groups: &[&str]) -> String {
    let mut config = checks_test_config();
    filter::apply(&mut config, &ids(only), &ids(groups)).expect_err("expected error")
}

// ---------------------------------------------------------------------------
// filter::apply (criteria 1, 2, 3, 6)
// ---------------------------------------------------------------------------

#[test]
fn no_filters_keep_every_check() {
    assert_eq!(filtered(&[], &[]).len(), 5);
}

#[test]
fn only_keeps_listed_checks_in_config_order() {
    assert_eq!(
        filtered(&["phpstan", "php-lint"], &[]),
        pairs(&[("fast", "php-lint"), ("analysis", "phpstan")])
    );
}

#[test]
fn group_keeps_checks_of_listed_groups() {
    assert_eq!(
        filtered(&[], &["tests", "fast"]),
        pairs(&[
            ("fast", "php-lint"),
            ("fast", "yaml-lint"),
            ("tests", "phpunit")
        ])
    );
}

#[test]
fn only_and_group_intersect() {
    assert_eq!(
        filtered(&["php-lint", "phpstan"], &["fast"]),
        pairs(&[("fast", "php-lint")])
    );
}

#[test]
fn empty_intersection_is_an_error() {
    let err = filter_err(&["phpstan"], &["fast"]);
    assert!(err.contains("no checks"), "{err}");
}

#[test]
fn unknown_check_errors_listing_valid_ids() {
    let err = filter_err(&["php-lint", "nope"], &[]);
    assert!(err.contains("`nope`"), "{err}");
    assert!(
        err.contains("cache-warmup, php-lint, yaml-lint, phpstan, phpunit"),
        "{err}"
    );
}

#[test]
fn unknown_group_errors_listing_valid_groups() {
    let err = filter_err(&[], &["nope"]);
    assert!(err.contains("`nope`"), "{err}");
    assert!(err.contains("warmup, fast, analysis, tests"), "{err}");
}

#[test]
fn only_does_not_force_untriggered_checks() {
    // yaml-lint only runs for changed YAML files: a PHP change leaves it skipped
    let mut config = checks_test_config();
    filter::apply(&mut config, &ids(&["yaml-lint"]), &[]).unwrap();
    let changed = ChangedFiles {
        files: ids(&["src/A.php"]),
        base_ref: "main".to_string(),
    };
    let checks = determine_checks(&config, &changed, Path::new("."));
    assert_eq!(checks.len(), 1);
    assert_eq!(checks[0].id(), "yaml-lint");
    assert_ne!(checks[0].decision(), Decision::Run);
}

// ---------------------------------------------------------------------------
// Binary: --simple, --fix, --list and error exit (criteria 3, 4, 5)
// ---------------------------------------------------------------------------

// Edge case: the binary reads a YAML file from disk, so this config stays raw
// YAML (builder fixtures produce `CiConfig` only).
/// Local-mode config: each check / fix touches its own marker file
const MARKER_CONFIG: &str = r#"version: 2
runner: local
git:
  base_branch: main
  fallback_branch: HEAD~1
file_patterns: {}
checks:
  first:
    pre_commands:
      - name: Pre
        command: touch first_pre_ran
    checks:
      a:
        name: A
        command: touch a_ran
        fix_command: touch a_fixed
  second:
    checks:
      b:
        name: B
        command: touch b_ran
        fix_command: touch b_fixed
"#;

/// Run the binary in a tempdir holding [`MARKER_CONFIG`]; returns the dir and output.
fn run_binary(args: &[&str]) -> (tempfile::TempDir, std::process::Output) {
    let tmp = tempfile::TempDir::new().unwrap();
    std::fs::write(tmp.path().join("ci-tui.yaml"), MARKER_CONFIG).unwrap();
    let out = std::process::Command::new(env!("CARGO_BIN_EXE_ci-tui"))
        .current_dir(tmp.path())
        .args(["--files", "x.rs"])
        .args(args)
        .output()
        .unwrap();
    (tmp, out)
}

fn assert_success(out: &std::process::Output) {
    assert!(
        out.status.success(),
        "{}\n{}",
        String::from_utf8_lossy(&out.stdout),
        String::from_utf8_lossy(&out.stderr)
    );
}

#[test]
fn binary_simple_runs_only_selected_check() {
    let (tmp, out) = run_binary(&["--simple", "--only", "b"]);
    assert_success(&out);
    assert!(tmp.path().join("b_ran").exists(), "selected check not run");
    assert!(!tmp.path().join("a_ran").exists(), "filtered check ran");
    assert!(
        !tmp.path().join("first_pre_ran").exists(),
        "pre_command of filtered-out group ran"
    );
}

#[test]
fn binary_fix_runs_only_selected_group() {
    let (tmp, out) = run_binary(&["--fix", "--group", "first"]);
    assert_success(&out);
    assert!(tmp.path().join("a_fixed").exists(), "selected fix not run");
    assert!(!tmp.path().join("b_fixed").exists(), "filtered fix ran");
}

#[test]
fn binary_list_shows_only_selected_checks() {
    let (_tmp, out) = run_binary(&["--list", "--only", "a,b", "--group", "second"]);
    assert_success(&out);
    let stdout = String::from_utf8_lossy(&out.stdout);
    assert!(stdout.contains("run        b  B"), "{stdout}");
    assert!(!stdout.contains(" a  A"), "{stdout}");
    assert!(stdout.contains("1 run, 0 on-demand, 0 skipped"), "{stdout}");
}

#[rstest::rstest]
#[case::only(&["--list", "--only", "a,nope"], "a, b")]
#[case::group(&["--simple", "--group", "nope"], "first, second")]
fn binary_unknown_id_exits_listing_valid_ids(#[case] args: &[&str], #[case] valid: &str) {
    let (tmp, out) = run_binary(args);
    let stderr = String::from_utf8_lossy(&out.stderr);
    assert_eq!(out.status.code(), Some(2), "{stderr}");
    assert!(stderr.contains("`nope`"), "{stderr}");
    assert!(stderr.contains(valid), "{stderr}");
    assert!(out.stdout.is_empty(), "nothing listed or run on error");
    assert!(!tmp.path().join("a_ran").exists());
}
