//! Tests for simple.rs - simple console output mode
//!
//! Covers print_result (smoke test over all CheckStatus variants) and
//! format_failed_check output content.
//!
//! run()/run_sequential()/run_parallel() orchestration is covered in
//! tests/simple_executor_tests.rs via run_with_executor + MockCommandExecutor.

use ci_tui::runner::{CheckResult, CheckStatus};
use ci_tui::simple::{format_failed_check, format_result, print_result};
use rstest::rstest;

/// Create a CheckResult with the given status for testing
fn make_result(check_id: &str, status: CheckStatus, duration_ms: u64) -> CheckResult {
    CheckResult {
        check_id: check_id.to_string(),
        status,
        output: String::new(),
        error_output: String::new(),
        duration_ms,
        started_at: None,
        finished_at: None,
    }
}

/// Create a failed CheckResult with stdout/stderr output
fn make_failed_result(check_id: &str, output: &str, error_output: &str) -> CheckResult {
    CheckResult {
        check_id: check_id.to_string(),
        status: CheckStatus::Failed,
        output: output.to_string(),
        error_output: error_output.to_string(),
        duration_ms: 1000,
        started_at: None,
        finished_at: None,
    }
}

// Tests verify that print_result handles all CheckStatus variants without panic.
// Since print_result uses println!, we verify it completes successfully rather
// than capturing output (which would require stdout redirection).

#[rstest]
#[case(CheckStatus::Passed, "passed-check", 1500)]
#[case(CheckStatus::Failed, "failed-check", 3200)]
#[case(CheckStatus::Running, "running-check", 0)]
#[case(CheckStatus::Pending, "pending-check", 0)]
#[case(CheckStatus::Skipped, "skipped-check", 0)]
#[case(CheckStatus::OnDemand, "on-demand-check", 0)]
#[case(CheckStatus::TimedOut, "timed-out-check", 1000)]
fn print_result_handles_all_status_variants(
    #[case] status: CheckStatus,
    #[case] check_id: &str,
    #[case] duration_ms: u64,
) {
    let result = make_result(check_id, status, duration_ms);
    // Should complete without panic
    print_result(&result);
}

// --- format_failed_check tests ---

#[test]
fn format_failed_check_shows_full_stdout() {
    let output = "Compiling ci-tui v0.1.0\nwarning: unused variable\nerror: aborting";
    let result = make_failed_result("clippy", output, "");

    let formatted = format_failed_check(&result, None);

    // All stdout lines must be present (no truncation)
    assert!(formatted.contains("Compiling ci-tui v0.1.0"));
    assert!(formatted.contains("warning: unused variable"));
    assert!(formatted.contains("error: aborting"));
}

#[test]
fn format_failed_check_shows_full_stderr() {
    let stderr = "error[E0433]: failed to resolve\nerror: could not compile";
    let result = make_failed_result("build", "", stderr);

    let formatted = format_failed_check(&result, None);

    assert!(formatted.contains("error[E0433]: failed to resolve"));
    assert!(formatted.contains("error: could not compile"));
}

#[test]
fn format_failed_check_shows_both_stdout_and_stderr() {
    let result = make_failed_result("test", "stdout line", "stderr line");

    let formatted = format_failed_check(&result, None);

    assert!(formatted.contains("stdout line"));
    assert!(formatted.contains("stderr line"));
}

#[test]
fn format_failed_check_no_truncation_for_long_output() {
    // Simulate cargo output: 50 lines of download noise + actual errors
    let mut lines: Vec<String> = Vec::new();
    for i in 0..50 {
        lines.push(format!("  Downloaded crate-{} v0.1.{}", i, i));
    }
    lines.push("warning: this function has too many lines (232/100)".to_string());
    lines.push("error: could not compile `ci-tui`".to_string());
    let output = lines.join("\n");

    let result = make_failed_result("clippy", &output, "");
    let formatted = format_failed_check(&result, None);

    // Both download noise AND actual errors must be present
    assert!(formatted.contains("Downloaded crate-0 v0.1.0"));
    assert!(formatted.contains("Downloaded crate-49 v0.1.49"));
    assert!(formatted.contains("warning: this function has too many lines (232/100)"));
    assert!(formatted.contains("error: could not compile `ci-tui`"));
    // No truncation markers
    assert!(!formatted.contains("... ("));
    assert!(!formatted.contains("more lines)"));
    assert!(!formatted.contains("lines omitted)"));
}

#[test]
fn format_failed_check_includes_box_frame() {
    let result = make_failed_result("clippy", "some output", "");

    let formatted = format_failed_check(&result, None);

    assert!(formatted.contains("┌─ clippy ─┐"));
    assert!(formatted.contains("└──────────┘"));
}

#[test]
fn format_failed_check_includes_fix_command() {
    let result = make_failed_result("fmt", "bad formatting", "");

    let formatted = format_failed_check(&result, Some("cargo fmt"));

    assert!(formatted.contains("Fix command:"));
    assert!(formatted.contains("cargo fmt"));
}

#[test]
fn format_failed_check_omits_fix_when_none() {
    let result = make_failed_result("clippy", "warning", "");

    let formatted = format_failed_check(&result, None);

    assert!(!formatted.contains("Fix command:"));
}

#[test]
fn format_failed_check_empty_output() {
    let result = make_failed_result("check", "", "");

    let formatted = format_failed_check(&result, None);

    // Should still have the box frame
    assert!(formatted.contains("┌─ check ─┐"));
    assert!(formatted.contains("└─────────┘"));
}

#[test]
fn format_result_timed_out_shows_label_and_duration() {
    let line = format_result(&make_result("hang", CheckStatus::TimedOut, 1000));
    assert!(line.contains("hang"), "got: {line}");
    assert!(line.contains("timed out"), "got: {line}");
    assert!(line.contains("1.0s"), "got: {line}");
}

#[test]
fn format_result_failed_is_not_timed_out() {
    let line = format_result(&make_result("x", CheckStatus::Failed, 1000));
    assert!(!line.contains("timed out"), "got: {line}");
}
