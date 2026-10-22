//! Simple console output mode for CI pipelines (no TUI).
//!
//! This module provides a non-interactive execution mode that prints check
//! results directly to the console. It's designed for CI/CD pipelines where
//! a full terminal UI is not needed or available.
//!
//! # Usage
//!
//! Invoked via `--simple` CLI flag. Runs all checks sequentially or in parallel
//! (per group config) and prints results with colored output.
//! `--format json|junit` adds a report document ([`crate::report`]): on
//! stdout instead of the text, or in `--output <file>`; under GitHub Actions
//! the text output gets `::group::` / `::error` / `::warning` workflow
//! commands.
//!
//! # Exit Codes
//!
//! - `0`: All checks passed (failures of `allow_failure` checks are warnings)
//! - `1`: One or more checks without `allow_failure` failed

use crate::cache::ResultCache;
use crate::checks::{group_checks, severity_in, CheckToRun, Severity};
use crate::color::{cprint, cprintln};
use crate::config::CiConfig;
use crate::git::ChangedFiles;
use crate::report::{self, Report, ReportOptions, Summary};
use crate::runner::{CheckResult, CheckStatus, ExecTarget};
use crate::utils::time;
use anyhow::Result;
use std::fmt::Write;
use std::path::{Path, PathBuf};
use std::time::Instant;

/// Orchestrate check execution with the supplied executor (test-facing).
///
/// Groups checks by config-declared groups, runs parallel groups via
/// [`run_parallel`] and sequential groups via [`run_sequential`]. Returns the
/// full list of results in group order (within parallel groups, order is
/// non-deterministic).
pub async fn run_with_executor(
    config: CiConfig,
    checks: Vec<CheckToRun>,
    project_root: PathBuf,
    executor: std::sync::Arc<dyn crate::runner::CommandExecutor>,
) -> Result<Vec<CheckResult>> {
    // One run: each service's container resolved once
    let executor = crate::runner::CachingExecutor::wrap(executor);
    let target = &config.runner;
    let max_output_lines = config.max_output_lines;
    let grouped = group_checks(&checks);
    let mut all_results: Vec<CheckResult> = Vec::new();
    for (group_name, group_checks) in grouped {
        let group = config.get_group(group_name);
        let parallel = group.is_some_and(|g| g.parallel);
        let results = if parallel {
            run_parallel(
                group_checks,
                &project_root,
                target,
                executor.clone(),
                max_output_lines,
                config.group_parallel_limit(group),
            )
            .await
        } else {
            run_sequential(
                group_checks,
                &project_root,
                target,
                executor.as_ref(),
                max_output_lines,
            )
            .await
        };
        all_results.extend(results);
    }
    Ok(all_results)
}

