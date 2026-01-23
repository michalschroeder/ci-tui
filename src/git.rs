//! Git operations for detecting changed files.
//!
//! This module provides functionality to detect which files have changed compared
//! to a base branch or commit reference. It supports multiple fallback strategies
//! for determining the comparison base.
//!
//! # Key Types
//!
//! - [`ChangedFiles`]: Collection of changed file paths with filtering methods
//!
//! # Key Functions
//!
//! - [`detect_changes`]: Main entry point for change detection with fallback logic
//! - [`current_branch`]: Get the current git branch name

use crate::config::GitConfig;
use anyhow::{Context, Result};
use std::collections::HashSet;
use std::path::Path;
use std::process::Command;

/// Files that have changed compared to a base git reference
#[derive(Debug, Clone)]
pub struct ChangedFiles {
    /// List of changed file paths relative to the repository root
    pub files: Vec<String>,
    /// The git reference used as the comparison base (e.g., "origin/development")
    pub base_ref: String,
}

impl ChangedFiles {
    /// Check if there are no changed files
    pub fn is_empty(&self) -> bool {
        self.files.is_empty()
    }

    /// Get the number of changed files
    pub fn len(&self) -> usize {
        self.files.len()
    }

    /// Filter files by regex pattern
    pub fn filter_by_pattern(&self, pattern: &str) -> Vec<&str> {
        let re = match regex::Regex::new(pattern) {
            Ok(r) => r,
            Err(_) => return vec![],
        };

        self.files
            .iter()
            .filter(|f| re.is_match(f))
            .map(|s| s.as_str())
            .collect()
    }

    /// Get unique file extensions
    pub fn extensions(&self) -> HashSet<&str> {
        self.files
            .iter()
            .filter_map(|f| Path::new(f).extension())
            .filter_map(|e| e.to_str())
            .collect()
    }

    /// Remove files matching ignore patterns from the list
    pub fn apply_ignore_patterns(&mut self, ignore_patterns: &[String]) {
        use regex::Regex;

        // Compile all patterns once
        let compiled: Vec<Regex> = ignore_patterns
            .iter()
            .filter_map(|p| Regex::new(p).ok())
            .collect();

        if compiled.is_empty() {
            return;
        }

        self.files
            .retain(|file| !compiled.iter().any(|re| re.is_match(file)));
    }
}

/// Detect changed files by trying multiple base references in order of preference.
///
/// Tries: `origin/{base_branch}`, `{base_branch}`, `{fallback_branch}`.
/// Returns empty list if all references fail.
///
/// # Errors
///
/// Returns an error if git operations fail unexpectedly.
pub fn detect_changes(project_root: &Path, git_config: &GitConfig) -> Result<ChangedFiles> {
    // Try different base refs in order
    let base_refs = [
        format!("origin/{}", git_config.base_branch),
        git_config.base_branch.clone(),
        git_config.fallback_branch.clone(),
    ];

    for base_ref in &base_refs {
        match get_changed_files(project_root, base_ref) {
            Ok(changed_files) => {
                return Ok(changed_files);
            }
            Err(_) => continue,
        }
    }

    // If all fail, return empty
    Ok(ChangedFiles {
        files: vec![],
        base_ref: "HEAD".to_string(),
    })
}

/// Get list of files changed compared to a specific git reference.
///
/// # Errors
///
/// Returns an error if the git command fails or the reference doesn't exist.
pub fn get_changed_files(project_root: &Path, base_ref: &str) -> Result<ChangedFiles> {
    let output = Command::new("git")
        .args(["diff", "--name-only", base_ref])
        .current_dir(project_root)
        .output()
        .context("Failed to run git diff")?;

    if !output.status.success() {
        anyhow::bail!(
            "git diff failed: {}",
            String::from_utf8_lossy(&output.stderr)
        );
    }

    let files = String::from_utf8_lossy(&output.stdout)
        .lines()
        .filter(|line| !line.is_empty())
        .map(String::from)
        .collect();

    Ok(ChangedFiles {
        files,
        base_ref: base_ref.to_string(),
    })
}

/// Get the current branch name.
///
/// # Errors
///
/// Returns an error if the git command fails.
pub fn current_branch(project_root: &Path) -> Result<String> {
    let output = Command::new("git")
        .args(["rev-parse", "--abbrev-ref", "HEAD"])
        .current_dir(project_root)
        .output()
        .context("Failed to get current branch")?;

    Ok(String::from_utf8_lossy(&output.stdout).trim().to_string())
}

