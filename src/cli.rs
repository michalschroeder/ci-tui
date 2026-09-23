//! Command-line interface: argument parsing and config-path resolution.

use clap::{CommandFactory, Parser, Subcommand};
use std::ffi::OsString;
use std::path::{Path, PathBuf};

/// CI TUI - Run CI checks for changed files
#[derive(Parser)]
#[command(
    name = "ci-tui",
    version,
    about,
    long_about = None
)]
pub struct Cli {
    #[command(subcommand)]
    pub command: Option<Command>,

    /// Path to config file [default: ci-tui.yaml if present; create one with `ci-tui init`]
    #[arg(short, long, global = true)]
    pub config: Option<PathBuf>,

    /// Run in simple console mode (no TUI)
    #[arg(short, long)]
    pub simple: bool,

    /// Run only fix commands (skip checks)
    #[arg(long)]
    pub fix: bool,

    /// Run checks on specific files instead of git-detected changes
    #[arg(short, long, num_args = 1.., value_parser = parse_file_arg)]
    pub files: Vec<PathBuf>,
}

/// `--files` value parser. `--files` takes 1.. values, so a trailing subcommand
/// (`-f a.rs validate`) would otherwise be swallowed as a file path.
fn parse_file_arg(value: &str) -> Result<PathBuf, String> {
    if <Command as Subcommand>::has_subcommand(value) {
        return Err(format!(
            "`{value}` is a subcommand, not a file (subcommands cannot be combined with --files)"
        ));
    }
    Ok(PathBuf::from(value))
}

impl Cli {
    /// Parse process args, exiting with a clap error on invalid combinations.
    pub fn parse_checked() -> Self {
        Self::try_parse_checked(std::env::args_os()).unwrap_or_else(|e| e.exit())
    }

    /// Parse `args` and reject run-mode flags combined with a subcommand.
    ///
    /// Not `args_conflicts_with_subcommands`: that also rejects the global `--config`.
    pub fn try_parse_checked<I, T>(args: I) -> Result<Self, clap::Error>
    where
        I: IntoIterator<Item = T>,
        T: Into<OsString> + Clone,
    {
        let cli = Self::try_parse_from(args)?;
        if cli.command.is_some() && (cli.simple || cli.fix || !cli.files.is_empty()) {
            return Err(Self::command().error(
                clap::error::ErrorKind::ArgumentConflict,
                "--simple, --fix and --files cannot be used with a subcommand",
            ));
        }
        Ok(cli)
    }
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
        "--config <CONFIG> is required: no ci-tui.yaml in the current directory (run `ci-tui init` to create one)",
    )
}

impl Command {
    /// Config path for `init` / `validate`: positional arg, then `--config`,
    /// then `ci-tui.yaml`.
    pub fn config_path(&self, config: Option<PathBuf>) -> PathBuf {
        let (Self::Init { path } | Self::Validate { path }) = self;
        path.clone()
            .or(config)
            .unwrap_or_else(|| PathBuf::from(crate::commands::DEFAULT_CONFIG_FILE))
    }
}

/// Config path for running checks: `--config`, else `ci-tui.yaml` in `cwd` if
/// present. `None` means no config was given or found.
pub fn run_config_path(config: Option<PathBuf>, cwd: &Path) -> Option<PathBuf> {
    config.or_else(|| {
        cwd.join(crate::commands::DEFAULT_CONFIG_FILE)
            .is_file()
            .then(|| PathBuf::from(crate::commands::DEFAULT_CONFIG_FILE))
    })
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
        let cli = Cli::try_parse_checked(args).unwrap();
        cli.command
            .expect("expected a subcommand")
            .config_path(cli.config)
    }

    #[rstest::rstest]
    #[case::fix_before_validate(&["ci-tui", "--fix", "validate"])]
    #[case::simple_before_init(&["ci-tui", "-s", "init"])]
    #[case::files_before_init(&["ci-tui", "--files", "a.rs", "-s", "init"])]
    fn test_run_flags_conflict_with_subcommands(#[case] args: &[&str]) {
        let err = Cli::try_parse_checked(args)
            .err()
            .expect("expected conflict");
        assert_eq!(err.kind(), clap::error::ErrorKind::ArgumentConflict);
    }

    #[rstest::rstest]
    #[case::validate(&["ci-tui", "-c", "x.yaml", "-f", "a.rs", "validate"])]
    #[case::init(&["ci-tui", "-f", "a.rs", "init"])]
    fn test_files_rejects_subcommand_name(#[case] args: &[&str]) {
        let err = Cli::try_parse_checked(args).err().expect("expected error");
        assert_eq!(err.kind(), clap::error::ErrorKind::ValueValidation);
    }

    #[rstest::rstest]
    #[case::config_before_subcommand(&["ci-tui", "-c", "a.yaml", "validate"], "a.yaml")]
    #[case::config_after_subcommand(&["ci-tui", "validate", "--config", "a.yaml"], "a.yaml")]
    #[case::positional_path(&["ci-tui", "validate", "b.yaml"], "b.yaml")]
    #[case::positional_beats_config(&["ci-tui", "-c", "a.yaml", "validate", "b.yaml"], "b.yaml")]
    #[case::validate_default(&["ci-tui", "validate"], "ci-tui.yaml")]
    #[case::init_default(&["ci-tui", "init"], "ci-tui.yaml")]
    #[case::init_honors_config(&["ci-tui", "-c", "a.yaml", "init"], "a.yaml")]
    fn test_subcommand_config_path(#[case] args: &[&str], #[case] expected: &str) {
        assert_eq!(resolved(args), PathBuf::from(expected));
    }

    #[test]
    fn test_run_config_path_none_without_config_or_default_file() {
        let dir = tempfile::tempdir().unwrap();
        assert_eq!(run_config_path(None, dir.path()), None);
    }

    #[test]
    fn test_run_config_path_falls_back_to_default_file() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(dir.path().join("ci-tui.yaml"), "").unwrap();
        assert_eq!(
            run_config_path(None, dir.path()),
            Some(PathBuf::from("ci-tui.yaml"))
        );
    }

    #[test]
    fn test_run_config_path_prefers_config_flag() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(dir.path().join("ci-tui.yaml"), "").unwrap();
        assert_eq!(
            run_config_path(Some("a.yaml".into()), dir.path()),
            Some(PathBuf::from("a.yaml"))
        );
    }
}
