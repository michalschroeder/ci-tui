//! Tests for fix.rs - fix command resolution and execution
//!
//! This test file covers:
//! - resolve_fix_command: placeholder substitution for {files}
//!
//! Note: run_fix_command and run() are not tested directly as they require
//! Docker execution. These are validated via --fix mode in CI.

use ci_tui::fix::resolve_fix_command;
use pretty_assertions::assert_eq;
use rstest::rstest;

#[test]
fn empty_files_with_placeholder_returns_trimmed_command() {
    let result = resolve_fix_command("cargo fmt -- {files}", &[]);
    assert_eq!(result, "cargo fmt --");
}

#[test]
fn single_file_substitutes_correctly() {
    let result = resolve_fix_command("cargo fmt -- {files}", &["src/main.rs"]);
    assert_eq!(result, "cargo fmt -- src/main.rs");
}

#[test]
fn multiple_files_are_space_separated() {
    let result = resolve_fix_command(
        "cargo fmt -- {files}",
        &["src/main.rs", "src/lib.rs", "src/config.rs"],
    );
    assert_eq!(result, "cargo fmt -- src/main.rs src/lib.rs src/config.rs");
}

#[test]
fn command_without_placeholder_is_unchanged() {
    let result = resolve_fix_command("cargo fmt", &["src/main.rs"]);
    assert_eq!(result, "cargo fmt");
}

#[test]
fn leading_whitespace_is_trimmed_after_substitution() {
    let result = resolve_fix_command("{files} cargo fmt", &[]);
    assert_eq!(result, "cargo fmt");
}

#[test]
fn trailing_whitespace_is_trimmed_after_substitution() {
    let result = resolve_fix_command("cargo fmt {files}", &[]);
    assert_eq!(result, "cargo fmt");
}

#[test]
fn placeholder_in_middle_of_command() {
    let result = resolve_fix_command("php-cs-fixer fix {files} --dry-run", &["src/App.php"]);
    assert_eq!(result, "php-cs-fixer fix src/App.php --dry-run");
}

#[rstest]
#[case("cmd {files}", &[], "cmd")]
#[case("cmd {files}", &["a.rs"], "cmd a.rs")]
#[case("cmd {files}", &["a.rs", "b.rs"], "cmd a.rs b.rs")]
#[case("{files} cmd", &[], "cmd")]
#[case("{files} cmd", &["a.rs"], "a.rs cmd")]
#[case("cmd", &["a.rs", "b.rs"], "cmd")]
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
