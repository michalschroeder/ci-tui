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
/// Errors (one line per problem, both flags reported) on an id / group not in
/// the config, listing the valid ones; or when the filters together select no
/// check, naming each `--only` check's groups.
pub fn apply(config: &mut CiConfig, only: &[String], groups: &[String]) -> Result<(), String> {
    if only.is_empty() && groups.is_empty() {
        return Ok(());
    }

    // IndexSet: dedup ids shared across groups, keep config order.
    let check_ids: IndexSet<&str> = config
        .groups()
        .flat_map(|(_, g)| g.checks.keys().map(String::as_str))
        .collect();
    let group_ids: IndexSet<&str> = config.groups().map(|(id, _)| id).collect();
    let errors: Vec<String> = [
        unknown_error("check", "--only", only, &check_ids),
        unknown_error("group", "--group", groups, &group_ids),
    ]
    .into_iter()
    .flatten()
    .collect();
    if !errors.is_empty() {
        return Err(errors.join("\n"));
    }

    let selected = |list: &[String], id: &str| list.is_empty() || list.iter().any(|s| s == id);
    let any_selected = config.groups().any(|(group_id, group)| {
        selected(groups, group_id) && group.checks.keys().any(|id| selected(only, id))
    });
    if !any_selected {
        return Err(no_match_error(config, only));
    }

    config.checks.retain(|group_id, group| {
        if !selected(groups, group_id) {
            return false;
        }
        group.checks.retain(|id, _| selected(only, id));
        !group.checks.is_empty()
    });
    Ok(())
}

/// Console notice that a filter is active, e.g. `Filtered: --only a,b --group g`
/// (so a green run can't hide that other checks were excluded). `None` if no
/// filter.
pub fn describe(only: &[String], groups: &[String]) -> Option<String> {
    let mut parts = Vec::new();
    if !only.is_empty() {
        parts.push(format!("--only {}", only.join(",")));
    }
    if !groups.is_empty() {
        parts.push(format!("--group {}", groups.join(",")));
    }
    (!parts.is_empty()).then(|| format!("Filtered: {}", parts.join(" ")))
}

/// Error naming every value in `requested` missing from `valid`, if any.
fn unknown_error(
    kind: &str,
    flag: &str,
    requested: &[String],
    valid: &IndexSet<&str>,
) -> Option<String> {
    let unknown: Vec<String> = requested
        .iter()
        .filter(|id| !valid.contains(id.as_str()))
        .map(|id| format!("`{id}`"))
        .collect();
    if unknown.is_empty() {
        return None;
    }
    let valid: Vec<&str> = valid.iter().copied().collect();
    Some(format!(
        "unknown {kind} {} in {flag}; valid {kind}s: {}",
        unknown.join(", "),
        valid.join(", ")
    ))
}

/// Empty-selection error: names the groups of each `--only` check so the
/// user can see which `--group` would have matched.
fn no_match_error(config: &CiConfig, only: &[String]) -> String {
    let mut msg = "--only / --group select no checks".to_string();
    for id in only {
        let in_groups: Vec<&str> = config
            .groups()
            .filter(|(_, g)| g.checks.contains_key(id))
            .map(|(g, _)| g)
            .collect();
        msg.push_str(&format!(
            "\n`{id}` is in group(s): {}",
            in_groups.join(", ")
        ));
    }
    msg
}
