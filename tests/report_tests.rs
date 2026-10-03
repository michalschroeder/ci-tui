//! End-to-end `--format json|junit` / `--output` reports and GitHub Actions
//! annotations of the binary in simple mode (see `ci_tui::report`).

use ci_tui::exit;

mod common;
use common::{assert_code, run_ci_tui_env, stdout};

// Edge case: the binary reads a YAML file from disk, so configs stay raw YAML.
/// Local-mode config: group `lint` with passing `ok`, group `tests` with
/// `bad` running `bad_command`. `ok` triggers on `.rs` files only.
fn config(bad_command: &str) -> String {
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
      ok:
        name: OK
        command: 'true'
        triggers:
          file_pattern: rust
  tests:
    name: Unit Tests
    checks:
      bad:
        name: Bad
        command: {bad_command}
        triggers:
          file_pattern: rust
"
    )
}

/// Config whose `bad` check prints `bad <x>` and fails
fn failing() -> String {
    config("echo 'bad <x>' && false")
}

/// Tempdir holding `ci-tui.yaml` = `config`; runs the binary with `args` and `env`
fn run(
    config: &str,
    args: &[&str],
    env: &[(&str, &str)],
) -> (tempfile::TempDir, std::process::Output) {
    let tmp = tempfile::TempDir::new().unwrap();
    std::fs::write(tmp.path().join("ci-tui.yaml"), config).unwrap();
    let out = run_ci_tui_env(tmp.path(), args, env);
    (tmp, out)
}

#[test]
fn json_on_stdout_is_only_the_report_and_exits_1() {
    let (_tmp, out) = run(&failing(), &["--format", "json", "-f", "a.rs"], &[]);
    assert_code(&out, exit::CHECKS_FAILED);
    let doc: serde_json::Value = serde_json::from_str(&stdout(&out)).unwrap();
    assert_eq!(doc["version"], 1);
    assert_eq!(doc["changed_files"], 1);
    assert_eq!(doc["summary"]["failed"], 1);
    assert_eq!(doc["checks"][0]["id"], "ok");
    assert_eq!(doc["checks"][0]["group"], "lint");
    assert_eq!(doc["checks"][1]["status"], "failed");
    assert_eq!(doc["checks"][1]["output"], "bad <x>\n");
}

#[test]
fn junit_output_file_keeps_text_on_stdout() {
    let args = ["--format", "junit", "--output", "r.xml", "-f", "a.rs"];
    let (tmp, out) = run(&failing(), &args, &[]);
    assert_code(&out, exit::CHECKS_FAILED);
    let doc = std::fs::read_to_string(tmp.path().join("r.xml")).unwrap();
    assert!(doc.starts_with("<?xml"), "{doc}");
    assert!(
        doc.contains("<testsuite name=\"tests\" tests=\"1\" failures=\"1\""),
        "{doc}"
    );
    assert!(
        doc.contains("<failure message=\"failed\">bad &lt;x&gt;"),
        "{doc}"
    );
    let text = stdout(&out);
    assert!(text.contains("── Summary ──"), "{text}");
    assert!(!text.contains("<?xml"), "{text}");
}

#[test]
fn passing_json_output_file_exits_0() {
    let args = ["--format", "json", "--output", "r.json", "-f", "a.rs"];
    let (tmp, out) = run(&config("'true'"), &args, &[]);
    assert_code(&out, exit::SUCCESS);
    let doc = std::fs::read_to_string(tmp.path().join("r.json")).unwrap();
    let doc: serde_json::Value = serde_json::from_str(&doc).unwrap();
    assert_eq!(doc["summary"]["passed"], 2);
}

#[test]
fn no_checks_run_still_writes_valid_docs() {
    // Nothing matches `.rs`: both checks wait on-demand, which simple mode never runs
    let (_tmp, out) = run(&failing(), &["--format", "json", "-f", "README.md"], &[]);
    assert_code(&out, exit::SUCCESS);
    let doc: serde_json::Value = serde_json::from_str(&stdout(&out)).unwrap();
    assert_eq!(doc["checks"], serde_json::json!([]));
    let (_tmp, out) = run(&failing(), &["--format", "junit", "-f", "README.md"], &[]);
    assert_code(&out, exit::SUCCESS);
    assert!(stdout(&out).trim_end().ends_with("</testsuites>"));
}

#[test]
fn unwritable_output_file_is_an_error() {
    let args = [
        "--format",
        "json",
        "--output",
        "no/such/dir/r.json",
        "-f",
        "a.rs",
    ];
    let (_tmp, out) = run(&config("'true'"), &args, &[]);
    assert_code(&out, exit::ENV_ERROR);
    let stderr = String::from_utf8_lossy(&out.stderr);
    assert!(stderr.contains("Failed to write report"), "{stderr}");
}

#[test]
fn format_with_list_is_a_usage_error() {
    let (_tmp, out) = run(&failing(), &["--format", "json", "--list"], &[]);
    assert_code(&out, exit::CONFIG_ERROR);
}

#[test]
fn github_actions_annotates_text_output() {
    let env = [("GITHUB_ACTIONS", "true")];
    let (_tmp, out) = run(&failing(), &["-s", "-f", "a.rs"], &env);
    assert_code(&out, exit::CHECKS_FAILED);
    let text = stdout(&out);
    assert!(text.contains("::group::LINT\n"), "{text}");
    assert!(text.contains("::group::UNIT TESTS\n"), "{text}");
    assert_eq!(text.matches("::endgroup::").count(), 2, "{text}");
    assert!(
        text.contains("::error title=bad::bad failed%0Abad <x>\n"),
        "{text}"
    );
}

#[test]
fn github_actions_does_not_annotate_report_on_stdout() {
    let env = [("GITHUB_ACTIONS", "true")];
    let (_tmp, out) = run(&failing(), &["--format", "json", "-f", "a.rs"], &env);
    assert_code(&out, exit::CHECKS_FAILED);
    let text = stdout(&out);
    assert!(!text.contains("::"), "{text}");
    serde_json::from_str::<serde_json::Value>(&text).unwrap();
}

#[test]
fn text_output_without_github_actions_has_no_annotations() {
    let (_tmp, out) = run(&failing(), &["-s", "-f", "a.rs"], &[]);
    assert_code(&out, exit::CHECKS_FAILED);
    let text = stdout(&out);
    assert!(
        !text.contains("::group::") && !text.contains("::error"),
        "{text}"
    );
    assert!(text.contains("── UNIT TESTS ──"), "{text}");
}
