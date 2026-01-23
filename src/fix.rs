use crate::config::{CiConfig, DockerConfig};
use crate::git::ChangedFiles;
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
        let mut group_has_fixes = false;

        for (check_id, check) in &group_config.checks {
            // Skip checks without fix_command
            if check.fix_command.is_none() {
                continue;
            }

            // Determine which files this check applies to
            let matching_files = if let Some(triggers) = &check.triggers {
                // If check has file_pattern trigger, filter changed files by that pattern
                if let Some(pattern_key) = &triggers.file_pattern {
                    if let Some(pattern) = config.get_file_pattern(pattern_key) {
                        changed_files.filter_by_pattern(pattern)
                    } else {
                        vec![]
                    }
                } else {
                    // Has triggers but no file_pattern - use all changed files
                    changed_files.files.iter().map(|s| s.as_str()).collect()
                }
            } else {
                // No triggers - check applies to all changed files
                changed_files.files.iter().map(|s| s.as_str()).collect()
            };

            // If no files match and the fix_command uses {files} placeholder, skip
            let Some(fix_command) = check.fix_command.as_ref() else {
                // Should not reach here after is_none() check above, but handle gracefully
                continue;
            };
            if matching_files.is_empty() && fix_command.contains("{files}") {
                continue;
            }

            // Print group header on first fix in group
            if !group_has_fixes {
                println!("\x1b[1;36m── {} ──\x1b[0m", display_name.to_uppercase());
                group_has_fixes = true;
            }

            // Resolve the fix command with {files} placeholder
            let resolved_command = resolve_fix_command(fix_command, &matching_files);

            // Print what we're running
            println!(
                "  \x1b[33m●\x1b[0m {} \x1b[90m(running fix...)\x1b[0m",
                check_id
            );

            // Execute the fix command
            let result = run_fix_command(
                &resolved_command,
                check_id,
                &project_root,
                docker_config,
                check.container.as_deref(),
            )
            .await;

            fix_count += 1;
            match result {
                Ok(duration_ms) => {
                    pass_count += 1;
                    let duration = format_duration_ms(duration_ms);
                    println!(
                        "  \x1b[32m✓\x1b[0m {} \x1b[90m{}\x1b[0m",
                        check_id, duration
                    );
                }
                Err(e) => {
                    fail_count += 1;
                    has_failures = true;
                    println!("  \x1b[31m✗\x1b[0m {} \x1b[90mfailed\x1b[0m", check_id);
                    println!("  \x1b[31mError:\x1b[0m {}", e);
                }
            }
        }

        if group_has_fixes {
            println!();
        }
    }

    // Print summary
    let elapsed = start_time.elapsed();
    let elapsed_str = format_duration(elapsed);

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

/// Resolve {files} placeholder in fix command
fn resolve_fix_command(command: &str, files: &[&str]) -> String {
    let files_str = if files.is_empty() {
        String::new()
    } else {
        files.join(" ")
    };
    command.replace("{files}", &files_str).trim().to_string()
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

    // Check if container is running, use exec if yes, run if no
    let docker_cmd = if is_container_running(container_name) {
        // Container is running, use docker exec
        format!(
            "docker exec {} bash -c '{}'",
            container_name,
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
            " {} bash -c '{}'",
            image_name,
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
