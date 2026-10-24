//! Simple console output mode for CI pipelines (no TUI).
//!
//! This module provides a non-interactive execution mode that prints check
//! results directly to the console. It's designed for CI/CD pipelines where
//! a full terminal UI is not needed or available.
//!
//! # Usage
//!
//! Invoked via `--simple` CLI flag. Runs each group's `pre_commands` first
//! (through [`CheckRunner::run_group_setup`], `on_failure` as in the TUI;
//! none when every check of the group is on-demand or cached), then its
//! checks sequentially or in parallel (per group config), and prints results
//! with colored output. Checks a failed pre-command or an earlier
//! `stop_on_failure` group's failed check ([`RunStop`]) kept from running
//! fail as [`CheckResult::blocked`]; failed / skipped pre-commands print under
//! the group header (on stderr when a report document owns stdout).
//! `--format json|junit` adds a report document ([`crate::report`]): on
//! stdout instead of the text, or in `--output <file>`; under GitHub Actions
//! the text output gets `::group::` / `::error` / `::warning` workflow
//! commands.
//!
//! # Exit Codes
//!
//! - `0`: All checks passed (failures of `allow_failure` checks are warnings)
//! - `1`: One or more checks without `allow_failure` failed, or a failed
//!   pre-command kept a check from running

use crate::cache::ResultCache;
use crate::checks::{group_checks, severity_in, CheckToRun, Severity};
use crate::color::{ceprint, cprint, cprintln};
use crate::config::CiConfig;
use crate::git::ChangedFiles;
use crate::report::{self, format_pre_command_event, Report, ReportOptions, Summary};
use crate::runner::{
    CheckResult, CheckRunner, CheckStatus, CommandExecutor, ExecTarget, RunStop, RunnerEvent,
    SetupFailure,
};
use crate::utils::time;
use anyhow::Result;
use std::fmt::Write;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::Instant;

/// Orchestrate check execution with the supplied executor (test-facing).
///
/// Same group loop as [`run`] (pre-commands with their `on_failure`, then
/// parallel groups via [`run_parallel`], sequential ones via
/// [`run_sequential`]), without cache; prints only failed / skipped
/// pre-commands, on stderr. Returns the results in
/// group order, input order within a group (blocked checks included).
pub async fn run_with_executor(
    config: CiConfig,
    checks: Vec<CheckToRun>,
    project_root: PathBuf,
    executor: Arc<dyn CommandExecutor>,
) -> Result<Vec<CheckResult>> {
    let run = SimpleRun::new(config, &project_root, executor, Console::QUIET);
    Ok(run.run_groups(&checks, &mut ResultCache::disabled()).await)
}

