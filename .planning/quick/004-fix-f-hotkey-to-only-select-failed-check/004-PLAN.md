---
phase: quick-004
plan: 01
type: execute
wave: 1
depends_on: []
files_modified:
  - src/ui/app.rs
autonomous: true

must_haves:
  truths:
    - "Pressing 'f' only shows items that actually failed"
    - "Successful pre_commands are hidden when failed filter is active"
    - "Failed pre_commands remain visible when failed filter is active"
  artifacts:
    - path: "src/ui/app.rs"
      provides: "Fixed get_selectable_items filter logic"
      contains: "PreCommandStatus::Failed"
  key_links:
    - from: "get_selectable_items()"
      to: "StatusFilter::Failed"
      via: "conditional pre-command inclusion"
      pattern: "status_filter.*PreCommandStatus"
---

<objective>
Fix the 'f' hotkey to only show actually failed items, excluding successful pre_commands.

Purpose: When user presses 'f' to filter failed items, they expect to see only failed checks and failed pre_commands - not successful pre_commands that happened to run before a failed check.

Output: Modified `get_selectable_items()` method that respects the StatusFilter for both checks AND pre_commands.
</objective>

<execution_context>
@./.claude/get-shit-done/workflows/execute-plan.md
@./.claude/get-shit-done/templates/summary.md
</execution_context>

<context>
@src/ui/app.rs
</context>

<tasks>

<task type="auto">
  <name>Task 1: Fix pre-command filtering in get_selectable_items()</name>
  <files>src/ui/app.rs</files>
  <action>
Modify the `get_selectable_items()` method (around line 654) to filter pre-commands based on the current `status_filter`.

Current code at lines 659-662 always adds pre-commands:
```rust
for pre_cmd in self.pre_commands.iter().filter(|p| p.group == group) {
    items.push(SelectableItem::PreCommand(pre_cmd));
}
```

Change to respect the filter:
```rust
for pre_cmd in self.pre_commands.iter().filter(|p| p.group == group) {
    let include = match self.status_filter {
        StatusFilter::All => true,
        StatusFilter::Failed => pre_cmd.status == PreCommandStatus::Failed,
    };
    if include {
        items.push(SelectableItem::PreCommand(pre_cmd));
    }
}
```

This ensures that when the user presses 'f' to toggle the failed filter, only pre-commands that actually failed (PreCommandStatus::Failed) will be shown, not successful ones (PreCommandStatus::Passed).
  </action>
  <verify>Run `cargo test` to ensure existing tests pass and the filter logic is correct.</verify>
  <done>The get_selectable_items() method filters pre-commands when StatusFilter::Failed is active, only including pre-commands with PreCommandStatus::Failed.</done>
</task>

<task type="auto">
  <name>Task 2: Add unit test for pre-command filtering</name>
  <files>src/ui/app.rs</files>
  <action>
Add a test to the existing test module in app.rs that verifies pre-commands are filtered correctly when StatusFilter::Failed is active.

Add test after the existing tests (around line 1200):
```rust
#[test]
fn test_failed_filter_excludes_passed_pre_commands() {
    let mut app = make_app();

    // Add a pre-command that passed
    app.pre_commands.push(PreCommandState {
        group: "fast".to_string(),
        name: "init-db".to_string(),
        status: PreCommandStatus::Passed,
        output: String::new(),
        duration_ms: 100,
    });

    // Add a pre-command that failed
    app.pre_commands.push(PreCommandState {
        group: "fast".to_string(),
        name: "setup-env".to_string(),
        status: PreCommandStatus::Failed,
        output: "Error".to_string(),
        duration_ms: 50,
    });

    // Mark a check as failed so we have something to filter
    app.results.get_mut("php-lint").unwrap().status = CheckStatus::Failed;

    // With All filter, both pre-commands should be visible
    app.status_filter = StatusFilter::All;
    let items = app.get_selectable_items();
    let pre_cmd_count = items.iter().filter(|i| matches!(i, SelectableItem::PreCommand(_))).count();
    assert_eq!(pre_cmd_count, 2, "All filter should show all pre-commands");

    // With Failed filter, only the failed pre-command should be visible
    app.status_filter = StatusFilter::Failed;
    let items = app.get_selectable_items();
    let pre_cmds: Vec<_> = items.iter().filter_map(|i| {
        if let SelectableItem::PreCommand(pc) = i { Some(pc) } else { None }
    }).collect();
    assert_eq!(pre_cmds.len(), 1, "Failed filter should only show failed pre-commands");
    assert_eq!(pre_cmds[0].name, "setup-env", "Should only show the failed pre-command");
}
```
  </action>
  <verify>Run `cargo test test_failed_filter_excludes_passed_pre_commands` to verify the new test passes.</verify>
  <done>Test exists and passes, verifying that the failed filter correctly excludes passed pre-commands.</done>
</task>

</tasks>

<verification>
1. `cargo test` - All existing tests pass
2. `cargo test test_failed_filter_excludes_passed_pre_commands` - New test passes
3. `cargo clippy` - No new warnings
</verification>

<success_criteria>
- The 'f' hotkey (toggle_failed_filter) only shows items that actually failed
- Successful pre_commands are hidden when failed filter is active
- Failed pre_commands remain visible when failed filter is active
- All existing tests continue to pass
</success_criteria>

<output>
After completion, create `.planning/quick/004-fix-f-hotkey-to-only-select-failed-check/004-SUMMARY.md`
</output>