/// Run checks in simple console mode (no TUI).
///
/// Prints results directly to stdout with colored output. Returns exit code
/// 1 if any check without `allow_failure` failed, else 0. Checks `cache`
/// finds unchanged since their last pass print as cached (counted as
/// passed) and do not run; finished runs are recorded in it.
///
/// `report` picks the outputs: a `--format json|junit` document (written
/// before exiting, to stdout instead of the text or to `--output`), and
/// GitHub Actions `::group::` / `::error` / `::warning` commands in the
/// text output.
///
/// # Errors
///
/// Returns an error if Docker operations fail or the report cannot be written.
pub async fn run(
    config: CiConfig,
    changed_files: ChangedFiles,
    checks: Vec<CheckToRun>,
    project_root: PathBuf,
    mut cache: ResultCache,
    report: ReportOptions,
) -> Result<i32> {
    let start_time = Instant::now();
    let target = &config.runner;
    let max_output_lines = config.max_output_lines;
    let (text, annotate) = (report.text_on_stdout(), report.annotate());

    if text {
        cprintln!(
            "\x1b[1mCI Checks\x1b[0m - {} files changed vs {}",
            changed_files.len(),
            changed_files.base_ref
        );
        cprintln!();
    }

    // One run: each service's container resolved once
    let executor = crate::runner::CachingExecutor::wrap(std::sync::Arc::new(
        crate::runner::RealCommandExecutor,
    ));
    let grouped = group_checks(&checks);
    let mut all_results: Vec<CheckResult> = Vec::new();

    for (group_name, group_checks) in grouped {
        let group = config.get_group(group_name);
        let display_name = group
            .map(|g| g.display_name(group_name))
            .unwrap_or(group_name)
            .to_uppercase();
        if annotate {
            println!("{}", report::github_group(&display_name));
        }
        if text {
            cprintln!("\x1b[1;36m── {} ──\x1b[0m", display_name);
        }

        let parallel = group.is_some_and(|g| g.parallel);

        // Unchanged since their last pass: listed first as cached, not run
        let (cached, group_checks) = cache.split_fresh(group_checks);
        let mut results: Vec<CheckResult> =
            cached.iter().map(|c| CheckResult::cached(c.id())).collect();
        results.extend(if parallel {
            run_parallel(
                group_checks,
                &project_root,
                target,
                executor.clone(),
                max_output_lines,
                config.group_parallel_limit(group),
            )
            .await
        } else {
            run_sequential(
                group_checks,
                &project_root,
                target,
                executor.as_ref(),
                max_output_lines,
            )
            .await
        });

        if text {
            results
                .iter()
                .for_each(|r| print_result(r, severity_in(&checks, r)));
        }
        for result in results {
            cache
                .record_for(&checks, &result)
                .unwrap_or_else(|e| eprintln!("Warning: result cache not saved: {e}"));
            all_results.push(result);
        }
        if text {
            cprintln!();
        }
        if annotate {
            println!("{}", report::GITHUB_ENDGROUP);
        }
    }

    let elapsed = start_time.elapsed();
    let run = Report {
        base_ref: &changed_files.base_ref,
        changed_files: changed_files.len(),
        duration_ms: u64::try_from(elapsed.as_millis()).unwrap_or(u64::MAX),
        checks: &checks,
        results: &all_results,
        summary: Summary::of(&checks, &all_results),
    };
    if text {
        print_summary(&run, elapsed, annotate);
    }
    // Failed checks decide the exit code over a failed report write
    match report.write(&run) {
        Err(e) if run.summary.failed == 0 => return Err(e),
        Err(e) => eprintln!("Error: {e:#}"),
        Ok(()) => {}
    }

    Ok(match run.summary.failed {
        0 => crate::exit::SUCCESS,
        _ => crate::exit::CHECKS_FAILED,
    })
}

/// Print [`format_summary`] (plain with color off, see [`crate::color`])
fn print_summary(run: &Report, elapsed: std::time::Duration, annotate: bool) {
    let elapsed = time::format_from_duration(elapsed);
    cprint!("{}", format_summary(run, &elapsed, annotate));
}

/// Text summary after all groups: pass count, or failure / warning counts
/// plus each failed check's output and fix command (and its `::error` /
/// `::warning` annotation with `annotate`), allowed failures in their own
/// block. Without checks, says there was nothing to run.
fn format_summary(run: &Report, elapsed: &str, annotate: bool) -> String {
    let summary = &run.summary;
    if run.checks.is_empty() {
        return "\x1b[33mNo checks to run for changed files.\x1b[0m\n".to_string();
    }
    let mut buf = String::from("\x1b[1m── Summary ──\x1b[0m\n");
    let (passed, total) = (summary.passed, summary.total);
    let (failed, warnings) = (summary.failed, summary.warnings);
    let (color, mark, counts) = match (failed, warnings) {
        (0, 0) => (32, "✓ ", format!("All {total} checks passed")),
        (0, _) => (33, "", format!("{passed}/{total} checks passed")),
        _ => (31, "✗ ", format!("{passed}/{total} checks passed")),
    };
    let tail = if failed + warnings > 0 {
        report::outcome_tail(failed, warnings)
    } else {
        String::new()
    };
    let _ = writeln!(buf, "\x1b[{color}m{mark}{counts}{tail} in {elapsed}\x1b[0m");

    push_failures(&mut buf, run, Severity::Error, annotate);
    push_failures(&mut buf, run, Severity::Warning, annotate);
    buf
}

