//! Extracted helper functions for determine_checks().
//!
//! These functions handle specific responsibilities:
//! - Processing always-run checks
//! - Matching files to check triggers
//! - Handling test discovery
//! - Building CheckToRun instances

use crate::config::{CheckDefinition, CiConfig};
use crate::git::ChangedFiles;
use crate::test_discovery;
use std::collections::HashSet;
use std::path::Path;

use super::{resolve_command, CheckToRun};

/// Process a check that always runs (has no triggers)
pub(super) fn process_always_run_check(
    check_id: &str,
    check: &CheckDefinition,
    group_name: &str,
    service: String,
) -> CheckToRun {
    let resolved = resolve_command(check, &[], false);
    let resolved_fix = check
        .fix_command
        .as_ref()
        .map(|_| resolve_command(check, &[], true));

    CheckToRun {
        id: check_id.to_string(),
        group: group_name.to_string(),
        definition: check.clone(),
        service,
        files: vec![],
        resolved_command: resolved,
        resolved_fix_command: resolved_fix,
        on_demand: false,
        skipped_no_files: false,
    }
}

/// Process a single check definition and return a CheckToRun if applicable
pub(super) fn process_check(
    config: &CiConfig,
    changed_files: &ChangedFiles,
    project_root: &Path,
    group_name: &str,
    check_id: &str,
    check: &CheckDefinition,
    default_service: &str,
) -> Option<CheckToRun> {
    let service = check.service_or_default(default_service).to_string();

    if check.always_run() {
        return Some(process_always_run_check(
            check_id, check, group_name, service,
        ));
    }

    process_triggered_check(
        config,
        changed_files,
        project_root,
        group_name,
        check_id,
        check,
        &service,
    )
}

/// Match changed files against a file pattern trigger
pub(super) fn match_file_pattern(
    config: &CiConfig,
    changed_files: &ChangedFiles,
    pattern_key: &str,
) -> Vec<String> {
    if let Some(re) = config.get_compiled_file_pattern(pattern_key) {
        changed_files
            .filter_by_pattern(re)
            .into_iter()
            .map(String::from)
            .collect()
    } else {
        vec![]
    }
}

/// Process test_discovery trigger logic
pub(super) fn process_test_discovery(
    config: &CiConfig,
    changed_files: &ChangedFiles,
    project_root: &Path,
    discovery: &crate::config::TestDiscoveryConfig,
    check: &CheckDefinition,
    matched_files: &mut Vec<String>,
) -> Option<TestDiscoveryResult> {
    let re = config.get_compiled_file_pattern(&discovery.source_pattern)?;
    let source_files = changed_files.filter_by_pattern(re);
    if source_files.is_empty() {
        return None;
    }

    let related_tests =
        test_discovery::find_related_tests(&discovery.strategies, &source_files, project_root);

    if !related_tests.is_empty() {
        matched_files.extend(related_tests);
        return None;
    }

    if !matched_files.is_empty() {
        return None;
    }

    // No tests found and no file_pattern matches
    if check.on_demand {
        return Some(TestDiscoveryResult::OnDemand);
    }

    if !check.command.contains("{files}") {
        matched_files.push("(source files changed - running all)".to_string());
    }

    None
}

/// Result from test discovery processing
pub(super) enum TestDiscoveryResult {
    OnDemand,
}

/// Build CheckToRun for a check with matched files
pub(super) fn build_check_to_run(
    check_id: &str,
    check: &CheckDefinition,
    group_name: &str,
    service: String,
    matched_files: Vec<String>,
) -> CheckToRun {
    let file_list: Vec<&str> = matched_files.iter().map(|s| s.as_str()).collect();
    let resolved = resolve_command(check, &file_list, false);
    let resolved_fix = check
        .fix_command
        .as_ref()
        .map(|_| resolve_command(check, &file_list, true));

    CheckToRun {
        id: check_id.to_string(),
        group: group_name.to_string(),
        definition: check.clone(),
        service,
        files: matched_files,
        resolved_command: resolved,
        resolved_fix_command: resolved_fix,
        on_demand: false,
        skipped_no_files: false,
    }
}

