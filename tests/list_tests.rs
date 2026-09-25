//! Tests for `--list` / `--dry-run`: per-check decision + reason, rendered
//! report, and an end-to-end binary run proving nothing executes.

use ci_tui::config::CheckTriggers;
use ci_tui::git::{ChangedFiles, CLI_FILES_BASE_REF};
use ci_tui::list::{explain_checks, render, CheckExplanation, Decision};
use std::path::Path;

mod common;
use common::configs::{
    checks_test_config, rust_discovery_config, rust_project_config, CheckBuilder, ConfigBuilder,
};

fn changed(files: &[&str], base_ref: &str) -> ChangedFiles {
    ChangedFiles {
        files: files.iter().map(|s| s.to_string()).collect(),
        base_ref: base_ref.to_string(),
    }
}

fn find<'a>(explained: &'a [CheckExplanation], id: &str) -> &'a CheckExplanation {
    explained
        .iter()
        .find(|e| e.id == id)
        .unwrap_or_else(|| panic!("check `{id}` missing"))
}

// ---------------------------------------------------------------------------
// explain_checks: decisions (criterion 4) and reasons (criterion 5)
// ---------------------------------------------------------------------------

#[test]
fn every_check_listed_once_in_config_group_order() {
    let config = checks_test_config();
    let explained = explain_checks(&config, &changed(&["src/A.php"], "main"), Path::new("."));
    let ids: Vec<(&str, &str)> = explained
        .iter()
        .map(|e| (e.group.as_str(), e.id.as_str()))
        .collect();
    assert_eq!(
        ids,
        vec![
            ("warmup", "cache-warmup"),
            ("fast", "php-lint"),
            ("fast", "yaml-lint"),
            ("analysis", "phpstan"),
            ("tests", "phpunit"),
        ]
    );
}

#[test]
fn always_run_check_reason() {
    let config = checks_test_config();
    let explained = explain_checks(&config, &changed(&[], "main"), Path::new("."));
    let e = find(&explained, "cache-warmup");
    assert_eq!(e.decision, Some(Decision::Run));
    assert_eq!(e.reasons, vec!["no triggers: always runs"]);
}

#[test]
fn file_pattern_match_reason_lists_pattern_and_files() {
    let config = checks_test_config();
    let cf = changed(&["src/A.php", "README.txt", "src/B.php"], "main");
    let explained = explain_checks(&config, &cf, Path::new("."));
    let e = find(&explained, "php-lint");
    assert_eq!(e.decision, Some(Decision::Run));
    assert_eq!(
        e.reasons,
        vec!["file_pattern `php` matched: src/A.php, src/B.php"]
    );
}

#[test]
fn file_pattern_no_match_with_files_placeholder_is_skipped() {
    let config = checks_test_config();
    let explained = explain_checks(&config, &changed(&["src/A.php"], "main"), Path::new("."));
    let e = find(&explained, "yaml-lint");
    assert_eq!(e.decision, Some(Decision::Skipped));
    assert_eq!(
        e.reasons,
        vec!["file_pattern `yaml`: no changed file matched"]
    );
}

#[test]
fn file_pattern_no_match_without_placeholder_is_on_demand() {
    // `cargo clippy` has no `{files}` — UI offers a manual 't' trigger
    let config = rust_project_config();
    let explained = explain_checks(&config, &changed(&[], "main"), Path::new("."));
    let e = find(&explained, "clippy");
    assert_eq!(e.decision, Some(Decision::OnDemand));
    assert_eq!(
        e.reasons,
        vec![
            "file_pattern `rust`: no changed file matched",
            "manual trigger only ('t' in TUI)",
        ]
    );
}

