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

/// Find related test files for a list of source files using provided strategies
pub fn find_related_tests(
    strategies: &[TestDiscoveryStrategy],
    source_files: &[&str],
    project_root: &Path,
) -> Vec<String> {
    let mut all_tests: HashSet<String> = HashSet::new();

    for strategy in strategies {
        match strategy {
            TestDiscoveryStrategy::PathMapping { rules } => {
                for source_file in source_files {
                    let tests = apply_path_mapping(source_file, rules, project_root);
                    all_tests.extend(tests);
                }
            }
            TestDiscoveryStrategy::GrepSearch {
                search_dirs,
                pattern,
            } => {
                for source_file in source_files {
                    let tests = grep_search(source_file, search_dirs, pattern, project_root);
                    all_tests.extend(tests);
                }
            }
        }
    }

    let mut result: Vec<String> = all_tests.into_iter().collect();
    result.sort();
    result
}

/// Apply path mapping rules to find test files
fn apply_path_mapping(
    source_file: &str,
    rules: &[PathMappingRule],
    project_root: &Path,
) -> Vec<String> {
    let mut found_tests = Vec::new();

    for rule in rules {
        if let Some(path_capture) = extract_path_from_pattern(source_file, &rule.source) {
            for test_pattern in &rule.tests {
                let test_path = test_pattern.replace("{path}", &path_capture);
                let full_path = project_root.join(&test_path);

                if full_path.exists() {
                    found_tests.push(test_path);
                }
            }
        }
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
fn grep_search(
    source_file: &str,
    search_dirs: &[String],
    pattern: &str,
    project_root: &Path,
) -> Vec<String> {
    // Expand placeholders in the pattern
    let expanded_pattern = expand_placeholders(pattern, source_file);

    let mut found_tests = Vec::new();

    for search_dir in search_dirs {
        let dir_path = project_root.join(search_dir);
        if !dir_path.exists() {
            continue;
        }

        // Use grep to find files containing the pattern
        if let Ok(output) = std::process::Command::new("grep")
            .args(["-rl", &expanded_pattern])
            .current_dir(&dir_path)
            .output()
        {
            if output.status.success() {
                let stdout = String::from_utf8_lossy(&output.stdout);
                for line in stdout.lines() {
                    let test_path = format!("{}/{}", search_dir, line.trim());
                    found_tests.push(test_path);
                }
            }
        }
    }

    found_tests
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

    #[test]
    fn test_extract_path_from_pattern() {
        assert_eq!(
            extract_path_from_pattern("src/Platform/Service/Foo.php", "src/{path}.php"),
            Some("Platform/Service/Foo".to_string())
        );

        assert_eq!(
            extract_path_from_pattern("lib/src/Money/Currency.php", "lib/src/{path}.php"),
            Some("Money/Currency".to_string())
        );

        assert_eq!(
            extract_path_from_pattern("tests/something.php", "src/{path}.php"),
            None
        );
    }

    #[test]
    fn test_expand_placeholders() {
        let pattern = "CoversClass\\(.*\\\\{basename}(::class)?\\)";
        let source = "src/Platform/Service/UserService.php";

        let expanded = expand_placeholders(pattern, source);
        assert_eq!(expanded, "CoversClass\\(.*\\\\UserService(::class)?\\)");
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
    fn test_expand_placeholders_dirname() {
        let pattern = "{dirname}";
        let source = "src/Platform/Service/UserService.php";

        let expanded = expand_placeholders(pattern, source);
        assert_eq!(expanded, "src/Platform/Service");
    }
}
