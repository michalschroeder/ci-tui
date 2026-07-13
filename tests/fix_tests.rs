//! Tests for fix.rs - fix command resolution
//!
//! Covers resolve_fix_command placeholder substitution. Execution paths
//! (run_fix_command_with_executor, run_with_executor) are covered in
//! tests/fix_executor_tests.rs via MockCommandExecutor.

use ci_tui::fix::resolve_fix_command;
use pretty_assertions::assert_eq;
use rstest::rstest;

#[rstest]
#[case("cmd {files}", &[], "cmd")]
#[case("cmd {files}", &["a.rs"], "cmd a.rs")]
#[case("cmd {files}", &["a.rs", "b.rs"], "cmd a.rs b.rs")]
#[case("{files} cmd", &[], "cmd")]
#[case("{files} cmd", &["a.rs"], "a.rs cmd")]
#[case("cmd", &["a.rs", "b.rs"], "cmd")]
#[case("cargo fmt -- {files}", &[], "cargo fmt --")]
#[case("php-cs-fixer fix {files} --dry-run", &["src/App.php"], "php-cs-fixer fix src/App.php --dry-run")]
fn parameterized_placeholder_substitution(
    #[case] template: &str,
    #[case] files: &[&str],
    #[case] expected: &str,
) {
    let result = resolve_fix_command(template, files);
    assert_eq!(result, expected);
}

#[test]
fn files_with_spaces_preserved() {
    // Files with spaces in names are passed through as-is
    // (shell quoting is responsibility of the caller/executor)
    let result = resolve_fix_command("cmd {files}", &["path with spaces/file.rs"]);
    assert_eq!(result, "cmd path with spaces/file.rs");
}

#[test]
fn multiple_placeholders_all_replaced() {
    // If someone uses {files} multiple times, all instances are replaced
    let result = resolve_fix_command("echo {files} && process {files}", &["a.rs"]);
    assert_eq!(result, "echo a.rs && process a.rs");
}

#[test]
fn empty_command_with_placeholder_returns_files_only() {
    let result = resolve_fix_command("{files}", &["a.rs", "b.rs"]);
    assert_eq!(result, "a.rs b.rs");
}

#[test]
fn empty_command_without_files_returns_empty() {
    let result = resolve_fix_command("{files}", &[]);
    assert_eq!(result, "");
}