#[test]
fn empty_triggers_block_is_listed_as_excluded() {
    // Edge case: `triggers: {}` — determine_checks drops it; --list shows it
    // as excluded, outside the run / on-demand / skipped counts
    let mut check = CheckBuilder::new("Never", "never").build();
    check.triggers = Some(CheckTriggers::default());
    let config = ConfigBuilder::new().with_check("g", "never", check).build();
    let cf = changed(&["a.rs"], "main");
    let explained = explain_checks(&config, &cf, Path::new("."));
    let e = find(&explained, "never");
    assert_eq!(e.decision, None);
    assert_eq!(
        e.reasons,
        vec!["empty triggers block (no file_pattern / test_discovery): never runs"]
    );
    let out = render(&config, &cf, None, &explained);
    assert!(out.contains("excluded   never  Never\n"), "{out}");
    assert!(
        out.contains("0 run, 0 on-demand, 0 skipped (nothing executed)"),
        "{out}"
    );
}

#[test]
fn test_discovery_found_tests_reason() {
    let tmp = tempfile::TempDir::new().unwrap();
    std::fs::create_dir_all(tmp.path().join("tests")).unwrap();
    std::fs::write(tmp.path().join("tests/foo_test.rs"), "").unwrap();
    let explained = explain_checks(
        &rust_discovery_config(),
        &changed(&["src/foo.rs"], "main"),
        tmp.path(),
    );
    let e = find(&explained, "unit");
    assert_eq!(e.decision, Some(Decision::Run));
    assert_eq!(
        e.reasons,
        vec!["test_discovery `rust_src` (src/foo.rs): discovered tests: tests/foo_test.rs"]
    );
}

#[test]
fn test_discovery_no_tests_reasons() {
    let tmp = tempfile::TempDir::new().unwrap();
    let explained = explain_checks(
        &rust_discovery_config(),
        &changed(&["src/foo.rs"], "main"),
        tmp.path(),
    );

    let unit = find(&explained, "unit");
    assert_eq!(unit.decision, Some(Decision::Skipped));
    assert_eq!(
        unit.reasons,
        vec!["test_discovery `rust_src` (src/foo.rs): no tests found; command needs {files}"]
    );

    let slow = find(&explained, "slow");
    assert_eq!(slow.decision, Some(Decision::OnDemand));
    assert_eq!(
        slow.reasons,
        vec![
            "test_discovery `rust_src` (src/foo.rs): no tests found; on_demand",
            "manual trigger only ('t' in TUI)",
        ]
    );

    let suite = find(&explained, "suite");
    assert_eq!(suite.decision, Some(Decision::Run));
    assert_eq!(
        suite.reasons,
        vec!["test_discovery `rust_src` (src/foo.rs): no tests found; runs full command (no {files})"]
    );
}

#[test]
fn test_discovery_no_tests_with_file_pattern_match_runs() {
    // file_pattern match decides the run; no-tests fallback (on_demand) doesn't apply
    let tmp = tempfile::TempDir::new().unwrap();
    let explained = explain_checks(
        &rust_discovery_config(),
        &changed(&["src/foo.rs"], "main"),
        tmp.path(),
    );
    let e = find(&explained, "mixed");
    assert_eq!(e.decision, Some(Decision::Run));
    assert_eq!(
        e.reasons,
        vec![
            "file_pattern `rust_src` matched: src/foo.rs",
            "test_discovery `rust_src` (src/foo.rs): no tests found",
        ]
    );
}

#[test]
fn test_discovery_no_sources_reason() {
    let tmp = tempfile::TempDir::new().unwrap();
    let explained = explain_checks(
        &rust_discovery_config(),
        &changed(&["README.txt"], "main"),
        tmp.path(),
    );
    let e = find(&explained, "unit");
    assert_eq!(e.decision, Some(Decision::Skipped));
    assert_eq!(
        e.reasons,
        vec!["test_discovery `rust_src`: no changed file matched"]
    );
}

// ---------------------------------------------------------------------------
// render: base ref + changed files header (criteria 2, 3)
// ---------------------------------------------------------------------------

fn render_for(cf: &ChangedFiles, base_override: Option<&str>) -> String {
    let config = checks_test_config(); // git.base_branch: development
    let explained = explain_checks(&config, cf, Path::new("."));
    render(&config, cf, base_override, &explained)
}

