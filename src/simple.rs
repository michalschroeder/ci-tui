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
//!
//! # Exit Codes
//!
//! - `0`: All checks passed
//! - `1`: One or more checks failed

use crate::checks::{group_checks, CheckToRun};
use crate::config::{CiConfig, DockerConfig};
use crate::git::ChangedFiles;
use crate::runner::{CheckResult, CheckStatus};
use crate::utils::{docker, time};
use anyhow::Result;
use std::path::{Path, PathBuf};
use std::time::Instant;
use tokio::process::Command;

/// Run checks in simple console mode (no TUI).
///
/// Prints results directly to stdout with colored output. Exits with code 1
/// if any check fails.
///
/// # Errors
///
/// Returns an error if Docker operations fail.
pub async fn run(
    config: CiConfig,
    changed_files: ChangedFiles,
    checks: Vec<CheckToRun>,
    project_root: PathBuf,
) -> Result<()> {
    let start_time = Instant::now();
    let docker_config = &config.docker;

    // Print header
    println!(
        "\x1b[1mCI Checks\x1b[0m - {} files changed vs {}",
        changed_files.len(),
        changed_files.base_ref
    );
    println!();

    if checks.is_empty() {
        println!("\x1b[33mNo checks to run for changed files.\x1b[0m");
        return Ok(());
    }

    let grouped = group_checks(&checks);
    let mut all_results: Vec<CheckResult> = Vec::new();
    let mut has_failures = false;

    for (group_name, group_checks) in grouped {
        let display_name = config
            .get_group(group_name)
            .map(|g| g.display_name(group_name))
            .unwrap_or(group_name);
        println!("\x1b[1;36m── {} ──\x1b[0m", display_name.to_uppercase());

        // Check if group runs in parallel
        let parallel = config
            .get_group(group_name)
            .map(|g| g.parallel)
            .unwrap_or(false);

        let results = if parallel {
            run_parallel(group_checks, &project_root, docker_config).await
        } else {
            run_sequential(group_checks, &project_root, docker_config).await
        };

        for result in results {
            print_result(&result);
            if result.status == CheckStatus::Failed {
                has_failures = true;
            }
            all_results.push(result);
        }
        println!();
    }

    // Print summary
    let elapsed = start_time.elapsed();
    let elapsed_str = time::format_from_duration(elapsed);
    let passed = all_results
        .iter()
        .filter(|r| r.status == CheckStatus::Passed)
        .count();
    let failed = all_results
        .iter()
        .filter(|r| r.status == CheckStatus::Failed)
        .count();

    println!("\x1b[1m── Summary ──\x1b[0m");
    if has_failures {
        println!(
            "\x1b[31m✗ {}/{} checks passed, {} failed in {}\x1b[0m",
            passed,
            all_results.len(),
            failed,
            elapsed_str
        );

        // Show failed checks with details
        println!();
        println!("\x1b[1;31mFailed checks:\x1b[0m");
        for result in &all_results {
            if result.status == CheckStatus::Failed {
                println!();
                println!("\x1b[31m┌─ {} ─┐\x1b[0m", result.check_id);

                // Show error output
                if !result.output.is_empty() {
                    // Limit output to avoid flooding console
                    let lines: Vec<&str> = result.output.lines().collect();
                    let show_lines = if lines.len() > 30 { 30 } else { lines.len() };
                    for line in lines.iter().take(show_lines) {
                        println!("  {}", line);
                    }
                    if lines.len() > 30 {
                        println!("  \x1b[90m... ({} more lines)\x1b[0m", lines.len() - 30);
                    }
                }

                if !result.error_output.is_empty() {
                    let lines: Vec<&str> = result.error_output.lines().collect();
                    let show_lines = if lines.len() > 10 { 10 } else { lines.len() };
                    for line in lines.iter().take(show_lines) {
                        println!("  \x1b[31m{}\x1b[0m", line);
                    }
                }

                // Show fix command if available
                if let Some(check) = checks.iter().find(|c| c.id() == result.check_id) {
                    if let Some(fix_cmd) = &check.resolved_fix_command {
                        println!();
                        println!("  \x1b[33m💡 Fix command:\x1b[0m");
                        println!("  \x1b[36m{}\x1b[0m", fix_cmd);
                    }
                }

                println!("\x1b[31m└{}┘\x1b[0m", "─".repeat(result.check_id.len() + 4));
            }
        }

        std::process::exit(1);
    } else {
        println!(
            "\x1b[32m✓ All {} checks passed in {}\x1b[0m",
            all_results.len(),
            elapsed_str
        );
    }

    Ok(())
}

