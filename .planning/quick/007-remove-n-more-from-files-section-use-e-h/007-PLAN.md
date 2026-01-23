---
phase: quick
plan: 007
type: execute
wave: 1
depends_on: []
files_modified:
  - src/ui/dashboard.rs
autonomous: true

must_haves:
  truths:
    - "Files section shows truncated format (first 2 files) by default"
    - "Pressing 'e' expands Files section to show all files"
    - "No '+N more' text appears in the output panel"
  artifacts:
    - path: "src/ui/dashboard.rs"
      provides: "Files display with expand toggle"
      contains: "show_full_command"
  key_links:
    - from: "src/ui/dashboard.rs"
      to: "app.show_full_command"
      via: "conditional file display"
      pattern: "show_full_command"
---

<objective>
Remove the "+N more" text from the Files section in the check output panel, using the existing 'e' hotkey to expand and show all files.

Purpose: Simplify the UI by removing redundant information - users already have 'e' to expand details.
Output: Files section that shows truncated list by default, full list when expanded via 'e' key.
</objective>

<execution_context>
@./.claude/get-shit-done/workflows/execute-plan.md
@./.claude/get-shit-done/templates/summary.md
</execution_context>

<context>
@src/ui/dashboard.rs (lines 673-692 - Files display logic)
@src/ui/app.rs (show_full_command field and toggle_full_command method)

Current behavior (lines 673-692):
- Shows all files if <= 3 files
- Shows first 2 files + "+N more" if > 3 files
- The 'e' key toggles `show_full_command` but only affects command display, not files

Target behavior:
- Default: Show first 2-3 files (truncated)
- Expanded ('e' pressed): Show all files
- Never show "+N more" text
</context>

<tasks>

<task type="auto">
  <name>Task 1: Update Files display to use show_full_command toggle</name>
  <files>src/ui/dashboard.rs</files>
  <action>
Modify the Files display logic in `render_check_output()` (around lines 673-692) to:

1. When `app.show_full_command` is false (collapsed):
   - If <= 3 files: show all files joined by comma
   - If > 3 files: show first 3 files joined by comma (no "+N more" text)

2. When `app.show_full_command` is true (expanded):
   - Show all files, one per line for better readability when there are many files
   - Use format: "Files:\n  - file1\n  - file2\n  ..." for expanded view

The current code block to modify:
```rust
// Files (collapsed style)
if let Some(files) = app
    .checks
    .iter()
    .find(|c| c.id() == check.id())
    .map(|c| &c.files)
{
    if !files.is_empty() && !files[0].starts_with('(') {
        let file_count = files.len();
        if file_count <= 3 {
            raw_output
                .push_str(&format!("\x1b[90mFiles: {}\x1b[0m\n\n", files.join(", ")));
        } else {
            raw_output.push_str(&format!(
                "\x1b[90mFiles: {} (+{} more)\x1b[0m\n\n",
                files.iter().take(2).cloned().collect::<Vec<_>>().join(", "),
                file_count - 2
            ));
        }
    }
}
```

Replace with logic that checks `app.show_full_command` to determine display mode.
  </action>
  <verify>
Run `make clippy` to verify no warnings.
Run `make test` to ensure no regressions.
Manual verification: Build and run the TUI, select a check with multiple files, verify:
- Default view shows truncated files without "+N more"
- Pressing 'e' shows all files in expanded format
  </verify>
  <done>
Files section displays truncated format by default (no "+N more" text), and 'e' key expands to show all files.
  </done>
</task>

</tasks>

<verification>
- `make clippy` passes with no warnings
- `make test` passes
- Files section no longer shows "+N more" text
- 'e' key toggles between collapsed (truncated) and expanded (all files) view
</verification>

<success_criteria>
- The "+N more" text is completely removed from the Files section
- Default view shows first 2-3 files in collapsed format
- Expanded view (via 'e' key) shows all files
- No regressions in existing functionality
</success_criteria>

<output>
After completion, create `.planning/quick/007-remove-n-more-from-files-section-use-e-h/007-SUMMARY.md`
</output>
