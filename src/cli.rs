//! Command-line interface: argument parsing and config-path resolution.

use clap::{CommandFactory, Parser, Subcommand};
use std::path::PathBuf;

/// CI TUI - Run CI checks for changed files
#[derive(Parser)]
#[command(name = "ci-tui", version, about, long_about = None)]
pub struct Cli {
    #[command(subcommand)]
    pub command: Option<Command>,

    /// Path to config file (required unless a subcommand is given; create one with `ci-tui init`)
    #[arg(short, long, global = true)]
    pub config: Option<PathBuf>,

    /// Run in simple console mode (no TUI)
    #[arg(short, long)]
    pub simple: bool,

    /// Run only fix commands (skip checks)
    #[arg(long)]
    pub fix: bool,

    /// Run checks on specific files instead of git-detected changes
    #[arg(short, long, num_args = 1..)]
    pub files: Vec<PathBuf>,
}

#[derive(Subcommand)]
pub enum Command {
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
pub fn missing_config_error() -> clap::Error {
    Cli::command().error(
        clap::error::ErrorKind::MissingRequiredArgument,
        "--config <CONFIG> is required (or run `ci-tui init` to create one)",
    )
}

/// Resolve the config path for `init` / `validate`: positional arg, then
/// `--config`, then `ci-tui.yaml`.
pub fn resolve_config_path(path: Option<PathBuf>, config: Option<PathBuf>) -> PathBuf {
    path.or(config)
        .unwrap_or_else(|| PathBuf::from(crate::commands::DEFAULT_CONFIG_FILE))
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
