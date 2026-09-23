use anyhow::Result;
use ci_tui::cli::{missing_config_error, resolve_config_path, Cli, Command};
use ci_tui::{checks, commands, config, fix, git, simple, ui};
use clap::Parser;
use std::io::IsTerminal;

fn git_detect_changes_or_exit(
    project_root: &std::path::Path,
    git_config: &ci_tui::config::GitConfig,
) -> git::ChangedFiles {
    match git::detect_changes(project_root, git_config) {
        Ok(c) => c,
        Err(e) => {
            eprintln!("Error: {e}");
            eprintln!(
                "\nNo base ref could be resolved against the current repository. \
                If this is intentional, bypass git with --files <paths...>."
            );
            std::process::exit(1);
        }
    }
}

#[tokio::main]
async fn main() -> Result<()> {
    // Install color-eyre for better panic handling (errors are ignored if it fails)
    let _ = color_eyre::install();

    let cli = Cli::parse();

    match cli.command {
        Some(Command::Init { path }) => {
            let path = resolve_config_path(path, cli.config);
            commands::init(&path)?;
            println!("Wrote {}", path.display());
            println!(
                "Edit the checks section, then run: ci-tui --config {}",
                path.display()
            );
            return Ok(());
        }
        Some(Command::Validate { path }) => {
            let path = resolve_config_path(path, cli.config);
            commands::validate(&path)?;
            println!("OK: {} is valid", path.display());
            return Ok(());
        }
        None => {}
    }

    // Enforced here, not via clap `required`: clap forbids required global args.
    let Some(config_path) = cli.config else {
        missing_config_error().exit();
    };

    // Auto-detect TUI mode: use simple mode if stdout is not a terminal
    let simple_mode = cli.simple || !std::io::stdout().is_terminal();

    // Use current working directory as project root
    let project_root = std::env::current_dir()?;

    // Load configuration
    let config = config::load_config(&config_path)?;

    // Get changed files: from --files arg or git detection
    let mut changed_files = if cli.files.is_empty() {
        git_detect_changes_or_exit(&project_root, &config.git)
    } else {
        git::ChangedFiles {
            files: cli
                .files
                .iter()
                .map(|p| p.to_string_lossy().into_owned())
                .collect(),
            base_ref: git::CLI_FILES_BASE_REF.to_string(),
        }
    };
    changed_files.apply_ignore_patterns(config.compiled_ignore_patterns());

    // Run fix mode if requested
    if cli.fix {
        return fix::run(config, changed_files, project_root).await;
    }

    // Determine which checks to run
    let checks_to_run = checks::determine_checks(&config, &changed_files, &project_root);

    if simple_mode {
        // Run in simple console mode
        simple::run(config, changed_files, checks_to_run, project_root).await
    } else {
        // Run the TUI
        ui::run(config, changed_files, checks_to_run, project_root).await
    }
}
