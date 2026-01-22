//! CI TUI - A terminal UI for running CI checks on changed files
//!
//! This library provides the core functionality for detecting changed files,
//! determining which CI checks to run, and executing them in Docker containers.
//!
//! # Modules
//!
//! - `config`: Configuration loading and parsing from YAML
//! - `git`: Git operations for detecting changed files
//! - `checks`: Logic for determining which checks to run
//! - `runner`: Check execution in Docker containers
//! - `test_discovery`: Finding related test files for source changes
//! - `ui`: Terminal UI using ratatui
//! - `simple`: Simple console output mode (no TUI)

pub mod checks;
pub mod config;
pub mod git;
pub mod runner;
pub mod simple;
pub mod test_discovery;
pub mod ui;

// Re-export commonly used types for convenience
pub use checks::{determine_checks, CheckToRun};
pub use config::{load_config, CiConfig};
pub use git::{detect_changes, ChangedFiles};
pub use runner::{CheckResult, CheckRunner, CheckStatus};

#[cfg(test)]
mod tests {
    use super::*;
    use pretty_assertions::assert_eq;
    use rstest::*;

    #[test]
    fn test_infrastructure_works() {
        // Basic test to verify test infrastructure is set up
        assert_eq!(1 + 1, 2);
    }

    #[rstest]
    #[case(2, 2, 4)]
    #[case(3, 3, 6)]
    fn test_parameterized(#[case] a: i32, #[case] b: i32, #[case] expected: i32) {
        assert_eq!(a + b, expected);
    }
}
