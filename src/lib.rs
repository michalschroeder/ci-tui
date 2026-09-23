//! CI TUI - A terminal UI for running CI checks on changed files
//!
//! This library provides the core functionality for detecting changed files,
//! determining which CI checks to run, and executing them in Docker containers.
//!
//! # Modules
//!
//! - `checks`: Logic for determining which checks to run
//! - `cli`: Command-line argument parsing
//! - `commands`: `init` / `validate` subcommands (scaffold and check config)
//! - `config`: Configuration loading and parsing from YAML
//! - `fix`: Auto-fix mode (`--fix`) running fix commands for matched checks
//! - `git`: Git operations for detecting changed files
//! - `runner`: Check execution in Docker containers
//! - `simple`: Simple console output mode (no TUI)
//! - `test_discovery`: Finding related test files for source changes
//! - `ui`: Terminal UI using ratatui

pub mod checks;
pub mod cli;
pub mod commands;
pub mod config;
pub mod fix;
pub mod git;
pub mod runner;
pub mod simple;
pub mod test_discovery;
pub mod ui;
mod utils;

// Re-export commonly used types for convenience
pub use checks::{determine_checks, CheckToRun};
pub use config::{load_config, CiConfig};
pub use git::{detect_changes, ChangedFiles};
pub use runner::{CheckResult, CheckRunner, CheckStatus};
