//! `--list` / `--dry-run`: explain which checks would run and why, without
//! executing anything.
//!
//! Decisions come from [`determine_checks`] (single source of truth for what
//! runs); reasons are derived alongside by re-evaluating each trigger. Test
//! discovery is therefore evaluated twice for discovery checks — acceptable
//! for a one-shot dry run, and it keeps selection logic untouched.

use crate::checks::{
    determine_checks, match_file_pattern, run_test_discovery, CheckToRun, DiscoveryOutcome,
};
use crate::config::{CheckDefinition, CiConfig};
use crate::git::ChangedFiles;
use std::fmt::Write;
use std::path::Path;

/// What would happen to a check on a real run.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Decision {
    /// Runs automatically
    Run,
    /// Waits for a manual trigger ('t' in the TUI); never runs in `--simple`
    OnDemand,
    /// Nothing to run
    Skipped,
}

impl Decision {
    fn label(self) -> &'static str {
        match self {
            Decision::Run => "run",
            Decision::OnDemand => "on-demand",
            Decision::Skipped => "skipped",
        }
    }
}

/// One configured check with its decision and the reasons behind it.
#[derive(Debug, Clone)]
pub struct CheckExplanation {
    /// Group key (config order)
    pub group: String,
    /// Check key
    pub id: String,
    /// Display name
    pub name: String,
    pub decision: Decision,
    /// Human-readable reasons, one per evaluated trigger
    pub reasons: Vec<String>,
}

/// Explain every configured check, in config group order.
pub fn explain_checks(
    config: &CiConfig,
    changed_files: &ChangedFiles,
    project_root: &Path,
) -> Vec<CheckExplanation> {
    let determined = determine_checks(config, changed_files, project_root);
    config
        .groups()
        .flat_map(|(group, g)| g.checks.iter().map(move |(id, check)| (group, id, check)))
        .map(|(group, id, check)| {
            let run = determined.iter().find(|c| c.group == group && c.id == *id);
            let decision = decision_for(run);
            let mut reasons = trigger_reasons(config, changed_files, project_root, check);
            if decision == Decision::OnDemand {
                reasons.push("manual trigger only ('t' in TUI)".to_string());
            }
            CheckExplanation {
                group: group.to_string(),
                id: id.clone(),
                name: check.name.clone(),
                decision,
                reasons,
            }
        })
        .collect()
}

/// Mirrors the UI's initial status. `None` = omitted by `determine_checks`
/// (empty `triggers` block).
fn decision_for(run: Option<&CheckToRun>) -> Decision {
    match run {
        None => Decision::Skipped,
        Some(c) if c.is_skipped_no_files() => Decision::Skipped,
        Some(c) if c.is_on_demand() => Decision::OnDemand,
        Some(_) => Decision::Run,
    }
}

/// One reason per trigger configured on `check`.
fn trigger_reasons(
    config: &CiConfig,
    changed_files: &ChangedFiles,
    project_root: &Path,
    check: &CheckDefinition,
) -> Vec<String> {
    let Some(triggers) = &check.triggers else {
        return vec!["no triggers: always runs".to_string()];
    };
    let mut reasons = Vec::new();

    if let Some(key) = &triggers.file_pattern {
        let matched = match_file_pattern(config, changed_files, key);
        reasons.push(if matched.is_empty() {
            format!("file_pattern `{key}`: no changed file matched")
        } else {
            format!("file_pattern `{key}` matched: {}", matched.join(", "))
        });
    }

    if let Some(discovery) = &triggers.test_discovery {
        let key = &discovery.source_pattern;
        let sources = match_file_pattern(config, changed_files, key).join(", ");
        let prefix = format!("test_discovery `{key}` ({sources})");
        reasons.push(
            match run_test_discovery(config, changed_files, project_root, discovery, check) {
                DiscoveryOutcome::NoSources => {
                    format!("test_discovery `{key}`: no changed file matched")
                }
                DiscoveryOutcome::TestsFound(tests) => {
                    format!("{prefix}: discovered tests: {}", tests.join(", "))
                }
                DiscoveryOutcome::NoTestsOnDemand => format!("{prefix}: no tests found; on_demand"),
                DiscoveryOutcome::NoTestsRunAll => {
                    format!("{prefix}: no tests found; runs full command (no {{files}})")
                }
                DiscoveryOutcome::NoTestsSkip => {
                    format!("{prefix}: no tests found; command needs {{files}}")
                }
            },
        );
    }

    if reasons.is_empty() {
        reasons.push(
            "empty triggers block (no file_pattern / test_discovery): never runs".to_string(),
        );
    }
    reasons
}

/// Render the full `--list` report: base ref, changed files, checks by group.
///
/// `base_override` is the `--base` value, if given.
pub fn render(
    config: &CiConfig,
    changed_files: &ChangedFiles,
    base_override: Option<&str>,
    explained: &[CheckExplanation],
) -> String {
    let mut out = String::new();
    // `write!` into a String cannot fail.
    let _ = writeln!(
        out,
        "Base ref: {}",
        base_ref_line(config, changed_files, base_override)
    );

    let files = &changed_files.files;
    if files.is_empty() {
        out.push_str("Changed files (0): none\n");
    } else {
        let _ = writeln!(out, "Changed files ({}):", files.len());
        for file in files {
            let _ = writeln!(out, "  {file}");
        }
    }

    for (group, group_config) in config.groups() {
        let _ = writeln!(out, "\n{}", group_config.display_name(group));
        for e in explained.iter().filter(|e| e.group == group) {
            render_check(&mut out, e);
        }
    }

    let count = |d: Decision| explained.iter().filter(|e| e.decision == d).count();
    let _ = writeln!(
        out,
        "\n{} run, {} on-demand, {} skipped (nothing executed)",
        count(Decision::Run),
        count(Decision::OnDemand),
        count(Decision::Skipped)
    );
    out
}

/// Decision line plus one indented line per reason.
fn render_check(out: &mut String, e: &CheckExplanation) {
    let _ = writeln!(out, "  {:<10} {}  {}", e.decision.label(), e.id, e.name);
    for reason in &e.reasons {
        let _ = writeln!(out, "      - {reason}");
    }
}

/// Base ref plus where it came from: `--files`, `--base`, or config resolution.
fn base_ref_line(
    config: &CiConfig,
    changed_files: &ChangedFiles,
    base_override: Option<&str>,
) -> String {
    if changed_files.is_cli_files() {
        return "none (git bypassed: --files)".to_string();
    }
    let base_ref = &changed_files.base_ref;
    if base_override.is_some() {
        return format!("{base_ref} (--base)");
    }
    let branch = &config.git.base_branch;
    if *base_ref == format!("origin/{branch}") || base_ref == branch {
        format!("{base_ref} (git.base_branch `{branch}`)")
    } else {
        format!("{base_ref} (fallback: git.base_branch `{branch}` not found)")
    }
}