/// Run checks in simple console mode (no TUI).
///
/// Prints results directly to stdout with colored output. Returns exit code
/// 1 if any check without `allow_failure` failed (or a failed pre-command
/// kept one from running), else 0. Checks `cache` finds unchanged since
/// their last pass print as cached (counted as passed) and do not run;
/// finished runs are recorded in it.
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
    let console = Console::of(&report);
    if console.text {
        cprintln!(
            "\x1b[1mCI Checks\x1b[0m - {} files changed vs {}",
            changed_files.len(),
            changed_files.base_ref
        );
        cprintln!();
    }

    let executor = Arc::new(crate::runner::RealCommandExecutor);
    let run = SimpleRun::new(config, &project_root, executor, console);
    let all_results = run.run_groups(&checks, &mut cache).await;

    let elapsed = start_time.elapsed();
    let run = Report {
        base_ref: &changed_files.base_ref,
        changed_files: changed_files.len(),
        duration_ms: u64::try_from(elapsed.as_millis()).unwrap_or(u64::MAX),
        checks: &checks,
        results: &all_results,
        summary: Summary::of(&checks, &all_results),
    };
    if console.text {
        print_summary(&run, elapsed, console.annotate);
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

/// What a run prints, and where
#[derive(Debug, Clone, Copy)]
struct Console {
    /// Group headers, check lines and failed / skipped pre-command lines on
    /// stdout; without, pre-command lines go to stderr (stdout carries the
    /// report document, or nothing in [`run_with_executor`])
    text: bool,
    /// GitHub Actions `::group::` commands around each group
    annotate: bool,
}

impl Console {
    /// Only pre-command lines, on stderr ([`run_with_executor`])
    const QUIET: Self = Self {
        text: false,
        annotate: false,
    };

    /// Console for a [`run`] with `report`
    fn of(report: &ReportOptions) -> Self {
        Self {
            text: report.text_on_stdout(),
            annotate: report.annotate(),
        }
    }

    /// Print a pre-command event's line ([`format_pre_command_event`])
    fn pre_command(&self, event: &RunnerEvent) {
        match format_pre_command_event(event) {
            Some(line) if self.text => cprint!("{line}"),
            Some(line) => ceprint!("{line}"),
            None => {}
        }
    }
}

/// One simple-mode run: group setup and checks through one runner, so each
/// service's container is resolved once
struct SimpleRun {
    runner: CheckRunner,
    console: Console,
}

impl SimpleRun {
    fn new(
        config: CiConfig,
        project_root: &Path,
        executor: Arc<dyn CommandExecutor>,
        console: Console,
    ) -> Self {
        Self {
            runner: CheckRunner::with_executor(config, project_root, executor),
            console,
        }
    }

    fn config(&self) -> &CiConfig {
        self.runner.config()
    }

    /// Run `checks` group by group (see [`Self::run_group`]), printing each
    /// group and recording its results in `cache`; all results in order
    async fn run_groups(&self, checks: &[CheckToRun], cache: &mut ResultCache) -> Vec<CheckResult> {
        let mut all_results = Vec::new();
        let mut stopped = None;
        for (group_name, group_checks) in group_checks(checks) {
            self.print_group_start(group_name);
            // Unchanged since their last pass: listed first as cached, not run
            let (cached, group_checks) = cache.split_fresh(group_checks);
            let mut results: Vec<CheckResult> =
                cached.iter().map(|c| CheckResult::cached(c.id())).collect();
            let ran = self.run_group(group_name, group_checks, &mut stopped).await;
            record(cache, checks, &ran);
            results.extend(ran);
            self.print_results(checks, &results);
            all_results.extend(results);
            self.print_group_end();
        }
        all_results
    }

    /// Run `group`'s `checks` (on-demand ones skipped): its pre-commands
    /// first when one is left, and only then the checks. A pre-command
    /// stopping the group, or an earlier group having `stopped` the run
    /// (then the group's pre-commands print skipped), blocks them
    /// ([`CheckResult::blocked`], with [`RunStop::reason`]). Sets `stopped`
    /// when this group stops the run: aborting pre-command, or a failed
    /// check of a `stop_on_failure` group after all its checks finished.
    async fn run_group(
        &self,
        group: &str,
        checks: Vec<&CheckToRun>,
        stopped: &mut Option<RunStop>,
    ) -> Vec<CheckResult> {
        let checks: Vec<&CheckToRun> = checks.into_iter().filter(|c| !c.is_on_demand()).collect();
        if checks.is_empty() {
            return Vec::new();
        }
        let blocked = |reason: &str| -> Vec<CheckResult> {
            let results = checks.iter().map(|c| CheckResult::blocked(c.id(), reason));
            results.collect()
        };
        if let Some(stop) = stopped {
            let reason = stop.reason();
            self.print_setup_skipped(group, &reason);
            return blocked(&reason);
        }
        if let Err(failure) = setup(&self.runner, self.console, group).await {
            let reason = failure.group_reason();
            *stopped = failure.aborts.then_some(RunStop::Aborted(failure));
            return blocked(&reason);
        }
        let results = self.run_checks(group, &checks).await;
        let mut stops = checks.iter().zip(&results);
        *stopped = stops.find_map(|(c, r)| RunStop::after(self.config(), c, r));
        results
    }

    /// Run `checks` of `group`, in parallel when it says so; results in
    /// input order
    async fn run_checks(&self, group: &str, checks: &[&CheckToRun]) -> Vec<CheckResult> {
        let config = self.config();
        let group = config.get_group(group);
        let (target, root) = (&config.runner, self.runner.project_root());
        let (executor, max_output_lines) = (self.runner.executor(), config.max_output_lines);
        if group.is_some_and(|g| g.parallel) {
            let limit = config.group_parallel_limit(group);
            run_parallel(
                checks,
                root,
                target,
                executor.clone(),
                max_output_lines,
                limit,
            )
            .await
        } else {
            run_sequential(checks, root, target, executor.as_ref(), max_output_lines).await
        }
    }

    /// Print `group`'s pre-commands skipped for `reason`
    fn print_setup_skipped(&self, group: &str, reason: &str) {
        for pre_command in self.config().pre_commands(group) {
            self.console.pre_command(&RunnerEvent::PreCommandSkipped {
                group: group.to_string(),
                name: pre_command.name.clone(),
                reason: reason.to_string(),
            });
        }
    }

    /// Group header (and `::group::` with annotations)
    fn print_group_start(&self, group_name: &str) {
        let group = self.config().get_group(group_name);
        let display_name = group
            .map(|g| g.display_name(group_name))
            .unwrap_or(group_name)
            .to_uppercase();
        if self.console.annotate {
            println!("{}", report::github_group(&display_name));
        }
        if self.console.text {
            cprintln!("\x1b[1;36m── {} ──\x1b[0m", display_name);
        }
    }

    /// Result lines of `results` (checks looked up in `checks`)
    fn print_results(&self, checks: &[CheckToRun], results: &[CheckResult]) {
        if self.console.text {
            results
                .iter()
                .for_each(|r| print_result(r, severity_in(checks, r)));
        }
    }

    /// Blank line after a group (and `::endgroup::` with annotations)
    fn print_group_end(&self) {
        if self.console.text {
            cprintln!();
        }
        if self.console.annotate {
            println!("{}", report::GITHUB_ENDGROUP);
        }
    }
}

/// Run `group`'s pre-commands via `runner`, printing their events to
/// `console` as they arrive; the failure when one stops the group
async fn setup(
    runner: &CheckRunner,
    console: Console,
    group: &str,
) -> std::result::Result<(), SetupFailure> {
    let (tx, mut rx) = tokio::sync::mpsc::channel(16);
    // `tx` dropped when setup ends, ending the printing
    let setup = async move { runner.run_group_setup(group, &tx).await };
    let print = async {
        while let Some(event) = rx.recv().await {
            console.pre_command(&event);
        }
    };
    tokio::join!(setup, print).0
}

/// Record `results` (of `checks`) in `cache` (blocked ones change
/// nothing); a write error only warns
fn record(cache: &mut ResultCache, checks: &[CheckToRun], results: &[CheckResult]) {
    for result in results {
        cache
            .record_for(checks, result)
            .unwrap_or_else(|e| eprintln!("Warning: result cache not saved: {e}"));
    }
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
/// - Failed: red X with duration ("not run" instead when it never ran:
///   a failed pre-command blocked it, its task panicked)
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
        CheckStatus::Failed if result.not_run => {
            format!("  \x1b[31m✗\x1b[0m {id} \x1b[31mnot run\x1b[0m")
        }
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
    checks: &[&CheckToRun],
    project_root: &Path,
    target: &ExecTarget,
    executor: &dyn CommandExecutor,
    max_output_lines: usize,
) -> Vec<CheckResult> {
    let mut results = Vec::new();
    for &check in checks {
        let result =
            run_check_with_executor(check, project_root, target, executor, max_output_lines).await;
        results.push(result);
    }
    results
}

/// Run `checks` concurrently, at most `limit` at a time; results in input order.
async fn run_parallel(
    checks: &[&CheckToRun],
    project_root: &Path,
    target: &ExecTarget,
    executor: Arc<dyn CommandExecutor>,
    max_output_lines: usize,
    limit: usize,
) -> Vec<CheckResult> {
    let permits = Arc::new(tokio::sync::Semaphore::new(limit));
    let mut handles = Vec::new();

    for &check in checks {
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
    executor: &dyn CommandExecutor,
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

    #[test]
    fn test_not_run_result_line_says_not_run() {
        let line = format_result(&CheckResult::blocked("c", "Not run: x"), Severity::Error);
        assert_eq!(crate::color::paint(&line, false), "  ✗ c not run");
        assert!(line.starts_with("  \x1b[31m✗"), "{line:?}");
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
            &[&check("ls")],
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
            &[&allowed],
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
