//! `--list` / `--dry-run`: explain which checks would run and why, without
//! executing anything.
//!
//! Evaluates each check once via [`Selection`] — the same evaluation
//! `determine_checks` uses — and derives both decision and reasons from it.

pub use crate::checks::Decision;
use crate::checks::{DiscoveryOutcome, Selection};
use crate::config::{CheckDefinition, CiConfig};
use crate::git::{self, ChangedFiles};
use std::fmt::Write;
use std::path::Path;

/// Label for checks `determine_checks` drops (never shown in TUI / simple mode).
const EXCLUDED_LABEL: &str = "excluded";

/// One configured check with its decision and the reasons behind it.
#[derive(Debug, Clone)]
pub struct CheckExplanation {
    /// Group key (config order)
    pub group: String,
    /// Check key
    pub id: String,
    /// Display name
    pub name: String,
    /// `None`: excluded from the run entirely (empty `triggers` block)
    pub decision: Option<Decision>,
    /// Human-readable reasons, one per evaluated trigger
    pub reasons: Vec<String>,
}

/// Explain every configured check, in config group order.
pub fn explain_checks(
    config: &CiConfig,
    changed_files: &ChangedFiles,
    project_root: &Path,
) -> Vec<CheckExplanation> {
    config
        .groups()
        .flat_map(|(group, g)| g.checks.iter().map(move |(id, check)| (group, id, check)))
        .map(|(group, id, check)| {
            let (decision, reasons) = explain(config, changed_files, project_root, check);
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

/// Decision plus one reason per configured trigger.
fn explain(
    config: &CiConfig,
    changed_files: &ChangedFiles,
    project_root: &Path,
    check: &CheckDefinition,
) -> (Option<Decision>, Vec<String>) {
    let selection = Selection::evaluate(config, changed_files, project_root, check);
    let mut reasons = selection_reasons(&selection);
    let decision = selection
        .into_check_files()
        .map(|files| files.decision(check));
    if decision == Some(Decision::OnDemand) {
        reasons.push("manual trigger only ('t' in TUI)".to_string());
    }
    (decision, reasons)
}

/// One reason per evaluated trigger.
fn selection_reasons(selection: &Selection) -> Vec<String> {
    let Selection::Triggered(eval) = selection else {
        return vec!["no triggers: always runs".to_string()];
    };
    let mut reasons = Vec::new();

    if let Some((key, matched)) = &eval.file_pattern {
        reasons.push(if matched.is_empty() {
            format!("file_pattern `{key}`: no changed file matched")
        } else {
            format!("file_pattern `{key}` matched: {}", matched.join(", "))
        });
    }

    if let Some((discovery, sources, outcome)) = &eval.discovery {
        reasons.push(discovery_reason(
            &discovery.source_pattern,
            sources,
            outcome,
            eval.file_pattern_matched(),
        ));
    }

    if reasons.is_empty() {
        reasons.push(
            "empty triggers block (no file_pattern / test_discovery): never runs".to_string(),
        );
    }
    reasons
}

/// Reason for a `test_discovery` trigger. With `file_pattern_matched`, the
/// file_pattern match decides the run, so no-tests fallbacks don't apply.
fn discovery_reason(
    key: &str,
    sources: &[&str],
    outcome: &DiscoveryOutcome,
    file_pattern_matched: bool,
) -> String {
    let no_tests = |fallback: &str| match file_pattern_matched {
        true => "no tests found".to_string(),
        false => format!("no tests found; {fallback}"),
    };
    let detail = match outcome {
        DiscoveryOutcome::NoSources => {
            return format!("test_discovery `{key}`: no changed file matched")
        }
        DiscoveryOutcome::TestsFound(tests) => format!("discovered tests: {}", tests.join(", ")),
        DiscoveryOutcome::NoTestsOnDemand => no_tests("on_demand"),
        DiscoveryOutcome::NoTestsRunAll => no_tests("runs full command (no {files})"),
        DiscoveryOutcome::NoTestsSkip => no_tests("command needs {files}"),
    };
    format!("test_discovery `{key}` ({}): {detail}", sources.join(", "))
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

    let count = |d: Decision| explained.iter().filter(|e| e.decision == Some(d)).count();
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
    let label = e
        .decision
        .map_or(EXCLUDED_LABEL.to_string(), |d| d.to_string());
    let _ = writeln!(out, "  {label:<10} {}  {}", e.id, e.name);
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
    let [origin, local, _fallback] = git::base_ref_candidates(&config.git);
    if *base_ref == origin || *base_ref == local {
        format!("{base_ref} (git.base_branch `{branch}`)")
    } else {
        format!("{base_ref} (fallback: git.base_branch `{branch}` not found)")
    }
}
