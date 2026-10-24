//! Tests for git operations
//!
//! Most use MockGitExecutor to verify git command parsing logic without a
//! Git repository; `real_repo` runs the binary against one.

mod common;

use ci_tui::config::GitConfig;
use ci_tui::git::{
    current_branch_with_executor, detect_changes_with_executor, get_changed_files_with_executor,
    short_commit_with_executor, MockGitExecutor,
};
use common::{assert_code, git, mock_git_error, mock_git_with_output, run_ci_tui_env, write};
use rstest::rstest;
use std::path::Path;

// Module organization: group tests by function
mod get_changed_files {
    use super::*;
    use pretty_assertions::assert_eq;

    #[test]
    fn success_with_multiple_files() {
        let mock = mock_git_with_output("src/foo.rs\0src/bar.rs\0tests/test.rs\0");
        let result = get_changed_files_with_executor(Path::new("/tmp"), "main", &mock).unwrap();

        assert_eq!(result.files.len(), 3);
        assert!(result.files.contains(&"src/foo.rs".to_string()));
        assert!(result.files.contains(&"src/bar.rs".to_string()));
        assert!(result.files.contains(&"tests/test.rs".to_string()));
        assert_eq!(result.base_ref, "main");
    }

    #[test]
    fn success_with_empty_result() {
        let mock = mock_git_with_output("");
        let result = get_changed_files_with_executor(Path::new("/tmp"), "main", &mock).unwrap();

        assert!(result.files.is_empty());
        assert_eq!(result.base_ref, "main");
    }

    #[test]
    fn skips_empty_entries() {
        let mock = mock_git_with_output("file1.rs\0\0file2.rs\0\0\0");
        let result = get_changed_files_with_executor(Path::new("/tmp"), "main", &mock).unwrap();

        // Empty entries should be filtered out
        assert_eq!(result.files.len(), 2);
        assert!(result.files.contains(&"file1.rs".to_string()));
        assert!(result.files.contains(&"file2.rs".to_string()));
    }

    #[test]
    fn error_propagation() {
        let mock = mock_git_error("fatal: bad revision 'invalid-ref'");
        let result = get_changed_files_with_executor(Path::new("/tmp"), "invalid-ref", &mock);

        assert!(result.is_err());
        let err_msg = format!("{:#}", result.unwrap_err());
        assert!(err_msg.contains("bad revision"));
        assert!(err_msg.contains("invalid-ref"), "error should name the ref");
    }

    #[test]
    fn committed_diff_guards_ref_against_options_and_paths() {
        let mut mock = MockGitExecutor::new();
        mock.expect_run_command()
            .withf(|_, args: &[String]| {
                args.iter().any(|a| a == "--merge-base")
                    && args.ends_with(&[
                        "--end-of-options".to_string(),
                        "--cached".to_string(),
                        "HEAD".to_string(),
                        "--".to_string(),
                    ])
            })
            .times(1)
            .returning(|_, _| Ok(String::new()));
        mock.expect_run_command()
            .withf(|_, args: &[String]| !args.iter().any(|a| a == "--merge-base"))
            .times(2)
            .returning(|_, _| Ok(String::new()));

        get_changed_files_with_executor(Path::new("/tmp"), "--cached", &mock).unwrap();
    }

    #[rstest]
    #[case("origin/main", vec!["src/foo.rs"])]
    #[case("main", vec!["src/bar.rs", "tests/test.rs"])]
    #[case("HEAD~1", vec![])]
    fn various_base_refs(#[case] base_ref: &str, #[case] expected_files: Vec<&str>) {
        let output = expected_files.join("\0");
        let mock = mock_git_with_output(&output);

        let result = get_changed_files_with_executor(Path::new("/tmp"), base_ref, &mock).unwrap();

        assert_eq!(result.files.len(), expected_files.len());
        assert_eq!(result.base_ref, base_ref);
        for file in expected_files {
            assert!(result.files.contains(&file.to_string()));
        }
    }
}

mod detect_changes {
    use super::*;
    use pretty_assertions::assert_eq;

