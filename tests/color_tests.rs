//! End-to-end `--color auto|always|never` of the binary with piped stdout
//! (see `ci_tui::color`): `auto` (default) prints no ANSI escapes there.

use ci_tui::exit;

mod common;
use common::{assert_code, run_ci_tui_env};

// Edge case: the binary reads a YAML file from disk, so configs stay raw YAML.
/// Local-mode config with one passing always-run check; simple mode colors
/// its header and summary
const CONFIG: &str = "version: 2
runner: local
git:
  base_branch: main
  fallback_branch: HEAD~1
file_patterns: {}
checks:
  g:
    checks:
      c:
        name: C
        command: 'true'
";

/// Runs `--simple --files a.rs` plus `args` with `env` (stdout piped);
/// whether stdout holds an ESC byte
fn has_escape(args: &[&str], env: &[(&str, &str)]) -> bool {
    let tmp = tempfile::TempDir::new().unwrap();
    std::fs::write(tmp.path().join("ci-tui.yaml"), CONFIG).unwrap();
    let base = ["--simple", "--files", "a.rs"];
    let args: Vec<&str> = base.iter().chain(args).copied().collect();
    let out = run_ci_tui_env(tmp.path(), &args, env);
    assert_code(&out, exit::SUCCESS);
    out.stdout.contains(&0x1b)
}

#[test]
fn piped_default_has_no_escapes() {
    assert!(!has_escape(&[], &[]));
}

#[test]
fn piped_auto_has_no_escapes() {
    assert!(!has_escape(&["--color", "auto"], &[]));
}

#[test]
fn piped_always_has_escapes() {
    assert!(has_escape(&["--color", "always"], &[]));
}

#[test]
fn always_beats_no_color_env() {
    assert!(has_escape(&["--color=always"], &[("NO_COLOR", "1")]));
}

#[test]
fn never_and_no_color_have_no_escapes() {
    assert!(!has_escape(&["--color", "never"], &[]));
    assert!(!has_escape(&["--no-color"], &[]));
}

#[test]
fn color_with_no_color_is_usage_error() {
    let tmp = tempfile::TempDir::new().unwrap();
    let args = ["--no-color", "--color", "always", "--list"];
    let out = run_ci_tui_env(tmp.path(), &args, &[]);
    assert_code(&out, exit::CONFIG_ERROR);
}
