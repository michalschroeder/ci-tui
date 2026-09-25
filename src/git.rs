//! Git operations for detecting changed files.
//!
//! This module provides functionality to detect which files have changed compared
//! to a base branch or commit reference. It supports multiple fallback strategies
//! for determining the comparison base.
//!
//! # Key Types
//!
//! - [`ChangedFiles`]: Collection of changed file paths with filtering methods
//! - [`GitExecutor`]: Trait abstraction for git command execution
//!
//! # Key Functions
//!
//! - [`detect_changes`]: Main entry point for change detection with fallback logic
//! - [`current_branch`]: Get the current git branch name

use crate::config::GitConfig;
use anyhow::{Context, Result};
use regex::Regex;
use std::collections::HashSet;
use std::path::{Path, PathBuf};
use std::process::Command;

/// Trait for executing git commands.
///
/// This abstraction allows mocking git operations in tests without requiring
/// a real Git repository. The production implementation uses `std::process::Command`.
#[cfg_attr(any(test, feature = "test"), mockall::automock)]
pub trait GitExecutor {
    /// Execute a git command with the given arguments.
    ///
    /// # Arguments
    ///
    /// * `project_root` - Working directory for the git command
    /// * `args` - Command line arguments to pass to git
    ///
    /// # Returns
    ///
    /// Returns the stdout output as a string if successful.
    ///
    /// # Errors
    ///
    /// Returns an error if the command fails or git exits with non-zero status.
    fn run_command(&self, project_root: &Path, args: &[String]) -> Result<String>;
}

/// Real implementation of GitExecutor that invokes the git binary.
pub struct RealGitExecutor;

impl GitExecutor for RealGitExecutor {
    fn run_command(&self, project_root: &Path, args: &[String]) -> Result<String> {
        let output = Command::new("git")
            .args(args)
            .current_dir(project_root)
            .output()
            .context("Failed to run git command")?;

        if !output.status.success() {
            anyhow::bail!(
                "git command failed: {}",
                String::from_utf8_lossy(&output.stderr).trim_end()
            );
        }

        Ok(String::from_utf8_lossy(&output.stdout).to_string())
    }
}

/// Sentinel `base_ref` used when files come from the `--files` CLI flag
/// instead of git change detection. Rendered in headers as "vs --files (manual)".
pub const CLI_FILES_BASE_REF: &str = "--files (manual)";

/// Files that have changed compared to a base git reference
#[derive(Debug, Clone)]
pub struct ChangedFiles {
    /// List of changed file paths relative to the repository root
    pub files: Vec<String>,
    /// The git reference used as the comparison base (e.g., "origin/development")
    pub base_ref: String,
}

impl ChangedFiles {
    /// True when files came from `--files` (no git base to refresh against)
    pub fn is_cli_files(&self) -> bool {
        self.base_ref == CLI_FILES_BASE_REF
    }

    /// Check if there are no changed files
    pub fn is_empty(&self) -> bool {
        self.files.is_empty()
    }

    /// Get the number of changed files
    pub fn len(&self) -> usize {
        self.files.len()
    }

    /// Filter files by a pre-compiled regex pattern
    pub fn filter_by_pattern(&self, re: &Regex) -> Vec<&str> {
        self.files
            .iter()
            .filter(|f| re.is_match(f))
            .map(|s| s.as_str())
            .collect()
    }

    /// Remove files matching any of the given compiled ignore patterns
    pub fn apply_ignore_patterns(&mut self, ignore_patterns: &[Regex]) {
        if ignore_patterns.is_empty() {
            return;
        }
        self.files
            .retain(|file| !ignore_patterns.iter().any(|re| re.is_match(file)));
    }
}

/// Base refs tried by [`detect_changes`], in order: `origin/{base_branch}`,
/// `{base_branch}`, `{fallback_branch}`.
pub fn base_ref_candidates(git_config: &GitConfig) -> [String; 3] {
    [
        format!("origin/{}", git_config.base_branch),
        git_config.base_branch.clone(),
        git_config.fallback_branch.clone(),
    ]
}

