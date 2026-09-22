//! Fix mode for running auto-fix commands on changed files.
//!
//! This module provides functionality to run fix commands (like `cargo fmt`,
//! `php-cs-fixer`, etc.) on files that have changed. It iterates through all
//! checks with `fix_command` defined and executes them.
//!
//! # Usage
//!
//! Invoked via `--fix` CLI flag. Runs fix commands sequentially and reports
//! success/failure for each.
//!
//! # Exit Codes
//!
//! - `0`: All fix commands passed
//! - `1`: One or more fix commands failed

use crate::config::{CiConfig, DockerConfig};
use crate::git::ChangedFiles;
use crate::utils::time;
use anyhow::Result;
use std::path::{Path, PathBuf};
use std::time::Instant;

/// Summary of a fix run — consumed by `run` (prints + exits) or by tests.
#[derive(Debug, Clone)]
pub struct FixSummary {
    pub fix_count: usize,
    pub pass_count: usize,
    pub fail_count: usize,
    pub has_failures: bool,
    pub elapsed: std::time::Duration,
}

/// Run fix commands for all checks using the supplied executor (test-facing).
pub async fn run_with_executor(
    config: CiConfig,
    changed_files: ChangedFiles,
    project_root: PathBuf,
    executor: &dyn crate::runner::CommandExecutor,
) -> Result<FixSummary> {
    let start_time = Instant::now();
    let docker_config = &config.docker;

    let mut fix_count = 0;
    let mut pass_count = 0;
    let mut fail_count = 0;
    let mut has_failures = false;

    for (group_name, group_config) in config.groups() {
        let display_name = group_config.display_name(group_name);
        let (fc, pc, flc, hf) = run_group_fixes_with_executor(
            &config,
            group_config,
            display_name,
            &changed_files,
            &project_root,
            docker_config,
            executor,
        )
        .await;
        fix_count += fc;
        pass_count += pc;
        fail_count += flc;
        has_failures |= hf;
    }

    Ok(FixSummary {
        fix_count,
        pass_count,
        fail_count,
        has_failures,
        elapsed: start_time.elapsed(),
    })
}

/// Run fix commands for all checks with fix_command defined
pub async fn run(
    config: CiConfig,
    changed_files: ChangedFiles,
    project_root: PathBuf,
) -> Result<()> {
    println!(
        "\x1b[1mFix Mode\x1b[0m - {} files changed vs {}",
        changed_files.len(),
        changed_files.base_ref
    );
    println!();

    let summary = run_with_executor(
        config,
        changed_files,
        project_root,
        &crate::runner::RealCommandExecutor,
    )
    .await?;

    let elapsed_str = time::format_from_duration(summary.elapsed);
    if summary.fix_count == 0 {
        println!("\x1b[33mNo fix commands found in config.\x1b[0m");
        return Ok(());
    }

    println!("\x1b[1m── Summary ──\x1b[0m");
    if summary.has_failures {
        println!(
            "\x1b[31m✗ {}/{} fixes passed, {} failed in {}\x1b[0m",
            summary.pass_count, summary.fix_count, summary.fail_count, elapsed_str
        );
        std::process::exit(1);
    } else {
        println!(
            "\x1b[32m✓ All {} fixes passed in {}\x1b[0m",
            summary.fix_count, elapsed_str
        );
    }

    Ok(())
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

/// Run fix commands for a single group, returns (fix_count, pass_count, fail_count, has_failures)
async fn run_group_fixes_with_executor(
    config: &CiConfig,
    group_config: &crate::config::GroupConfig,
    display_name: &str,
    changed_files: &ChangedFiles,
    project_root: &Path,
    docker_config: &DockerConfig,
    executor: &dyn crate::runner::CommandExecutor,
) -> (usize, usize, usize, bool) {
    let mut fix_count = 0;
    let mut pass_count = 0;
    let mut fail_count = 0;
    let mut has_failures = false;
    let mut group_has_fixes = false;

    for (check_id, check) in &group_config.checks {
        let Some(resolved_command) = resolve_check_fix(config, check, changed_files) else {
            continue;
        };

        if !group_has_fixes {
            println!("\x1b[1;36m── {} ──\x1b[0m", display_name.to_uppercase());
            group_has_fixes = true;
        }

        println!(
            "  \x1b[33m●\x1b[0m {} \x1b[90m(running fix...)\x1b[0m",
            check_id
        );

        let result = run_fix_command_with_executor(
            &resolved_command,
            check_id,
            project_root,
            docker_config,
            check.container.as_deref(),
            executor,
        )
        .await;

        fix_count += 1;
        let success = print_fix_result(check_id, &result);
        pass_count += usize::from(success);
        fail_count += usize::from(!success);
        has_failures |= !success;
    }

    if group_has_fixes {
        println!();
    }

    (fix_count, pass_count, fail_count, has_failures)
}

/// Determine which changed files a check applies to based on its triggers
fn resolve_matching_files<'a>(
    config: &CiConfig,
    check: &crate::config::CheckDefinition,
    changed_files: &'a ChangedFiles,
) -> Vec<&'a str> {
    let Some(triggers) = &check.triggers else {
        return changed_files.files.iter().map(|s| s.as_str()).collect();
    };
    let Some(pattern_key) = &triggers.file_pattern else {
        return changed_files.files.iter().map(|s| s.as_str()).collect();
    };
    config
        .get_compiled_file_pattern(pattern_key)
        .map(|re| changed_files.filter_by_pattern(re))
        .unwrap_or_default()
}

/// Print the result of a fix command execution, returns true if successful
fn print_fix_result(check_id: &str, result: &Result<u64>) -> bool {
    match result {
        Ok(duration_ms) => {
            let duration = time::format(*duration_ms);
            println!(
                "  \x1b[32m✓\x1b[0m {} \x1b[90m{}\x1b[0m",
                check_id, duration
            );
            true
        }
        Err(e) => {
            println!("  \x1b[31m✗\x1b[0m {} \x1b[90mfailed\x1b[0m", check_id);
            println!("  \x1b[31mError:\x1b[0m {}", e);
            false
        }
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
    docker_config: &DockerConfig,
    check_container: Option<&str>,
    executor: &dyn crate::runner::CommandExecutor,
) -> Result<u64> {
    let start = Instant::now();

    let default_container = docker_config.container_name();
    let container_name = check_container.unwrap_or(&default_container);

    let env = std::collections::HashMap::new();
    let docker_cmd = if executor.is_container_running(container_name) {
        crate::runner::build_docker_exec_command(
            container_name,
            &env,
            command,
            &docker_config.shell,
        )
    } else {
        crate::runner::build_docker_run_command(docker_config, &env, command)
    };

    let output = executor.execute(&docker_cmd, project_root).await;
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
                test_discovery: None,
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
                test_discovery: None,
            }));
            let cf = changed(&["src/main.rs"]);
            let files = resolve_matching_files(&cfg, &check, &cf);
            assert!(files.is_empty());
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
                test_discovery: None,
            }));
            let cf = changed(&["README.md"]);
            assert!(resolve_check_fix(&cfg, &check, &cf).is_none());
        }

        #[test]
        fn returns_resolved_command_when_files_match() {
            let cfg = cfg_with_pattern("rust", r"\.rs$");
            let check = mk_check_with_triggers(Some(CheckTriggers {
                file_pattern: Some("rust".into()),
                test_discovery: None,
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
                test_discovery: None,
            }));
            check.fix_command = Some("fmt-all".into());
            let cf = changed(&["README.md"]);
            let out = resolve_check_fix(&cfg, &check, &cf).unwrap();
            assert_eq!(out, "fmt-all");
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
