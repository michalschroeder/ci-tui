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
    config: &CiConfig,
    check_id: &str,
    check: &CheckDefinition,
    group_name: &str,
    service: String,
) -> CheckToRun {
    let resolved = resolve_command(config, check, &[], false);
    let resolved_fix = check
        .fix_command
        .as_ref()
        .map(|_| resolve_command(config, check, &[], true));

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
            config, check_id, check, group_name, service,
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
    config: &CiConfig,
    check_id: &str,
    check: &CheckDefinition,
    group_name: &str,
    service: String,
    matched_files: Vec<String>,
) -> CheckToRun {
    let file_list: Vec<&str> = matched_files.iter().map(|s| s.as_str()).collect();
    let resolved = resolve_command(config, check, &file_list, false);
    let resolved_fix = check
        .fix_command
        .as_ref()
        .map(|_| resolve_command(config, check, &file_list, true));

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
    config: &CiConfig,
    check_id: &str,
    check: &CheckDefinition,
    group_name: &str,
    service: String,
    has_files_placeholder: bool,
) -> CheckToRun {
    let resolved = resolve_command(config, check, &[], false);
    let resolved_fix = check
        .fix_command
        .as_ref()
        .map(|_| resolve_command(config, check, &[], true));

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
    config: &CiConfig,
    check_id: &str,
    check: &CheckDefinition,
    group_name: &str,
    service: String,
) -> CheckToRun {
    let resolved = resolve_command(config, check, &[], false);
    let resolved_fix = check
        .fix_command
        .as_ref()
        .map(|_| resolve_command(config, check, &[], true));

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
                config,
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
            config,
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
                config,
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
