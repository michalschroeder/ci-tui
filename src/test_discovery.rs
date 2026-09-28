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
//!   to the changed source files — one `grep` per search dir, all patterns
//!   batched on stdin, bounded by [`GREP_TIMEOUT`]; failures become warnings
//!
//! # Key Functions
//!
//! - [`find_related_tests`]: Main entry point for test discovery

use crate::config::{PathMappingRule, TestDiscoveryStrategy};
use std::collections::HashSet;
use std::io::{Read, Write};
use std::path::Path;
use std::process::{Command, Stdio};
use std::sync::mpsc;
use std::time::Duration;

/// Upper bound for one `grep` invocation, so a hung grep (huge tree, stale
/// network mount) can't stall check selection. Fixed, not configurable.
pub(crate) const GREP_TIMEOUT: Duration = Duration::from_secs(30);

/// Related tests found by [`find_related_tests`], plus non-fatal warnings
/// (e.g. a `grep_search` that failed or timed out).
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct DiscoveredTests {
    /// Test file paths, deduped and sorted
    pub tests: Vec<String>,
    /// Human-readable warnings, one per failed grep invocation
    pub warnings: Vec<String>,
}

/// Output of a grep-style process invocation.
#[derive(Debug, Clone)]
pub(crate) struct ProcessOutput {
    /// Exit code; `None` when the process was killed by a signal
    pub code: Option<i32>,
    pub stdout: String,
    pub stderr: String,
}

/// Abstraction over the grep invocation so tests can inject controlled outputs.
/// `stdin` is fed to the process (grep reads its patterns from it).
/// A timeout is reported as an `Err` of kind [`std::io::ErrorKind::TimedOut`].
#[cfg_attr(any(test, feature = "test"), mockall::automock)]
pub(crate) trait ProcessRunner: Send + Sync {
    fn run(&self, dir: &Path, args: &[String], stdin: &str) -> std::io::Result<ProcessOutput>;
}

/// Production implementation — shells out to `grep`, killed after [`GREP_TIMEOUT`].
pub(crate) struct RealProcessRunner;

impl ProcessRunner for RealProcessRunner {
    fn run(&self, dir: &Path, args: &[String], stdin: &str) -> std::io::Result<ProcessOutput> {
        let mut command = Command::new("grep");
        command.args(args).current_dir(dir);
        run_with_timeout(command, stdin, GREP_TIMEOUT)
    }
}

/// Run `command` with `stdin` as input to completion, capturing stdout/stderr;
/// kill it and return a `TimedOut` error once `timeout` elapses.
///
/// Sync (discovery runs in blocking contexts), no wait-timeout crate: waits
/// for stdout EOF with `recv_timeout`, which arrives as soon as the child exits.
fn run_with_timeout(
    mut command: Command,
    stdin: &str,
    timeout: Duration,
) -> std::io::Result<ProcessOutput> {
    let mut child = command
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()?;
    // Feed stdin on a thread: a large input must not block us before the
    // timeout is armed. Write errors (EPIPE: child exited without reading
    // it all) are ignored — the exit status tells the story.
    if let Some(mut pipe) = child.stdin.take() {
        let input = stdin.to_owned();
        std::thread::spawn(move || {
            let _ = pipe.write_all(input.as_bytes());
        });
    }
    // Drain both pipes on threads so a chatty child never blocks on a full pipe
    let stdout = drain(child.stdout.take());
    let stderr = drain(child.stderr.take());

    let stdout = match stdout.recv_timeout(timeout) {
        Err(mpsc::RecvTimeoutError::Timeout) => {
            let _ = child.kill();
            let _ = child.wait();
            return Err(std::io::Error::new(
                std::io::ErrorKind::TimedOut,
                format!(
                    "timed out after {}",
                    crate::utils::time::format_from_duration(timeout)
                ),
            ));
        }
        result => result.unwrap_or_default(),
    };
    let status = child.wait()?;

    Ok(ProcessOutput {
        code: status.code(),
        stdout,
        stderr: stderr.recv().unwrap_or_default(),
    })
}

/// Read a child pipe to EOF on a background thread (lossy UTF-8); the
/// contents arrive on the returned channel.
fn drain(pipe: Option<impl Read + Send + 'static>) -> mpsc::Receiver<String> {
    let (tx, rx) = mpsc::channel();
    std::thread::spawn(move || {
        let mut buf = Vec::new();
        if let Some(mut pipe) = pipe {
            let _ = pipe.read_to_end(&mut buf);
        }
        let _ = tx.send(String::from_utf8_lossy(&buf).into_owned());
    });
    rx
}

