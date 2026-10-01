//! Fix mode for running auto-fix commands on changed files.
//!
//! This module provides functionality to run fix commands (like `cargo fmt`,
//! `php-cs-fixer`, etc.) on files that have changed. It iterates through all
//! checks with `fix_command` defined and executes them.
//!
//! # Usage
//!
//! Invoked via `--fix` CLI flag. Runs fix commands sequentially and reports
//! success/failure for each. A passing fix is verified by re-running its
//! check (unless `--no-verify`): only checks the run selected (not
//! on-demand / skipped), after the group's pre-commands (once per group).
//!
//! # Exit Codes
//!
//! - `0`: All fix commands (and verifications) passed
//! - `1`: One or more fix commands or verifications failed

use crate::checks::{keeps, CheckToRun, Decision, FilePatternEval};
use crate::color::{cprint, cprintln};
use crate::config::CiConfig;
use crate::git::ChangedFiles;
use crate::runner::{CheckResult, CheckRunner, CheckStatus, CommandExecutor, ExecTarget};
use crate::utils::time;
use anyhow::Result;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::Instant;

/// Summary of a fix run — consumed by `run` (prints + exits) or by tests.
#[derive(Debug, Clone, Default)]
pub struct FixSummary {
    pub fix_count: usize,
    pub pass_count: usize,
    pub fail_count: usize,
    /// Verification on (not `--no-verify`)
    pub verify: bool,
    /// Passed fixes whose check was re-run and passed / failed (or whose
    /// group setup failed)
    pub verify_pass_count: usize,
    pub verify_fail_count: usize,
    /// Passed fixes not verified: check not selected, or on-demand
    pub unverified_count: usize,
    /// A fix or a verification failed
    pub has_failures: bool,
    pub elapsed: std::time::Duration,
}

impl FixSummary {
    /// Passed fixes whose check was re-run (or whose group setup failed)
    pub fn verify_count(&self) -> usize {
        self.verify_pass_count + self.verify_fail_count
    }
}

/// Run fix commands for all checks using the supplied executor (test-facing).
///
/// `verify`: the checks the run selected ([`crate::checks::select_checks`]),
/// to verify passing fixes against; `None` skips verification.
pub async fn run_with_executor(
    config: CiConfig,
    changed_files: ChangedFiles,
    project_root: PathBuf,
    verify: Option<Vec<CheckToRun>>,
    executor: Arc<dyn CommandExecutor>,
) -> Result<FixSummary> {
    let start_time = Instant::now();
    let config = Arc::new(config);
    let runner = CheckRunner::with_executor(Arc::clone(&config), &project_root, executor.clone());
    let ctx = FixCtx {
        config: &config,
        changed_files: &changed_files,
        project_root: &project_root,
        executor: executor.as_ref(),
        verify: verify.as_deref(),
        runner: &runner,
    };
    let mut summary = FixSummary {
        verify: verify.is_some(),
        ..FixSummary::default()
    };

    for (group_name, group_config) in config.groups() {
        run_group_fixes(&ctx, group_name, group_config, &mut summary).await;
    }

    summary.elapsed = start_time.elapsed();
    Ok(summary)
}

/// Run fix commands for all checks with fix_command defined, verifying
/// passing ones against `verify` (see [`run_with_executor`])
pub async fn run(
    config: CiConfig,
    changed_files: ChangedFiles,
    project_root: PathBuf,
    verify: Option<Vec<CheckToRun>>,
) -> Result<()> {
    cprintln!(
        "\x1b[1mFix Mode\x1b[0m - {} files changed vs {}",
        changed_files.len(),
        changed_files.base_ref
    );
    cprintln!();

    let executor = Arc::new(crate::runner::RealCommandExecutor);
    let summary = run_with_executor(config, changed_files, project_root, verify, executor).await?;

    if summary.fix_count == 0 {
        cprintln!("\x1b[33mNo fix commands found in config.\x1b[0m");
        return Ok(());
    }

    cprintln!("\x1b[1m── Summary ──\x1b[0m");
    cprintln!("{}", format_summary(&summary));
    if summary.has_failures {
        std::process::exit(crate::exit::CHECKS_FAILED);
    }

    Ok(())
}

