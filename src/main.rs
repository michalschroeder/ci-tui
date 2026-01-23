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

    // Detect changed files and apply ignore patterns
    let mut changed_files = git::detect_changes(&project_root, &config.git)?;
    changed_files.apply_ignore_patterns(&config.ignore_patterns);

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
