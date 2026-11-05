//! Git operations for detecting changed files.
//!
//! This module provides functionality to detect which files have changed compared
//! to a base branch or commit reference, falling back only when a ref does not
//! exist.
//!
//! # Key Types
//!
//! - [`ChangedFiles`]: Collection of changed file paths with filtering methods
//! - [`GitExecutor`]: Trait abstraction for git command execution
//! - [`GitFailed`]: Non-zero git exit, keeping the exit code
//! - [`BaseRefNotFound`]: No candidate base ref exists
//!
//! # Key Functions
//!
//! - [`detect_changes`]: Main entry point for change detection with fallback logic
//! - [`current_branch`]: Get the current git branch name

use crate::config::GitConfig;
use anyhow::{Context, Result};
use indexmap::IndexSet;
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
    /// Returns an error if the command fails to start, or [`GitFailed`] if git
    /// exits with non-zero status.
    fn run_command(&self, project_root: &Path, args: &[String]) -> Result<String>;
}

/// Real implementation of GitExecutor that invokes the git binary.
pub struct RealGitExecutor;

impl GitExecutor for RealGitExecutor {
    fn run_command(&self, project_root: &Path, args: &[String]) -> Result<String> {
        let output = crate::utils::own_process_group(&mut Command::new("git"))
            .args(args)
            .current_dir(project_root)
            .output()
            .context("Failed to run git command")?;

        if !output.status.success() {
            let stderr = String::from_utf8_lossy(&output.stderr);
            return Err(GitFailed {
                code: output.status.code(),
                stderr: stderr.trim_end().to_string(),
            }
            .into());
        }

        String::from_utf8(output.stdout).map_err(|e| NonUtf8Output::new(e.as_bytes()).into())
    }
}

/// Git ran and exited non-zero. Typed so callers can tell exit codes apart
/// through the `anyhow::Error` of [`GitExecutor::run_command`].
#[derive(Debug, thiserror::Error)]
#[error("git command failed: {stderr}")]
pub struct GitFailed {
    /// Exit code; `None` when a signal killed git
    pub code: Option<i32>,
    pub stderr: String,
}

/// No candidate base ref exists (the only error that means "bad base ref":
/// the hint in `main` keys on it).
#[derive(Debug, thiserror::Error)]
#[error("could not resolve git base ref (tried: {})", .0.join(", "))]
pub struct BaseRefNotFound(Vec<String>);

/// Git printed bytes that are not UTF-8, e.g. a non-UTF-8 file name. Change
/// detection fails on it (exit 3) instead of dropping or mangling the path,
/// which would let the file skip its checks.
#[derive(Debug, thiserror::Error)]
#[error(
    "git printed a non-UTF-8 path `{0}`: ci-tui needs UTF-8 file names, \
     rename it (or bypass git with --files)"
)]
pub struct NonUtf8Output(String);

impl NonUtf8Output {
    /// From git's raw stdout: names the first NUL-separated entry that is
    /// not UTF-8 (lossy, for the message). Not split on newlines: `-z`
    /// names may contain them; a trailing one (non-`-z` output) is dropped.
    pub fn new(stdout: &[u8]) -> Self {
        let entry = stdout
            .split(|b| *b == 0)
            .find(|entry| std::str::from_utf8(entry).is_err())
            .unwrap_or(stdout);
        let entry = entry.strip_suffix(b"\n").unwrap_or(entry);
        Self(String::from_utf8_lossy(entry).into_owned())
    }
}

/// Sentinel `base_ref` used when files come from the `--files` CLI flag
/// instead of git change detection. Rendered in headers as "vs --files (manual)".
pub const CLI_FILES_BASE_REF: &str = "--files (manual)";

/// Sentinel `base_ref` used with the `--staged` CLI flag: files come from the
/// git index only. Rendered in headers as "vs --staged (git index)".
pub const STAGED_BASE_REF: &str = "--staged (git index)";