/// Get short commit hash.
///
/// # Errors
///
/// Returns an error if the git command fails.
pub fn short_commit(project_root: &Path) -> Result<String> {
    let output = Command::new("git")
        .args(["rev-parse", "--short", "HEAD"])
        .current_dir(project_root)
        .output()
        .context("Failed to get commit hash")?;

    Ok(String::from_utf8_lossy(&output.stdout).trim().to_string())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn make_changed_files(files: Vec<&str>) -> ChangedFiles {
        ChangedFiles {
            files: files.into_iter().map(String::from).collect(),
            base_ref: "origin/development".to_string(),
        }
    }

    #[test]
    fn test_is_empty() {
        let empty = make_changed_files(vec![]);
        assert!(empty.is_empty());

        let non_empty = make_changed_files(vec!["src/Foo.php"]);
        assert!(!non_empty.is_empty());
    }

    #[test]
    fn test_len() {
        let empty = make_changed_files(vec![]);
        assert_eq!(empty.len(), 0);

        let three_files = make_changed_files(vec!["a.php", "b.php", "c.php"]);
        assert_eq!(three_files.len(), 3);
    }

    #[test]
    fn test_filter_by_pattern_php_files() {
        let files = make_changed_files(vec![
            "src/Service/Foo.php",
            "tests/Unit/FooTest.php",
            "config/services.yaml",
            "README.md",
        ]);

        let php_files = files.filter_by_pattern(r"\.php$");
        assert_eq!(php_files.len(), 2);
        assert!(php_files.contains(&"src/Service/Foo.php"));
        assert!(php_files.contains(&"tests/Unit/FooTest.php"));
    }

    #[test]
    fn test_filter_by_pattern_tests_only() {
        let files = make_changed_files(vec![
            "src/Service/Foo.php",
            "tests/Unit/FooTest.php",
            "tests/Integration/BarTest.php",
        ]);

        let test_files = files.filter_by_pattern(r"^tests/.*\.php$");
        assert_eq!(test_files.len(), 2);
        assert!(!test_files.contains(&"src/Service/Foo.php"));
    }

    #[test]
    fn test_filter_by_pattern_invalid_regex() {
        let files = make_changed_files(vec!["src/Foo.php"]);

        // Invalid regex should return empty vec
        let result = files.filter_by_pattern(r"[invalid");
        assert!(result.is_empty());
    }

    #[test]
    fn test_extensions() {
        let files = make_changed_files(vec![
            "src/Service/Foo.php",
            "src/Service/Bar.php",
            "config/services.yaml",
            "config/routes.yml",
            "README.md",
            "Makefile", // No extension
        ]);

        let extensions = files.extensions();
        assert_eq!(extensions.len(), 4);
        assert!(extensions.contains("php"));
        assert!(extensions.contains("yaml"));
        assert!(extensions.contains("yml"));
        assert!(extensions.contains("md"));
    }

    #[test]
    fn test_extensions_empty_files() {
        let empty = make_changed_files(vec![]);
        assert!(empty.extensions().is_empty());
    }

    #[test]
    fn test_apply_ignore_patterns() {
        let mut files = make_changed_files(vec![
            "src/Service/Foo.php",
            "README.md",
            "docs/CONTRIBUTING.md",
            ".github/workflows/ci.yml",
            "tests/Unit/FooTest.php",
        ]);

        let ignore_patterns = vec![r"\.md$".to_string(), r"\.github/".to_string()];

        files.apply_ignore_patterns(&ignore_patterns);

        assert_eq!(files.len(), 2);
        assert!(files.files.contains(&"src/Service/Foo.php".to_string()));
        assert!(files.files.contains(&"tests/Unit/FooTest.php".to_string()));
    }

    #[test]
    fn test_apply_ignore_patterns_empty_patterns() {
        let mut files = make_changed_files(vec!["src/Foo.php", "README.md"]);

        files.apply_ignore_patterns(&[]);

        // Should retain all files when no patterns
        assert_eq!(files.len(), 2);
    }

    #[test]
    fn test_apply_ignore_patterns_invalid_regex() {
        let mut files = make_changed_files(vec!["src/Foo.php", "README.md"]);

        let ignore_patterns = vec![
            r"[invalid".to_string(), // Invalid regex - should be skipped
            r"\.md$".to_string(),
        ];

        files.apply_ignore_patterns(&ignore_patterns);

        // Should still apply valid pattern
        assert_eq!(files.len(), 1);
        assert!(files.files.contains(&"src/Foo.php".to_string()));
    }

    #[test]
    fn test_base_ref_preserved() {
        let files = ChangedFiles {
            files: vec!["test.php".to_string()],
            base_ref: "custom-branch".to_string(),
        };

        assert_eq!(files.base_ref, "custom-branch");
    }
}
