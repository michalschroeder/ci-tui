use crate::checks::{group_checks, CheckToRun};
use crate::config::CiConfig;
use crate::git::ChangedFiles;
use crate::runner::{CheckResult, CheckStatus};
use anyhow::Result;
use std::path::{Path, PathBuf};
use std::time::Instant;
use tokio::process::Command;

pub async fn run(
    config: CiConfig,
    changed_files: ChangedFiles,
    checks: Vec<CheckToRun>,
    project_root: PathBuf,
) -> Result<()> {
    let start_time = Instant::now();
    let container_name = config.docker.container_name();

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
            run_parallel(group_checks, &project_root, &container_name).await
        } else {
            run_sequential(group_checks, &project_root, &container_name).await
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
    let elapsed_str = format_duration(elapsed);
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
    let duration = format_duration_ms(result.duration_ms);

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
    container_name: &str,
) -> Vec<CheckResult> {
    let mut results = Vec::new();
    for check in checks {
        // Skip on-demand checks in simple mode
        if check.on_demand {
            continue;
        }
        let result = run_check(check, project_root, container_name).await;
        results.push(result);
    }
    results
}

async fn run_parallel(
    checks: Vec<&CheckToRun>,
    project_root: &Path,
    container_name: &str,
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
        let container_name = container_name.to_string();

        let handle =
            tokio::spawn(async move { run_check(&check, &project_root, &container_name).await });
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

/// Check if a Docker container is currently running
fn is_container_running(container_name: &str) -> bool {
    let output = std::process::Command::new("docker")
        .args(["inspect", "-f", "{{.State.Running}}", container_name])
        .output();

    match output {
        Ok(output) => {
            let result = String::from_utf8_lossy(&output.stdout);
            result.trim() == "true"
        }
        Err(_) => false,
    }
}

async fn run_check(check: &CheckToRun, project_root: &Path, container_name: &str) -> CheckResult {
    let check_id = check.id().to_string();
    let start = Instant::now();

    // Check if container is running, use exec if yes, run if no
    let docker_cmd = if is_container_running(container_name) {
        // Container is running, use docker exec
        format!(
            "docker exec {} bash -c '{}'",
            container_name,
            check.resolved_command.replace('\'', "'\\''")
        )
    } else {
        // Container not running, use docker run with --rm
        // Strip the -1 suffix to get image name (e.g., "myproject-app-1" -> "myproject-app")
        let image_name = container_name.trim_end_matches("-1");
        format!(
            "docker run --rm -w /app {} bash -c '{}'",
            image_name,
            check.resolved_command.replace('\'', "'\\''")
        )
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

fn format_duration(duration: std::time::Duration) -> String {
    let secs = duration.as_secs();
    if secs < 60 {
        format!("{}s", secs)
    } else if secs < 3600 {
        let mins = secs / 60;
        let remaining_secs = secs % 60;
        format!("{}m {}s", mins, remaining_secs)
    } else {
        let hours = secs / 3600;
        let mins = (secs % 3600) / 60;
        format!("{}h {}m", hours, mins)
    }
}

fn format_duration_ms(ms: u64) -> String {
    if ms < 1000 {
        format!("{}ms", ms)
    } else if ms < 60_000 {
        format!("{:.1}s", ms as f64 / 1000.0)
    } else {
        let mins = ms / 60_000;
        let secs = (ms % 60_000) / 1000;
        format!("{}m {}s", mins, secs)
    }
}