/// Change types every `git diff` path reports: all but deletions (lowercase
/// `d` excludes), so type changes (file <-> symlink, `T`) and unmerged paths
/// (`U`) still reach their checks; an allow-list would drop new classes silently.
const DIFF_FILTER: &str = "--diff-filter=d";

/// Files that have changed compared to a base git reference
#[derive(Debug, Clone, Default)]
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

    /// True when files came from `--staged` (refresh re-reads the git index)
    pub fn is_staged(&self) -> bool {
        self.base_ref == STAGED_BASE_REF
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
/// Tries: `origin/{base_branch}`, `{base_branch}`, `{fallback_branch}`, and
/// diffs against the first that exists.
///
/// # Errors
///
/// Returns [`BaseRefNotFound`] if none of the base references exist. Any other
/// git failure is returned as is, without trying the next reference: failing
/// to resolve a candidate (e.g. not a git repository), or a diff or
/// untracked-files command failing once one resolved (e.g. no merge base with
/// HEAD in a shallow clone).
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
    changed_files_against_first(project_root, &base_ref_candidates(git_config), executor)
}

/// Get list of files changed compared to a specific git reference.
///
/// # Errors
///
/// Returns [`BaseRefNotFound`] if the reference doesn't exist, else the error
/// of the git command that failed.
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
    changed_files_against_first(project_root, &[base_ref.to_string()], executor)
}

/// Files changed against the first of `candidates` that exists. The diff gets
/// the ref's name, not its sha: `ChangedFiles::base_ref` is shown and refreshed.
fn changed_files_against_first(
    project_root: &Path,
    candidates: &[String],
    executor: &impl GitExecutor,
) -> Result<ChangedFiles> {
    let base_ref = resolve_base_ref(project_root, executor, candidates)?;
    let committed = run_committed_diff(project_root, base_ref, executor)?;
    let uncommitted = run_uncommitted_diff(project_root, executor)?;
    let untracked = run_untracked_list(project_root, executor)?;
    Ok(ChangedFiles {
        files: union_paths([committed, uncommitted, untracked]),
        base_ref: base_ref.to_string(),
    })
}

/// First of `candidates` that exists. `git rev-parse --verify` exits 1 for a
/// ref that does not exist, the only failure that moves on: any other (not a
/// repo, git missing) would fail the same on the next candidate or hide a real
/// problem behind the wrong base.
///
/// No `^{commit}` peel: it would report a ref to a missing object as absent,
/// and `:/text` would swallow the suffix.
fn resolve_base_ref<'a>(
    project_root: &Path,
    executor: &impl GitExecutor,
    candidates: &'a [String],
) -> Result<&'a str> {
    for candidate in candidates {
        // As in `run_committed_diff`, a leading `-` must not parse as an option
        let args = [
            "rev-parse",
            "--verify",
            "--quiet",
            "--end-of-options",
            candidate.as_str(),
        ]
        .map(String::from);
        match executor.run_command(project_root, &args) {
            Ok(_) => return Ok(candidate.as_str()),
            Err(e) if is_missing_ref(&e) => {}
            Err(e) => return Err(e),
        }
    }
    Err(BaseRefNotFound(candidates.to_vec()).into())
}

/// True for `git rev-parse --verify --quiet`'s exit 1: no such ref
fn is_missing_ref(err: &anyhow::Error) -> bool {
    err.downcast_ref::<GitFailed>()
        .is_some_and(|failed| failed.code == Some(1))
}

/// Get files staged in the git index (`git diff --cached`), for `--staged`.
///
/// Only the index is read: no base ref, unstaged changes or untracked files.
///
/// # Errors
///
/// Returns an error if the git command fails (e.g. not a git repository).
pub fn get_staged_files(project_root: &Path) -> Result<ChangedFiles> {
    get_staged_files_with_executor(project_root, &RealGitExecutor)
}

