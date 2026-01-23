---
phase: quick
plan: 006
type: execute
wave: 1
depends_on: []
files_modified:
  - src/ui/mod.rs
autonomous: true

must_haves:
  truths:
    - "Pressing 'r' refreshes git changes before retrying the selected check"
    - "The check runs with the updated file list from refreshed git state"
    - "The check uses the same ID but with newly determined files/command"
  artifacts:
    - path: "src/ui/mod.rs"
      provides: "Updated r-hotkey handler with git refresh"
      contains: "get_changed_files"
  key_links:
    - from: "src/ui/mod.rs"
      to: "git::get_changed_files"
      via: "function call before run_single_check"
      pattern: "get_changed_files.*run_single_check"
---

<objective>
Modify the 'r' hotkey to refresh git changes before retrying a single check, ensuring the check runs against the latest list of changed files.

Purpose: When a user fixes code and presses 'r' to retry, the check should detect the new git state (files may have been modified, staged, or committed) rather than using the stale file list from initial startup.

Output: Updated `handle_key_event` function where 'r' key handler refreshes git changes, re-determines the specific check with updated files, then runs it.
</objective>

<execution_context>
@./.claude/get-shit-done/workflows/execute-plan.md
@./.claude/get-shit-done/templates/summary.md
</execution_context>

<context>
@.planning/STATE.md
@src/ui/mod.rs (lines 250-266 for current r-handler, lines 328-346 for R-handler pattern)
@src/git.rs (get_changed_files function)
@src/checks.rs (determine_checks function, CheckToRun struct)
</context>

<tasks>

<task type="auto">
  <name>Task 1: Update r-hotkey to refresh git and re-determine check</name>
  <files>src/ui/mod.rs</files>
  <action>
Modify the `(KeyCode::Char('r'), KeyModifiers::NONE)` handler (around line 250) to:

1. Before calling `run_single_check`, refresh git changes using the same pattern as the 'R' handler:
   - Get the current `base_ref` from `app.changed_files.base_ref.clone()`
   - Call `get_changed_files(&channels.project_root, &base_ref)` to get fresh git state
   - Apply ignore patterns with `new_changed_files.apply_ignore_patterns(&config.ignore_patterns)`
   - Call `determine_checks(config, &new_changed_files, &channels.project_root)` to get updated checks

2. Find the matching check by ID from the new checks list:
   - Get the selected check's ID: `check.id().to_string()`
   - Find the check in new_checks with matching ID: `new_checks.iter().find(|c| c.id() == check_id)`

3. If the check is found in the new list:
   - Use the NEW check (with updated files/command) for `run_single_check`
   - Update `app.changed_files` to the new changed files (so UI reflects current state)
   - Update the check in app.checks to reflect the new files/command

4. If the check is NOT found (e.g., no longer relevant after git changes):
   - Set a status message like "Check no longer applicable after git refresh"
   - Do not run the check

5. Handle errors from `get_changed_files` gracefully - if git refresh fails, fall back to running with existing check (current behavior) and optionally show a status message.

The key insight: The 'R' handler already shows the pattern for refreshing git state. Apply the same refresh logic, but instead of restarting all checks, find and run just the one check with updated state.
  </action>
  <verify>
1. `cargo check` passes
2. `make clippy` passes
3. Manual test: In a project with ci-tui config:
   - Start ci-tui with some changed files
   - Make additional changes to a file
   - Press 'r' on a check - it should detect the new changes
   - The check should run with the updated file list
  </verify>
  <done>
The 'r' hotkey refreshes git changes before retrying, and the check runs with the latest file list. If a check becomes inapplicable after git changes, user sees a status message.
  </done>
</task>

<task type="auto">
  <name>Task 2: Update App state with refreshed check info</name>
  <files>src/ui/mod.rs</files>
  <action>
Ensure the App state is properly updated when 'r' refreshes and re-runs a check:

1. After refreshing git and finding the updated check, update `app.changed_files` to the new state so the UI footer shows accurate file counts.

2. Update the specific check in `app.checks` with the new `CheckToRun` (containing updated files and resolved_command) so the UI shows the correct file list for the retried check.

3. The `reset_check_for_retry` call should happen AFTER we have the new check, using the new check's ID.

This may require:
- Adding a method to App like `update_check_for_retry(&mut self, check_id: &str, new_check: CheckToRun, new_changed_files: ChangedFiles)`
- Or inline updating in the key handler before spawning the async task

The goal is that after 'r' is pressed, the UI immediately reflects:
- Updated file count in footer (from new changed_files)
- Updated file list for the check being retried (from new check)
  </action>
  <verify>
1. `cargo check` passes
2. `make clippy` passes
3. Manual test: After pressing 'r', the UI should show the updated file list for that check, not the stale list from startup
  </verify>
  <done>
The UI state (file count, check file list) updates to reflect the refreshed git state when 'r' is pressed.
  </done>
</task>

</tasks>

<verification>
- `cargo check` compiles without errors
- `make clippy` passes without warnings
- `make test` passes (existing tests should not break)
- Manual verification: 'r' hotkey refreshes git state before retrying
</verification>

<success_criteria>
- Pressing 'r' on a check first refreshes git changes via `get_changed_files`
- The check runs with the updated file list from the refreshed git state
- The UI reflects the updated file count and check file list
- If the check is no longer applicable after refresh, user sees informative message
- Error handling: git refresh failure falls back to existing behavior
</success_criteria>

<output>
After completion, create `.planning/quick/006-r-hotkey-refresh-git-changes-before-retr/006-SUMMARY.md`
</output>