/// Detect changed files by trying multiple base references in order of preference.
///
/// Tries: `origin/{base_branch}`, `{base_branch}`, `{fallback_branch}`.
///
/// # Errors
///
/// Returns an error if none of the base references resolve, or if the
/// uncommitted-diff / untracked-files git commands fail.
pub fn detect_changes(project_root: &Path, git_config: &GitConfig) -> Result<ChangedFiles> {
    detect_changes_with_executor(project_root, git_config, &RealGitExecutor)
}

/// Detect changed files using a custom executor (testable version).
///
/// See [`detect_changes`] for details.
pub fn detect_changes_with_executor(
    project_root: &Path,
    git_config: &GitConfig,
    executor: &impl GitExecutor,
) -> Result<ChangedFiles> {
    let base_refs = base_ref_candidates(git_config);

    // Find the first base ref whose committed diff resolves.
    let mut resolved: Option<(String, String)> = None;
    for base_ref in &base_refs {
        if let Ok(out) = run_committed_diff(project_root, base_ref, executor) {
            resolved = Some((base_ref.clone(), out));
            break;
        }
    }

    let (base_ref, committed) = resolved.ok_or_else(|| {
        anyhow::anyhow!(
            "could not resolve any git base ref (tried: {})",
            base_refs.join(", ")
        )
    })?;

    // Uncommitted diff runs once; error here surfaces directly.
    let uncommitted = run_uncommitted_diff(project_root, executor)?;
    let untracked = run_untracked_list(project_root, executor)?;

    Ok(ChangedFiles {
        files: union_lines(&[&committed, &uncommitted, &untracked]),
        base_ref,
    })
}

/// Get list of files changed compared to a specific git reference.
///
/// # Errors
///
/// Returns an error if the git command fails or the reference doesn't exist.
pub fn get_changed_files(project_root: &Path, base_ref: &str) -> Result<ChangedFiles> {
    get_changed_files_with_executor(project_root, base_ref, &RealGitExecutor)
}

/// Get changed files using a custom executor (testable version).
///
/// See [`get_changed_files`] for details.
pub fn get_changed_files_with_executor(
    project_root: &Path,
    base_ref: &str,
    executor: &impl GitExecutor,
) -> Result<ChangedFiles> {
    let committed = run_committed_diff(project_root, base_ref, executor)
        .with_context(|| format!("could not resolve git base ref `{base_ref}`"))?;
    let uncommitted = run_uncommitted_diff(project_root, executor)?;
    let untracked = run_untracked_list(project_root, executor)?;
    Ok(ChangedFiles {
        files: union_lines(&[&committed, &uncommitted, &untracked]),
        base_ref: base_ref.to_string(),
    })
}

fn run_committed_diff(
    project_root: &Path,
    base_ref: &str,
    executor: &impl GitExecutor,
) -> Result<String> {
    let args = vec![
        "diff".to_string(),
        "--name-only".to_string(),
        "--diff-filter=ACMR".to_string(),
        "--merge-base".to_string(),
        // Refs come from config / `--base`: a leading `-` must not parse as an
        // option, and `--` keeps a ref that is also a path unambiguous.
        "--end-of-options".to_string(),
        base_ref.to_string(),
        "HEAD".to_string(),
        "--".to_string(),
    ];
    executor.run_command(project_root, &args)
}

fn run_uncommitted_diff(project_root: &Path, executor: &impl GitExecutor) -> Result<String> {
    let args = vec![
        "diff".to_string(),
        "--name-only".to_string(),
        "--diff-filter=ACMR".to_string(),
        "HEAD".to_string(),
    ];
    executor.run_command(project_root, &args)
}

fn run_untracked_list(project_root: &Path, executor: &impl GitExecutor) -> Result<String> {
    let args = vec![
        "ls-files".to_string(),
        "--others".to_string(),
        "--exclude-standard".to_string(),
    ];
    executor.run_command(project_root, &args)
}

fn union_lines(sources: &[&str]) -> Vec<String> {
    let mut seen = HashSet::new();
    let mut files = Vec::new();
    for line in sources.iter().flat_map(|s| s.lines()) {
        if line.is_empty() {
            continue;
        }
        if seen.insert(line.to_string()) {
            files.push(line.to_string());
        }
    }
    files
}