/// Get staged files using a custom executor (testable version).
///
/// See [`get_staged_files`] for details.
pub fn get_staged_files_with_executor(
    project_root: &Path,
    executor: &impl GitExecutor,
) -> Result<ChangedFiles> {
    let args = ["--cached", "--name-only", DIFF_FILTER];
    let mut files = list_paths(project_root, executor, "diff", &args)
        .context("could not read staged files from the git index")?;
    // Checks read the working tree: drop staged paths since deleted on disk.
    // `--full-name -- :/` keeps paths repo-relative like `git diff` from subdirs.
    let args = ["--deleted", "--full-name", "--", ":/"];
    let deleted: HashSet<String> = list_paths(project_root, executor, "ls-files", &args)
        .context("could not list staged files deleted from the working tree")?
        .into_iter()
        .collect();
    files.retain(|f| !deleted.contains(f));
    Ok(ChangedFiles {
        files,
        base_ref: STAGED_BASE_REF.to_string(),
    })
}

/// Paths changed between the merge base of `base_ref`, which resolved, and
/// HEAD. Git's errors stay as they are, e.g. no merge base in a shallow clone.
fn run_committed_diff(
    project_root: &Path,
    base_ref: &str,
    executor: &impl GitExecutor,
) -> Result<Vec<String>> {
    let args = [
        "--name-only",
        DIFF_FILTER,
        "--merge-base",
        // Refs come from config / `--base`: a leading `-` must not parse as an
        // option, and `--` keeps a ref that is also a path unambiguous.
        "--end-of-options",
        base_ref,
        "HEAD",
        "--",
    ];
    list_paths(project_root, executor, "diff", &args)
}

fn run_uncommitted_diff(project_root: &Path, executor: &impl GitExecutor) -> Result<Vec<String>> {
    let args = ["--name-only", DIFF_FILTER, "HEAD"];
    list_paths(project_root, executor, "diff", &args)
}

fn run_untracked_list(project_root: &Path, executor: &impl GitExecutor) -> Result<Vec<String>> {
    let args = ["--others", "--exclude-standard"];
    list_paths(project_root, executor, "ls-files", &args)
}

/// Paths `git <subcommand> -z <args>` lists. `-z` prints names verbatim,
/// NUL-terminated: without it git C-quotes names with non-ASCII bytes, `"`,
/// `\` or control chars, so they match no pattern and skip their checks.
fn list_paths(
    project_root: &Path,
    executor: &impl GitExecutor,
    subcommand: &str,
    args: &[&str],
) -> Result<Vec<String>> {
    let args: Vec<String> = [subcommand, "-z"]
        .iter()
        .chain(args)
        .map(|a| a.to_string())
        .collect();
    let out = executor.run_command(project_root, &args)?;
    let paths = out.split('\0').filter(|path| !path.is_empty());
    Ok(paths.map(str::to_string).collect())
}

/// `lists` concatenated, deduped in first-seen order
fn union_paths<const N: usize>(lists: [Vec<String>; N]) -> Vec<String> {
    let paths: IndexSet<String> = lists.into_iter().flatten().collect();
    paths.into_iter().collect()
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
    rev_parse_path(cwd, executor, "--show-toplevel", "git repository root")
}

/// Absolute git dir of the repo containing `cwd` (`.git`, or
/// `.git/worktrees/<name>` in a linked worktree). Errors outside a repo.
pub fn git_dir(cwd: &Path) -> Result<PathBuf> {
    git_dir_with_executor(cwd, &RealGitExecutor)
}

/// Git dir using a custom executor (testable version). See [`git_dir`].
pub(crate) fn git_dir_with_executor(cwd: &Path, executor: &impl GitExecutor) -> Result<PathBuf> {
    rev_parse_path(cwd, executor, "--absolute-git-dir", "git dir")
}