/// Find related test files for a list of source files using provided strategies
pub fn find_related_tests(
    strategies: &[TestDiscoveryStrategy],
    source_files: &[&str],
    project_root: &Path,
) -> DiscoveredTests {
    find_related_tests_with_runner(strategies, source_files, project_root, &RealProcessRunner)
}

pub(crate) fn find_related_tests_with_runner(
    strategies: &[TestDiscoveryStrategy],
    source_files: &[&str],
    project_root: &Path,
    runner: &dyn ProcessRunner,
) -> DiscoveredTests {
    let mut all_tests: HashSet<String> = HashSet::new();
    let mut warnings = Vec::new();

    for strategy in strategies {
        let found = apply_strategy_with_runner(strategy, source_files, project_root, runner);
        all_tests.extend(found.tests);
        warnings.extend(found.warnings);
    }

    let mut tests: Vec<String> = all_tests.into_iter().collect();
    tests.sort();
    DiscoveredTests { tests, warnings }
}

/// Apply a single discovery strategy to source files
fn apply_strategy_with_runner(
    strategy: &TestDiscoveryStrategy,
    source_files: &[&str],
    project_root: &Path,
    runner: &dyn ProcessRunner,
) -> DiscoveredTests {
    match strategy {
        TestDiscoveryStrategy::PathMapping { rules } => DiscoveredTests {
            tests: source_files
                .iter()
                .flat_map(|f| apply_path_mapping(f, rules, project_root))
                .collect(),
            warnings: Vec::new(),
        },
        TestDiscoveryStrategy::GrepSearch {
            search_dirs,
            pattern,
        } => grep_search_with_runner(source_files, search_dirs, pattern, project_root, runner),
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

/// Search test files for content matching a pattern with placeholders.
///
/// One `grep -rl -f - .` per existing search dir, the distinct expanded
/// patterns fed on stdin one per line — command size stays constant however
/// many files changed, and patterns starting with `-` are never options.
/// Exit 1 means no match. Exit >= 2 keeps any matches grep printed and adds a
/// warning. With several patterns, exit >= 2 also triggers a cheap per-pattern
/// validity check (see [`grep_dir`]) so one invalid pattern doesn't cost the
/// others their matches. Spawn errors, timeouts and signal kills drop that
/// dir's results with a warning. Other dirs are unaffected.
fn grep_search_with_runner(
    source_files: &[&str],
    search_dirs: &[String],
    pattern: &str,
    project_root: &Path,
    runner: &dyn ProcessRunner,
) -> DiscoveredTests {
    let mut found = DiscoveredTests::default();

    let mut seen = HashSet::new();
    let patterns: Vec<String> = source_files
        .iter()
        .map(|f| expand_placeholders(pattern, f))
        .filter(|p| seen.insert(p.clone()))
        .collect();
    if patterns.is_empty() {
        return found;
    }

    // Pattern validity doesn't depend on the dir: checked at most once
    let mut checked = None;
    for search_dir in search_dirs {
        let dir_path = project_root.join(search_dir);
        if !dir_path.exists() {
            continue;
        }

        let run = grep_dir(runner, &dir_path, search_dir, &patterns, &mut checked);
        found.tests.extend(run.matches);
        found.warnings.extend(run.warnings);
    }

    // An invalid pattern is reported by every dir that hit it: keep one
    let mut seen = HashSet::new();
    found.warnings.retain(|w| seen.insert(w.clone()));
    found
}

/// Batched grep in one search dir. On exit >= 2 with several patterns, each
/// pattern is validated against `/dev/null` (no tree walk) — exit >= 2 may just
/// be an unreadable file. All valid: the batched result stands. Otherwise the
/// batch reruns once with only the valid patterns, plus a warning per invalid one.
fn grep_dir(
    runner: &dyn ProcessRunner,
    dir_path: &Path,
    search_dir: &str,
    patterns: &[String],
    checked: &mut Option<PatternCheck>,
) -> GrepRun {
    let batched = run_grep(runner, dir_path, search_dir, patterns);
    if !batched.exit_error || patterns.len() < 2 {
        return batched;
    }
    let check = checked.get_or_insert_with(|| check_patterns(runner, dir_path, patterns));
    if check.invalid.is_empty() {
        return batched;
    }
    let mut run = if check.valid.is_empty() {
        GrepRun::default()
    } else {
        run_grep(runner, dir_path, search_dir, &check.valid)
    };
    run.warnings.splice(0..0, check.invalid.iter().cloned());
    run
}

/// Patterns split by validity: valid ones, and a warning per invalid one.
struct PatternCheck {
    valid: Vec<String>,
    invalid: Vec<String>,
}

/// Validate each pattern alone with `grep -f - /dev/null`: exit >= 2 means
/// grep rejected the pattern. A run that can't tell (spawn error, timeout,
/// signal) counts as valid — the batched run already reported it.
fn check_patterns(runner: &dyn ProcessRunner, dir: &Path, patterns: &[String]) -> PatternCheck {
    let args = ["-f", "-", "/dev/null"].map(String::from);
    let mut check = PatternCheck {
        valid: Vec::new(),
        invalid: Vec::new(),
    };
    for pattern in patterns {
        match runner.run(dir, &args, &format!("{pattern}\n")) {
            Ok(output) if output.code.is_some_and(|c| c >= 2) => check.invalid.push(format!(
                "test discovery: invalid grep pattern `{pattern}`: {}",
                exit_cause(output.code, &output.stderr)
            )),
            _ => check.valid.push(pattern.clone()),
        }
    }
    check
}

/// Result of one grep run in a search dir.
#[derive(Default)]
struct GrepRun {
    /// `search_dir/`-prefixed matches grep printed
    matches: Vec<String>,
    warnings: Vec<String>,
    /// grep ran and exited >= 2 (invalid pattern or unreadable file) — worth
    /// a pattern check, unlike a spawn error, timeout or signal
    exit_error: bool,
}

/// Run `grep -rl -f - .` in `dir_path` with `patterns` on stdin.
fn run_grep(
    runner: &dyn ProcessRunner,
    dir_path: &Path,
    search_dir: &str,
    patterns: &[String],
) -> GrepRun {
    let args = ["-rl", "-f", "-", "."].map(String::from);
    // Newline-terminated so an empty pattern still counts (matches every line)
    let stdin: String = patterns.iter().map(|p| format!("{p}\n")).collect();

    let output = match runner.run(dir_path, &args, &stdin) {
        Ok(output) => output,
        Err(e) => {
            return GrepRun {
                warnings: vec![grep_warning(search_dir, e)],
                ..GrepRun::default()
            }
        }
    };
    let (warnings, exit_error) = match output.code {
        Some(0 | 1) => (Vec::new(), false),
        code => (
            vec![grep_warning(search_dir, exit_cause(code, &output.stderr))],
            code.is_some(),
        ),
    };
    let matches = output
        .stdout
        .lines()
        .map(str::trim)
        .filter(|line| !line.is_empty())
        .map(|line| format!("{search_dir}/{}", line.strip_prefix("./").unwrap_or(line)))
        .collect();
    GrepRun {
        matches,
        warnings,
        exit_error,
    }
}

/// Warning for a failed `grep_search` in `search_dir`.
fn grep_warning(search_dir: &str, cause: impl std::fmt::Display) -> String {
    format!("test discovery: grep in `{search_dir}` failed: {cause}")
}

/// Cause for an unexpected grep exit: status plus stderr's first line.
fn exit_cause(code: Option<i32>, stderr: &str) -> String {
    let status = code.map_or_else(
        || "killed by signal".to_string(),
        |c| format!("exit status {c}"),
    );
    match stderr.lines().map(str::trim).find(|l| !l.is_empty()) {
        Some(line) => format!("{status}: {line}"),
        None => status,
    }
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

            let result = find_related_tests(&strategies, &["src/Foo.php"], temp_dir.path()).tests;
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

            let result = find_related_tests(&strategies, &["src/Foo.php"], temp_dir.path()).tests;
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

            let result = find_related_tests(&strategies, &["src/Foo.php"], temp_dir.path()).tests;
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

            let result = find_related_tests(&strategies, &[], temp_dir.path()).tests;
            assert!(result.is_empty());
        }

        #[test]
        fn test_empty_strategies_returns_empty() {
            let strategies: Vec<TestDiscoveryStrategy> = vec![];
            let result = find_related_tests(&strategies, &["src/Foo.php"], Path::new("/")).tests;
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

        let results = grep_search_with_runner(
            &[source_file],
            &search_dirs,
            pattern,
            temp_dir.path(),
            &RealProcessRunner,
        )
        .tests;

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

        let results = grep_search_with_runner(
            &[source_file],
            &search_dirs,
            pattern,
            temp_dir.path(),
            &RealProcessRunner,
        )
        .tests;

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

        let results = grep_search_with_runner(
            &[source_file],
            &search_dirs,
            pattern,
            temp_dir.path(),
            &RealProcessRunner,
        )
        .tests;

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

        let results = grep_search_with_runner(
            &[source_file],
            &search_dirs,
            pattern,
            temp_dir.path(),
            &RealProcessRunner,
        )
        .tests;

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

        let results = grep_search_with_runner(
            &[source_file],
            &search_dirs,
            pattern,
            temp_dir.path(),
            &RealProcessRunner,
        )
        .tests;

        assert_eq!(results.len(), 2);
    }

    #[test]
    fn test_grep_search_empty_search_dirs() {
        let results = grep_search_with_runner(
            &["src/Domain/Attachment.php"],
            &[],
            "CoversClass({basename}::class)",
            Path::new("/"),
            &RealProcessRunner,
        )
        .tests;
        assert!(results.is_empty());
    }

    #[test]
    fn test_grep_search_empty_existing_directory() {
        use std::fs;
        use tempfile::TempDir;

        let temp_dir = TempDir::new().unwrap();
        fs::create_dir_all(temp_dir.path().join("tests/Empty")).unwrap();

        let results = grep_search_with_runner(
            &["src/Foo.php"],
            &["tests/Empty".to_string()],
            "CoversClass({basename}::class)",
            temp_dir.path(),
            &RealProcessRunner,
        )
        .tests;
        assert!(results.is_empty(), "got: {results:?}");
    }

    mod grep_search_runner_tests {
        use super::*;
        use tempfile::TempDir;

        fn out(code: Option<i32>, stdout: &str, stderr: &str) -> ProcessOutput {
            ProcessOutput {
                code,
                stdout: stdout.into(),
                stderr: stderr.into(),
            }
        }

        /// Temp project root with `dirs` created.
        fn root_with(dirs: &[&str]) -> TempDir {
            let temp_dir = TempDir::new().unwrap();
            for d in dirs {
                std::fs::create_dir_all(temp_dir.path().join(d)).unwrap();
            }
            temp_dir
        }

        fn search(
            sources: &[&str],
            dirs: &[&str],
            pattern: &str,
            root: &Path,
            runner: &dyn ProcessRunner,
        ) -> DiscoveredTests {
            let dirs: Vec<String> = dirs.iter().map(|d| d.to_string()).collect();
            grep_search_with_runner(sources, &dirs, pattern, root, runner)
        }

        #[test]
        fn one_invocation_per_existing_dir_with_all_patterns_on_stdin() {
            let root = root_with(&["tests/A", "tests/B"]);
            let mut mock = MockProcessRunner::new();
            // 3 sources x 2 existing dirs (+1 missing) -> exactly 2 runs;
            // src/Foo.php and lib/Foo.php expand to the same pattern -> sent once
            mock.expect_run()
                .times(2)
                .withf(|_, args, stdin| {
                    args == ["-rl", "-f", "-", "."]
                        && stdin == "CoversClass(Foo::class)\nCoversClass(Bar::class)\n"
                })
                .returning(|_, _, _| Ok(out(Some(1), "", "")));

            let found = search(
                &["src/Foo.php", "lib/Foo.php", "src/Bar.php"],
                &["tests/A", "tests/Missing", "tests/B"],
                "CoversClass({basename}::class)",
                root.path(),
                &mock,
            );
            assert_eq!(found, DiscoveredTests::default());
        }

        #[test]
        fn no_source_files_runs_nothing() {
            let root = root_with(&["tests"]);
            let mut mock = MockProcessRunner::new();
            mock.expect_run().times(0);

            let found = search(&[], &["tests"], "p", root.path(), &mock);
            assert_eq!(found, DiscoveredTests::default());
        }

        #[test]
        fn dash_leading_pattern_passed_on_stdin_not_args() {
            let root = root_with(&["tests"]);
            let mut mock = MockProcessRunner::new();
            mock.expect_run()
                .times(1)
                .withf(|_, args, stdin| args == ["-rl", "-f", "-", "."] && stdin == "--Foo\n")
                .returning(|_, _, _| Ok(out(Some(1), "", "")));

            search(
                &["src/Foo.php"],
                &["tests"],
                "--{basename}",
                root.path(),
                &mock,
            );
        }

        #[test]
        fn spawn_failure_warns_with_dir_and_cause() {
            let root = root_with(&["tests/Unit"]);
            let mut mock = MockProcessRunner::new();
            mock.expect_run().returning(|_, _, _| {
                Err(std::io::Error::new(
                    std::io::ErrorKind::NotFound,
                    "grep not found",
                ))
            });

            let found = search(&["src/Foo.php"], &["tests/Unit"], "p", root.path(), &mock);
            assert!(found.tests.is_empty());
            assert_eq!(
                found.warnings,
                vec!["test discovery: grep in `tests/Unit` failed: grep not found"]
            );
        }

        #[test]
        fn exit_1_is_no_match_without_warning() {
            let root = root_with(&["tests/Unit"]);
            let mut mock = MockProcessRunner::new();
            mock.expect_run()
                .returning(|_, _, _| Ok(out(Some(1), "", "ignored")));

            let found = search(&["src/Foo.php"], &["tests/Unit"], "p", root.path(), &mock);
            assert_eq!(found, DiscoveredTests::default());
        }

        #[test]
        fn exit_2_keeps_partial_matches_and_warns_with_stderr_first_line() {
            let root = root_with(&["tests/Unit"]);
            let mut mock = MockProcessRunner::new();
            mock.expect_run().returning(|_, _, _| {
                Ok(out(
                    Some(2),
                    "./Partial.php\n",
                    "\ngrep: Unmatched ( or \\(\nsecond line\n",
                ))
            });

            let found = search(&["src/Foo.php"], &["tests/Unit"], "p", root.path(), &mock);
            assert_eq!(found.tests, vec!["tests/Unit/Partial.php"]);
            assert_eq!(
                found.warnings,
                vec![
                    "test discovery: grep in `tests/Unit` failed: exit status 2: grep: Unmatched ( or \\("
                ]
            );
        }

        #[test]
        fn exit_without_stderr_or_code_still_warns() {
            let root = root_with(&["a", "b"]);
            let mut mock = MockProcessRunner::new();
            mock.expect_run().returning(|dir, _, _| {
                Ok(if dir.ends_with("a") {
                    out(Some(3), "", "")
                } else {
                    out(None, "", "")
                })
            });

            let found = search(&["src/Foo.php"], &["a", "b"], "p", root.path(), &mock);
            assert_eq!(
                found.warnings,
                vec![
                    "test discovery: grep in `a` failed: exit status 3",
                    "test discovery: grep in `b` failed: killed by signal",
                ]
            );
        }

        #[test]
        fn failing_dir_does_not_drop_other_dirs_results() {
            let root = root_with(&["tests/A", "tests/B"]);
            let mut mock = MockProcessRunner::new();
            mock.expect_run().times(2).returning(|dir, _, _| {
                Ok(if dir.ends_with("tests/A") {
                    out(Some(0), "./FromA.php\n", "")
                } else {
                    out(Some(2), "", "grep: boom")
                })
            });

            let found = search(
                &["src/Foo.php"],
                &["tests/A", "tests/B"],
                "p",
                root.path(),
                &mock,
            );
            assert_eq!(found.tests, vec!["tests/A/FromA.php".to_string()]);
            assert_eq!(
                found.warnings,
                vec!["test discovery: grep in `tests/B` failed: exit status 2: grep: boom"]
            );
        }

        const SEARCH: [&str; 4] = ["-rl", "-f", "-", "."];
        const VALIDATE: [&str; 3] = ["-f", "-", "/dev/null"];

        #[test]
        fn invalid_pattern_is_dropped_and_valid_ones_rebatched() {
            let root = root_with(&["tests"]);
            let mut mock = MockProcessRunner::new();
            mock.expect_run()
                .times(1)
                .withf(|_, args, stdin| args == SEARCH && stdin == "A\nBad[\nC\n")
                .returning(|_, _, _| Ok(out(Some(2), "", "grep: Unmatched [")));
            mock.expect_run()
                .times(3)
                .withf(|_, args, _| args == VALIDATE)
                .returning(|_, _, stdin| {
                    Ok(match stdin {
                        "Bad[\n" => out(Some(2), "", "grep: Unmatched ["),
                        _ => out(Some(1), "", ""),
                    })
                });
            mock.expect_run()
                .times(1)
                .withf(|_, args, stdin| args == SEARCH && stdin == "A\nC\n")
                .returning(|_, _, _| Ok(out(Some(0), "./ATest.php\n./CTest.php\n", "")));

            let found = search(
                &["src/A.php", "src/Bad[.php", "src/C.php"],
                &["tests"],
                "{basename}",
                root.path(),
                &mock,
            );
            assert_eq!(found.tests, vec!["tests/ATest.php", "tests/CTest.php"]);
            assert_eq!(
                found.warnings,
                vec![
                    "test discovery: invalid grep pattern `Bad[`: exit status 2: grep: Unmatched ["
                ]
            );
        }

        #[test]
        fn all_patterns_valid_keeps_batched_result_without_rerun() {
            // e.g. an unreadable file: exit 2 although every pattern is fine
            let root = root_with(&["tests"]);
            let mut mock = MockProcessRunner::new();
            mock.expect_run()
                .times(1)
                .withf(|_, args, _| args == SEARCH)
                .returning(|_, _, _| {
                    Ok(out(
                        Some(2),
                        "./ATest.php\n",
                        "grep: ./x: Permission denied",
                    ))
                });
            mock.expect_run()
                .times(2)
                .withf(|_, args, _| args == VALIDATE)
                .returning(|_, _, _| Ok(out(Some(1), "", "")));

            let found = search(
                &["src/A.php", "src/B.php"],
                &["tests"],
                "{basename}",
                root.path(),
                &mock,
            );
            assert_eq!(found.tests, vec!["tests/ATest.php"]);
            assert_eq!(
                found.warnings,
                vec!["test discovery: grep in `tests` failed: exit status 2: grep: ./x: Permission denied"]
            );
        }

        #[test]
        fn patterns_validated_once_across_dirs_and_warning_deduped() {
            let root = root_with(&["a", "b"]);
            let mut mock = MockProcessRunner::new();
            mock.expect_run()
                .times(2)
                .withf(|_, args, stdin| args == SEARCH && stdin == "A\nBad[\n")
                .returning(|_, _, _| Ok(out(Some(2), "", "grep: Unmatched [")));
            mock.expect_run()
                .times(2)
                .withf(|_, args, _| args == VALIDATE)
                .returning(|_, _, stdin| {
                    Ok(out(
                        if stdin == "Bad[\n" { Some(2) } else { Some(1) },
                        "",
                        "",
                    ))
                });
            mock.expect_run()
                .times(2)
                .withf(|_, args, stdin| args == SEARCH && stdin == "A\n")
                .returning(|_, _, _| Ok(out(Some(1), "", "")));

            let found = search(
                &["src/A.php", "src/Bad[.php"],
                &["a", "b"],
                "{basename}",
                root.path(),
                &mock,
            );
            assert_eq!(
                found.warnings,
                vec!["test discovery: invalid grep pattern `Bad[`: exit status 2"]
            );
        }

        #[test]
        fn only_invalid_patterns_skip_rebatch() {
            let root = root_with(&["tests"]);
            let mut mock = MockProcessRunner::new();
            mock.expect_run()
                .times(1)
                .withf(|_, args, _| args == SEARCH)
                .returning(|_, _, _| Ok(out(Some(2), "", "")));
            mock.expect_run()
                .times(2)
                .withf(|_, args, _| args == VALIDATE)
                .returning(|_, _, _| Ok(out(Some(2), "", "")));

            let found = search(
                &["src/[.php", "src/\\(.php"],
                &["tests"],
                "{basename}",
                root.path(),
                &mock,
            );
            assert!(found.tests.is_empty());
            assert_eq!(found.warnings.len(), 2, "{:?}", found.warnings);
        }

        #[test]
        fn timeout_or_spawn_error_with_several_patterns_is_not_rerun() {
            let root = root_with(&["a", "b"]);
            let mut mock = MockProcessRunner::new();
            mock.expect_run().times(2).returning(|dir, _, _| {
                let kind = if dir.ends_with("a") {
                    std::io::ErrorKind::TimedOut
                } else {
                    std::io::ErrorKind::NotFound
                };
                Err(std::io::Error::new(kind, "nope"))
            });

            let found = search(
                &["src/A.php", "src/B.php"],
                &["a", "b"],
                "{basename}",
                root.path(),
                &mock,
            );
            assert!(found.tests.is_empty());
            assert_eq!(
                found.warnings,
                vec![
                    "test discovery: grep in `a` failed: nope",
                    "test discovery: grep in `b` failed: nope",
                ]
            );
        }

        #[test]
        fn signal_kill_with_several_patterns_is_not_rerun() {
            let root = root_with(&["tests"]);
            let mut mock = MockProcessRunner::new();
            mock.expect_run()
                .times(1)
                .returning(|_, _, _| Ok(out(None, "", "")));

            let found = search(
                &["src/A.php", "src/B.php"],
                &["tests"],
                "{basename}",
                root.path(),
                &mock,
            );
            assert_eq!(
                found.warnings,
                vec!["test discovery: grep in `tests` failed: killed by signal"]
            );
        }

        #[test]
        fn parses_grep_stdout_and_prepends_search_dir() {
            let root = root_with(&["tests/Unit"]);
            let mut mock = MockProcessRunner::new();
            mock.expect_run().returning(|_, _, _| {
                Ok(out(
                    Some(0),
                    "./Domain/FooTest.php\n./Domain/BarTest.php\n",
                    "",
                ))
            });

            let found = search(&["src/Foo.php"], &["tests/Unit"], "p", root.path(), &mock);
            assert_eq!(
                found.tests,
                vec![
                    "tests/Unit/Domain/FooTest.php".to_string(),
                    "tests/Unit/Domain/BarTest.php".to_string(),
                ]
            );
        }

        #[test]
        fn trims_whitespace_from_grep_output_lines() {
            let root = root_with(&["tests"]);
            let mut mock = MockProcessRunner::new();
            mock.expect_run().returning(|_, _, _| {
                Ok(out(Some(0), "  ./FooTest.php  \n\t./BarTest.php\t\n", ""))
            });

            let found = search(&["src/Foo.php"], &["tests"], "p", root.path(), &mock);
            assert_eq!(
                found.tests,
                vec![
                    "tests/FooTest.php".to_string(),
                    "tests/BarTest.php".to_string(),
                ]
            );
        }

        #[test]
        fn missing_directory_is_skipped_without_running_runner() {
            let root = root_with(&[]);
            let mut mock = MockProcessRunner::new();
            mock.expect_run().times(0);

            let found = search(&["src/Foo.php"], &["tests/Nope"], "p", root.path(), &mock);
            assert_eq!(found, DiscoveredTests::default());
        }

        #[test]
        fn find_related_tests_collects_grep_warnings() {
            let root = root_with(&["tests"]);
            let mut mock = MockProcessRunner::new();
            mock.expect_run()
                .returning(|_, _, _| Ok(out(Some(2), "", "grep: boom")));
            let strategies = vec![TestDiscoveryStrategy::GrepSearch {
                search_dirs: vec!["tests".to_string()],
                pattern: "p".to_string(),
            }];

            let found =
                find_related_tests_with_runner(&strategies, &["src/Foo.php"], root.path(), &mock);
            assert!(found.tests.is_empty());
            assert_eq!(
                found.warnings,
                vec!["test discovery: grep in `tests` failed: exit status 2: grep: boom"]
            );
        }
    }

    mod run_with_timeout_tests {
        use super::*;

        #[test]
        fn kills_child_and_returns_timed_out() {
            let mut command = Command::new("sleep");
            command.arg("5");
            let started = std::time::Instant::now();

            let err = run_with_timeout(command, "", Duration::from_secs(1)).unwrap_err();
            assert_eq!(err.kind(), std::io::ErrorKind::TimedOut);
            assert_eq!(err.to_string(), "timed out after 1s");
            assert!(started.elapsed() < Duration::from_secs(3));
        }

        #[test]
        fn captures_exit_code_stdout_and_stderr() {
            let mut command = Command::new("sh");
            command.args(["-c", "echo out; echo err >&2; exit 2"]);

            let output = run_with_timeout(command, "", Duration::from_secs(10)).unwrap();
            assert_eq!(output.code, Some(2));
            assert_eq!(output.stdout, "out\n");
            assert_eq!(output.stderr, "err\n");
        }

        #[test]
        fn feeds_stdin() {
            let command = Command::new("cat");
            let output = run_with_timeout(command, "a\nb\n", Duration::from_secs(10)).unwrap();
            assert_eq!(output.stdout, "a\nb\n");
        }

        #[test]
        fn child_ignoring_large_stdin_is_not_an_error() {
            // `true` exits without reading: the writer hits EPIPE, ignored
            let command = Command::new("true");
            let input = "x".repeat(1 << 20);
            let output = run_with_timeout(command, &input, Duration::from_secs(10)).unwrap();
            assert_eq!(output.code, Some(0));
        }

        #[test]
        fn spawn_failure_is_error() {
            let command = Command::new("definitely-not-a-real-binary-ci-tui");
            assert!(run_with_timeout(command, "", Duration::from_secs(1)).is_err());
        }
    }

    mod grep_search_real_grep_tests {
        use super::*;
        use tempfile::TempDir;

        fn write(root: &Path, path: &str, content: &str) {
            let full = root.join(path);
            std::fs::create_dir_all(full.parent().unwrap()).unwrap();
            std::fs::write(full, content).unwrap();
        }

        #[test]
        fn multiple_sources_union_deduped_sorted_across_dirs() {
            let root = TempDir::new().unwrap();
            let r = root.path();
            write(r, "tests/A/FooTest.php", "CoversClass(Foo::class)");
            write(r, "tests/A/BarTest.php", "CoversClass(Bar::class)");
            write(
                r,
                "tests/A/BothTest.php",
                "CoversClass(Foo::class) CoversClass(Bar::class)",
            );
            write(r, "tests/A/OtherTest.php", "CoversClass(Other::class)");
            write(r, "tests/B/nested/BarTest.php", "CoversClass(Bar::class)");
            let strategies = vec![TestDiscoveryStrategy::GrepSearch {
                search_dirs: vec!["tests/A".to_string(), "tests/B".to_string()],
                pattern: "CoversClass({basename}::class)".to_string(),
            }];

            let found = find_related_tests(&strategies, &["src/Foo.php", "src/Bar.php"], r);
            assert_eq!(
                found.tests,
                vec![
                    "tests/A/BarTest.php",
                    "tests/A/BothTest.php",
                    "tests/A/FooTest.php",
                    "tests/B/nested/BarTest.php",
                ]
            );
            assert!(found.warnings.is_empty(), "{:?}", found.warnings);
        }

        #[test]
        fn dash_leading_pattern_matches_literally() {
            let root = TempDir::new().unwrap();
            write(root.path(), "tests/FlagTest.sh", "run --Foo now");
            write(root.path(), "tests/Other.sh", "nothing");

            let found = search_real(root.path(), &["src/Foo.sh"], "--{basename}");
            assert_eq!(found.tests, vec!["tests/FlagTest.sh"]);
            assert!(found.warnings.is_empty(), "{:?}", found.warnings);
        }

        #[test]
        fn invalid_regex_warns_with_grep_stderr() {
            let root = TempDir::new().unwrap();
            write(root.path(), "tests/FooTest.php", "Foo");

            let found = search_real(root.path(), &["src/Foo.php"], "\\({basename}");
            assert!(found.tests.is_empty());
            assert_eq!(found.warnings.len(), 1, "{:?}", found.warnings);
            assert!(
                found.warnings[0]
                    .starts_with("test discovery: grep in `tests` failed: exit status 2: "),
                "{}",
                found.warnings[0]
            );
        }

        #[test]
        fn invalid_pattern_does_not_drop_valid_patterns_matches() {
            let root = TempDir::new().unwrap();
            write(root.path(), "tests/FooTest.php", "uses Foo");
            write(root.path(), "tests/PageTest.tsx", "renders [[...slug]]");

            let found = search_real(
                root.path(),
                &["app/[[...slug]].tsx", "src/Foo.php"],
                "{basename}",
            );
            assert_eq!(found.tests, vec!["tests/FooTest.php"]);
            assert_eq!(found.warnings.len(), 1, "{:?}", found.warnings);
            assert!(
                found.warnings[0].starts_with(
                    "test discovery: invalid grep pattern `[[...slug]]`: exit status 2"
                ),
                "{}",
                found.warnings[0]
            );
        }

        fn search_real(root: &Path, sources: &[&str], pattern: &str) -> DiscoveredTests {
            grep_search_with_runner(
                sources,
                &["tests".to_string()],
                pattern,
                root,
                &RealProcessRunner,
            )
        }
    }
}
