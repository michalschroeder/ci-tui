use anyhow::Result;
use ci_tui::{checks, commands, config, fix, git, simple, ui};
use clap::{CommandFactory, Parser, Subcommand};
use std::io::IsTerminal;
use std::path::PathBuf;

/// CI TUI - Run CI checks for changed files
#[derive(Parser)]
#[command(name = "ci-tui", version, about, long_about = None)]
struct Cli {
    #[command(subcommand)]
    command: Option<Command>,

    /// Path to config file (required unless a subcommand is given; create one with `ci-tui init`)
    #[arg(short, long, global = true)]
    config: Option<PathBuf>,

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

#[derive(Subcommand)]
enum Command {
    /// Generate a starter ci-tui.yaml
    Init {
        /// Output path [default: --config value, else ci-tui.yaml]
        path: Option<PathBuf>,
    },
    /// Validate a config file (exit non-zero if invalid)
    Validate {
        /// Config path [default: --config value, else ci-tui.yaml]
        path: Option<PathBuf>,
    },
}

/// Clap-styled usage error for a missing `--config` (exit code 2).
fn missing_config_error() -> clap::Error {
    Cli::command().error(
        clap::error::ErrorKind::MissingRequiredArgument,
        "--config <CONFIG> is required (or run `ci-tui init` to create one)",
    )
}

/// Resolve the config path for `init` / `validate`: positional arg, then
/// `--config`, then `ci-tui.yaml`.
fn resolve_config_path(path: Option<PathBuf>, config: Option<PathBuf>) -> PathBuf {
    path.or(config)
        .unwrap_or_else(|| PathBuf::from(commands::DEFAULT_CONFIG_FILE))
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_missing_config_error() {
        let cli = Cli::try_parse_from(["ci-tui"]).unwrap();
        assert!(cli.command.is_none() && cli.config.is_none());
        let err = missing_config_error();
        assert_eq!(err.kind(), clap::error::ErrorKind::MissingRequiredArgument);
        assert_eq!(err.exit_code(), 2);
    }

    /// Parse `args` and return the resolved `init` / `validate` config path.
    fn resolved(args: &[&str]) -> PathBuf {
        let cli = Cli::try_parse_from(args).unwrap();
        let path = match cli.command {
            Some(Command::Init { path } | Command::Validate { path }) => path,
            None => panic!("expected a subcommand"),
        };
        resolve_config_path(path, cli.config)
    }

    #[rstest::rstest]
    #[case::config_before_subcommand(&["ci-tui", "-c", "a.yaml", "validate"], "a.yaml")]
    #[case::config_after_subcommand(&["ci-tui", "validate", "--config", "a.yaml"], "a.yaml")]
    #[case::positional_path(&["ci-tui", "validate", "b.yaml"], "b.yaml")]
    #[case::positional_beats_config(&["ci-tui", "-c", "a.yaml", "validate", "b.yaml"], "b.yaml")]
    #[case::validate_default(&["ci-tui", "validate"], "ci-tui.yaml")]
    #[case::init_default(&["ci-tui", "init"], "ci-tui.yaml")]
    #[case::init_honors_config(&["ci-tui", "-c", "a.yaml", "init"], "a.yaml")]
    fn test_resolve_config_path(#[case] args: &[&str], #[case] expected: &str) {
        assert_eq!(resolved(args), PathBuf::from(expected));
    }
}