/// Path printed by `git rev-parse <flag>` (trimmed, non-empty); `what` names
/// it in errors
fn rev_parse_path(
    cwd: &Path,
    executor: &impl GitExecutor,
    flag: &str,
    what: &str,
) -> Result<PathBuf> {
    let args = vec!["rev-parse".to_string(), flag.to_string()];
    let output = executor
        .run_command(cwd, &args)
        .with_context(|| format!("failed to resolve {what}"))?;
    let path = output.trim();
    anyhow::ensure!(
        !path.is_empty(),
        "failed to resolve {what}: empty `git rev-parse {flag}` output"
    );
    Ok(PathBuf::from(path))
}

/// Rewrite a cwd-relative path relative to `root` (`cwd.join(path)` stripped
/// of `root`). Paths outside `root` are returned unchanged.
pub fn to_relative(cwd: &Path, root: &Path, path: &Path) -> String {
    let joined = cwd.join(path);
    match joined.strip_prefix(root) {
        Ok(rel) => rel.to_string_lossy().into_owned(),
        Err(_) => path.to_string_lossy().into_owned(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn to_relative_rewrites_cwd_relative_paths() {
        let root = Path::new("/repo");
        let cwd = Path::new("/repo/sub");
        assert_eq!(to_relative(cwd, root, Path::new("a.rs")), "sub/a.rs");
        assert_eq!(to_relative(root, root, Path::new("src/a.rs")), "src/a.rs");
        assert_eq!(to_relative(cwd, root, Path::new("/repo/b.rs")), "b.rs");
        assert_eq!(
            to_relative(cwd, root, Path::new("/other/c.rs")),
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
    fn git_dir_uses_absolute_git_dir() {
        let mut mock = MockGitExecutor::new();
        mock.expect_run_command()
            .withf(|_, args: &[String]| args == ["rev-parse", "--absolute-git-dir"])
            .times(1)
            .returning(|_, _| Ok("/home/u/repo/.git/worktrees/wt\n".to_string()));
        let dir = git_dir_with_executor(Path::new("/home/u/wt"), &mock).unwrap();
        assert_eq!(dir, PathBuf::from("/home/u/repo/.git/worktrees/wt"));
    }

    #[test]
    fn git_dir_err_on_empty_output() {
        let mut mock = MockGitExecutor::new();
        mock.expect_run_command()
            .returning(|_, _| Ok("\n".to_string()));
        assert!(git_dir_with_executor(Path::new("/tmp"), &mock).is_err());
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

    /// Mock whose first candidate resolves: exactly one `git rev-parse`
    fn mock_resolving() -> MockGitExecutor {
        let mut mock = MockGitExecutor::new();
        mock.expect_run_command()
            .withf(|_, args: &[String]| args[0] == "rev-parse")
            .times(1)
            .returning(|_, _| Ok(String::new()));
        mock
    }

    #[test]
    fn get_changed_files_passes_diff_filter_excluding_deletions() {
        let mut mock = mock_resolving();
        mock.expect_run_command()
            .withf(|_, args: &[String]| args.iter().any(|a| a == "--diff-filter=d"))
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
        let mut mock = mock_resolving();
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
    fn get_changed_files_resolves_then_unions_three_listings() {
        use mockall::Sequence;
        let mut mock = mock_resolving();
        let mut seq = Sequence::new();

        // First listing: committed (merge-base) — returns a.rs, b.rs
        mock.expect_run_command()
            .withf(|_, args: &[String]| args.iter().any(|a| a == "--merge-base"))
            .times(1)
            .in_sequence(&mut seq)
            .returning(|_, _| Ok("a.rs\0b.rs\0".to_string()));

        // Second listing: uncommitted — returns b.rs, c.rs
        mock.expect_run_command()
            .withf(|_, args: &[String]| {
                !args.iter().any(|a| a == "--merge-base")
                    && args.iter().any(|a| a == "HEAD")
                    && args.iter().any(|a| a == DIFF_FILTER)
            })
            .times(1)
            .in_sequence(&mut seq)
            .returning(|_, _| Ok("b.rs\0c.rs\0".to_string()));

        // Third listing: untracked (ls-files --others --exclude-standard) — returns c.rs, d.rs
        mock.expect_run_command()
            .withf(|_, args: &[String]| {
                args.iter().any(|a| a == "ls-files")
                    && args.iter().any(|a| a == "--others")
                    && args.iter().any(|a| a == "--exclude-standard")
            })
            .times(1)
            .in_sequence(&mut seq)
            .returning(|_, _| Ok("c.rs\0d.rs\0".to_string()));

        let result = get_changed_files_with_executor(Path::new("/tmp"), "main", &mock).unwrap();

        // Committed-first order, untracked appended, deduped across all three sources
        assert_eq!(result.files, vec!["a.rs", "b.rs", "c.rs", "d.rs"]);
        assert_eq!(result.base_ref, "main");
    }

    #[test]
    fn get_changed_files_includes_untracked_when_no_diff() {
        let mut mock = mock_resolving();

        mock.expect_run_command()
            .withf(|_, args: &[String]| args.iter().any(|a| a == DIFF_FILTER))
            .times(2)
            .returning(|_, _| Ok(String::new()));

        // Mock returns only non-gitignored paths — the real git binary applies
        // --exclude-standard, we just test plumbing.
        mock.expect_run_command()
            .withf(|_, args: &[String]| args.iter().any(|a| a == "ls-files"))
            .times(1)
            .returning(|_, _| Ok("new_file.rs\0src/new_mod.rs\0".to_string()));

        let result = get_changed_files_with_executor(Path::new("/tmp"), "main", &mock).unwrap();
        assert_eq!(result.files, vec!["new_file.rs", "src/new_mod.rs"]);
    }

    #[test]
    fn get_changed_files_committed_failure_propagates() {
        let mut mock = mock_resolving();
        mock.expect_run_command()
            .withf(|_, args: &[String]| args.iter().any(|a| a == "--merge-base"))
            .times(1)
            .returning(|_, _| Err(anyhow::anyhow!("fatal: no merge base found")));

        let err = get_changed_files_with_executor(Path::new("/tmp"), "main", &mock).unwrap_err();
        assert_eq!(format!("{err:#}"), "fatal: no merge base found");
    }

    fn default_git_config() -> crate::config::GitConfig {
        crate::config::GitConfig {
            base_branch: "main".to_string(),
            fallback_branch: "HEAD~1".to_string(),
        }
    }

    #[test]
    fn detect_changes_errors_when_uncommitted_diff_fails() {
        let mut mock = mock_resolving();

        mock.expect_run_command()
            .withf(|_, args: &[String]| args.iter().any(|a| a == "--merge-base"))
            .times(1)
            .returning(|_, _| Ok("committed.rs\0".to_string()));

        mock.expect_run_command()
            .withf(|_, args: &[String]| {
                !args.iter().any(|a| a == "--merge-base") && args.iter().any(|a| a == DIFF_FILTER)
            })
            .times(1)
            .returning(|_, _| Err(anyhow::anyhow!("uncommitted diff blew up")));

        let result = detect_changes_with_executor(Path::new("/tmp"), &default_git_config(), &mock);
        let msg = result.unwrap_err().to_string();
        assert!(msg.contains("uncommitted diff blew up"), "got: {msg}");
    }

    #[test]
    fn detect_changes_errors_when_untracked_list_fails() {
        let mut mock = mock_resolving();

        mock.expect_run_command()
            .withf(|_, args: &[String]| args.iter().any(|a| a == "--merge-base"))
            .times(1)
            .returning(|_, _| Ok("a.rs\0".to_string()));

        mock.expect_run_command()
            .withf(|_, args: &[String]| {
                !args.iter().any(|a| a == "--merge-base") && args.iter().any(|a| a == DIFF_FILTER)
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
    fn get_staged_files_reads_only_the_index() {
        let mut mock = MockGitExecutor::new();
        // Index diff + deleted-on-disk list only: no base ref, unstaged diff or untracked
        mock.expect_run_command()
            .withf(|_, args: &[String]| {
                args == ["diff", "-z", "--cached", "--name-only", DIFF_FILTER]
            })
            .times(1)
            .returning(|_, _| Ok("a.rs\0src/b.rs\0gone.rs\0".to_string()));
        mock.expect_run_command()
            .withf(|_, args: &[String]| {
                args == ["ls-files", "-z", "--deleted", "--full-name", "--", ":/"]
            })
            .times(1)
            .returning(|_, _| Ok("gone.rs\0".to_string()));

        let result = get_staged_files_with_executor(Path::new("/tmp"), &mock).unwrap();

        // gone.rs staged but deleted on disk: dropped
        assert_eq!(result.files, vec!["a.rs", "src/b.rs"]);
        assert_eq!(result.base_ref, STAGED_BASE_REF);
        assert!(result.is_staged());
        assert!(!result.is_cli_files());
    }

    #[test]
    fn path_listings_are_nul_separated() {
        let mut mock = mock_resolving();
        // Right after the subcommand: before `--end-of-options` / `--`
        mock.expect_run_command()
            .withf(|_, args: &[String]| args[1] == "-z")
            .times(3)
            .returning(|_, _| Ok(String::new()));
        get_changed_files_with_executor(Path::new("/tmp"), "main", &mock).unwrap();
    }

    #[test]
    fn nul_separated_paths_keep_quotes_newlines_and_unicode() {
        let mut mock = MockGitExecutor::new();
        mock.expect_run_command()
            .withf(|_, args: &[String]| args.iter().any(|a| a == "--merge-base"))
            .returning(|_, _| Ok("src/café.py\0src/new\nline.py\0".to_string()));
        mock.expect_run_command()
            .withf(|_, args: &[String]| !args.iter().any(|a| a == "--merge-base"))
            .returning(|_, _| Ok("src/a\"b.py\0src/back\\slash.py\0".to_string()));

        let result = get_changed_files_with_executor(Path::new("/tmp"), "main", &mock).unwrap();
        assert_eq!(
            result.files,
            vec![
                "src/café.py",
                "src/new\nline.py",
                "src/a\"b.py",
                "src/back\\slash.py"
            ]
        );
    }

    #[test]
    fn non_utf8_output_names_the_bad_entry() {
        let err = NonUtf8Output::new(b"ok.rs\0src/caf\xff.py\0");
        assert_eq!(
            err.to_string(),
            "git printed a non-UTF-8 path `src/caf\u{fffd}.py`: ci-tui needs UTF-8 file names, \
             rename it (or bypass git with --files)"
        );
    }

    #[test]
    fn non_utf8_output_keeps_newlines_inside_names() {
        // `-z` entry with a newline: named whole, not just its last line
        let err = NonUtf8Output::new(b"ok.rs\0dir\nsub/caf\xff.py\0");
        assert_eq!(err.0, "dir\nsub/caf\u{fffd}.py");
        // Non-`-z` output (rev-parse): trailing newline dropped
        assert_eq!(
            NonUtf8Output::new(b"/caf\xff/repo\n").0,
            "/caf\u{fffd}/repo"
        );
    }

    /// [`mock_resolving`] whose committed diff prints a non-UTF-8 path
    fn mock_non_utf8_diff() -> MockGitExecutor {
        let mut mock = mock_resolving();
        mock.expect_run_command()
            .withf(|_, args: &[String]| args.iter().any(|a| a == "--merge-base"))
            .times(1)
            .returning(|_, _| Err(NonUtf8Output::new(b"caf\xff.py\0").into()));
        mock
    }

    #[test]
    fn detect_changes_does_not_fall_back_past_non_utf8_output() {
        // Only origin/main is diffed: falling back would use the wrong base
        let mock = mock_non_utf8_diff();
        let err = detect_changes_with_executor(Path::new("/tmp"), &default_git_config(), &mock)
            .unwrap_err();
        assert!(err.is::<NonUtf8Output>(), "got: {err:#}");
    }

    #[test]
    fn get_changed_files_non_utf8_is_not_a_bad_ref() {
        let mock = mock_non_utf8_diff();
        let err = get_changed_files_with_executor(Path::new("/tmp"), "main", &mock).unwrap_err();
        assert!(err.is::<NonUtf8Output>(), "got: {err:#}");
    }

    #[test]
    fn non_utf8_output_is_found_through_context() {
        let mut mock = MockGitExecutor::new();
        mock.expect_run_command()
            .returning(|_, _| Err(NonUtf8Output::new(b"caf\xff.py\0").into()));

        let err = get_staged_files_with_executor(Path::new("/tmp"), &mock).unwrap_err();
        assert!(format!("{err:#}").starts_with("could not read staged files"));
        assert!(err.is::<NonUtf8Output>(), "got: {err:#}");
    }

    #[test]
    fn get_staged_files_propagates_git_error() {
        let mut mock = MockGitExecutor::new();
        mock.expect_run_command()
            .times(1)
            .returning(|_, _| Err(anyhow::anyhow!("not a git repository")));

        let err = get_staged_files_with_executor(Path::new("/tmp"), &mock).unwrap_err();
        assert!(
            format!("{err:#}").contains("not a git repository"),
            "got: {err:#}"
        );
    }

    /// [`GitFailed`] of the real binary running `args` in `dir`
    fn git_failure(dir: &Path, args: &[&str]) -> GitFailed {
        let args: Vec<String> = args.iter().map(|a| a.to_string()).collect();
        RealGitExecutor
            .run_command(dir, &args)
            .unwrap_err()
            .downcast::<GitFailed>()
            .unwrap()
    }

    #[test]
    fn real_executor_keeps_exit_code_of_missing_ref() {
        if crate::watch::tests::in_git_hook() {
            return;
        }
        let tmp = tempfile::TempDir::new().unwrap();
        let init = Command::new("git")
            .args(["init", "-q"])
            .current_dir(tmp.path())
            .status()
            .unwrap();
        assert!(init.success());

        let failed = git_failure(tmp.path(), &["rev-parse", "--verify", "--quiet", "nosuch"]);
        assert_eq!(failed.code, Some(1));
    }

    #[test]
    fn real_executor_keeps_exit_code_outside_a_repo() {
        let tmp = tempfile::TempDir::new().unwrap();
        // Explicit dir: a plain tempdir could sit in a work tree, or a hook's
        // GIT_DIR could redirect git to one
        let args = [
            "--git-dir=not-a-repo",
            "rev-parse",
            "--verify",
            "--quiet",
            "nosuch",
        ];
        let failed = git_failure(tmp.path(), &args);
        assert_eq!(failed.code, Some(128));
        assert_eq!(
            failed.to_string(),
            format!("git command failed: {}", failed.stderr)
        );
        assert!(failed.stderr.contains("not a git repository"), "{failed}");
    }

    #[test]
    fn is_staged_false_for_base_ref_and_cli_files() {
        assert!(!make_changed_files(vec![]).is_staged());
        let cli = ChangedFiles {
            files: vec![],
            base_ref: CLI_FILES_BASE_REF.to_string(),
        };
        assert!(!cli.is_staged());
    }

    #[test]
    fn cli_files_base_ref_is_human_readable() {
        assert_eq!(CLI_FILES_BASE_REF, "--files (manual)");
        assert_ne!(CLI_FILES_BASE_REF, "cli");
    }
}
