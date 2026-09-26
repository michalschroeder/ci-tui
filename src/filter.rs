//! `--only` / `--group`: restrict a run to a subset of configured checks.
//!
//! Filters the config itself, before checks are determined, so every mode
//! (TUI incl. refresh, `--simple`, `--fix`, `--list`) sees the same subset.
//! Triggers still apply: a selected check that no changed file triggers stays
//! skipped / on-demand.

use crate::config::CiConfig;
use indexmap::IndexSet;

/// Keep checks whose id is in `only` and whose group is in `groups`; an empty
/// list does not restrict. Groups left without checks are dropped, so their
/// `pre_commands` don't run.
///
/// Errors on an id / group not in the config (listing the valid ones), or when
/// the filters together select no check.
pub fn apply(config: &mut CiConfig, only: &[String], groups: &[String]) -> Result<(), String> {
    if only.is_empty() && groups.is_empty() {
        return Ok(());
    }

    // IndexSet: dedup ids shared across groups, keep config order.
    let check_ids: Vec<&str> = config
        .groups()
        .flat_map(|(_, g)| g.checks.keys().map(String::as_str))
        .collect::<IndexSet<_>>()
        .into_iter()
        .collect();
    let group_ids: Vec<&str> = config.groups().map(|(id, _)| id).collect();
    check_known("check", "--only", only, &check_ids)?;
    check_known("group", "--group", groups, &group_ids)?;

    let selected = |list: &[String], id: &str| list.is_empty() || list.iter().any(|s| s == id);
    config.checks.retain(|group_id, group| {
        if !selected(groups, group_id) {
            return false;
        }
        group.checks.retain(|id, _| selected(only, id));
        !group.checks.is_empty()
    });

    if config.checks.is_empty() {
        return Err("--only / --group select no checks".to_string());
    }
    Ok(())
}

/// Error naming every value in `requested` missing from `valid`.
fn check_known(kind: &str, flag: &str, requested: &[String], valid: &[&str]) -> Result<(), String> {
    let unknown: Vec<String> = requested
        .iter()
        .filter(|id| !valid.contains(&id.as_str()))
        .map(|id| format!("`{id}`"))
        .collect();
    if unknown.is_empty() {
        return Ok(());
    }
    Err(format!(
        "unknown {kind} {} in {flag}; valid {kind}s: {}",
        unknown.join(", "),
        valid.join(", ")
    ))
}
