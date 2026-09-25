//! Extracted helper functions for determine_checks().
//!
//! These functions handle specific responsibilities:
//! - Matching files to check triggers
//! - Handling test discovery
//! - Building CheckToRun instances via new_check_to_run

use crate::config::{CheckDefinition, CheckTriggers, CiConfig, TestDiscoveryConfig};
use crate::git::ChangedFiles;
use crate::test_discovery;
use std::collections::HashSet;
use std::path::Path;

use super::{resolve_command, CheckFiles, CheckToRun};

/// Single constructor for CheckToRun: resolves command + fix command from files.
pub(super) fn new_check_to_run(
    check_id: &str,
    check: &CheckDefinition,
    group_name: &str,
    service: Option<String>,
    files: CheckFiles,
) -> CheckToRun {
    let file_list: Vec<&str> = files.paths().iter().map(|s| s.as_str()).collect();
    let resolved_command = resolve_command(check, &file_list, false);
    let resolved_fix_command = check
        .fix_command
        .as_ref()
        .map(|_| resolve_command(check, &file_list, true));

    CheckToRun {
        id: check_id.to_string(),
        group: group_name.to_string(),
        definition: check.clone(),
        service,
        files,
        resolved_command,
        resolved_fix_command,
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
    default_service: Option<&str>,
) -> Option<CheckToRun> {
    let service = check.service_or_default(default_service).map(str::to_owned);
    let files =
        Selection::evaluate(config, changed_files, project_root, check).into_check_files()?;
    Some(new_check_to_run(
        check_id, check, group_name, service, files,
    ))
}

/// Match changed files against a file pattern trigger
pub(super) fn match_file_pattern<'a>(
    config: &CiConfig,
    changed_files: &'a ChangedFiles,
    pattern_key: &str,
) -> Vec<&'a str> {
    config
        .get_compiled_file_pattern(pattern_key)
        .map(|re| changed_files.filter_by_pattern(re))
        .unwrap_or_default()
}

/// Outcome of evaluating a test_discovery trigger.
#[derive(Debug, PartialEq, Eq)]
pub(crate) enum DiscoveryOutcome {
    /// source_pattern unknown, or no changed files matched it
    NoSources,
    /// Related test files were discovered
    TestsFound(Vec<String>),
    /// No tests found and the check is marked on_demand — offer manual trigger
    NoTestsOnDemand,
    /// No tests found; command has no {files} placeholder — run the full suite
    NoTestsRunAll,
    /// No tests found; command uses {files} — nothing to run
    NoTestsSkip,
}

/// Evaluate a test_discovery trigger: matched source files plus outcome.
pub(super) fn run_test_discovery<'a>(
    config: &CiConfig,
    changed_files: &'a ChangedFiles,
    project_root: &Path,
    discovery: &TestDiscoveryConfig,
    check: &CheckDefinition,
) -> (Vec<&'a str>, DiscoveryOutcome) {
    let sources = match_file_pattern(config, changed_files, &discovery.source_pattern);
    if sources.is_empty() {
        return (sources, DiscoveryOutcome::NoSources);
    }

    let related_tests =
        test_discovery::find_related_tests(&discovery.strategies, &sources, project_root);
    let outcome = if !related_tests.is_empty() {
        DiscoveryOutcome::TestsFound(related_tests)
    } else if check.on_demand {
        DiscoveryOutcome::NoTestsOnDemand
    } else if check.command.contains("{files}") {
        DiscoveryOutcome::NoTestsSkip
    } else {
        DiscoveryOutcome::NoTestsRunAll
    };
    (sources, outcome)
}

/// How a check is selected. Single evaluation shared by `determine_checks`
/// and `--list`, so both always agree.
#[derive(Debug)]
pub(crate) enum Selection<'a> {
    /// No `triggers`: runs unconditionally
    AlwaysRun,
    /// Result of evaluating the check's triggers
    Triggered(TriggerEval<'a>),
}

impl<'a> Selection<'a> {
    pub(crate) fn evaluate(
        config: &CiConfig,
        changed_files: &'a ChangedFiles,
        project_root: &Path,
        check: &'a CheckDefinition,
    ) -> Self {
        match &check.triggers {
            None => Self::AlwaysRun,
            Some(triggers) => Self::Triggered(TriggerEval::evaluate(
                config,
                changed_files,
                project_root,
                triggers,
                check,
            )),
        }
    }

