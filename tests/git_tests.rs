//! Mock-based tests for git operations
//!
//! These tests use MockGitExecutor to verify git command parsing logic
//! without requiring an actual Git repository.

mod common;

use ci_tui::git::{detect_changes_with_executor, get_changed_files_with_executor,
                   current_branch_with_executor, short_commit_with_executor, MockGitExecutor};
use ci_tui::config::GitConfig;
use common::{mock_git_error, mock_git_with_output};
use rstest::rstest;
use std::path::Path;

// Module organization: group tests by function
mod get_changed_files {
    use super::*;
    use pretty_assertions::assert_eq;

    #[test]
    fn success_with_multiple_files() {
        let mock = mock_git_with_output("src/foo.rs\nsrc/bar.rs\ntests/test.rs\n");
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
    fn handles_trailing_newlines() {
        let mock = mock_git_with_output("file1.rs\n\nfile2.rs\n\n\n");
        let result = get_changed_files_with_executor(Path::new("/tmp"), "main", &mock).unwrap();

        // Empty lines should be filtered out
        assert_eq!(result.files.len(), 2);
        assert!(result.files.contains(&"file1.rs".to_string()));
        assert!(result.files.contains(&"file2.rs".to_string()));
    }

    #[test]
    fn error_propagation() {
        let mock = mock_git_error("fatal: bad revision 'invalid-ref'");
        let result = get_changed_files_with_executor(Path::new("/tmp"), "invalid-ref", &mock);

        assert!(result.is_err());
        let err_msg = result.unwrap_err().to_string();
        assert!(err_msg.contains("bad revision"));
    }

    #[rstest]
    #[case("origin/main", vec!["src/foo.rs"])]
    #[case("main", vec!["src/bar.rs", "tests/test.rs"])]
    #[case("HEAD~1", vec![])]
    fn various_base_refs(#[case] base_ref: &str, #[case] expected_files: Vec<&str>) {
        let output = expected_files.join("\n");
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

        // First call (origin/master) fails
        mock.expect_run_command()
            .times(1)
            .returning(|_, args: &[String]| {
                if args.contains(&"origin/master".to_string()) {
                    Err(anyhow::anyhow!("Reference not found"))
                } else {
                    Ok("file1.rs\n".to_string())
                }
            });

        // Second call (master) succeeds
        mock.expect_run_command()
            .times(1)
            .returning(|_, _| Ok("file1.rs\n".to_string()));

        let config = GitConfig {
            base_branch: "master".to_string(),
            fallback_branch: "HEAD~1".to_string(),
        };

        let result = detect_changes_with_executor(Path::new("/tmp"), &config, &mock).unwrap();

        assert_eq!(result.files.len(), 1);
        assert!(result.files.contains(&"file1.rs".to_string()));
    }

    #[test]
    fn fallback_to_fallback_branch() {
        let mut mock = MockGitExecutor::new();

        // First two calls fail
        mock.expect_run_command()
            .times(2)
            .returning(|_, _| Err(anyhow::anyhow!("Reference not found")));

        // Third call (fallback) succeeds
        mock.expect_run_command()
            .times(1)
            .returning(|_, _| Ok("file2.rs\n".to_string()));

        let config = GitConfig {
            base_branch: "master".to_string(),
            fallback_branch: "HEAD~1".to_string(),
        };

        let result = detect_changes_with_executor(Path::new("/tmp"), &config, &mock).unwrap();

        assert_eq!(result.files.len(), 1);
        assert!(result.files.contains(&"file2.rs".to_string()));
        assert_eq!(result.base_ref, "HEAD~1");
    }

    #[test]
    fn returns_empty_when_all_refs_fail() {
        let mock = mock_git_error("fatal: not a git repository");

        let config = GitConfig {
            base_branch: "master".to_string(),
            fallback_branch: "HEAD~1".to_string(),
        };

        let result = detect_changes_with_executor(Path::new("/tmp"), &config, &mock).unwrap();

        assert!(result.files.is_empty());
        assert_eq!(result.base_ref, "HEAD");
    }

    #[test]
    fn uses_origin_base_first() {
        let mut mock = MockGitExecutor::new();

        // First call (origin/development) succeeds
        mock.expect_run_command()
            .times(1)
            .returning(|_, args: &[String]| {
                if args.contains(&"origin/development".to_string()) {
                    Ok("src/main.rs\n".to_string())
                } else {
                    panic!("Should try origin/development first");
                }
            });

        let config = GitConfig {
            base_branch: "development".to_string(),
            fallback_branch: "HEAD~1".to_string(),
        };

        let result = detect_changes_with_executor(Path::new("/tmp"), &config, &mock).unwrap();

        assert_eq!(result.base_ref, "origin/development");
        assert!(result.files.contains(&"src/main.rs".to_string()));
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
