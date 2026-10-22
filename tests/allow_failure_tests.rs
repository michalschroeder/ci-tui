//! End-to-end per-check `allow_failure` (#151) through the binary: config
//! parsing, simple-mode and `--fix` exit codes, the JSON summary. Output
//! formatting is unit-tested in `simple` / `report`.

use ci_tui::exit;

mod common;
use common::configs::YAML_HEAD;
use common::{assert_code, run_config, stdout};

// Edge case: the binary reads a YAML file from disk, so configs stay raw YAML.
/// Local-mode config, group `g` (sequential): `ok` passes, `warn`
/// (`allow_failure: true`) runs `warn` and gets `extra` fields, then `more`
/// raw check entries
fn config(warn: &str, extra: &str, more: &str) -> String {
    format!(
        "{YAML_HEAD}runner: local
checks:
  g:
    checks:
      ok: {{ name: OK, command: 'true' }}
      warn:
        name: Warn
        command: {warn}
        allow_failure: true
{extra}{more}"
    )
}

/// [`config`] whose `warn` fails
fn warn_only() -> String {
    config("'false'", "", "")
}

/// [`warn_only`] plus a normal failing check `bad`
fn warn_and_bad() -> String {
    config(
        "'false'",
        "",
        "      bad: { name: Bad, command: 'false' }\n",
    )
}

/// Loads `config` from a file through the real loader
fn load(config: &str) -> anyhow::Result<ci_tui::config::CiConfig> {
    let tmp = tempfile::TempDir::new().unwrap();
    let path = tmp.path().join("ci-tui.yaml");
    std::fs::write(&path, config).unwrap();
    ci_tui::config::load_config(&path)
}

#[test]
fn allow_failure_parses_and_defaults_false() {
    let config = load(&warn_only()).expect("allow_failure parses");
    let checks = &config.get_group("g").unwrap().checks;
    assert!(checks["warn"].allow_failure);
    assert!(!checks["ok"].allow_failure, "default false");
}

#[test]
fn misspelled_allow_failure_is_rejected() {
    let config = warn_only().replace("allow_failure:", "allow_failures:");
    let err = load(&config).unwrap_err();
    assert!(format!("{err:#}").contains("unknown field"), "{err:#}");
}

#[test]
fn allowed_failure_only_exits_0() {
    let (_tmp, out) = run_config(&warn_only(), &["-s", "-f", "a.rs"]);
    assert_code(&out, exit::SUCCESS);
}

#[test]
fn allowed_and_normal_failure_exits_1() {
    let (_tmp, out) = run_config(&warn_and_bad(), &["-s", "-f", "a.rs"]);
    assert_code(&out, exit::CHECKS_FAILED);
}

#[test]
fn allowed_timeout_exits_0() {
    let config = config("sleep 5", "        timeout: 1s\n", "");
    let (_tmp, out) = run_config(&config, &["-s", "-f", "a.rs"]);
    assert_code(&out, exit::SUCCESS);
}

#[test]
fn json_summary_counts_allowed_failure_as_warning() {
    let (_tmp, out) = run_config(&warn_and_bad(), &["--format", "json", "-f", "a.rs"]);
    assert_code(&out, exit::CHECKS_FAILED);
    let doc: serde_json::Value = serde_json::from_str(&stdout(&out)).unwrap();
    assert_eq!(
        doc["summary"],
        serde_json::json!({"total": 3, "passed": 1, "failed": 1, "warnings": 1, "cached": 0})
    );
}

/// [`config`] whose `warn` fails and has fix command `fix`; `pre` adds a
/// failing group pre-command
fn fixable(fix: &str, pre: bool) -> String {
    let config = config("'false'", &format!("        fix_command: {fix}\n"), "");
    if pre {
        config.replace(
            "  g:\n",
            "  g:\n    pre_commands:\n      - { name: seed, command: 'false' }\n",
        )
    } else {
        config
    }
}

#[test]
fn fix_verify_of_allowed_check_failing_exits_0() {
    let (_tmp, out) = run_config(&fixable("'true'", false), &["--fix", "-f", "a.rs"]);
    assert_code(&out, exit::SUCCESS);
}

#[test]
fn failing_fix_of_allowed_check_exits_1() {
    let (_tmp, out) = run_config(&fixable("'false'", false), &["--fix", "-f", "a.rs"]);
    assert_code(&out, exit::CHECKS_FAILED);
}

#[test]
fn fix_verify_pre_command_failure_exits_1() {
    let (_tmp, out) = run_config(&fixable("'true'", true), &["--fix", "-f", "a.rs"]);
    assert_code(&out, exit::CHECKS_FAILED);
}