    #[test]
    fn fallback_from_origin_to_base() {
        let mut mock = MockGitExecutor::new();

        // First call: committed diff against origin/master — fails.
        mock.expect_run_command()
            .withf(|_, args: &[String]| args.iter().any(|a| a == "origin/master"))
            .times(1)
            .returning(|_, _| Err(anyhow::anyhow!("Reference not found")));

        // Next: committed diff against master — succeeds.
        mock.expect_run_command()
            .withf(|_, args: &[String]| {
                args.iter().any(|a| a == "master") && args.iter().any(|a| a == "--merge-base")
            })
            .times(1)
            .returning(|_, _| Ok("file1.rs\0".to_string()));

        // Then: uncommitted diff + untracked ls-files (no --merge-base) — succeed.
        mock.expect_run_command()
            .withf(|_, args: &[String]| !args.iter().any(|a| a == "--merge-base"))
            .times(2)
            .returning(|_, _| Ok(String::new()));

        let config = GitConfig {
            base_branch: "master".to_string(),
            fallback_branch: "HEAD~1".to_string(),
        };

        let result = detect_changes_with_executor(Path::new("/tmp"), &config, &mock).unwrap();

        assert_eq!(result.files, vec!["file1.rs"]);
        assert_eq!(result.base_ref, "master");
    }

    #[test]
    fn fallback_to_fallback_branch() {
        let mut mock = MockGitExecutor::new();

        // First two committed calls (origin/master, master) fail.
        mock.expect_run_command()
            .withf(|_, args: &[String]| {
                args.iter().any(|a| a == "--merge-base")
                    && (args.iter().any(|a| a == "origin/master")
                        || args.iter().any(|a| a == "master"))
            })
            .times(2)
            .returning(|_, _| Err(anyhow::anyhow!("Reference not found")));

        // Committed against HEAD~1 — succeeds.
        mock.expect_run_command()
            .withf(|_, args: &[String]| {
                args.iter().any(|a| a == "--merge-base") && args.iter().any(|a| a == "HEAD~1")
            })
            .times(1)
            .returning(|_, _| Ok("file2.rs\0".to_string()));

        // Uncommitted + untracked — succeed with no output.
        mock.expect_run_command()
            .withf(|_, args: &[String]| !args.iter().any(|a| a == "--merge-base"))
            .times(2)
            .returning(|_, _| Ok(String::new()));

        let config = GitConfig {
            base_branch: "master".to_string(),
            fallback_branch: "HEAD~1".to_string(),
        };

        let result = detect_changes_with_executor(Path::new("/tmp"), &config, &mock).unwrap();

        assert_eq!(result.files, vec!["file2.rs"]);
        assert_eq!(result.base_ref, "HEAD~1");
    }

    #[test]
    fn errors_when_all_refs_fail_listing_refs_tried() {
        let mock = mock_git_error("fatal: not a git repository");

        let config = GitConfig {
            base_branch: "master".to_string(),
            fallback_branch: "HEAD~1".to_string(),
        };

        let result = detect_changes_with_executor(Path::new("/tmp"), &config, &mock);

        assert!(result.is_err());
        let msg = result.unwrap_err().to_string();
        assert!(msg.contains("origin/master"), "msg was: {msg}");
        assert!(msg.contains("master"), "msg was: {msg}");
        assert!(msg.contains("HEAD~1"), "msg was: {msg}");
    }

    #[test]
    fn uses_origin_base_first() {
        let mut mock = MockGitExecutor::new();

        // Committed against origin/development succeeds on first try.
        mock.expect_run_command()
            .withf(|_, args: &[String]| {
                args.iter().any(|a| a == "--merge-base")
                    && args.iter().any(|a| a == "origin/development")
            })
            .times(1)
            .returning(|_, _| Ok("src/main.rs\0".to_string()));

        // Uncommitted + untracked follow.
        mock.expect_run_command()
            .withf(|_, args: &[String]| !args.iter().any(|a| a == "--merge-base"))
            .times(2)
            .returning(|_, _| Ok(String::new()));

        let config = GitConfig {
            base_branch: "development".to_string(),
            fallback_branch: "HEAD~1".to_string(),
        };

        let result = detect_changes_with_executor(Path::new("/tmp"), &config, &mock).unwrap();

        assert_eq!(result.base_ref, "origin/development");
        assert_eq!(result.files, vec!["src/main.rs"]);
    }
}

mod current_branch {
    use super::*;
    use pretty_assertions::assert_eq;

    #[test]
    fn returns_trimmed_branch_name() {
        let mock = mock_git_with_output("feature/test-branch\n");
        let result = current_branch_with_executor(Path::new("/tmp"), &mock).unwrap();

        assert_eq!(result, "feature/test-branch");
    }

    #[test]
    fn handles_detached_head() {
        let mock = mock_git_with_output("HEAD\n");
        let result = current_branch_with_executor(Path::new("/tmp"), &mock).unwrap();

        assert_eq!(result, "HEAD");
    }

    #[test]
    fn trims_whitespace() {
        let mock = mock_git_with_output("  main  \n\n");
        let result = current_branch_with_executor(Path::new("/tmp"), &mock).unwrap();

        assert_eq!(result, "main");
    }

