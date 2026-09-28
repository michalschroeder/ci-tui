//! End-to-end exit codes of the binary: 0 pass, 1 checks failed, 2 config
//! error, 3 git / environment error (see `ci_tui::exit`).

use ci_tui::exit;

mod common;
use common::{git, run_ci_tui};

// Edge case: the binary reads a YAML file from disk, so configs stay raw YAML.
/// Config head shared by the fixtures below
const HEAD: &str = "version: 2
git:
  base_branch: main
  fallback_branch: HEAD~1
file_patterns: {}
";

/// One always-run check `c` running `command`
fn check(command: &str) -> String {
    format!("checks:\n  g:\n    checks:\n      c:\n        name: C\n        command: {command}\n")
}

/// Local-mode config with one always-run check running `command`
fn local_config(command: &str) -> String {
    format!("{HEAD}runner: local\n{}", check(command))
}

/// Docker-mode config (used with `--list` only: no daemon needed)
fn docker_config() -> String {
    format!(
        "{HEAD}docker:\n  project_dir: .\n  service: app\n  shell: sh\n{}",
        check("'true'")
    )
}

/// Tempdir holding `ci-tui.yaml` = `config`; runs the binary with `args`
fn run_with(config: &str, args: &[&str]) -> (tempfile::TempDir, std::process::Output) {
    let tmp = tempfile::TempDir::new().unwrap();
    std::fs::write(tmp.path().join("ci-tui.yaml"), config).unwrap();
    let out = run_ci_tui(tmp.path(), args);
    (tmp, out)
}

fn assert_code(out: &std::process::Output, code: i32) {
    assert_eq!(
        out.status.code(),
        Some(code),
        "stdout: {}\nstderr: {}",
        String::from_utf8_lossy(&out.stdout),
        String::from_utf8_lossy(&out.stderr)
    );
}

#[test]
fn passing_checks_exit_0() {
    let (_tmp, out) = run_with(&local_config("'true'"), &["--simple", "--files", "a.rs"]);
    assert_code(&out, exit::SUCCESS);
}

#[test]
fn failing_check_exits_1() {
    let (_tmp, out) = run_with(&local_config("'false'"), &["--simple", "--files", "a.rs"]);
    assert_code(&out, exit::CHECKS_FAILED);
}

#[test]
fn unparsable_config_exits_2_with_message() {
    let (_tmp, out) = run_with("checks: [not, a, map", &["--list", "--files", "a.rs"]);
    assert_code(&out, exit::CONFIG_ERROR);
    let stderr = String::from_utf8_lossy(&out.stderr);
    assert!(stderr.contains("ci-tui.yaml"), "{stderr}");
}

#[test]
fn missing_config_file_exits_2() {
    let (_tmp, out) = run_with("", &["-c", "nope.yaml", "--list", "--files", "a.rs"]);
    assert_code(&out, exit::CONFIG_ERROR);
}

#[test]
fn validate_invalid_config_exits_2() {
    let (_tmp, out) = run_with("checks: [not, a, map", &["validate"]);
    assert_code(&out, exit::CONFIG_ERROR);
}

#[test]
fn validate_valid_config_exits_0() {
    let (_tmp, out) = run_with(&local_config("'true'"), &["validate"]);
    assert_code(&out, exit::SUCCESS);
}

#[test]
fn git_detection_outside_repo_exits_3() {
    let (_tmp, out) = run_with(&local_config("'true'"), &["--list"]);
    assert_code(&out, exit::ENV_ERROR);
    let stderr = String::from_utf8_lossy(&out.stderr);
    assert!(stderr.contains("No base ref could be resolved"), "{stderr}");
}

#[test]
fn unknown_base_ref_exits_3() {
    let tmp = tempfile::TempDir::new().unwrap();
    git(tmp.path(), &["init", "-q"]);
    std::fs::write(tmp.path().join("ci-tui.yaml"), local_config("'true'")).unwrap();
    let out = run_ci_tui(tmp.path(), &["--list", "--base", "no-such-ref"]);
    assert_code(&out, exit::ENV_ERROR);
}

#[test]
fn list_in_docker_mode_does_not_probe_docker() {
    // No daemon needed: --list executes nothing
    let (_tmp, out) = run_with(&docker_config(), &["--list", "--files", "a.rs"]);
    assert_code(&out, exit::SUCCESS);
}