/// Build CheckToRun for a skipped check (on-demand)
pub(super) fn build_skipped_check(
    check_id: &str,
    check: &CheckDefinition,
    group_name: &str,
    service: String,
    has_files_placeholder: bool,
) -> CheckToRun {
    let resolved = resolve_command(check, &[], false);
    let resolved_fix = check
        .fix_command
        .as_ref()
        .map(|_| resolve_command(check, &[], true));

    CheckToRun {
        id: check_id.to_string(),
        group: group_name.to_string(),
        definition: check.clone(),
        service,
        files: vec!["(skipped - no matching files)".to_string()],
        resolved_command: resolved,
        resolved_fix_command: resolved_fix,
        on_demand: true,
        skipped_no_files: has_files_placeholder,
    }
}

/// Build CheckToRun for an on-demand check (test discovery found no tests)
pub(super) fn build_on_demand_check(
    check_id: &str,
    check: &CheckDefinition,
    group_name: &str,
    service: String,
) -> CheckToRun {
    let resolved = resolve_command(check, &[], false);
    let resolved_fix = check
        .fix_command
        .as_ref()
        .map(|_| resolve_command(check, &[], true));

    CheckToRun {
        id: check_id.to_string(),
        group: group_name.to_string(),
        definition: check.clone(),
        service,
        files: vec!["(on-demand - press 't' to run)".to_string()],
        resolved_command: resolved,
        resolved_fix_command: resolved_fix,
        on_demand: true,
        skipped_no_files: false,
    }
}

/// Process a triggered check (has triggers defined)
pub(super) fn process_triggered_check(
    config: &CiConfig,
    changed_files: &ChangedFiles,
    project_root: &Path,
    group_name: &str,
    check_id: &str,
    check: &CheckDefinition,
    service: &str,
) -> Option<CheckToRun> {
    let triggers = check.triggers.as_ref()?;
    let mut matched_files = Vec::new();
    let mut has_source_trigger = false;

    // Check file pattern trigger
    if let Some(pattern_key) = &triggers.file_pattern {
        let files = match_file_pattern(config, changed_files, pattern_key);
        matched_files.extend(files);
    }

    // Check test_discovery trigger
    if let Some(discovery) = &triggers.test_discovery {
        has_source_trigger = true;
        if process_test_discovery(
            config,
            changed_files,
            project_root,
            discovery,
            check,
            &mut matched_files,
        )
        .is_some()
        {
            return Some(build_on_demand_check(
                check_id,
                check,
                group_name,
                service.to_string(),
            ));
        }
    }

    // Deduplicate while preserving insertion order — output feeds {files} expansion + UI
    let mut seen = HashSet::new();
    matched_files.retain(|f| seen.insert(f.clone()));

    // Build appropriate CheckToRun based on whether files matched
    if !matched_files.is_empty() {
        Some(build_check_to_run(
            check_id,
            check,
            group_name,
            service.to_string(),
            matched_files,
        ))
    } else {
        let has_file_trigger = triggers.file_pattern.is_some();
        if has_file_trigger || has_source_trigger {
            let has_files_placeholder = check.command.contains("{files}");
            Some(build_skipped_check(
                check_id,
                check,
                group_name,
                service.to_string(),
                has_files_placeholder,
            ))
        } else {
            None
        }
    }
}

#[cfg(test)]
mod tests {
    #![allow(unused_imports, dead_code)]

    use super::*;
    use crate::config::{
        CheckDefinition, CheckTriggers, CiConfig, DockerConfig, FilePattern, GitConfig,
        PathMappingRule, TestDiscoveryConfig, TestDiscoveryStrategy,
    };
    use crate::git::ChangedFiles;
    use indexmap::IndexMap;
    use std::collections::HashMap;
    use std::path::PathBuf;

    fn base_config() -> CiConfig {
        let mut patterns = HashMap::new();
        patterns.insert(
            "rust".to_string(),
            FilePattern {
                pattern: r"\.rs$".to_string(),
                color: None,
            },
        );
        patterns.insert(
            "rust_src".to_string(),
            FilePattern {
                pattern: r"^src/.*\.rs$".to_string(),
                color: None,
            },
        );
        CiConfig::new(
            2,
            DockerConfig {
                project_dir: ".".to_string(),
                service: "app".to_string(),
                container: None,
                image: None,
                volume_mount: None,
                work_dir: None,
                shell: "bash".to_string(),
                env: HashMap::new(),
            },
            GitConfig {
                base_branch: "main".to_string(),
                fallback_branch: "HEAD~1".to_string(),
            },
            patterns,
            IndexMap::new(),
            Vec::new(),
        )
    }

