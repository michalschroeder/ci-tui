//! Finding related test files when source files change.
//!
//! This module implements test discovery strategies to automatically find which
//! test files should run when source files are modified. It supports path mapping
//! (e.g., `src/Foo.php` -> `tests/FooTest.php`) and grep-based content search.
//!
//! # Strategies
//!
//! - **Path Mapping**: Maps source paths to test paths using configurable rules
//! - **Grep Search**: Searches test directories for files containing references
//!   to the changed source files
//!
//! # Key Functions
//!
//! - [`find_related_tests`]: Main entry point for test discovery

use crate::config::{PathMappingRule, TestDiscoveryStrategy};
use std::collections::HashSet;
use std::path::Path;

/// Output of a grep-style process invocation.
#[derive(Debug, Clone)]
pub(crate) struct ProcessOutput {
    pub success: bool,
    pub stdout: String,
}

/// Abstraction over the grep invocation so tests can inject controlled outputs.
#[cfg_attr(any(test, feature = "test"), mockall::automock)]
pub(crate) trait ProcessRunner: Send + Sync {
    fn run(&self, dir: &Path, args: &[String]) -> std::io::Result<ProcessOutput>;
}

/// Production implementation — shells out to `grep`.
pub(crate) struct RealProcessRunner;

impl ProcessRunner for RealProcessRunner {
    fn run(&self, dir: &Path, args: &[String]) -> std::io::Result<ProcessOutput> {
        let output = std::process::Command::new("grep")
            .args(args)
            .current_dir(dir)
            .output()?;
        Ok(ProcessOutput {
            success: output.status.success(),
            stdout: String::from_utf8_lossy(&output.stdout).to_string(),
        })
    }
}

/// Find related test files for a list of source files using provided strategies
pub fn find_related_tests(
    strategies: &[TestDiscoveryStrategy],
    source_files: &[&str],
    project_root: &Path,
) -> Vec<String> {
    find_related_tests_with_runner(strategies, source_files, project_root, &RealProcessRunner)
}

pub(crate) fn find_related_tests_with_runner(
    strategies: &[TestDiscoveryStrategy],
    source_files: &[&str],
    project_root: &Path,
    runner: &dyn ProcessRunner,
) -> Vec<String> {
    let mut all_tests: HashSet<String> = HashSet::new();

    for strategy in strategies {
        all_tests.extend(apply_strategy_with_runner(
            strategy,
            source_files,
            project_root,
            runner,
        ));
    }

    let mut result: Vec<String> = all_tests.into_iter().collect();
    result.sort();
    result
}

/// Apply a single discovery strategy to source files
fn apply_strategy_with_runner(
    strategy: &TestDiscoveryStrategy,
    source_files: &[&str],
    project_root: &Path,
    runner: &dyn ProcessRunner,
) -> Vec<String> {
    match strategy {
        TestDiscoveryStrategy::PathMapping { rules } => source_files
            .iter()
            .flat_map(|f| apply_path_mapping(f, rules, project_root))
            .collect(),
        TestDiscoveryStrategy::GrepSearch {
            search_dirs,
            pattern,
        } => source_files
            .iter()
            .flat_map(|f| grep_search_with_runner(f, search_dirs, pattern, project_root, runner))
            .collect(),
    }
}

/// Apply path mapping rules to find test files
fn apply_path_mapping(
    source_file: &str,
    rules: &[PathMappingRule],
    project_root: &Path,
) -> Vec<String> {
    let mut found_tests = Vec::new();

    for rule in rules {
        let Some(path_capture) = extract_path_from_pattern(source_file, &rule.source) else {
            continue;
        };
        found_tests.extend(rule.tests.iter().filter_map(|test_pattern| {
            let test_path = test_pattern.replace("{path}", &path_capture);
            let full_path = project_root.join(&test_path);
            full_path.exists().then_some(test_path)
        }));
    }

    found_tests
}

/// Extract the {path} variable from a source file based on a pattern
/// e.g., "src/Platform/Service/Foo.php" with pattern "src/{path}.php" returns "Platform/Service/Foo"
fn extract_path_from_pattern(file: &str, pattern: &str) -> Option<String> {
    // Pattern like "src/{path}.php" - find prefix and suffix
    let parts: Vec<&str> = pattern.split("{path}").collect();
    if parts.len() != 2 {
        return None;
    }

    let prefix = parts[0];
    let suffix = parts[1];

    if file.starts_with(prefix) && file.ends_with(suffix) {
        let start = prefix.len();
        let end = file.len() - suffix.len();
        if start < end {
            return Some(file[start..end].to_string());
        }
    }

    None
}