/// Block listing the failed results of `severity` (`Failed checks:` /
/// `Allowed failures:`) with their output and fix command, each followed
/// by its workflow command with `annotate`; nothing without such results
fn push_failures(buf: &mut String, run: &Report, severity: Severity, annotate: bool) {
    let mut title = Some(match severity {
        Severity::Warning => "\x1b[1;33mAllowed failures:",
        _ => "\x1b[1;31mFailed checks:",
    });
    for result in run.results {
        let check = run.check(result);
        if Severity::of(result, check.is_some_and(CheckToRun::allow_failure)) != severity {
            continue;
        }
        if let Some(title) = title.take() {
            let _ = writeln!(buf, "\n{title}\x1b[0m");
        }
        let fix_cmd = check.and_then(|c| c.resolved_fix_command.as_deref());
        buf.push('\n');
        buf.push_str(&format_failed_check(result, fix_cmd, severity));
        if annotate {
            let _ = writeln!(buf, "{}", report::github_annotation(result, severity));
        }
    }
}

/// Print a check result to stdout with colored status indicator (plain
/// with color off, see [`crate::color`]).
///
/// See [`format_result`] for the per-status layout.
pub fn print_result(result: &CheckResult, severity: Severity) {
    cprintln!("{}", format_result(result, severity));
}

/// One colored status line for a check result:
/// - Passed: green checkmark with duration ("cached" instead when served
///   from the result cache)
/// - Failed: red X with duration
/// - TimedOut: magenta hourglass, "timed out" and duration
/// - Cancelled: gray crossed circle, "cancelled" and duration
/// - Running: yellow dot with "(running)"
/// - Pending: gray circle with "(pending)"
/// - Queued: blue dotted circle with "(queued)"
/// - Skipped: gray slashed circle with "(skipped)"
/// - OnDemand: cyan diamond with "(on-demand)"
///
/// A [`Severity::Warning`] failure or timeout is yellow and ends in
/// "allowed failure".
pub fn format_result(result: &CheckResult, severity: Severity) -> String {
    let id = &result.check_id;
    let duration = time::format(result.duration_ms);
    let (failed, timed_out, note) = match severity {
        Severity::Warning => (33, 33, " \x1b[33mallowed failure\x1b[0m"),
        _ => (31, 35, ""),
    };

    match result.status {
        CheckStatus::Passed if result.cached => {
            format!("  \x1b[32m✓\x1b[0m {id} \x1b[90mcached\x1b[0m")
        }
        CheckStatus::Passed => format!("  \x1b[32m✓\x1b[0m {id} \x1b[90m{duration}\x1b[0m"),
        CheckStatus::Failed => {
            format!("  \x1b[{failed}m✗\x1b[0m {id} \x1b[90m{duration}\x1b[0m{note}")
        }
        CheckStatus::TimedOut => format!(
            "  \x1b[{timed_out}m⧗\x1b[0m {id} \x1b[{timed_out}mtimed out\x1b[0m \x1b[90m{duration}\x1b[0m{note}"
        ),
        CheckStatus::Cancelled => {
            format!("  \x1b[90m⊗\x1b[0m {id} \x1b[90mcancelled {duration}\x1b[0m")
        }
        CheckStatus::Running => format!("  \x1b[33m●\x1b[0m {id} \x1b[90m(running)\x1b[0m"),
        CheckStatus::Pending => format!("  \x1b[90m○\x1b[0m {id} \x1b[90m(pending)\x1b[0m"),
        CheckStatus::Queued => format!("  \x1b[34m◌\x1b[0m {id} \x1b[90m(queued)\x1b[0m"),
        CheckStatus::Skipped => format!("  \x1b[90m⊘\x1b[0m {id} \x1b[90m(skipped)\x1b[0m"),
        CheckStatus::OnDemand => format!("  \x1b[36m◇\x1b[0m {id} \x1b[90m(on-demand)\x1b[0m"),
    }
}