    /// Files state for the check; `None` drops it (empty `triggers` block).
    pub(crate) fn into_check_files(self) -> Option<CheckFiles> {
        match self {
            Self::AlwaysRun => Some(CheckFiles::Files(vec![])),
            Self::Triggered(eval) => eval.into_check_files(),
        }
    }
}

/// Per-trigger evaluation of a triggered check, each trigger paired with its config.
#[derive(Debug)]
pub(crate) struct TriggerEval<'a> {
    /// `file_pattern` key + changed files it matched, if configured
    pub file_pattern: Option<(&'a str, Vec<&'a str>)>,
    /// `test_discovery` config + matched source files + outcome, if configured
    pub discovery: Option<(&'a TestDiscoveryConfig, Vec<&'a str>, DiscoveryOutcome)>,
}

impl<'a> TriggerEval<'a> {
    fn evaluate(
        config: &CiConfig,
        changed_files: &'a ChangedFiles,
        project_root: &Path,
        triggers: &'a CheckTriggers,
        check: &CheckDefinition,
    ) -> Self {
        Self {
            file_pattern: triggers
                .file_pattern
                .as_deref()
                .map(|key| (key, match_file_pattern(config, changed_files, key))),
            discovery: triggers.test_discovery.as_ref().map(|discovery| {
                let (sources, outcome) =
                    run_test_discovery(config, changed_files, project_root, discovery, check);
                (discovery, sources, outcome)
            }),
        }
    }

    /// True when the `file_pattern` trigger matched at least one file.
    pub(crate) fn file_pattern_matched(&self) -> bool {
        self.file_pattern
            .as_ref()
            .is_some_and(|(_, matched)| !matched.is_empty())
    }