/// Search test files for content matching a pattern with placeholders
fn grep_search_with_runner(
    source_file: &str,
    search_dirs: &[String],
    pattern: &str,
    project_root: &Path,
    runner: &dyn ProcessRunner,
) -> Vec<String> {
    let expanded_pattern = expand_placeholders(pattern, source_file);
    let args = ["-rl".to_string(), expanded_pattern, ".".to_string()];
    let mut found_tests = Vec::new();

    for search_dir in search_dirs {
        let dir_path = project_root.join(search_dir);
        if !dir_path.exists() {
            continue;
        }

        let output = match runner.run(&dir_path, &args) {
            Ok(o) => o,
            Err(_) => continue,
        };
        if !output.success {
            continue;
        }
        for line in output.stdout.lines() {
            let relative_path = line.trim().strip_prefix("./").unwrap_or(line.trim());
            let test_path = format!("{}/{}", search_dir, relative_path);
            found_tests.push(test_path);
        }
    }

    found_tests
}

#[cfg(test)]
fn grep_search(
    source_file: &str,
    search_dirs: &[String],
    pattern: &str,
    project_root: &Path,
) -> Vec<String> {
    grep_search_with_runner(
        source_file,
        search_dirs,
        pattern,
        project_root,
        &RealProcessRunner,
    )
}

/// Expand placeholders in pattern based on source file path
///
/// Available placeholders:
/// - {basename}  - filename without extension (e.g., "UserService")
/// - {filename}  - filename with extension (e.g., "UserService.php")
/// - {extension} - file extension only (e.g., "php")
/// - {dirname}   - directory path (e.g., "src/Platform/Service")
/// - {path}      - full path without extension (e.g., "src/Platform/Service/UserService")
fn expand_placeholders(pattern: &str, source_file: &str) -> String {
    let path = Path::new(source_file);

    let basename = path.file_stem().and_then(|s| s.to_str()).unwrap_or("");
    let filename = path.file_name().and_then(|s| s.to_str()).unwrap_or("");
    let extension = path.extension().and_then(|s| s.to_str()).unwrap_or("");
    let dirname = path.parent().and_then(|p| p.to_str()).unwrap_or("");
    let full_path = path
        .with_extension("")
        .to_str()
        .map(|s| s.to_string())
        .unwrap_or_default();

    pattern
        .replace("{basename}", basename)
        .replace("{filename}", filename)
        .replace("{extension}", extension)
        .replace("{dirname}", dirname)
        .replace("{path}", &full_path)
}

#[cfg(test)]
mod tests {
    use super::*;
    use rstest::rstest;

    mod test_extract_path_from_pattern {
        use super::*;

