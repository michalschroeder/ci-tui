//! CI TUI - A terminal UI for running CI checks on changed files
//!
//! This library provides the core functionality for detecting changed files,
//! determining which CI checks to run, and executing them in Docker containers.
//!
//! # Modules
//!
//! - `cache`: Result cache skipping checks unchanged since their last passing run
//! - `checks`: Logic for determining which checks to run
//! - `cli`: Command-line argument parsing
//! - `commands`: `init` / `validate` subcommands (scaffold and check config)
//! - `color`: `--no-color` / `NO_COLOR` decision and ANSI stripping for console output
//! - `config`: Configuration loading and parsing from YAML
//! - `exit`: Process exit codes (0 pass, 1 checks failed, 2 config, 3 git/env)
//! - `filter`: `--only` / `--group` check subset selection
//! - `fix`: Auto-fix mode (`--fix`) running fix commands for matched checks
//! - `git`: Git operations for detecting changed files
//! - `list`: `--list` / `--dry-run` report of which checks would run and why
//! - `preflight`: Docker startup probe warning when changed files won't resolve in the container
//! - `report`: `--format json|junit` reports and GitHub Actions annotations (simple mode)
//! - `runner`: Check execution in Docker containers
//! - `schema`: JSON Schema for the config file (`ci-tui schema`)
//! - `simple`: Simple console output mode (no TUI)
//! - `test_discovery`: Finding related test files for source changes
//! - `ui`: Terminal UI using ratatui
//! - `watch`: `--watch` file watcher re-running checks affected by saves

pub mod cache;
pub mod checks;
pub mod cli;
pub mod color;
pub mod commands;
pub mod config;
pub mod exit;
pub mod filter;
pub mod fix;
pub mod git;
pub mod list;
pub mod preflight;
pub mod report;
pub mod runner;
pub mod schema;
pub mod simple;
pub mod test_discovery;
pub mod ui;
mod utils;
pub mod watch;

// Re-export commonly used types for convenience
pub use checks::{determine_checks, CheckToRun};
pub use config::{load_config, CiConfig};
pub use git::{detect_changes, ChangedFiles};
pub use runner::{CheckResult, CheckRunner, CheckStatus};