/// Get the current branch name.
///
/// # Errors
///
/// Returns an error if the git command fails.
pub fn current_branch(project_root: &Path) -> Result<String> {
    current_branch_with_executor(project_root, &RealGitExecutor)
}

/// Get current branch using a custom executor (testable version).
///
/// See [`current_branch`] for details.
pub fn current_branch_with_executor(
    project_root: &Path,
    executor: &impl GitExecutor,
) -> Result<String> {
    let args = vec![
        "rev-parse".to_string(),
        "--abbrev-ref".to_string(),
        "HEAD".to_string(),
    ];
    let output = executor.run_command(project_root, &args)?;
    Ok(output.trim().to_string())
}

/// Get short commit hash.
///
/// # Errors
///
/// Returns an error if the git command fails.
pub fn short_commit(project_root: &Path) -> Result<String> {
    short_commit_with_executor(project_root, &RealGitExecutor)
}

/// Get short commit using a custom executor (testable version).
///
/// See [`short_commit`] for details.
pub fn short_commit_with_executor(
    project_root: &Path,
    executor: &impl GitExecutor,
) -> Result<String> {
    let args = vec![
        "rev-parse".to_string(),
        "--short".to_string(),
        "HEAD".to_string(),
    ];
    let output = executor.run_command(project_root, &args)?;
    Ok(output.trim().to_string())
}

/// Top-level directory of the git repo containing `cwd`. Errors outside a repo.
pub fn repo_root(cwd: &Path) -> Result<PathBuf> {
    repo_root_with_executor(cwd, &RealGitExecutor)
}

/// Repo root using a custom executor (testable version). See [`repo_root`].
pub(crate) fn repo_root_with_executor(cwd: &Path, executor: &impl GitExecutor) -> Result<PathBuf> {
    let args = vec!["rev-parse".to_string(), "--show-toplevel".to_string()];
    let output = executor
        .run_command(cwd, &args)
        .context("failed to resolve git repository root")?;
    let root = output.trim();
    anyhow::ensure!(
        !root.is_empty(),
        "failed to resolve git repository root: empty `git rev-parse --show-toplevel` output"
    );
    Ok(PathBuf::from(root))
}

