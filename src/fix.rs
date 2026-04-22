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
use crate::utils::{docker, time};
use anyhow::Result;
use std::path::{Path, PathBuf};
use std::time::Instant;
use tokio::process::Command;

/// Run fix commands for all checks with fix_command defined
pub async fn run(
    config: CiConfig,
    changed_files: ChangedFiles,
    project_root: PathBuf,
) -> Result<()> {
    let start_time = Instant::now();
    let docker_config = &config.docker;

    // Print header
    println!(
        "\x1b[1mFix Mode\x1b[0m - {} files changed vs {}",
        changed_files.len(),
        changed_files.base_ref
    );
    println!();

    let mut fix_count = 0;
    let mut pass_count = 0;
    let mut fail_count = 0;
    let mut has_failures = false;

    // Iterate through all groups and checks to find fix commands
    for (group_name, group_config) in config.groups() {
        let display_name = group_config.display_name(group_name);
        let (fc, pc, flc, hf) = run_group_fixes(
            &config,
            group_config,
            display_name,
            &changed_files,
            &project_root,
            docker_config,
        )
        .await;
        fix_count += fc;
        pass_count += pc;
        fail_count += flc;
        has_failures |= hf;
    }

    // Print summary
    let elapsed = start_time.elapsed();
    let elapsed_str = time::format_from_duration(elapsed);

    if fix_count == 0 {
        println!("\x1b[33mNo fix commands found in config.\x1b[0m");
        return Ok(());
    }

    println!("\x1b[1m── Summary ──\x1b[0m");
    if has_failures {
        println!(
            "\x1b[31m✗ {}/{} fixes passed, {} failed in {}\x1b[0m",
            pass_count, fix_count, fail_count, elapsed_str
        );
        std::process::exit(1);
    } else {
        println!(
            "\x1b[32m✓ All {} fixes passed in {}\x1b[0m",
            fix_count, elapsed_str
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
async fn run_group_fixes(
    config: &CiConfig,
    group_config: &crate::config::GroupConfig,
    display_name: &str,
    changed_files: &ChangedFiles,
    project_root: &Path,
    docker_config: &DockerConfig,
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

        let result = run_fix_command(
            &resolved_command,
            check_id,
            project_root,
            docker_config,
            check.container.as_deref(),
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
/// Replaces the `{files}` placeholder with a space-separated list of file paths.
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
    let files_str = if files.is_empty() {
        String::new()
    } else {
        files.join(" ")
    };
    command.replace("{files}", &files_str).trim().to_string()
}

/// Execute a fix command via docker
async fn run_fix_command(
    command: &str,
    check_id: &str,
    project_root: &Path,
    docker_config: &DockerConfig,
    check_container: Option<&str>,
) -> Result<u64> {
    let start = Instant::now();

    // Use per-check container if specified, otherwise use default
    let default_container = docker_config.container_name();
    let container_name = check_container.unwrap_or(&default_container);

    // Get shell from config (default: bash)
    let shell = docker_config.shell();

    // Check if container is running, use exec if yes, run if no
    let docker_cmd = if docker::is_running(container_name) {
        // Container is running, use docker exec
        format!(
            "docker exec {} {} -c '{}'",
            container_name,
            shell,
            command.replace('\'', "'\\''")
        )
    } else {
        // Container not running, use docker run with --rm
        let image_name = docker_config.image_name();
        let work_dir = docker_config.working_dir();
        let mut cmd = format!("docker run --rm -w {}", work_dir);
        if let Some(ref volume) = docker_config.volume_mount {
            cmd.push_str(&format!(" -v {}", volume));
        }
        cmd.push_str(&format!(
            " {} {} -c '{}'",
            image_name,
            shell,
            command.replace('\'', "'\\''")
        ));
        cmd
    };

    let output = Command::new("sh")
        .arg("-c")
        .arg(&docker_cmd)
        .current_dir(project_root)
        .output()
        .await?;

    let duration_ms = start.elapsed().as_millis() as u64;

    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        let stdout = String::from_utf8_lossy(&output.stdout);
        let error_msg = if !stderr.is_empty() {
            stderr.to_string()
        } else if !stdout.is_empty() {
            stdout.to_string()
        } else {
            format!(
                "Fix command '{}' failed with exit code {:?}",
                check_id,
                output.status.code()
            )
        };
        anyhow::bail!("{}", error_msg);
    }

    Ok(duration_ms)
}