    /// Files state implied by the evaluation; `None` for an empty `triggers`
    /// block (no `file_pattern`, no `test_discovery`).
    fn into_check_files(self) -> Option<CheckFiles> {
        if self.file_pattern.is_none() && self.discovery.is_none() {
            return None;
        }
        let mut matched_files: Vec<String> = self
            .file_pattern
            .into_iter()
            .flat_map(|(_, matched)| matched)
            .map(String::from)
            .collect();

        match (
            self.discovery.map(|(_, _, outcome)| outcome),
            matched_files.is_empty(),
        ) {
            (Some(DiscoveryOutcome::TestsFound(tests)), _) => matched_files.extend(tests),
            (Some(DiscoveryOutcome::NoTestsOnDemand), true) => return Some(CheckFiles::OnDemand),
            (Some(DiscoveryOutcome::NoTestsRunAll), true) => return Some(CheckFiles::RunAll),
            _ => {}
        }

        // Deduplicate while preserving insertion order — output feeds {files} expansion + UI
        let mut seen = HashSet::new();
        matched_files.retain(|f| seen.insert(f.clone()));

        Some(if matched_files.is_empty() {
            CheckFiles::SkippedNoMatch
        } else {
            CheckFiles::Files(matched_files)
        })
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
            timeout: None,
        }
    }

    fn changed(files: &[&str]) -> ChangedFiles {
        ChangedFiles {
            files: files.iter().map(|s| s.to_string()).collect(),
            base_ref: "main".to_string(),
        }
    }

    mod match_file_pattern_tests {
        use super::*;

        #[test]
        fn returns_matching_files() {
            let cfg = base_config();
            let cf = changed(&["src/main.rs", "README.md", "lib/foo.rs"]);
            let out = match_file_pattern(&cfg, &cf, "rust");
            assert_eq!(out, vec!["src/main.rs", "lib/foo.rs"]);
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

    mod run_test_discovery_tests {
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
        fn no_source_files_match_returns_no_sources() {
            let cfg = base_config();
            let cf = changed(&["README.md"]);
            let check = mk_check("cmd {files}", None, None, false);
            let out =
                run_test_discovery(&cfg, &cf, &PathBuf::from("/tmp"), &discovery_cfg(), &check).1;
            assert_eq!(out, DiscoveryOutcome::NoSources);
        }

        #[test]
        fn unknown_source_pattern_returns_no_sources() {
            let cfg = base_config();
            let cf = changed(&["src/foo.rs"]);
            let check = mk_check("cmd {files}", None, None, false);
            let disc = TestDiscoveryConfig {
                source_pattern: "nonexistent".to_string(),
                strategies: vec![],
            };
            let out = run_test_discovery(&cfg, &cf, &PathBuf::from("/tmp"), &disc, &check).1;
            assert_eq!(out, DiscoveryOutcome::NoSources);
        }

        #[test]
        fn no_tests_found_and_on_demand_returns_on_demand() {
            let tmp = tempfile::TempDir::new().unwrap();
            let cfg = base_config();
            let cf = changed(&["src/foo.rs"]);
            let check = mk_check("cmd {files}", None, None, /*on_demand=*/ true);
            let out = run_test_discovery(&cfg, &cf, tmp.path(), &discovery_cfg(), &check).1;
            assert_eq!(out, DiscoveryOutcome::NoTestsOnDemand);
        }

        #[test]
        fn no_tests_and_no_files_placeholder_returns_run_all() {
            let tmp = tempfile::TempDir::new().unwrap();
            let cfg = base_config();
            let cf = changed(&["src/foo.rs"]);
            let check = mk_check("run-all-tests", None, None, false);
            let out = run_test_discovery(&cfg, &cf, tmp.path(), &discovery_cfg(), &check).1;
            assert_eq!(out, DiscoveryOutcome::NoTestsRunAll);
        }

        #[test]
        fn no_tests_with_files_placeholder_returns_skip() {
            let tmp = tempfile::TempDir::new().unwrap();
            let cfg = base_config();
            let cf = changed(&["src/foo.rs"]);
            let check = mk_check("test {files}", None, None, false);
            let out = run_test_discovery(&cfg, &cf, tmp.path(), &discovery_cfg(), &check).1;
            assert_eq!(out, DiscoveryOutcome::NoTestsSkip);
        }

        #[test]
        fn tests_found_returns_paths() {
            let tmp = tempfile::TempDir::new().unwrap();
            let test_path = tmp.path().join("tests/foo_test.rs");
            std::fs::create_dir_all(test_path.parent().unwrap()).unwrap();
            std::fs::write(&test_path, "").unwrap();

            let cfg = base_config();
            let cf = changed(&["src/foo.rs"]);
            let check = mk_check("cmd {files}", None, None, false);
            let out = run_test_discovery(&cfg, &cf, tmp.path(), &discovery_cfg(), &check).1;
            assert_eq!(
                out,
                DiscoveryOutcome::TestsFound(vec!["tests/foo_test.rs".to_string()])
            );
        }
    }

    mod new_check_to_run_tests {
        use super::*;

        #[test]
        fn resolves_command_and_fix_from_files() {
            let def = mk_check("cargo test {files}", Some("cargo fmt {files}"), None, false);
            let out = new_check_to_run(
                "id",
                &def,
                "g",
                Some("svc".to_string()),
                CheckFiles::Files(vec!["a.rs".into(), "b.rs".into()]),
            );
            assert_eq!(out.id, "id");
            assert_eq!(out.group, "g");
            assert_eq!(out.service.as_deref(), Some("svc"));
            assert_eq!(
                out.files,
                CheckFiles::Files(vec!["a.rs".to_string(), "b.rs".to_string()])
            );
            assert_eq!(out.resolved_command, "cargo test a.rs b.rs");
            assert_eq!(
                out.resolved_fix_command.as_deref(),
                Some("cargo fmt a.rs b.rs")
            );
            assert!(!out.is_on_demand());
            assert!(!out.is_skipped_no_files());
        }

        #[test]
        fn empty_files_strips_placeholder() {
            let def = mk_check("cargo check {files}", None, None, false);
            let out = new_check_to_run(
                "id",
                &def,
                "g",
                Some("svc".to_string()),
                CheckFiles::Files(vec![]),
            );
            assert!(out.files.paths().is_empty());
            assert_eq!(out.resolved_command, "cargo check");
            assert!(out.resolved_fix_command.is_none());
        }

        #[test]
        fn flags_derived_from_variant() {
            let with_files = mk_check("cmd {files}", None, None, false);
            let without_files = mk_check("cmd", None, None, false);
            // (variant, on_demand, skipped_no_files for a `{files}` command)
            for (files, on_demand, skipped) in [
                (CheckFiles::Files(vec![]), false, false),
                (CheckFiles::RunAll, false, false),
                (CheckFiles::SkippedNoMatch, true, true),
                (CheckFiles::OnDemand, true, false),
            ] {
                let out = new_check_to_run(
                    "id",
                    &with_files,
                    "g",
                    Some("svc".to_string()),
                    files.clone(),
                );
                assert_eq!(out.is_on_demand(), on_demand);
                assert_eq!(out.is_skipped_no_files(), skipped);

                // Without {files} the check is never skipped, only on-demand
                let out =
                    new_check_to_run("id", &without_files, "g", Some("svc".to_string()), files);
                assert_eq!(out.is_on_demand(), on_demand);
                assert!(!out.is_skipped_no_files());
            }
        }

        #[test]
        fn non_file_variants_resolve_with_empty_files() {
            let def = mk_check("cmd {files}", None, None, false);
            for files in [
                CheckFiles::SkippedNoMatch,
                CheckFiles::OnDemand,
                CheckFiles::RunAll,
            ] {
                let out = new_check_to_run("id", &def, "g", Some("svc".to_string()), files);
                assert_eq!(out.resolved_command, "cmd");
            }
        }

        // Regression: paren-prefixed paths used to be dropped by a starts_with('(') guard
        #[test]
        fn paren_prefixed_path_is_kept() {
            let def = mk_check("run {files}", None, None, false);
            let out = new_check_to_run(
                "id",
                &def,
                "g",
                Some("svc".to_string()),
                CheckFiles::Files(vec!["(x).rs".into()]),
            );
            assert_eq!(out.resolved_command, "run '(x).rs'");
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
                Some("default-svc"),
            );
            let run = out.expect("always-run should produce CheckToRun");
            assert!(run.files.paths().is_empty());
            assert_eq!(run.resolved_command, "cargo bench");
            assert_eq!(run.service.as_deref(), Some("default-svc"));
        }

        #[test]
        fn always_run_resolves_fix_command_without_files() {
            let cfg = base_config();
            let def = mk_check(
                "cargo clippy",
                Some("cargo clippy --fix {files}"),
                None,
                false,
            );
            let run = process_check(
                &cfg,
                &changed(&[]),
                &PathBuf::from("/tmp"),
                "g",
                "id",
                &def,
                Some("svc"),
            )
            .unwrap();
            assert_eq!(
                run.resolved_fix_command.as_deref(),
                Some("cargo clippy --fix")
            );
            assert!(!run.is_on_demand());
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
                Some("default-svc"),
            );
            assert_eq!(out.unwrap().service.as_deref(), Some("special"));
        }
    }

    mod process_check_triggered_tests {
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
            let out = process_check(
                &cfg,
                &cf,
                &PathBuf::from("/tmp"),
                "g",
                "id",
                &def,
                Some("svc"),
            );
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
            let out = process_check(
                &cfg,
                &cf,
                &PathBuf::from("/tmp"),
                "g",
                "id",
                &def,
                Some("svc"),
            );
            let run = out.expect("file_pattern match should produce CheckToRun");
            assert_eq!(
                run.files,
                CheckFiles::Files(vec!["src/main.rs".to_string()])
            );
            assert_eq!(run.resolved_command, "cargo check src/main.rs");
            assert!(!run.is_on_demand());
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
            let out = process_check(
                &cfg,
                &cf,
                &PathBuf::from("/tmp"),
                "g",
                "id",
                &def,
                Some("svc"),
            );
            let run = out.expect("should produce skipped CheckToRun");
            assert!(run.is_on_demand());
            assert!(run.is_skipped_no_files());
            assert_eq!(run.files, CheckFiles::SkippedNoMatch);
        }

        #[test]
        fn file_pattern_no_match_without_placeholder_is_on_demand_not_skipped() {
            let cfg = base_config();
            let def = mk_check(
                "cargo test",
                None,
                Some(triggers_file_pattern("rust")),
                false,
            );
            let cf = changed(&["README.md"]);
            let run = process_check(
                &cfg,
                &cf,
                &PathBuf::from("/tmp"),
                "g",
                "id",
                &def,
                Some("svc"),
            )
            .unwrap();
            assert_eq!(run.files, CheckFiles::SkippedNoMatch);
            assert!(run.is_on_demand());
            assert!(!run.is_skipped_no_files());
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
            let out = process_check(
                &cfg,
                &cf,
                &PathBuf::from("/tmp"),
                "g",
                "id",
                &def,
                Some("svc"),
            )
            .unwrap();
            assert_eq!(
                out.files,
                CheckFiles::Files(vec!["src/a.rs".to_string(), "src/b.rs".to_string()])
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
            let out = process_check(&cfg, &cf, tmp.path(), "g", "id", &def, Some("svc")).unwrap();
            assert!(out.is_on_demand());
            assert_eq!(out.files, CheckFiles::OnDemand);
        }
    }
}