#[test]
fn render_lists_changed_files_and_checks_by_group() {
    let out = render_for(
        &changed(&["src/A.php", "x.yaml"], "origin/development"),
        None,
    );
    assert!(
        out.contains("Changed files (2):\n  src/A.php\n  x.yaml\n"),
        "{out}"
    );
    assert!(out.contains("Cache Warmup\n"), "{out}");
    assert!(out.contains("Fast Checks\n"), "{out}");
    assert!(
        out.contains("run        php-lint  PHP syntax check\n"),
        "{out}"
    );
    assert!(
        out.contains("  - file_pattern `php` matched: src/A.php\n"),
        "{out}"
    );
    assert!(out.contains("skipped    phpunit  PHPUnit\n"), "{out}");
    assert!(
        out.contains("4 run, 0 on-demand, 1 skipped (nothing executed)"),
        "{out}"
    );
}

#[test]
fn render_no_changed_files() {
    let out = render_for(&changed(&[], "origin/development"), None);
    assert!(out.contains("Changed files (0): none\n"), "{out}");
}

#[rstest::rstest]
#[case::config(
    "origin/development",
    None,
    "origin/development (git.base_branch `development`)"
)]
#[case::fallback(
    "HEAD~1",
    None,
    "HEAD~1 (fallback: git.base_branch `development` not found)"
)]
#[case::override_("v1.0", Some("v1.0"), "v1.0 (--base)")]
#[case::files_bypass(CLI_FILES_BASE_REF, None, "none (git bypassed: --files)")]
fn render_base_ref_line(
    #[case] base_ref: &str,
    #[case] base_override: Option<&str>,
    #[case] expected: &str,
) {
    let out = render_for(&changed(&[], base_ref), base_override);
    assert!(out.starts_with(&format!("Base ref: {expected}\n")), "{out}");
}

// ---------------------------------------------------------------------------
// Binary: nothing executes, exit 0 (criteria 1, 6, 7)
// ---------------------------------------------------------------------------

// Edge case: the binary reads a YAML file from disk, so this config stays raw
// YAML (builder fixtures produce `CiConfig` only).
/// Local-mode config whose pre_command and check would each create a marker file
const MARKER_CONFIG: &str = r#"version: 2
runner: local
git:
  base_branch: main
  fallback_branch: HEAD~1
file_patterns:
  rust:
    pattern: '\.rs$'
ignore_patterns:
  - '\.md$'
checks:
  build:
    pre_commands:
      - name: Pre
        command: touch pre_ran
    checks:
      marker:
        name: Marker
        command: touch check_ran {files}
        triggers:
          file_pattern: rust
"#;

#[rstest::rstest]
#[case::list("--list")]
#[case::dry_run("--dry-run")]
fn binary_list_executes_nothing(#[case] flag: &str) {
    let tmp = tempfile::TempDir::new().unwrap();
    std::fs::write(tmp.path().join("ci-tui.yaml"), MARKER_CONFIG).unwrap();
    let out = std::process::Command::new(env!("CARGO_BIN_EXE_ci-tui"))
        .current_dir(tmp.path())
        .args([flag, "--files", "src/a.rs", "notes.md"])
        .output()
        .unwrap();
    let stdout = String::from_utf8_lossy(&out.stdout);
    assert!(
        out.status.success(),
        "{stdout}\n{}",
        String::from_utf8_lossy(&out.stderr)
    );
    assert!(
        stdout.contains("Base ref: none (git bypassed: --files)"),
        "{stdout}"
    );
    // notes.md removed by ignore_patterns
    assert!(
        stdout.contains("Changed files (1):\n  src/a.rs\n"),
        "{stdout}"
    );
    assert!(stdout.contains("run        marker  Marker"), "{stdout}");
    assert!(!tmp.path().join("pre_ran").exists(), "pre_command executed");
    assert!(!tmp.path().join("check_ran").exists(), "check executed");
}