    fn mk_check(
        cmd: &str,
        fix: Option<&str>,
        triggers: Option<CheckTriggers>,
        on_demand: bool,
    ) -> CheckDefinition {
        CheckDefinition {
            name: "N".to_string(),
            command: cmd.to_string(),
            service: None,
            container: None,
            fix_command: fix.map(String::from),
            triggers,
            on_demand,
            env: HashMap::new(),
        }
    }

    fn changed(files: &[&str]) -> ChangedFiles {
        ChangedFiles {
            files: files.iter().map(|s| s.to_string()).collect(),
            base_ref: "main".to_string(),
        }
    }

    mod process_always_run_check_tests {
        use super::*;

        #[test]
        fn builds_check_with_empty_files() {
            let def = mk_check("cargo check", None, None, false);
            let out = process_always_run_check("check-id", &def, "g", "svc".to_string());
            assert_eq!(out.id, "check-id");
            assert_eq!(out.group, "g");
            assert_eq!(out.service, "svc");
            assert!(out.files.is_empty());
            assert_eq!(out.resolved_command, "cargo check");
            assert!(out.resolved_fix_command.is_none());
            assert!(!out.on_demand);
            assert!(!out.skipped_no_files);
        }

        #[test]
        fn populates_resolved_fix_when_present() {
            let def = mk_check("cargo check", Some("cargo fix"), None, false);
            let out = process_always_run_check("id", &def, "g", "svc".to_string());
            assert_eq!(out.resolved_fix_command.as_deref(), Some("cargo fix"));
        }

        #[test]
        fn strips_files_placeholder_when_empty() {
            let def = mk_check("cargo check {files}", None, None, false);
            let out = process_always_run_check("id", &def, "g", "svc".to_string());
            assert_eq!(out.resolved_command, "cargo check");
        }
    }

    mod match_file_pattern_tests {
        use super::*;

        #[test]
        fn returns_matching_files() {
            let cfg = base_config();
            let cf = changed(&["src/main.rs", "README.md", "lib/foo.rs"]);
            let out = match_file_pattern(&cfg, &cf, "rust");
            assert_eq!(
                out,
                vec!["src/main.rs".to_string(), "lib/foo.rs".to_string()]
            );
        }

        #[test]
        fn returns_empty_when_pattern_key_missing() {
            let cfg = base_config();
            let cf = changed(&["src/main.rs"]);
            let out = match_file_pattern(&cfg, &cf, "nonexistent-key");
            assert!(out.is_empty());
        }

        #[test]
        fn returns_empty_when_no_files_match() {
            let cfg = base_config();
            let cf = changed(&["README.md", "doc.txt"]);
            let out = match_file_pattern(&cfg, &cf, "rust");
            assert!(out.is_empty());
        }

        #[test]
        fn empty_changed_files_returns_empty() {
            let cfg = base_config();
            let cf = changed(&[]);
            let out = match_file_pattern(&cfg, &cf, "rust");
            assert!(out.is_empty());
        }
    }

    mod process_test_discovery_tests {
        use super::*;

        fn discovery_cfg() -> TestDiscoveryConfig {
            TestDiscoveryConfig {
                source_pattern: "rust_src".to_string(),
                strategies: vec![TestDiscoveryStrategy::PathMapping {
                    rules: vec![PathMappingRule {
                        source: "src/{path}.rs".to_string(),
                        tests: vec!["tests/{path}_test.rs".to_string()],
                    }],
                }],
            }
        }

        #[test]
        fn returns_none_when_no_source_files_match() {
            let cfg = base_config();
            let cf = changed(&["README.md"]);
            let check = mk_check("cmd {files}", None, None, false);
            let disc = discovery_cfg();
            let mut matched = Vec::new();
            let out = process_test_discovery(
                &cfg,
                &cf,
                &PathBuf::from("/tmp"),
                &disc,
                &check,
                &mut matched,
            );
            assert!(out.is_none());
            assert!(matched.is_empty());
        }

        #[test]
        fn returns_on_demand_when_no_tests_found_and_on_demand() {
            let tmp = tempfile::TempDir::new().unwrap();
            let cfg = base_config();
            let cf = changed(&["src/foo.rs"]);
            let check = mk_check("cmd {files}", None, None, /*on_demand=*/ true);
            let disc = discovery_cfg();
            let mut matched = Vec::new();
            let out = process_test_discovery(&cfg, &cf, tmp.path(), &disc, &check, &mut matched);
            assert!(matches!(out, Some(TestDiscoveryResult::OnDemand)));
        }

