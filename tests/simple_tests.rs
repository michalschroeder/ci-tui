//! Tests for simple.rs - simple console output mode
//!
//! This test file covers:
//! - print_result: output formatting for all CheckStatus variants
//!
//! Note: run(), run_sequential(), run_parallel(), and run_check() are not tested
//! directly as they require Docker execution. These are validated via --simple
//! mode in CI, which is comprehensive integration testing.

use ci_tui::runner::{CheckResult, CheckStatus};
use ci_tui::simple::{format_failed_check, print_result};
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
fn print_result_handles_all_status_variants(
    #[case] status: CheckStatus,
    #[case] check_id: &str,
    #[case] duration_ms: u64,
) {
    let result = make_result(check_id, status, duration_ms);
    // Should complete without panic
    print_result(&result);
}

#[test]
fn print_result_passed_with_duration() {
    let result = make_result("clippy", CheckStatus::Passed, 2500);
    print_result(&result);
    // Function completes without panic - output contains green checkmark and duration
}

#[test]
fn print_result_failed_with_duration() {
    let result = make_result("unit-tests", CheckStatus::Failed, 5200);
    print_result(&result);
    // Function completes without panic - output contains red X and duration
}

#[test]
fn print_result_running_shows_status() {
    let result = make_result("integration", CheckStatus::Running, 0);
    print_result(&result);
    // Function completes without panic - output contains yellow dot and "(running)"
}

#[test]
fn print_result_pending_shows_status() {
    let result = make_result("build", CheckStatus::Pending, 0);
    print_result(&result);
    // Function completes without panic - output contains gray circle and "(pending)"
}

#[test]
fn print_result_skipped_shows_status() {
    let result = make_result("lint", CheckStatus::Skipped, 0);
    print_result(&result);
    // Function completes without panic - output contains gray slashed circle and "(skipped)"
}

#[test]
fn print_result_on_demand_shows_status() {
    let result = make_result("expensive-test", CheckStatus::OnDemand, 0);
    print_result(&result);
    // Function completes without panic - output contains cyan diamond and "(on-demand)"
}

#[test]
fn print_result_with_long_check_id() {
    let result = make_result(
        "very-long-check-name-that-exceeds-typical-lengths",
        CheckStatus::Passed,
        1000,
    );
    print_result(&result);
    // Function completes without panic - handles long check IDs
}

#[test]
fn print_result_with_zero_duration() {
    let result = make_result("instant", CheckStatus::Passed, 0);
    print_result(&result);
    // Function completes without panic - handles zero duration
}

#[test]
fn print_result_with_large_duration() {
    let result = make_result("slow-check", CheckStatus::Failed, 999_999);
    print_result(&result);
    // Function completes without panic - handles large duration values
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