/// Rewrite a cwd-relative path to repo-relative (`cwd.join(path)` stripped of
/// `repo_root`). Paths outside the repo are returned unchanged.
pub fn to_repo_relative(cwd: &Path, repo_root: &Path, path: &Path) -> String {
    let joined = cwd.join(path);
    match joined.strip_prefix(repo_root) {
        Ok(rel) => rel.to_string_lossy().into_owned(),
        Err(_) => path.to_string_lossy().into_owned(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn to_repo_relative_rewrites_cwd_relative_paths() {
        let root = Path::new("/repo");
        let cwd = Path::new("/repo/sub");
        assert_eq!(to_repo_relative(cwd, root, Path::new("a.rs")), "sub/a.rs");
        assert_eq!(
            to_repo_relative(root, root, Path::new("src/a.rs")),
            "src/a.rs"
        );
        assert_eq!(to_repo_relative(cwd, root, Path::new("/repo/b.rs")), "b.rs");
        assert_eq!(
            to_repo_relative(cwd, root, Path::new("/other/c.rs")),
            "/other/c.rs"
        );
    }

    #[test]
    fn repo_root_trims_show_toplevel_output() {
        let mut mock = MockGitExecutor::new();
        mock.expect_run_command()
            .withf(|_, args: &[String]| args == ["rev-parse", "--show-toplevel"])
            .times(1)
            .returning(|_, _| Ok("/home/u/repo\n".to_string()));
        let root = repo_root_with_executor(Path::new("/home/u/repo/src"), &mock).unwrap();
        assert_eq!(root, PathBuf::from("/home/u/repo"));
    }

    #[test]
    fn repo_root_err_on_git_error() {
        let mut mock = MockGitExecutor::new();
        mock.expect_run_command()
            .returning(|_, _| Err(anyhow::anyhow!("not a git repository")));
        let err = repo_root_with_executor(Path::new("/tmp"), &mock).unwrap_err();
        assert!(
            format!("{err:#}").contains("not a git repository"),
            "got: {err:#}"
        );
    }

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

    fn re(pattern: &str) -> Regex {
        Regex::new(pattern).unwrap()
    }

    fn res(patterns: &[&str]) -> Vec<Regex> {
        patterns.iter().map(|p| re(p)).collect()
    }

    #[test]
    fn test_filter_by_pattern_php_files() {
        let files = make_changed_files(vec![
            "src/Service/Foo.php",
            "tests/Unit/FooTest.php",
            "config/services.yaml",
            "README.md",
        ]);

        let php_files = files.filter_by_pattern(&re(r"\.php$"));
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

        let test_files = files.filter_by_pattern(&re(r"^tests/.*\.php$"));
        assert_eq!(test_files.len(), 2);
        assert!(!test_files.contains(&"src/Service/Foo.php"));
    }

    #[test]
    fn test_filter_by_pattern_preserves_order() {
        let files = make_changed_files(vec!["c.rs", "a.rs", "b.rs", "README.md"]);
        let matched = files.filter_by_pattern(&re(r"\.rs$"));
        // Preserves insertion order from ChangedFiles.files
        assert_eq!(matched, vec!["c.rs", "a.rs", "b.rs"]);
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

        files.apply_ignore_patterns(&res(&[r"\.md$", r"\.github/"]));

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
    fn test_base_ref_preserved() {
        let files = ChangedFiles {
            files: vec!["test.php".to_string()],
            base_ref: "custom-branch".to_string(),
        };

        assert_eq!(files.base_ref, "custom-branch");
    }

    #[test]
    fn get_changed_files_passes_diff_filter_acmr() {
        let mut mock = MockGitExecutor::new();
        mock.expect_run_command()
            .withf(|_, args: &[String]| args.iter().any(|a| a == "--diff-filter=ACMR"))
            .times(2)
            .returning(|_, _| Ok(String::new()));

        // ls-files (untracked) doesn't use --diff-filter
        mock.expect_run_command()
            .withf(|_, args: &[String]| args.iter().any(|a| a == "ls-files"))
            .times(1)
            .returning(|_, _| Ok(String::new()));

        let _ = get_changed_files_with_executor(Path::new("/tmp"), "main", &mock).unwrap();
    }

    #[test]
    fn get_changed_files_uses_merge_base_against_head() {
        let mut mock = MockGitExecutor::new();
        mock.expect_run_command()
            .withf(|_, args: &[String]| {
                args.iter().any(|a| a == "--merge-base")
                    && args.iter().any(|a| a == "HEAD")
                    && args.iter().any(|a| a == "origin/main")
            })
            .times(1)
            .returning(|_, _| Ok(String::new()));

        // Remaining calls (uncommitted diff + untracked ls-files) — no --merge-base
        mock.expect_run_command()
            .withf(|_, args: &[String]| !args.iter().any(|a| a == "--merge-base"))
            .times(2)
            .returning(|_, _| Ok(String::new()));

        let _ = get_changed_files_with_executor(Path::new("/tmp"), "origin/main", &mock).unwrap();
    }

    #[test]
    fn get_changed_files_issues_three_calls_and_unions_results() {
        use mockall::Sequence;
        let mut mock = MockGitExecutor::new();
        let mut seq = Sequence::new();

        // First call: committed (merge-base) — returns a.rs, b.rs
        mock.expect_run_command()
            .withf(|_, args: &[String]| args.iter().any(|a| a == "--merge-base"))
            .times(1)
            .in_sequence(&mut seq)
            .returning(|_, _| Ok("a.rs\nb.rs\n".to_string()));

        // Second call: uncommitted — returns b.rs, c.rs
        mock.expect_run_command()
            .withf(|_, args: &[String]| {
                !args.iter().any(|a| a == "--merge-base")
                    && args.iter().any(|a| a == "HEAD")
                    && args.iter().any(|a| a == "--diff-filter=ACMR")
            })
            .times(1)
            .in_sequence(&mut seq)
            .returning(|_, _| Ok("b.rs\nc.rs\n".to_string()));

        // Third call: untracked (ls-files --others --exclude-standard) — returns c.rs, d.rs
        mock.expect_run_command()
            .withf(|_, args: &[String]| {
                args.iter().any(|a| a == "ls-files")
                    && args.iter().any(|a| a == "--others")
                    && args.iter().any(|a| a == "--exclude-standard")
            })
            .times(1)
            .in_sequence(&mut seq)
            .returning(|_, _| Ok("c.rs\nd.rs\n".to_string()));

        let result = get_changed_files_with_executor(Path::new("/tmp"), "main", &mock).unwrap();

        // Committed-first order, untracked appended, deduped across all three sources
        assert_eq!(result.files, vec!["a.rs", "b.rs", "c.rs", "d.rs"]);
        assert_eq!(result.base_ref, "main");
    }

    #[test]
    fn get_changed_files_includes_untracked_when_no_diff() {
        let mut mock = MockGitExecutor::new();

        mock.expect_run_command()
            .withf(|_, args: &[String]| args.iter().any(|a| a == "--diff-filter=ACMR"))
            .times(2)
            .returning(|_, _| Ok(String::new()));

        // Mock returns only non-gitignored paths — the real git binary applies
        // --exclude-standard, we just test plumbing.
        mock.expect_run_command()
            .withf(|_, args: &[String]| args.iter().any(|a| a == "ls-files"))
            .times(1)
            .returning(|_, _| Ok("new_file.rs\nsrc/new_mod.rs\n".to_string()));

        let result = get_changed_files_with_executor(Path::new("/tmp"), "main", &mock).unwrap();
        assert_eq!(result.files, vec!["new_file.rs", "src/new_mod.rs"]);
    }

    #[test]
    fn get_changed_files_committed_failure_propagates() {
        let mut mock = MockGitExecutor::new();
        mock.expect_run_command()
            .times(1)
            .returning(|_, _| Err(anyhow::anyhow!("fatal: bad revision 'nope'")));

        let result = get_changed_files_with_executor(Path::new("/tmp"), "nope", &mock);
        assert!(result.is_err());
    }

    fn default_git_config() -> crate::config::GitConfig {
        crate::config::GitConfig {
            base_branch: "main".to_string(),
            fallback_branch: "HEAD~1".to_string(),
        }
    }

    #[test]
    fn detect_changes_errors_when_uncommitted_diff_fails() {
        let mut mock = MockGitExecutor::new();

        mock.expect_run_command()
            .withf(|_, args: &[String]| args.iter().any(|a| a == "--merge-base"))
            .times(1)
            .returning(|_, _| Ok("committed.rs\n".to_string()));

        mock.expect_run_command()
            .withf(|_, args: &[String]| {
                !args.iter().any(|a| a == "--merge-base")
                    && args.iter().any(|a| a == "--diff-filter=ACMR")
            })
            .times(1)
            .returning(|_, _| Err(anyhow::anyhow!("uncommitted diff blew up")));

        let result = detect_changes_with_executor(Path::new("/tmp"), &default_git_config(), &mock);
        let msg = result.unwrap_err().to_string();
        assert!(msg.contains("uncommitted diff blew up"), "got: {msg}");
    }

    #[test]
    fn detect_changes_errors_when_untracked_list_fails() {
        let mut mock = MockGitExecutor::new();

        mock.expect_run_command()
            .withf(|_, args: &[String]| args.iter().any(|a| a == "--merge-base"))
            .times(1)
            .returning(|_, _| Ok("a.rs\n".to_string()));

        mock.expect_run_command()
            .withf(|_, args: &[String]| {
                !args.iter().any(|a| a == "--merge-base")
                    && args.iter().any(|a| a == "--diff-filter=ACMR")
            })
            .times(1)
            .returning(|_, _| Ok(String::new()));

        mock.expect_run_command()
            .withf(|_, args: &[String]| args.iter().any(|a| a == "ls-files"))
            .times(1)
            .returning(|_, _| Err(anyhow::anyhow!("ls-files refused")));

        let result = detect_changes_with_executor(Path::new("/tmp"), &default_git_config(), &mock);
        let msg = result.unwrap_err().to_string();
        assert!(msg.contains("ls-files refused"), "got: {msg}");
    }

    #[test]
    fn cli_files_base_ref_is_human_readable() {
        assert_eq!(CLI_FILES_BASE_REF, "--files (manual)");
        assert_ne!(CLI_FILES_BASE_REF, "cli");
    }
}