fn print_result(result: &CheckResult) {
    let duration = time::format(result.duration_ms);

    match result.status {
        CheckStatus::Passed => {
            println!(
                "  \x1b[32m✓\x1b[0m {} \x1b[90m{}\x1b[0m",
                result.check_id, duration
            );
        }
        CheckStatus::Failed => {
            println!(
                "  \x1b[31m✗\x1b[0m {} \x1b[90m{}\x1b[0m",
                result.check_id, duration
            );
        }
        CheckStatus::Running => {
            println!(
                "  \x1b[33m●\x1b[0m {} \x1b[90m(running)\x1b[0m",
                result.check_id
            );
        }
        CheckStatus::Pending => {
            println!(
                "  \x1b[90m○\x1b[0m {} \x1b[90m(pending)\x1b[0m",
                result.check_id
            );
        }
        CheckStatus::Skipped => {
            println!(
                "  \x1b[90m⊘\x1b[0m {} \x1b[90m(skipped)\x1b[0m",
                result.check_id
            );
        }
        CheckStatus::OnDemand => {
            println!(
                "  \x1b[36m◇\x1b[0m {} \x1b[90m(on-demand)\x1b[0m",
                result.check_id
            );
        }
    }
}

async fn run_sequential(
    checks: Vec<&CheckToRun>,
    project_root: &Path,
    docker_config: &DockerConfig,
) -> Vec<CheckResult> {
    let mut results = Vec::new();
    for check in checks {
        // Skip on-demand checks in simple mode
        if check.on_demand {
            continue;
        }
        let result = run_check(check, project_root, docker_config).await;
        results.push(result);
    }
    results
}

async fn run_parallel(
    checks: Vec<&CheckToRun>,
    project_root: &Path,
    docker_config: &DockerConfig,
) -> Vec<CheckResult> {
    let mut handles = Vec::new();

    for check in checks {
        // Skip on-demand checks in simple mode
        if check.on_demand {
            continue;
        }
        let check_id = check.id().to_string();
        let check = check.clone();
        let project_root = project_root.to_path_buf();
        let docker_config = docker_config.clone();

        let handle =
            tokio::spawn(async move { run_check(&check, &project_root, &docker_config).await });
        handles.push((check_id, handle));
    }

    let mut results = Vec::new();
    for (_id, handle) in handles {
        if let Ok(result) = handle.await {
            results.push(result);
        }
    }
    results
}

async fn run_check(
    check: &CheckToRun,
    project_root: &Path,
    docker_config: &DockerConfig,
) -> CheckResult {
    let check_id = check.id().to_string();
    let start = Instant::now();

    // Use per-check container if specified, otherwise use default
    let default_container = docker_config.container_name();
    let container_name = check
        .definition
        .container
        .as_deref()
        .unwrap_or(&default_container);

    // Get shell from config (default: bash)
    let shell = docker_config.shell();

    // Check if container is running, use exec if yes, run if no
    let docker_cmd = if docker::is_running(container_name) {
        // Container is running, use docker exec
        format!(
            "docker exec {} {} -c '{}'",
            container_name,
            shell,
            check.resolved_command.replace('\'', "'\\''")
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
            check.resolved_command.replace('\'', "'\\''")
        ));
        cmd
    };

    let output = Command::new("sh")
        .arg("-c")
        .arg(&docker_cmd)
        .current_dir(project_root)
        .output()
        .await;

    let duration_ms = start.elapsed().as_millis() as u64;

    match output {
        Ok(output) => {
            let stdout = String::from_utf8_lossy(&output.stdout).to_string();
            let stderr = String::from_utf8_lossy(&output.stderr).to_string();
            let status = if output.status.success() {
                CheckStatus::Passed
            } else {
                CheckStatus::Failed
            };

            CheckResult {
                check_id,
                status,
                output: stdout,
                error_output: stderr,
                duration_ms,
                started_at: None,
                finished_at: None,
            }
        }
        Err(e) => CheckResult {
            check_id,
            status: CheckStatus::Failed,
            output: String::new(),
            error_output: format!("Failed to execute: {}", e),
            duration_ms,
            started_at: None,
            finished_at: None,
        },
    }
}