        #[rstest]
        #[case(
            "src/Platform/Service/Foo.php",
            "src/{path}.php",
            Some("Platform/Service/Foo")
        )]
        #[case(
            "lib/src/Money/Currency.php",
            "lib/src/{path}.php",
            Some("Money/Currency")
        )]
        #[case("tests/something.php", "src/{path}.php", None)] // Wrong prefix
        #[case("src/Foo.ts", "src/{path}.php", None)] // Wrong suffix
        #[case("", "src/{path}.php", None)] // Empty file
        #[case("src/Foo.php", "", None)] // Empty pattern
        #[case("src/Foo.php", "no-placeholder", None)] // No {path} placeholder
        #[case("src/.php", "src/{path}.php", None)] // Empty path capture
        #[case("src/Foo/Foo.php", "src/{path}/{path}.php", None)]
        #[case("a/b/c.php", "{path}{path}", None)]
        fn test_extract_path(
            #[case] file: &str,
            #[case] pattern: &str,
            #[case] expected: Option<&str>,
        ) {
            let result = extract_path_from_pattern(file, pattern);
            assert_eq!(result, expected.map(String::from));
        }
    }

    mod test_expand_placeholders {
        use super::*;

        #[rstest]
        #[case("{basename}", "src/Service/UserService.php", "UserService")]
        #[case("{filename}", "src/Service/UserService.php", "UserService.php")]
        #[case("{extension}", "src/Service/UserService.php", "php")]
        #[case("{dirname}", "src/Service/UserService.php", "src/Service")]
        #[case("{path}", "src/Service/UserService.php", "src/Service/UserService")]
        #[case("{basename}Test", "src/Foo.php", "FooTest")]
        #[case(
            "tests/{dirname}/{basename}Test.php",
            "src/Service/Foo.php",
            "tests/src/Service/FooTest.php"
        )]
        fn test_placeholder_expansion(
            #[case] pattern: &str,
            #[case] source: &str,
            #[case] expected: &str,
        ) {
            let result = expand_placeholders(pattern, source);
            assert_eq!(result, expected);
        }

        #[test]
        fn test_expand_placeholders_no_extension() {
            let result = expand_placeholders("{basename}.{extension}", "Makefile");
            assert_eq!(result, "Makefile.");
        }

        #[test]
        fn test_expand_placeholders_hidden_file() {
            let result = expand_placeholders("{basename}-{filename}", ".gitignore");
            assert_eq!(result, ".gitignore-.gitignore");
        }

        #[test]
        fn test_expand_placeholders_root_file() {
            let result = expand_placeholders("{dirname}/{basename}", "README.md");
            assert_eq!(result, "/README");
        }

        #[test]
        fn test_expand_placeholders_all() {
            let pattern = "{dirname}/{basename}.{extension} - {filename} - {path}";
            let source = "src/Platform/Service/UserService.php";

            let expanded = expand_placeholders(pattern, source);
            assert_eq!(
                expanded,
                "src/Platform/Service/UserService.php - UserService.php - src/Platform/Service/UserService"
            );
        }

        #[test]
        fn test_empty_source_returns_empty_placeholders() {
            let result = expand_placeholders("[{basename}][{filename}][{dirname}]", "");
            assert_eq!(result, "[][][]");
        }

        #[test]
        fn test_bare_filename_has_empty_dirname() {
            // Path::parent for "Cargo.toml" returns Some(""), not None — dirname resolves to "".
            let result = expand_placeholders("{dirname}|{basename}|{extension}", "Cargo.toml");
            assert_eq!(result, "|Cargo|toml");
        }
    }

    mod test_apply_path_mapping {
        use super::*;
        use tempfile::TempDir;

        #[test]
        fn test_finds_existing_test_file() {
            let temp_dir = TempDir::new().expect("Failed to create temp dir");
            // Create test file structure
            let test_path = temp_dir.path().join("tests/Unit/FooTest.php");
            std::fs::create_dir_all(test_path.parent().unwrap()).unwrap();
            std::fs::write(&test_path, "<?php").unwrap();

            let rules = vec![PathMappingRule {
                source: "src/{path}.php".to_string(),
                tests: vec!["tests/Unit/{path}Test.php".to_string()],
            }];

            let result = apply_path_mapping("src/Foo.php", &rules, temp_dir.path());
            assert_eq!(result, vec!["tests/Unit/FooTest.php"]);
        }

        #[test]
        fn test_returns_empty_when_test_not_exists() {
            let temp_dir = TempDir::new().expect("Failed to create temp dir");
            // Don't create any files

            let rules = vec![PathMappingRule {
                source: "src/{path}.php".to_string(),
                tests: vec!["tests/Unit/{path}Test.php".to_string()],
            }];

            let result = apply_path_mapping("src/Foo.php", &rules, temp_dir.path());
            assert!(result.is_empty());
        }

        #[test]
        fn test_empty_rules_returns_empty() {
            let temp_dir = TempDir::new().expect("Failed to create temp dir");
            let rules = vec![];

            let result = apply_path_mapping("src/Foo.php", &rules, temp_dir.path());
            assert!(result.is_empty());
        }

        #[test]
        fn test_multiple_test_patterns() {
            let temp_dir = TempDir::new().expect("Failed to create temp dir");

            // Create multiple test files
            let test_path1 = temp_dir.path().join("tests/Unit/FooTest.php");
            std::fs::create_dir_all(test_path1.parent().unwrap()).unwrap();
            std::fs::write(&test_path1, "<?php").unwrap();

            let test_path2 = temp_dir.path().join("tests/Integration/FooTest.php");
            std::fs::create_dir_all(test_path2.parent().unwrap()).unwrap();
            std::fs::write(&test_path2, "<?php").unwrap();

            let rules = vec![PathMappingRule {
                source: "src/{path}.php".to_string(),
                tests: vec![
                    "tests/Unit/{path}Test.php".to_string(),
                    "tests/Integration/{path}Test.php".to_string(),
                ],
            }];

            let result = apply_path_mapping("src/Foo.php", &rules, temp_dir.path());
            assert_eq!(result.len(), 2);
            assert!(result.contains(&"tests/Unit/FooTest.php".to_string()));
            assert!(result.contains(&"tests/Integration/FooTest.php".to_string()));
        }

        #[test]
        fn test_multiple_rules_first_match_wins() {
            let temp_dir = TempDir::new().expect("Failed to create temp dir");

            // Create test file that matches first rule
            let test_path = temp_dir.path().join("tests/Unit/FooTest.php");
            std::fs::create_dir_all(test_path.parent().unwrap()).unwrap();
            std::fs::write(&test_path, "<?php").unwrap();

            let rules = vec![
                PathMappingRule {
                    source: "src/{path}.php".to_string(),
                    tests: vec!["tests/Unit/{path}Test.php".to_string()],
                },
                PathMappingRule {
                    source: "src/{path}.php".to_string(),
                    tests: vec!["tests/Feature/{path}Test.php".to_string()],
                },
            ];

            let result = apply_path_mapping("src/Foo.php", &rules, temp_dir.path());
            // Both rules match, so we get tests from both
            assert_eq!(result.len(), 1);
            assert_eq!(result[0], "tests/Unit/FooTest.php");
        }

        #[test]
        fn test_rule_with_empty_tests_returns_empty() {
            let temp_dir = TempDir::new().expect("Failed to create temp dir");
            let rules = vec![PathMappingRule {
                source: "src/{path}.php".to_string(),
                tests: vec![],
            }];

            let result = apply_path_mapping("src/Foo.php", &rules, temp_dir.path());
            assert!(result.is_empty());
        }

        #[test]
        fn test_rule_matches_but_test_files_missing_returns_empty() {
            let temp_dir = TempDir::new().expect("Failed to create temp dir");
            let rules = vec![PathMappingRule {
                source: "src/{path}.php".to_string(),
                tests: vec!["tests/Unit/{path}Test.php".to_string()],
            }];

            let result = apply_path_mapping("src/Foo.php", &rules, temp_dir.path());
            assert!(result.is_empty());
        }
    }

    mod test_find_related_tests {
        use super::*;
        use crate::config::TestDiscoveryStrategy;
        use tempfile::TempDir;

        #[test]
        fn test_combines_multiple_strategies() {
            let temp_dir = TempDir::new().expect("Failed to create temp dir");

            // Create test file for path mapping
            let test_path1 = temp_dir.path().join("tests/Unit/FooTest.php");
            std::fs::create_dir_all(test_path1.parent().unwrap()).unwrap();
            std::fs::write(&test_path1, "<?php").unwrap();

            // Create test file for grep search
            let test_dir = temp_dir.path().join("tests/Integration");
            std::fs::create_dir_all(&test_dir).unwrap();
            let test_path2 = test_dir.join("FooIntegrationTest.php");
            std::fs::write(&test_path2, "#[CoversClass(Foo::class)]").unwrap();

            let strategies = vec![
                TestDiscoveryStrategy::PathMapping {
                    rules: vec![PathMappingRule {
                        source: "src/{path}.php".to_string(),
                        tests: vec!["tests/Unit/{path}Test.php".to_string()],
                    }],
                },
                TestDiscoveryStrategy::GrepSearch {
                    search_dirs: vec!["tests/Integration".to_string()],
                    pattern: "CoversClass({basename}::class)".to_string(),
                },
            ];

            let result = find_related_tests(&strategies, &["src/Foo.php"], temp_dir.path());
            assert_eq!(result.len(), 2);
            assert!(result.contains(&"tests/Unit/FooTest.php".to_string()));
            assert!(result.contains(&"tests/Integration/FooIntegrationTest.php".to_string()));
        }

        #[test]
        fn test_deduplicates_across_strategies() {
            let temp_dir = TempDir::new().expect("Failed to create temp dir");

            // Create a single test file
            let test_path = temp_dir.path().join("tests/Unit/FooTest.php");
            std::fs::create_dir_all(test_path.parent().unwrap()).unwrap();
            std::fs::write(&test_path, "#[CoversClass(Foo::class)]").unwrap();

            let strategies = vec![
                TestDiscoveryStrategy::PathMapping {
                    rules: vec![PathMappingRule {
                        source: "src/{path}.php".to_string(),
                        tests: vec!["tests/Unit/{path}Test.php".to_string()],
                    }],
                },
                TestDiscoveryStrategy::GrepSearch {
                    search_dirs: vec!["tests/Unit".to_string()],
                    pattern: "CoversClass({basename}::class)".to_string(),
                },
            ];

            let result = find_related_tests(&strategies, &["src/Foo.php"], temp_dir.path());
            // Should only have one entry even though both strategies found it
            assert_eq!(result.len(), 1);
            assert_eq!(result[0], "tests/Unit/FooTest.php");
        }

        #[test]
        fn test_sorts_results() {
            let temp_dir = TempDir::new().expect("Failed to create temp dir");

            // Create multiple test files with names that need sorting
            let test_paths = vec!["tests/ZTest.php", "tests/ATest.php", "tests/MTest.php"];

            for path in &test_paths {
                let full_path = temp_dir.path().join(path);
                std::fs::create_dir_all(full_path.parent().unwrap()).unwrap();
                std::fs::write(&full_path, "<?php").unwrap();
            }

            let strategies = vec![TestDiscoveryStrategy::PathMapping {
                rules: vec![PathMappingRule {
                    source: "src/{path}.php".to_string(),
                    tests: test_paths.iter().map(|s| s.to_string()).collect(),
                }],
            }];

            let result = find_related_tests(&strategies, &["src/Foo.php"], temp_dir.path());
            assert_eq!(result.len(), 3);
            // Results should be sorted alphabetically
            assert_eq!(result[0], "tests/ATest.php");
            assert_eq!(result[1], "tests/MTest.php");
            assert_eq!(result[2], "tests/ZTest.php");
        }

        #[test]
        fn test_empty_source_files() {
            let temp_dir = TempDir::new().expect("Failed to create temp dir");

            let strategies = vec![TestDiscoveryStrategy::PathMapping {
                rules: vec![PathMappingRule {
                    source: "src/{path}.php".to_string(),
                    tests: vec!["tests/Unit/{path}Test.php".to_string()],
                }],
            }];

            let result = find_related_tests(&strategies, &[], temp_dir.path());
            assert!(result.is_empty());
        }

        #[test]
        fn test_empty_strategies_returns_empty() {
            let strategies: Vec<TestDiscoveryStrategy> = vec![];
            let result = find_related_tests(&strategies, &["src/Foo.php"], Path::new("/"));
            assert!(result.is_empty());
        }
    }

    #[test]
    fn test_grep_search_finds_matching_files() {
        use std::fs;
        use tempfile::TempDir;

        // Create temp directory structure
        let temp_dir = TempDir::new().unwrap();
        let search_dir = temp_dir.path().join("tests/Unit");
        fs::create_dir_all(&search_dir).unwrap();

        // Create a test file with CoversClass annotation
        let test_file = search_dir.join("AttachmentTest.php");
        fs::write(
            &test_file,
            r#"<?php
namespace Tests\Unit;

use PHPUnit\Framework\Attributes\CoversClass;
use App\Domain\Attachment;

#[CoversClass(Attachment::class)]
class AttachmentTest extends TestCase
{
}
"#,
        )
        .unwrap();

        // Test grep_search
        let source_file = "src/Domain/Attachment.php";
        let search_dirs = vec!["tests/Unit".to_string()];
        let pattern = "CoversClass({basename}::class)";

        let results = grep_search(source_file, &search_dirs, pattern, temp_dir.path());

        assert_eq!(results.len(), 1);
        assert!(results[0].ends_with("AttachmentTest.php"));
        assert!(results[0].starts_with("tests/Unit"));
    }

    #[test]
    fn test_grep_search_no_matches() {
        use std::fs;
        use tempfile::TempDir;

        let temp_dir = TempDir::new().unwrap();
        let search_dir = temp_dir.path().join("tests/Unit");
        fs::create_dir_all(&search_dir).unwrap();

        // Create a test file WITHOUT the expected annotation
        let test_file = search_dir.join("OtherTest.php");
        fs::write(
            &test_file,
            r#"<?php
class OtherTest extends TestCase {}
"#,
        )
        .unwrap();

        let source_file = "src/Domain/Attachment.php";
        let search_dirs = vec!["tests/Unit".to_string()];
        let pattern = "CoversClass({basename}::class)";

        let results = grep_search(source_file, &search_dirs, pattern, temp_dir.path());

        assert!(results.is_empty());
    }

    #[test]
    fn test_grep_search_nested_directory() {
        use std::fs;
        use tempfile::TempDir;

        let temp_dir = TempDir::new().unwrap();
        let nested_dir = temp_dir.path().join("tests/Unit/Domain/Communication");
        fs::create_dir_all(&nested_dir).unwrap();

        // Create test file in nested directory
        let test_file = nested_dir.join("AttachmentTest.php");
        fs::write(
            &test_file,
            "#[CoversClass(Attachment::class)]\nclass AttachmentTest {}",
        )
        .unwrap();

        let source_file = "src/Domain/Communication/Attachment.php";
        let search_dirs = vec!["tests/Unit".to_string()];
        let pattern = "CoversClass({basename}::class)";

        let results = grep_search(source_file, &search_dirs, pattern, temp_dir.path());

        assert_eq!(results.len(), 1);
        assert!(results[0].contains("Domain/Communication/AttachmentTest.php"));
    }

    #[test]
    fn test_grep_search_nonexistent_directory() {
        use tempfile::TempDir;

        let temp_dir = TempDir::new().unwrap();

        let source_file = "src/Domain/Attachment.php";
        let search_dirs = vec!["nonexistent/dir".to_string()];
        let pattern = "CoversClass({basename}::class)";

        let results = grep_search(source_file, &search_dirs, pattern, temp_dir.path());

        assert!(results.is_empty());
    }

    #[test]
    fn test_grep_search_multiple_matches() {
        use std::fs;
        use tempfile::TempDir;

        let temp_dir = TempDir::new().unwrap();
        let search_dir = temp_dir.path().join("tests/Unit");
        fs::create_dir_all(&search_dir).unwrap();

        // Create multiple test files that reference the same class
        let test_file1 = search_dir.join("AttachmentTest.php");
        fs::write(&test_file1, "#[CoversClass(Attachment::class)]").unwrap();

        let test_file2 = search_dir.join("AttachmentIntegrationTest.php");
        fs::write(&test_file2, "#[CoversClass(Attachment::class)]").unwrap();

        let source_file = "src/Domain/Attachment.php";
        let search_dirs = vec!["tests/Unit".to_string()];
        let pattern = "CoversClass({basename}::class)";

        let results = grep_search(source_file, &search_dirs, pattern, temp_dir.path());

        assert_eq!(results.len(), 2);
    }

    #[test]
    fn test_grep_search_empty_search_dirs() {
        let results = grep_search(
            "src/Domain/Attachment.php",
            &[],
            "CoversClass({basename}::class)",
            Path::new("/"),
        );
        assert!(results.is_empty());
    }

    #[test]
    fn test_grep_search_empty_existing_directory() {
        use std::fs;
        use tempfile::TempDir;

        let temp_dir = TempDir::new().unwrap();
        fs::create_dir_all(temp_dir.path().join("tests/Empty")).unwrap();

        let results = grep_search(
            "src/Foo.php",
            &["tests/Empty".to_string()],
            "CoversClass({basename}::class)",
            temp_dir.path(),
        );
        assert!(results.is_empty(), "got: {results:?}");
    }

    mod grep_search_runner_tests {
        use super::*;
        use tempfile::TempDir;

        #[test]
        fn spawn_failure_returns_empty() {
            let temp_dir = TempDir::new().unwrap();
            let search_dir = temp_dir.path().join("tests/Unit");
            std::fs::create_dir_all(&search_dir).unwrap();

            let mut mock = MockProcessRunner::new();
            mock.expect_run().returning(|_, _| {
                Err(std::io::Error::new(
                    std::io::ErrorKind::NotFound,
                    "grep not found",
                ))
            });

            let results = grep_search_with_runner(
                "src/Foo.php",
                &["tests/Unit".to_string()],
                "CoversClass({basename}::class)",
                temp_dir.path(),
                &mock,
            );
            assert!(results.is_empty());
        }

        #[test]
        fn exit_code_nonzero_returns_empty() {
            let temp_dir = TempDir::new().unwrap();
            let search_dir = temp_dir.path().join("tests/Unit");
            std::fs::create_dir_all(&search_dir).unwrap();

            let mut mock = MockProcessRunner::new();
            // Grep exit code 1 (no matches) or >=2 (error) both produce success=false
            mock.expect_run().returning(|_, _| {
                Ok(ProcessOutput {
                    success: false,
                    stdout: String::new(),
                })
            });

            let results = grep_search_with_runner(
                "src/Foo.php",
                &["tests/Unit".to_string()],
                "pattern",
                temp_dir.path(),
                &mock,
            );
            assert!(results.is_empty());
        }

        #[test]
        fn parses_grep_stdout_and_prepends_search_dir() {
            let temp_dir = TempDir::new().unwrap();
            let search_dir = temp_dir.path().join("tests/Unit");
            std::fs::create_dir_all(&search_dir).unwrap();

            let mut mock = MockProcessRunner::new();
            mock.expect_run().returning(|_, _| {
                Ok(ProcessOutput {
                    success: true,
                    stdout: "./Domain/FooTest.php\n./Domain/BarTest.php\n".into(),
                })
            });

            let results = grep_search_with_runner(
                "src/Foo.php",
                &["tests/Unit".to_string()],
                "pattern",
                temp_dir.path(),
                &mock,
            );
            assert_eq!(
                results,
                vec![
                    "tests/Unit/Domain/FooTest.php".to_string(),
                    "tests/Unit/Domain/BarTest.php".to_string(),
                ]
            );
        }

        #[test]
        fn trims_whitespace_from_grep_output_lines() {
            let temp_dir = TempDir::new().unwrap();
            let search_dir = temp_dir.path().join("tests/Unit");
            std::fs::create_dir_all(&search_dir).unwrap();

            let mut mock = MockProcessRunner::new();
            mock.expect_run().returning(|_, _| {
                Ok(ProcessOutput {
                    success: true,
                    stdout: "  ./FooTest.php  \n\t./BarTest.php\t\n".into(),
                })
            });

            let results = grep_search_with_runner(
                "src/Foo.php",
                &["tests".to_string()],
                "pattern",
                temp_dir.path(),
                &mock,
            );
            assert_eq!(
                results,
                vec![
                    "tests/FooTest.php".to_string(),
                    "tests/BarTest.php".to_string(),
                ]
            );
        }

        #[test]
        fn multiple_search_dirs_each_spawn_runner() {
            let temp_dir = TempDir::new().unwrap();
            for d in &["tests/A", "tests/B"] {
                std::fs::create_dir_all(temp_dir.path().join(d)).unwrap();
            }

            let mut mock = MockProcessRunner::new();
            mock.expect_run().times(2).returning(|dir, _| {
                if dir.ends_with("tests/A") {
                    Ok(ProcessOutput {
                        success: true,
                        stdout: "./FromA.php\n".into(),
                    })
                } else {
                    Ok(ProcessOutput {
                        success: false,
                        stdout: String::new(),
                    })
                }
            });

            let results = grep_search_with_runner(
                "src/Foo.php",
                &["tests/A".to_string(), "tests/B".to_string()],
                "pattern",
                temp_dir.path(),
                &mock,
            );
            assert_eq!(results, vec!["tests/A/FromA.php".to_string()]);
        }

        #[test]
        fn missing_directory_is_skipped_without_running_runner() {
            let temp_dir = TempDir::new().unwrap();
            // tests/Nope doesn't exist on disk

            let mut mock = MockProcessRunner::new();
            mock.expect_run().times(0);

            let results = grep_search_with_runner(
                "src/Foo.php",
                &["tests/Nope".to_string()],
                "pattern",
                temp_dir.path(),
                &mock,
            );
            assert!(results.is_empty());
        }
    }
}