/// Format a failed check's output for display.
///
/// Shows `result.output`/`error_output` as stored (already capped at
/// `max_output_lines` upstream), framed with a box header/footer (yellow
/// for a [`Severity::Warning`], else red). Optionally includes a fix
/// command hint.
pub fn format_failed_check(
    result: &CheckResult,
    fix_command: Option<&str>,
    severity: Severity,
) -> String {
    let mut buf = String::new();
    let frame = if severity == Severity::Warning {
        33
    } else {
        31
    };

    let _ = writeln!(buf, "\x1b[{frame}m┌─ {} ─┐\x1b[0m", result.check_id);

    if !result.output.is_empty() {
        for line in result.output.lines() {
            let _ = writeln!(buf, "  {}", line);
        }
    }

    if !result.error_output.is_empty() {
        for line in result.error_output.lines() {
            let _ = writeln!(buf, "  \x1b[31m{}\x1b[0m", line);
        }
    }

    if let Some(fix_cmd) = fix_command {
        let _ = writeln!(buf);
        let _ = writeln!(buf, "  \x1b[33m💡 Fix command:\x1b[0m");
        let _ = writeln!(buf, "  \x1b[36m{}\x1b[0m", fix_cmd);
    }

    let _ = writeln!(
        buf,
        "\x1b[{frame}m└{}┘\x1b[0m",
        "─".repeat(result.check_id.len() + 4)
    );

    buf
}

async fn run_sequential(
    checks: Vec<&CheckToRun>,
    project_root: &Path,
    target: &ExecTarget,
    executor: &dyn crate::runner::CommandExecutor,
    max_output_lines: usize,
) -> Vec<CheckResult> {
    let mut results = Vec::new();
    for check in checks {
        if check.is_on_demand() {
            continue;
        }
        let result =
            run_check_with_executor(check, project_root, target, executor, max_output_lines).await;
        results.push(result);
    }
    results
}

/// Run `checks` concurrently, at most `limit` at a time; results in input order.
async fn run_parallel(
    checks: Vec<&CheckToRun>,
    project_root: &Path,
    target: &ExecTarget,
    executor: std::sync::Arc<dyn crate::runner::CommandExecutor>,
    max_output_lines: usize,
    limit: usize,
) -> Vec<CheckResult> {
    let permits = std::sync::Arc::new(tokio::sync::Semaphore::new(limit));
    let mut handles = Vec::new();

    for check in checks {
        if check.is_on_demand() {
            continue;
        }
        let check = check.clone();
        let id = check.id().to_string();
        let project_root = project_root.to_path_buf();
        let target = target.clone();
        let executor = executor.clone();
        let permits = permits.clone();

        let handle = tokio::spawn(async move {
            // Held until the check ends
            let _permit = permits.acquire().await.expect("semaphore never closed");
            run_check_with_executor(
                &check,
                &project_root,
                &target,
                executor.as_ref(),
                max_output_lines,
            )
            .await
        });
        handles.push((id, handle));
    }

    let mut results = Vec::new();
    for (id, handle) in handles {
        results.push(handle.await.unwrap_or_else(|e| task_failed(&id, &e)));
    }
    results
}

/// Failed result for a check whose task panicked, so it is not dropped from
/// the summary, the report and the exit code
fn task_failed(id: &str, err: &tokio::task::JoinError) -> CheckResult {
    CheckResult {
        status: CheckStatus::Failed,
        error_output: format!("ci-tui: check task failed: {err}"),
        not_run: true,
        ..CheckResult::pending(id)
    }
}