/// Colored summary line: fix counts, then verification counts when on
fn format_summary(summary: &FixSummary) -> String {
    let elapsed = time::format_from_duration(summary.elapsed);
    let mut verified = String::new();
    if summary.verify {
        verified = format!(
            ", {}/{} verified",
            summary.verify_pass_count,
            summary.verify_count()
        );
        if summary.unverified_count > 0 {
            verified += &format!(", {} not verified", summary.unverified_count);
        }
    }
    if summary.has_failures {
        format!(
            "\x1b[31m✗ {}/{} fixes passed, {} failed{verified} in {elapsed}\x1b[0m",
            summary.pass_count, summary.fix_count, summary.fail_count,
        )
    } else {
        format!(
            "\x1b[32m✓ All {} fixes passed{verified} in {elapsed}\x1b[0m",
            summary.fix_count
        )
    }
}

/// Resolve whether a check's fix command should run and return the resolved command
fn resolve_check_fix(
    config: &CiConfig,
    check: &crate::config::CheckDefinition,
    changed_files: &ChangedFiles,
) -> Option<String> {
    let fix_command = check.fix_command.as_ref()?;
    let matching_files = resolve_matching_files(config, check, changed_files);
    if matching_files.is_empty() && fix_command.contains("{files}") {
        return None;
    }
    Some(resolve_fix_command(fix_command, &matching_files))
}

/// Whether fix mode would run any fix command
pub fn has_fixes(config: &CiConfig, changed_files: &ChangedFiles) -> bool {
    config.groups().any(|(_, group)| {
        group
            .checks
            .values()
            .any(|check| resolve_check_fix(config, check, changed_files).is_some())
    })
}

/// Shared inputs of one fix run
struct FixCtx<'a> {
    config: &'a CiConfig,
    changed_files: &'a ChangedFiles,
    project_root: &'a Path,
    executor: &'a dyn CommandExecutor,
    /// Selected checks to verify against; `None` with `--no-verify`
    verify: Option<&'a [CheckToRun]>,
    /// Runs group pre-commands before the first verification
    runner: &'a CheckRunner,
}

/// Run fix commands for a single group (each passing one verified right
/// after), counting into `summary`
async fn run_group_fixes(
    ctx: &FixCtx<'_>,
    group_name: &str,
    group_config: &crate::config::GroupConfig,
    summary: &mut FixSummary,
) {
    let target: &ExecTarget = &ctx.config.runner;
    let mut group_has_fixes = false;
    // Group pre-commands outcome, once run (before the first verification)
    let mut setup: Option<bool> = None;

    for (check_id, check) in &group_config.checks {
        let Some(resolved_command) = resolve_check_fix(ctx.config, check, ctx.changed_files) else {
            continue;
        };

        if !group_has_fixes {
            let display_name = group_config.display_name(group_name);
            cprintln!("\x1b[1;36m── {} ──\x1b[0m", display_name.to_uppercase());
            group_has_fixes = true;
        }

        cprintln!(
            "  \x1b[33m●\x1b[0m {} \x1b[90m(running fix...)\x1b[0m",
            check_id
        );

        let result = run_fix_command_with_executor(
            &resolved_command,
            check_id,
            ctx.project_root,
            target,
            check.container.as_deref(),
            ctx.executor,
        )
        .await;

        summary.fix_count += 1;
        let success = print_fix_result(check_id, &result);
        summary.pass_count += usize::from(success);
        summary.fail_count += usize::from(!success);
        summary.has_failures |= !success;

        if let Some(checks) = ctx.verify.filter(|_| success) {
            let selected = checks
                .iter()
                .find(|c| c.group() == group_name && c.id() == check_id);
            verify_fix(ctx, check_id, selected, &mut setup, summary).await;
        }
    }

    if group_has_fixes {
        cprintln!();
    }
}

/// Re-run the `selected` check of a passing fix, after its group's
/// pre-commands (run once: `setup`), counting into `summary`. Not selected
/// or on-demand checks are only reported as not verified.
async fn verify_fix(
    ctx: &FixCtx<'_>,
    check_id: &str,
    selected: Option<&CheckToRun>,
    setup: &mut Option<bool>,
    summary: &mut FixSummary,
) {
    let check = match selected.map(|c| (c, c.decision())) {
        Some((check, Decision::Run)) => check,
        Some((_, Decision::OnDemand)) => return unverified(check_id, "on-demand", summary),
        _ => return unverified(check_id, "not selected", summary),
    };

    if setup.is_none() {
        *setup = Some(run_group_setup(ctx, check.group()).await);
    }
    let result = if *setup == Some(true) {
        cprintln!(
            "  \x1b[33m●\x1b[0m {} \x1b[90m(verifying...)\x1b[0m",
            check_id
        );
        crate::runner::run_single_check_with_executor(
            check,
            ctx.project_root,
            &ctx.config.runner,
            ctx.executor,
            ctx.config.max_output_lines,
        )
        .await
    } else {
        CheckResult::setup_failed(check_id, check.group())
    };

    let passed = result.status == CheckStatus::Passed;
    summary.verify_pass_count += usize::from(passed);
    summary.verify_fail_count += usize::from(!passed);
    summary.has_failures |= !passed;
    cprint!("{}", format_verify_result(&result));
}