        #[test]
        fn pushes_sentinel_when_no_tests_and_command_has_no_files_placeholder() {
            let tmp = tempfile::TempDir::new().unwrap();
            let cfg = base_config();
            let cf = changed(&["src/foo.rs"]);
            let check = mk_check("run-all-tests", None, None, false);
            let disc = discovery_cfg();
            let mut matched = Vec::new();
            let out = process_test_discovery(&cfg, &cf, tmp.path(), &disc, &check, &mut matched);
            assert!(out.is_none());
            assert_eq!(matched.len(), 1);
            assert!(matched[0].contains("source files changed"));
        }

        #[test]
        fn no_sentinel_when_command_has_files_placeholder() {
            let tmp = tempfile::TempDir::new().unwrap();
            let cfg = base_config();
            let cf = changed(&["src/foo.rs"]);
            let check = mk_check("test {files}", None, None, false);
            let disc = discovery_cfg();
            let mut matched = Vec::new();
            let out = process_test_discovery(&cfg, &cf, tmp.path(), &disc, &check, &mut matched);
            assert!(out.is_none());
            assert!(matched.is_empty());
        }

        #[test]
        fn extends_matched_files_when_tests_found() {
            let tmp = tempfile::TempDir::new().unwrap();
            let test_path = tmp.path().join("tests/foo_test.rs");
            std::fs::create_dir_all(test_path.parent().unwrap()).unwrap();
            std::fs::write(&test_path, "").unwrap();

            let cfg = base_config();
            let cf = changed(&["src/foo.rs"]);
            let check = mk_check("cmd {files}", None, None, false);
            let disc = discovery_cfg();
            let mut matched = Vec::new();
            let out = process_test_discovery(&cfg, &cf, tmp.path(), &disc, &check, &mut matched);
            assert!(out.is_none());
            assert_eq!(matched, vec!["tests/foo_test.rs".to_string()]);
        }
    }

    mod build_check_to_run_tests {
        use super::*;

        #[test]
        fn sets_fields_and_resolves_command_with_files() {
            let def = mk_check("cargo test {files}", Some("cargo fmt {files}"), None, false);
            let out = build_check_to_run(
                "id",
                &def,
                "g",
                "svc".to_string(),
                vec!["a.rs".into(), "b.rs".into()],
            );
            assert_eq!(out.id, "id");
            assert_eq!(out.group, "g");
            assert_eq!(out.service, "svc");
            assert_eq!(out.files, vec!["a.rs".to_string(), "b.rs".to_string()]);
            assert_eq!(out.resolved_command, "cargo test a.rs b.rs");
            assert_eq!(
                out.resolved_fix_command.as_deref(),
                Some("cargo fmt a.rs b.rs")
            );
            assert!(!out.on_demand);
            assert!(!out.skipped_no_files);
        }
    }

    mod build_skipped_check_tests {
        use super::*;

        #[test]
        fn marks_on_demand_and_copies_placeholder_flag() {
            let def = mk_check("cmd {files}", None, None, false);
            let out = build_skipped_check("id", &def, "g", "svc".to_string(), true);
            assert!(out.on_demand);
            assert!(out.skipped_no_files);
            assert_eq!(out.files, vec!["(skipped - no matching files)".to_string()]);
            assert_eq!(out.resolved_command, "cmd");
        }

        #[test]
        fn without_placeholder_flag_skipped_no_files_false() {
            let def = mk_check("always", None, None, false);
            let out = build_skipped_check("id", &def, "g", "svc".to_string(), false);
            assert!(out.on_demand);
            assert!(!out.skipped_no_files);
        }
    }

    mod build_on_demand_check_tests {
        use super::*;

        #[test]
        fn marks_on_demand_with_prompt_sentinel() {
            let def = mk_check("slow-test {files}", None, None, false);
            let out = build_on_demand_check("id", &def, "g", "svc".to_string());
            assert!(out.on_demand);
            assert!(!out.skipped_no_files);
            assert_eq!(
                out.files,
                vec!["(on-demand - press 't' to run)".to_string()]
            );
            assert_eq!(out.resolved_command, "slow-test");
        }
    }

    mod process_check_tests {
        use super::*;