    #[rstest]
    #[case("master\n", "master")]
    #[case("feature/foo-bar\n", "feature/foo-bar")]
    #[case("release/v1.0.0\n", "release/v1.0.0")]
    fn various_branch_names(#[case] output: &str, #[case] expected: &str) {
        let mock = mock_git_with_output(output);
        let result = current_branch_with_executor(Path::new("/tmp"), &mock).unwrap();

        assert_eq!(result, expected);
    }
}

mod short_commit {
    use super::*;
    use pretty_assertions::assert_eq;

    #[test]
    fn returns_trimmed_commit_hash() {
        let mock = mock_git_with_output("abc1234\n");
        let result = short_commit_with_executor(Path::new("/tmp"), &mock).unwrap();

        assert_eq!(result, "abc1234");
    }

    #[test]
    fn trims_whitespace() {
        let mock = mock_git_with_output("  def5678  \n");
        let result = short_commit_with_executor(Path::new("/tmp"), &mock).unwrap();

        assert_eq!(result, "def5678");
    }

    #[rstest]
    #[case("a1b2c3d\n", "a1b2c3d")]
    #[case("0000000\n", "0000000")]
    #[case("f9e8d7c\n", "f9e8d7c")]
    fn various_commit_hashes(#[case] output: &str, #[case] expected: &str) {
        let mock = mock_git_with_output(output);
        let result = short_commit_with_executor(Path::new("/tmp"), &mock).unwrap();

        assert_eq!(result, expected);
    }
}

mod real_repo {
    use super::*;
    use ci_tui::exit;

    // Edge case: `py` fails on every run and prints each file it gets
    const CONFIG: &str = r#"version: 2
runner: local
git:
  base_branch: main
  fallback_branch: HEAD~1
file_patterns:
  py:
    pattern: '^src/.*\.py$'
checks:
  g:
    checks:
      py:
        name: Py
        command: "printf '[%s]\n' {files}; false"
        triggers:
          file_pattern: py
"#;

    /// git's default, which C-quotes non-ASCII names, set even if the user's
    /// global config turns it off
    const QUOTE_PATH: [(&str, &str); 3] = [
        ("GIT_CONFIG_COUNT", "1"),
        ("GIT_CONFIG_KEY_0", "core.quotePath"),
        ("GIT_CONFIG_VALUE_0", "true"),
    ];

    /// Repo on branch `feature` off `main` (which has the config), committing
    /// `src/café.py` and leaving `src/a"b.py` untracked
    fn feature_repo() -> tempfile::TempDir {
        let tmp = tempfile::TempDir::new().unwrap();
        let dir = tmp.path();
        let identity = ["-c", "user.name=t", "-c", "user.email=t@t"];
        let commit = [&identity[..], &["commit", "-q", "-m", "c"]].concat();
        git(dir, &["init", "-q", "-b", "main"]);
        write(dir, "ci-tui.yaml", CONFIG);
        git(dir, &["add", "."]);
        git(dir, &commit);
        git(dir, &["checkout", "-q", "-b", "feature"]);
        write(dir, "src/café.py", "bad\n");
        git(dir, &["add", "."]);
        git(dir, &commit);
        write(dir, "src/a\"b.py", "bad\n");
        tmp
    }

    #[test]
    fn quoted_names_reach_their_check() {
        let tmp = feature_repo();
        let out = run_ci_tui_env(tmp.path(), &["--simple", "--no-color"], &QUOTE_PATH);
        assert_code(&out, exit::CHECKS_FAILED);
        let stdout = String::from_utf8_lossy(&out.stdout);
        assert!(stdout.contains("[src/café.py]"), "{stdout}");
        assert!(stdout.contains("[src/a\"b.py]"), "{stdout}");
    }

    /// Linux file systems accept non-UTF-8 names (macOS ones don't)
    #[cfg(target_os = "linux")]
    #[test]
    fn non_utf8_name_fails_detection() {
        use std::os::unix::ffi::OsStrExt;
        let tmp = feature_repo();
        let name = std::ffi::OsStr::from_bytes(b"caf\xff.py");
        std::fs::write(tmp.path().join("src").join(name), "bad\n").unwrap();

        let out = run_ci_tui_env(tmp.path(), &["--simple", "--no-color"], &QUOTE_PATH);
        assert_code(&out, exit::ENV_ERROR);
        let stderr = String::from_utf8_lossy(&out.stderr);
        assert!(
            stderr.contains("non-UTF-8 path `src/caf\u{fffd}.py`"),
            "{stderr}"
        );
        assert!(!stderr.contains("No base ref"), "{stderr}");
    }
}