/// Count and print a passing fix not verified, with the `reason`
fn unverified(check_id: &str, reason: &str, summary: &mut FixSummary) {
    cprintln!("{}", format_unverified(check_id, reason));
    summary.unverified_count += 1;
}

/// Run `group`'s pre-commands; false when one failed (its output printed)
async fn run_group_setup(ctx: &FixCtx<'_>, group: &str) -> bool {
    let (tx, mut rx) = tokio::sync::mpsc::channel(16);
    // `tx` dropped at the end of this block, ending `print`
    let setup = async move { ctx.runner.run_group_setup(group, &tx).await };
    let print = async {
        while let Some(event) = rx.recv().await {
            print_setup_failure(event);
        }
    };
    let (ok, ()) = tokio::join!(setup, print);
    ok
}

/// Print a failed pre-command's output; other setup events are ignored
fn print_setup_failure(event: crate::runner::RunnerEvent) {
    let crate::runner::RunnerEvent::PreCommandFinished {
        name,
        success: false,
        output,
        ..
    } = event
    else {
        return;
    };
    let output: String = output.lines().map(|l| format!("    {l}\n")).collect();
    cprint!("  \x1b[31m✗\x1b[0m pre-command {name} \x1b[90mfailed\x1b[0m\n{output}");
}

/// Line for a passing fix that is not verified, with the `reason`
fn format_unverified(check_id: &str, reason: &str) -> String {
    format!("  \x1b[90m○ {check_id} not verified ({reason})\x1b[0m")
}

/// Verification result: green "verified", or red "still failing" plus the
/// check's output
fn format_verify_result(result: &CheckResult) -> String {
    let (id, duration) = (&result.check_id, time::format(result.duration_ms));
    if result.status == CheckStatus::Passed {
        return format!("  \x1b[32m✓\x1b[0m {id} \x1b[90mverified {duration}\x1b[0m\n");
    }
    let failed = crate::simple::format_failed_check(result, None);
    format!(
        "  \x1b[31m✗\x1b[0m {id} \x1b[31mstill failing\x1b[0m \x1b[90m{duration}\x1b[0m\n{failed}"
    )
}

/// Determine which changed files a check applies to based on its triggers:
/// `file_pattern` matches (all changed files without one), minus the ones
/// `files_filter` drops
fn resolve_matching_files<'a>(
    config: &CiConfig,
    check: &'a crate::config::CheckDefinition,
    changed_files: &'a ChangedFiles,
) -> Vec<&'a str> {
    let all = || changed_files.files.iter().map(String::as_str);
    let Some(triggers) = &check.triggers else {
        return all().collect();
    };
    let filter = triggers.files_filter.as_ref();
    match triggers.file_pattern.as_deref() {
        Some(key) => FilePatternEval::evaluate(config, changed_files, key, filter).kept,
        None => all().filter(|f| keeps(filter, f)).collect(),
    }
}

/// Print the result of a fix command execution, returns true if successful
fn print_fix_result(check_id: &str, result: &Result<u64>) -> bool {
    cprintln!("{}", format_fix_result(check_id, result));
    result.is_ok()
}

/// Colored status line(s) for a fix result: green check and duration, or
/// red X plus the error
fn format_fix_result(check_id: &str, result: &Result<u64>) -> String {
    match result {
        Ok(duration_ms) => format!(
            "  \x1b[32m✓\x1b[0m {} \x1b[90m{}\x1b[0m",
            check_id,
            time::format(*duration_ms)
        ),
        Err(e) => format!(
            "  \x1b[31m✗\x1b[0m {} \x1b[90mfailed\x1b[0m\n  \x1b[31mError:\x1b[0m {}",
            check_id, e
        ),
    }
}