/// Execute a single check via the injected executor (test-facing).
pub async fn run_check_with_executor(
    check: &CheckToRun,
    project_root: &Path,
    target: &ExecTarget,
    executor: &dyn crate::runner::CommandExecutor,
    max_output_lines: usize,
) -> CheckResult {
    crate::runner::run_single_check_with_executor(
        check,
        project_root,
        target,
        executor,
        max_output_lines,
    )
    .await
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::checks::CheckFiles;
    use crate::config::{CheckDefinition, LocalConfig, DEFAULT_MAX_OUTPUT_LINES};
    use crate::runner::{CommandOutput, MockCommandExecutor};
    use std::collections::HashMap;

    fn check(command: &str) -> CheckToRun {
        CheckToRun {
            id: "c".into(),
            group: "g".into(),
            definition: CheckDefinition {
                name: "C".into(),
                command: command.into(),
                service: None,
                fix_command: None,
                triggers: None,
                on_demand: false,
                allow_failure: false,
                env: HashMap::new(),
                timeout: None,
                error_pattern: None,
            },
            files: CheckFiles::Files(vec![]),
            resolved_command: command.into(),
            resolved_fix_command: None,
            cache_key: None,
        }
    }

    fn failed_result() -> CheckResult {
        let mut result = CheckResult::pending("c");
        result.status = CheckStatus::Failed;
        result.output = "\x1b[31merror\x1b[0m: boom\n".into();
        result.error_output = "warn\n".into();
        result
    }

    #[test]
    fn test_output_has_ansi_only_with_color() {
        let result = failed_result();
        let lines = [
            format_result(&result, Severity::Error),
            format_failed_check(&result, Some("cargo fmt"), Severity::Error),
        ];
        for line in &lines {
            assert!(crate::color::paint(line, true).contains("\x1b["));
            let plain = crate::color::paint(line, false);
            assert!(!plain.contains("\x1b["), "got: {plain:?}");
        }
        assert!(crate::color::paint(&lines[1], false).contains("error: boom"));
    }

    #[test]
    fn test_cached_result_line() {
        let line = format_result(&CheckResult::cached("c"), Severity::Ok);
        assert_eq!(crate::color::paint(&line, false), "  ✓ c cached");
    }

    #[rstest::rstest]
    #[case::failed(CheckStatus::Failed, "  ✗ c 1.2s allowed failure")]
    #[case::timed_out(CheckStatus::TimedOut, "  ⧗ c timed out 1.2s allowed failure")]
    fn test_allowed_failure_line_is_yellow(#[case] status: CheckStatus, #[case] plain: &str) {
        let result = CheckResult {
            status,
            duration_ms: 1234,
            ..CheckResult::pending("c")
        };
        let line = format_result(&result, Severity::Warning);
        assert_eq!(crate::color::paint(&line, false), plain);
        assert!(line.starts_with("  \x1b[33m"), "{line:?}");
    }

    #[rstest::rstest]
    #[case::warning(Severity::Warning, "\x1b[33m")]
    #[case::error(Severity::Error, "\x1b[31m")]
    fn test_failed_check_box_color(#[case] severity: Severity, #[case] color: &str) {
        let boxed = format_failed_check(&failed_result(), None, severity);
        assert!(boxed.starts_with(&format!("{color}┌─ c ─┐")), "{boxed:?}");
        assert!(boxed.contains(&format!("{color}└─────┘")), "{boxed:?}");
    }

    /// Simple-mode report of `results` for checks `ids` (`allowed` ones
    /// marked `allow_failure`)
    fn summary_of(ids: &[(&str, bool)], results: &[CheckResult], annotate: bool) -> String {
        let checks: Vec<CheckToRun> = ids
            .iter()
            .map(|(id, allowed)| {
                let mut c = check("true");
                c.id = (*id).into();
                c.definition.allow_failure = *allowed;
                c
            })
            .collect();
        let run = Report {
            base_ref: "main",
            changed_files: 1,
            duration_ms: 0,
            checks: &checks,
            results,
            summary: Summary::of(&checks, results),
        };
        format_summary(&run, "12s", annotate)
    }

    fn finished(id: &str, status: CheckStatus) -> CheckResult {
        CheckResult {
            status,
            output: format!("{id} out\n"),
            ..CheckResult::pending(id)
        }
    }

    #[test]
    fn test_summary_only_warnings_is_yellow_without_mark() {
        let results = [
            finished("ok", CheckStatus::Passed),
            finished("w", CheckStatus::Failed),
        ];
        let text = summary_of(&[("ok", false), ("w", true)], &results, true);
        assert!(
            text.contains("\x1b[33m1/2 checks passed, 1 warning in 12s\x1b[0m\n"),
            "{text:?}"
        );
        assert!(
            text.contains("\x1b[1;33mAllowed failures:\x1b[0m"),
            "{text:?}"
        );
        assert!(!text.contains("Failed checks:"), "{text:?}");
        assert!(text.contains("\x1b[33m┌─ w ─┐\x1b[0m"), "{text:?}");
        assert!(text.contains("w out"), "{text:?}");
        assert!(text.contains("::warning title=w::w failed"), "{text:?}");
        assert!(!text.contains("::error"), "{text:?}");
    }

    #[test]
    fn test_summary_mixed_failures_and_warnings() {
        let results = [
            finished("ok", CheckStatus::Passed),
            finished("bad", CheckStatus::Failed),
            finished("w1", CheckStatus::Failed),
            finished("w2", CheckStatus::TimedOut),
        ];
        let ids = [("ok", false), ("bad", false), ("w1", true), ("w2", true)];
        let text = summary_of(&ids, &results, true);
        assert!(
            text.contains("\x1b[31m✗ 1/4 checks passed, 1 failed, 2 warnings in 12s\x1b[0m"),
            "{text:?}"
        );
        let plain = crate::color::paint(&text, false);
        let failed = plain.find("Failed checks:").expect(&plain);
        let allowed = plain.find("Allowed failures:").expect(&plain);
        assert!(failed < plain.find("┌─ bad ─┐").unwrap(), "{plain}");
        assert!(plain.find("┌─ bad ─┐").unwrap() < allowed, "{plain}");
        assert!(allowed < plain.find("┌─ w1 ─┐").unwrap(), "{plain}");
        assert!(plain.contains("::error title=bad::"), "{plain}");
        assert!(
            plain.contains("::warning title=w2::w2 timed out"),
            "{plain}"
        );
    }

    #[test]
    fn test_summary_all_passed_unchanged() {
        let results = [finished("ok", CheckStatus::Passed)];
        let text = summary_of(&[("ok", true)], &results, false);
        assert!(text.contains("✓ All 1 checks passed in 12s"), "{text:?}");
    }

    #[tokio::test]
    async fn panicked_parallel_check_counts_as_failed() {
        let mut mock = MockCommandExecutor::new();
        mock.expect_execute()
            .returning(|_, _, _| panic!("executor blew up"));
        let target = ExecTarget::Local(LocalConfig::default());
        let results = run_parallel(
            vec![&check("ls")],
            Path::new("."),
            &target,
            std::sync::Arc::new(mock),
            DEFAULT_MAX_OUTPUT_LINES,
            1,
        )
        .await;
        assert_eq!(results.len(), 1);
        assert_eq!(results[0].check_id, "c");
        assert_eq!(results[0].status, CheckStatus::Failed);
        assert!(
            results[0].error_output.contains("check task failed"),
            "{}",
            results[0].error_output
        );
    }

    #[tokio::test]
    async fn panicked_allowed_check_is_a_real_failure() {
        let mut mock = MockCommandExecutor::new();
        mock.expect_execute()
            .returning(|_, _, _| panic!("executor blew up"));
        let mut allowed = check("ls");
        allowed.definition.allow_failure = true;
        let target = ExecTarget::Local(LocalConfig::default());
        let results = run_parallel(
            vec![&allowed],
            Path::new("."),
            &target,
            std::sync::Arc::new(mock),
            DEFAULT_MAX_OUTPUT_LINES,
            1,
        )
        .await;
        let summary = Summary::of(&[allowed], &results);
        assert_eq!((summary.failed, summary.warnings), (1, 0));
    }

    #[tokio::test]
    async fn check_command_includes_global_env() {
        let local = LocalConfig {
            env: HashMap::from([("APP_ENV".to_string(), "ci".to_string())]),
            ..LocalConfig::default()
        };
        let mut mock = MockCommandExecutor::new();
        mock.expect_running_container().times(0);
        mock.expect_execute()
            .withf(|cmd, _, _| cmd == "env APP_ENV='ci' bash -c 'ls'")
            .times(1)
            .returning(|_, _, _| CommandOutput {
                success: true,
                stdout: String::new(),
                stderr: String::new(),
            });
        let target = ExecTarget::Local(local);
        let res = run_check_with_executor(
            &check("ls"),
            Path::new("."),
            &target,
            &mock,
            DEFAULT_MAX_OUTPUT_LINES,
        )
        .await;
        assert_eq!(res.status, CheckStatus::Passed);
    }
}
