//! Tests for `--list` / `--dry-run`: per-check decision + reason, rendered
//! report, and an end-to-end binary run proving nothing executes.

use ci_tui::config::{CheckTriggers, PathMappingRule, TestDiscoveryConfig, TestDiscoveryStrategy};
use ci_tui::git::{ChangedFiles, CLI_FILES_BASE_REF};
use ci_tui::list::{explain_checks, render, CheckExplanation, Decision};
use std::path::Path;

mod common;
use common::configs::{checks_test_config, CheckBuilder, ConfigBuilder};

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

/// Discovery config mapping `src/{path}.rs` -> `tests/{path}_test.rs`
fn rust_discovery() -> TestDiscoveryConfig {
    TestDiscoveryConfig {
        source_pattern: "rust_src".to_string(),
        strategies: vec![TestDiscoveryStrategy::PathMapping {
            rules: vec![PathMappingRule {
                source: "src/{path}.rs".to_string(),
                tests: vec!["tests/{path}_test.rs".to_string()],
            }],
        }],
    }
}

/// Config with test-discovery checks: on-demand, full-suite and `{files}` variants
fn discovery_config() -> ci_tui::config::CiConfig {
    ConfigBuilder::new()
        .with_file_pattern("rust_src", r"^src/.*\.rs$", None)
        .with_check(
            "tests",
            "unit",
            CheckBuilder::new("Unit", "cargo test {files}")
                .with_test_discovery(rust_discovery())
                .build(),
        )
        .with_check(
            "tests",
            "slow",
            CheckBuilder::new("Slow", "slow {files}")
                .with_test_discovery(rust_discovery())
                .on_demand()
                .build(),
        )
        .with_check(
            "tests",
            "suite",
            CheckBuilder::new("Suite", "cargo test")
                .with_test_discovery(rust_discovery())
                .build(),
        )
        .build()
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
    assert_eq!(e.decision, Decision::Run);
    assert_eq!(e.reasons, vec!["no triggers: always runs"]);
}

#[test]
fn file_pattern_match_reason_lists_pattern_and_files() {
    let config = checks_test_config();
    let cf = changed(&["src/A.php", "README.txt", "src/B.php"], "main");
    let explained = explain_checks(&config, &cf, Path::new("."));
    let e = find(&explained, "php-lint");
    assert_eq!(e.decision, Decision::Run);
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
    assert_eq!(e.decision, Decision::Skipped);
    assert_eq!(
        e.reasons,
        vec!["file_pattern `yaml`: no changed file matched"]
    );
}

#[test]
fn file_pattern_no_match_without_placeholder_is_on_demand() {
    // Edge case: `{files}`-less command — UI offers a manual 't' trigger
    let config = ConfigBuilder::new()
        .with_file_pattern("rust", r"\.rs$", None)
        .with_check(
            "lint",
            "clippy",
            CheckBuilder::new("Clippy", "cargo clippy")
                .with_file_pattern_trigger("rust")
                .build(),
        )
        .build();
    let explained = explain_checks(&config, &changed(&[], "main"), Path::new("."));
    let e = find(&explained, "clippy");
    assert_eq!(e.decision, Decision::OnDemand);
    assert_eq!(
        e.reasons,
        vec![
            "file_pattern `rust`: no changed file matched",
            "manual trigger only ('t' in TUI)",
        ]
    );
}

#[test]
fn empty_triggers_block_is_listed_as_skipped() {
    // Edge case: `triggers: {}` — determine_checks omits it, --list must not
    let mut check = CheckBuilder::new("Never", "never").build();
    check.triggers = Some(CheckTriggers::default());
    let config = ConfigBuilder::new().with_check("g", "never", check).build();
    let explained = explain_checks(&config, &changed(&["a.rs"], "main"), Path::new("."));
    let e = find(&explained, "never");
    assert_eq!(e.decision, Decision::Skipped);
    assert_eq!(
        e.reasons,
        vec!["empty triggers block (no file_pattern / test_discovery): never runs"]
    );
}

#[test]
fn test_discovery_found_tests_reason() {
    let tmp = tempfile::TempDir::new().unwrap();
    std::fs::create_dir_all(tmp.path().join("tests")).unwrap();
    std::fs::write(tmp.path().join("tests/foo_test.rs"), "").unwrap();
    let explained = explain_checks(
        &discovery_config(),
        &changed(&["src/foo.rs"], "main"),
        tmp.path(),
    );
    let e = find(&explained, "unit");
    assert_eq!(e.decision, Decision::Run);
    assert_eq!(
        e.reasons,
        vec!["test_discovery `rust_src` (src/foo.rs): discovered tests: tests/foo_test.rs"]
    );
}

#[test]
fn test_discovery_no_tests_reasons() {
    let tmp = tempfile::TempDir::new().unwrap();
    let explained = explain_checks(
        &discovery_config(),
        &changed(&["src/foo.rs"], "main"),
        tmp.path(),
    );

    let unit = find(&explained, "unit");
    assert_eq!(unit.decision, Decision::Skipped);
    assert_eq!(
        unit.reasons,
        vec!["test_discovery `rust_src` (src/foo.rs): no tests found; command needs {files}"]
    );

    let slow = find(&explained, "slow");
    assert_eq!(slow.decision, Decision::OnDemand);
    assert_eq!(
        slow.reasons,
        vec![
            "test_discovery `rust_src` (src/foo.rs): no tests found; on_demand",
            "manual trigger only ('t' in TUI)",
        ]
    );

    let suite = find(&explained, "suite");
    assert_eq!(suite.decision, Decision::Run);
    assert_eq!(
        suite.reasons,
        vec!["test_discovery `rust_src` (src/foo.rs): no tests found; runs full command (no {files})"]
    );
}

#[test]
fn test_discovery_no_sources_reason() {
    let tmp = tempfile::TempDir::new().unwrap();
    let explained = explain_checks(
        &discovery_config(),
        &changed(&["README.txt"], "main"),
        tmp.path(),
    );
    let e = find(&explained, "unit");
    assert_eq!(e.decision, Decision::Skipped);
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

#[test]
fn render_base_ref_resolved_from_config() {
    let out = render_for(&changed(&[], "origin/development"), None);
    assert!(
        out.starts_with("Base ref: origin/development (git.base_branch `development`)\n"),
        "{out}"
    );
}

#[test]
fn render_base_ref_fallback() {
    let out = render_for(&changed(&[], "HEAD~1"), None);
    assert!(
        out.starts_with("Base ref: HEAD~1 (fallback: git.base_branch `development` not found)\n"),
        "{out}"
    );
}

#[test]
fn render_base_ref_override() {
    let out = render_for(&changed(&[], "v1.0"), Some("v1.0"));
    assert!(out.starts_with("Base ref: v1.0 (--base)\n"), "{out}");
}

#[test]
fn render_files_bypass_git() {
    let out = render_for(&changed(&["a.php"], CLI_FILES_BASE_REF), None);
    assert!(
        out.starts_with("Base ref: none (git bypassed: --files)\n"),
        "{out}"
    );
}

// ---------------------------------------------------------------------------
// Binary: nothing executes, exit 0 (criteria 1, 6, 7)
// ---------------------------------------------------------------------------

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