/// Resolve {files} placeholder in fix command
///
/// Replaces the `{files}` placeholder with a space-separated list of shell-quoted file paths.
/// If no files are provided and the command contains `{files}`, the placeholder
/// is removed and the result is trimmed.
///
/// # Examples
///
/// ```
/// use ci_tui::fix::resolve_fix_command;
///
/// let cmd = resolve_fix_command("cargo fmt -- {files}", &["src/main.rs", "src/lib.rs"]);
/// assert_eq!(cmd, "cargo fmt -- src/main.rs src/lib.rs");
/// ```
pub fn resolve_fix_command(command: &str, files: &[&str]) -> String {
    crate::utils::shell::expand_files(command, files)
}

/// Select the error message for a failed Docker command.
///
/// Precedence: non-empty stderr > non-empty stdout > formatted fallback with exit code.
fn select_fix_error_message(
    stderr: &str,
    stdout: &str,
    check_id: &str,
    exit_code: Option<i32>,
) -> String {
    if !stderr.is_empty() {
        stderr.to_string()
    } else if !stdout.is_empty() {
        stdout.to_string()
    } else {
        format!(
            "Fix command '{}' failed with exit code {:?}",
            check_id, exit_code
        )
    }
}

/// Execute a fix command via the injected executor (test-facing).
///
/// Returns `Ok(duration_ms)` on success, or `Err(anyhow::Error)` with a message
/// selected by [`select_fix_error_message`]. Exit code is `None` at this boundary —
/// `CommandOutput` only carries a boolean success flag.
pub async fn run_fix_command_with_executor(
    command: &str,
    check_id: &str,
    project_root: &Path,
    target: &ExecTarget,
    check_container: Option<&str>,
    executor: &dyn crate::runner::CommandExecutor,
) -> Result<u64> {
    let start = Instant::now();

    let full_cmd = target.build_command(check_container, target.env(), command, executor);
    let output = crate::runner::execute_built(
        executor,
        &full_cmd,
        project_root,
        &crate::runner::OutputSink::none(),
    )
    .await;
    let duration_ms = start.elapsed().as_millis() as u64;

    if !output.success {
        let error_msg = select_fix_error_message(&output.stderr, &output.stdout, check_id, None);
        anyhow::bail!("{}", error_msg);
    }

    Ok(duration_ms)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::{
        CheckDefinition, CheckTriggers, CiConfig, DockerConfig, FilePattern, GitConfig,
    };
    use crate::git::ChangedFiles;
    use indexmap::IndexMap;
    use std::collections::HashMap;

    #[test]
    fn test_fix_result_has_ansi_only_with_color() {
        for result in [Ok(1500), Err(anyhow::anyhow!("boom"))] {
            let line = format_fix_result("fmt", &result);
            assert!(crate::color::paint(&line, true).contains("\x1b["));
            let plain = crate::color::paint(&line, false);
            assert!(!plain.contains("\x1b["), "got: {plain:?}");
            assert!(plain.contains("fmt"));
        }
    }

    #[test]
    fn test_verify_result_lines() {
        let paint = |line: &str| crate::color::paint(line, false).into_owned();
        let passed = CheckResult {
            status: CheckStatus::Passed,
            ..CheckResult::pending("fmt")
        };
        assert!(paint(&format_verify_result(&passed)).starts_with("  ✓ fmt verified "));

        let failed = CheckResult {
            status: CheckStatus::Failed,
            output: "diff in src/main.rs".into(),
            ..CheckResult::pending("fmt")
        };
        let text = paint(&format_verify_result(&failed));
        assert!(text.starts_with("  ✗ fmt still failing "), "{text}");
        assert!(text.contains("diff in src/main.rs"), "output shown: {text}");

        let line = paint(&format_unverified("fmt", "on-demand"));
        assert_eq!(line, "  ○ fmt not verified (on-demand)");
    }

    #[rstest::rstest]
    #[case::no_verify(false, (0, 0, 0), false, "✓ All 2 fixes passed in 0s")]
    #[case::verified(true, (2, 0, 0), false, "✓ All 2 fixes passed, 2/2 verified in 0s")]
    #[case::unverified(true, (1, 0, 1), false, "✓ All 2 fixes passed, 1/1 verified, 1 not verified in 0s")]
    #[case::still_failing(true, (1, 1, 0), true, "✗ 2/2 fixes passed, 0 failed, 1/2 verified in 0s")]
    fn test_summary_line(
        #[case] verify: bool,
        #[case] (verify_pass_count, verify_fail_count, unverified_count): (usize, usize, usize),
        #[case] has_failures: bool,
        #[case] expected: &str,
    ) {
        let summary = FixSummary {
            fix_count: 2,
            pass_count: 2,
            verify,
            verify_pass_count,
            verify_fail_count,
            unverified_count,
            has_failures,
            ..FixSummary::default()
        };
        let line = format_summary(&summary);
        let line = crate::color::paint(&line, false);
        assert_eq!(line, expected);
    }

    fn cfg_with_pattern(key: &str, pattern: &str) -> CiConfig {
        let mut patterns = HashMap::new();
        patterns.insert(
            key.to_string(),
            FilePattern {
                pattern: pattern.to_string(),
                color: None,
            },
        );
        CiConfig::new(
            2,
            DockerConfig {
                project_dir: ".".into(),
                service: "app".into(),
                container: None,
                image: None,
                volume_mount: None,
                work_dir: None,
                shell: "bash".into(),
                env: HashMap::new(),
            },
            GitConfig {
                base_branch: "main".into(),
                fallback_branch: "HEAD~1".into(),
            },
            patterns,
            IndexMap::new(),
            Vec::new(),
        )
    }

    fn mk_check_with_triggers(triggers: Option<CheckTriggers>) -> CheckDefinition {
        CheckDefinition {
            name: "N".into(),
            command: "cmd".into(),
            service: None,
            container: None,
            fix_command: Some("fix {files}".into()),
            triggers,
            on_demand: false,
            env: HashMap::new(),
            timeout: None,
            error_pattern: None,
        }
    }

    fn changed(files: &[&str]) -> ChangedFiles {
        ChangedFiles {
            files: files.iter().map(|s| s.to_string()).collect(),
            base_ref: "main".into(),
        }
    }

    mod select_fix_error_message {
        use super::*;

        #[test]
        fn stderr_preferred_over_stdout() {
            let msg = select_fix_error_message("bad things", "also things", "fmt", Some(1));
            assert_eq!(msg, "bad things");
        }

        #[test]
        fn stdout_used_when_stderr_empty() {
            let msg = select_fix_error_message("", "stdout-only", "fmt", Some(1));
            assert_eq!(msg, "stdout-only");
        }

        #[test]
        fn fallback_when_both_streams_empty() {
            let msg = select_fix_error_message("", "", "clippy", Some(42));
            assert_eq!(msg, "Fix command 'clippy' failed with exit code Some(42)");
        }

        #[test]
        fn fallback_uses_none_when_exit_code_missing() {
            let msg = select_fix_error_message("", "", "test", None);
            assert_eq!(msg, "Fix command 'test' failed with exit code None");
        }
    }

    mod resolve_matching_files_tests {
        use super::*;

        #[test]
        fn no_triggers_returns_all_files() {
            let cfg = cfg_with_pattern("rust", r"\.rs$");
            let check = mk_check_with_triggers(None);
            let cf = changed(&["src/main.rs", "README.md"]);
            let files = resolve_matching_files(&cfg, &check, &cf);
            assert_eq!(files, vec!["src/main.rs", "README.md"]);
        }

        #[test]
        fn triggers_present_but_no_file_pattern_returns_all_files() {
            let cfg = cfg_with_pattern("rust", r"\.rs$");
            let check = mk_check_with_triggers(Some(CheckTriggers::default()));
            let cf = changed(&["src/main.rs", "README.md"]);
            let files = resolve_matching_files(&cfg, &check, &cf);
            assert_eq!(files, vec!["src/main.rs", "README.md"]);
        }

        #[test]
        fn file_pattern_filters_changed_files() {
            let cfg = cfg_with_pattern("rust", r"\.rs$");
            let check = mk_check_with_triggers(Some(CheckTriggers {
                file_pattern: Some("rust".into()),
                ..Default::default()
            }));
            let cf = changed(&["src/main.rs", "README.md", "lib/a.rs"]);
            let files = resolve_matching_files(&cfg, &check, &cf);
            assert_eq!(files, vec!["src/main.rs", "lib/a.rs"]);
        }

        #[test]
        fn unknown_file_pattern_key_returns_empty() {
            let cfg = cfg_with_pattern("rust", r"\.rs$");
            let check = mk_check_with_triggers(Some(CheckTriggers {
                file_pattern: Some("nonexistent".into()),
                ..Default::default()
            }));
            let cf = changed(&["src/main.rs"]);
            let files = resolve_matching_files(&cfg, &check, &cf);
            assert!(files.is_empty());
        }

        #[test]
        fn files_filter_drops_non_matching_files() {
            let cfg = cfg_with_pattern("rust", r"\.rs$");
            let filter = Some(regex::Regex::new(r"_test\.rs$").unwrap());
            let with_pattern = mk_check_with_triggers(Some(CheckTriggers {
                file_pattern: Some("rust".into()),
                files_filter: filter.clone(),
                ..Default::default()
            }));
            let without_pattern = mk_check_with_triggers(Some(CheckTriggers {
                files_filter: filter,
                ..Default::default()
            }));
            let cf = changed(&["tests/a_test.rs", "tests/fixtures/case.rs", "README.md"]);
            for check in [&with_pattern, &without_pattern] {
                let files = resolve_matching_files(&cfg, check, &cf);
                assert_eq!(files, vec!["tests/a_test.rs"]);
            }
        }
    }

    mod resolve_check_fix_tests {
        use super::*;

        #[test]
        fn returns_none_when_no_fix_command() {
            let cfg = cfg_with_pattern("rust", r"\.rs$");
            let mut check = mk_check_with_triggers(None);
            check.fix_command = None;
            let cf = changed(&["src/main.rs"]);
            assert!(resolve_check_fix(&cfg, &check, &cf).is_none());
        }

        #[test]
        fn returns_none_when_no_files_match_and_command_uses_files_placeholder() {
            let cfg = cfg_with_pattern("rust", r"\.rs$");
            let check = mk_check_with_triggers(Some(CheckTriggers {
                file_pattern: Some("rust".into()),
                ..Default::default()
            }));
            let cf = changed(&["README.md"]);
            assert!(resolve_check_fix(&cfg, &check, &cf).is_none());
        }

        #[test]
        fn returns_none_when_files_filter_drops_every_match() {
            let cfg = cfg_with_pattern("rust", r"\.rs$");
            let check = mk_check_with_triggers(Some(CheckTriggers {
                file_pattern: Some("rust".into()),
                files_filter: Some(regex::Regex::new(r"_test\.rs$").unwrap()),
                ..Default::default()
            }));
            let cf = changed(&["tests/fixtures/case.rs"]);
            assert!(resolve_check_fix(&cfg, &check, &cf).is_none());
        }

        #[test]
        fn returns_resolved_command_when_files_match() {
            let cfg = cfg_with_pattern("rust", r"\.rs$");
            let check = mk_check_with_triggers(Some(CheckTriggers {
                file_pattern: Some("rust".into()),
                ..Default::default()
            }));
            let cf = changed(&["src/main.rs", "README.md"]);
            let out = resolve_check_fix(&cfg, &check, &cf).unwrap();
            assert_eq!(out, "fix src/main.rs");
        }

        #[test]
        fn runs_when_command_has_no_files_placeholder_even_with_no_matches() {
            let cfg = cfg_with_pattern("rust", r"\.rs$");
            let mut check = mk_check_with_triggers(Some(CheckTriggers {
                file_pattern: Some("rust".into()),
                ..Default::default()
            }));
            check.fix_command = Some("fmt-all".into());
            let cf = changed(&["README.md"]);
            let out = resolve_check_fix(&cfg, &check, &cf).unwrap();
            assert_eq!(out, "fmt-all");
        }
    }

    mod env_tests {
        use super::*;
        use crate::runner::{CommandOutput, MockCommandExecutor};

        #[tokio::test]
        async fn fix_command_includes_global_env() {
            let mut docker = cfg_with_pattern("rust", r"\.rs$").docker().unwrap().clone();
            docker.env.insert("APP_ENV".into(), "ci".into());
            let target = ExecTarget::Docker(docker);
            let mut mock = MockCommandExecutor::new();
            mock.expect_is_container_running().returning(|_| true);
            mock.expect_execute()
                .withf(|cmd, _, _| cmd.contains("-e APP_ENV='ci'"))
                .times(1)
                .returning(|_, _, _| CommandOutput {
                    success: true,
                    stdout: String::new(),
                    stderr: String::new(),
                });
            let res =
                run_fix_command_with_executor("fmt", "fmt", Path::new("."), &target, None, &mock)
                    .await;
            assert!(res.is_ok());
        }
    }

    mod print_fix_result_tests {
        use super::*;

        #[test]
        fn returns_true_on_ok() {
            let ok: anyhow::Result<u64> = Ok(1234);
            assert!(print_fix_result("fmt", &ok));
        }

        #[test]
        fn returns_false_on_err() {
            let err: anyhow::Result<u64> = Err(anyhow::anyhow!("boom"));
            assert!(!print_fix_result("fmt", &err));
        }
    }
}
