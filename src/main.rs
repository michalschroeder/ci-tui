use anyhow::Result;
use ci_tui::{checks, config, fix, git, simple, ui};
use clap::Parser;
use std::io::IsTerminal;
use std::path::PathBuf;

/// CI TUI - Run CI checks for changed files
#[derive(Parser)]
#[command(name = "ci-tui", version, about, long_about = None)]
struct Cli {
    /// Path to ci-config.yaml
    #[arg(short, long)]
    config: PathBuf,

    /// Run in simple console mode (no TUI)
    #[arg(short, long)]
    simple: bool,

    /// Run only fix commands (skip checks)
    #[arg(long)]
    fix: bool,

    /// Run checks on specific files instead of git-detected changes
    #[arg(short, long, num_args = 1..)]
    files: Vec<PathBuf>,
}

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

    // Auto-detect TUI mode: use simple mode if stdout is not a terminal
    let simple_mode = cli.simple || !std::io::stdout().is_terminal();

    // Use current working directory as project root
    let project_root = std::env::current_dir()?;

    // Load configuration
    let config = config::load_config(&cli.config)?;

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