        #[test]
        fn always_run_path_used_when_no_triggers() {
            let cfg = base_config();
            let def = mk_check("cargo bench", None, None, false);
            let cf = changed(&[]);
            let out = process_check(
                &cfg,
                &cf,
                &PathBuf::from("/tmp"),
                "g",
                "id",
                &def,
                "default-svc",
            );
            let run = out.expect("always-run should produce CheckToRun");
            assert!(run.files.is_empty());
            assert_eq!(run.resolved_command, "cargo bench");
            assert_eq!(run.service, "default-svc");
        }

        #[test]
        fn service_override_wins_over_default() {
            let cfg = base_config();
            let mut def = mk_check("cmd", None, None, false);
            def.service = Some("special".to_string());
            let cf = changed(&[]);
            let out = process_check(
                &cfg,
                &cf,
                &PathBuf::from("/tmp"),
                "g",
                "id",
                &def,
                "default-svc",
            );
            assert_eq!(out.unwrap().service, "special");
        }
    }

    mod process_triggered_check_tests {
        use super::*;

        fn triggers_file_pattern(key: &str) -> CheckTriggers {
            CheckTriggers {
                file_pattern: Some(key.to_string()),
                test_discovery: None,
            }
        }

        #[test]
        fn returns_none_when_no_triggers_match() {
            let cfg = base_config();
            let def = mk_check("cmd", None, Some(CheckTriggers::default()), false);
            let cf = changed(&["src/main.rs"]);
            let out =
                process_triggered_check(&cfg, &cf, &PathBuf::from("/tmp"), "g", "id", &def, "svc");
            assert!(out.is_none());
        }

        #[test]
        fn file_pattern_match_builds_check() {
            let cfg = base_config();
            let def = mk_check(
                "cargo check {files}",
                None,
                Some(triggers_file_pattern("rust")),
                false,
            );
            let cf = changed(&["src/main.rs", "README.md"]);
            let out =
                process_triggered_check(&cfg, &cf, &PathBuf::from("/tmp"), "g", "id", &def, "svc");
            let run = out.expect("file_pattern match should produce CheckToRun");
            assert_eq!(run.files, vec!["src/main.rs".to_string()]);
            assert_eq!(run.resolved_command, "cargo check src/main.rs");
            assert!(!run.on_demand);
        }

        #[test]
        fn file_pattern_no_match_builds_skipped() {
            let cfg = base_config();
            let def = mk_check(
                "cargo check {files}",
                None,
                Some(triggers_file_pattern("rust")),
                false,
            );
            let cf = changed(&["README.md"]);
            let out =
                process_triggered_check(&cfg, &cf, &PathBuf::from("/tmp"), "g", "id", &def, "svc");
            let run = out.expect("should produce skipped CheckToRun");
            assert!(run.on_demand);
            assert!(run.skipped_no_files);
            assert_eq!(run.files, vec!["(skipped - no matching files)".to_string()]);
        }

        #[test]
        fn duplicates_are_deduplicated_preserving_order() {
            let cfg = base_config();
            let def = mk_check(
                "cmd {files}",
                None,
                Some(triggers_file_pattern("rust")),
                false,
            );
            let cf = changed(&["src/a.rs", "src/b.rs", "src/a.rs"]);
            let out =
                process_triggered_check(&cfg, &cf, &PathBuf::from("/tmp"), "g", "id", &def, "svc")
                    .unwrap();
            assert_eq!(
                out.files,
                vec!["src/a.rs".to_string(), "src/b.rs".to_string()]
            );
        }

        #[test]
        fn test_discovery_no_tests_on_demand_builds_on_demand() {
            let tmp = tempfile::TempDir::new().unwrap();
            let cfg = base_config();
            let triggers = CheckTriggers {
                file_pattern: None,
                test_discovery: Some(TestDiscoveryConfig {
                    source_pattern: "rust_src".to_string(),
                    strategies: vec![TestDiscoveryStrategy::PathMapping {
                        rules: vec![PathMappingRule {
                            source: "src/{path}.rs".to_string(),
                            tests: vec!["tests/{path}_test.rs".to_string()],
                        }],
                    }],
                }),
            };
            let def = mk_check(
                "slow {files}",
                None,
                Some(triggers),
                /*on_demand=*/ true,
            );
            let cf = changed(&["src/foo.rs"]);
            let out =
                process_triggered_check(&cfg, &cf, tmp.path(), "g", "id", &def, "svc").unwrap();
            assert!(out.on_demand);
            assert_eq!(
                out.files,
                vec!["(on-demand - press 't' to run)".to_string()]
            );
        }
    }
}
