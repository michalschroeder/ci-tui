---
phase: quick
plan: 012
type: execute
wave: 1
depends_on: []
files_modified:
  - src/ui/app.rs
  - src/ui/dashboard.rs
autonomous: true

must_haves:
  truths:
    - "User can scroll down to see all output lines when output exceeds visible area"
    - "User can scroll up after scrolling down"
    - "Scroll position resets when selecting a different check"
    - "PageUp/PageDown keys scroll by 10 lines as before"
  artifacts:
    - path: "src/ui/app.rs"
      provides: "Improved scroll_down method with dynamic max_scroll"
    - path: "src/ui/dashboard.rs"
      provides: "Scroll indicator showing position in output"
  key_links:
    - from: "src/ui/app.rs"
      to: "src/ui/dashboard.rs"
      via: "output_scroll and output_area_height fields"
---

<objective>
Fix check output panel scrolling so users can scroll through all output when it exceeds the visible area.

Purpose: Currently the right-side output panel doesn't allow proper scrolling when output is large. The `scroll_down` method uses a hardcoded value of 5 for calculating max scroll, which doesn't account for the actual visible area height.

Output: Working scroll functionality with proper bounds and visual scroll indicator.
</objective>

<execution_context>
@./.claude/get-shit-done/workflows/execute-plan.md
@./.claude/get-shit-done/templates/summary.md
</execution_context>

<context>
@src/ui/app.rs
@src/ui/dashboard.rs
@src/ui/mod.rs
</context>

<tasks>

<task type="auto">
  <name>Task 1: Add output_area_height tracking and fix scroll bounds</name>
  <files>src/ui/app.rs, src/ui/dashboard.rs</files>
  <action>
  1. In `src/ui/app.rs`:
     - Add `output_area_height: u16` field to `App` struct (default 0)
     - Add `set_output_area_height(&mut self, height: u16)` method
     - Update `scroll_down` to use `self.output_area_height` instead of hardcoded 5:
       ```rust
       let visible_lines = self.output_area_height.saturating_sub(2) as usize; // -2 for borders
       let max_scroll = total_lines.saturating_sub(visible_lines);
       ```
     - Update `scroll_up` to just use saturating_sub (already correct)

  2. In `src/ui/dashboard.rs`:
     - In `render_output`, before rendering, call `app.set_output_area_height(area.height)` to track visible height
     - Note: This requires changing the `app` parameter to `&mut App` in render_output and its callers

  Alternative approach (simpler, no signature changes):
     - Pass the area height directly to scroll_down via a new method, or
     - Store the area height when rendering and use it for max_scroll calculation

  Actually, simpler approach - calculate max_scroll in scroll_down based on passed-in visible_lines:
     - Change `scroll_down(&mut self, n: usize)` to `scroll_down(&mut self, n: usize, visible_lines: usize)`
     - The caller in mod.rs can pass a reasonable default (e.g., 20) since we don't have area info there
     - OR: Store last known area height in App and update it during render (requires &mut in render)

  Recommended approach (minimal changes):
     - Add `output_visible_lines: usize` field to App (default 20)
     - In render_output, update this before rendering (requires &mut App)
     - scroll_down uses this field for max_scroll calculation

  Implementation:
     a) Add `output_visible_lines: usize` to App struct, initialize to 20 in new() and reset_for_retry()
     b) Add `pub fn set_output_visible_lines(&mut self, lines: usize)` method
     c) Update scroll_down to use `self.output_visible_lines` instead of 5
     d) In dashboard.rs render_output: change signature to take `&mut App` (not &App)
     e) At start of render_output, call `app.set_output_visible_lines((area.height.saturating_sub(2)) as usize)`
     f) Update render() and render_main() to pass &mut App instead of &App
     g) Update call site in mod.rs line 682: `dashboard::render(&mut app, f)`
  </action>
  <verify>
  - `cargo check` passes
  - `cargo test` passes (existing tests should not break)
  </verify>
  <done>
  - App tracks actual visible lines in output area
  - scroll_down respects actual visible area, allowing scrolling to bottom of long output
  - Signature changes propagated through render chain
  </done>
</task>

<task type="auto">
  <name>Task 2: Add scroll position indicator to output panel title</name>
  <files>src/ui/dashboard.rs</files>
  <action>
  In render_check_output (and other output renderers that use scroll):

  1. Calculate total lines in output content
  2. When output_scroll > 0 OR total_lines > visible_lines, show scroll indicator in title
  3. Format: ` CheckName [42/150] ` showing current position / total lines
  4. Only show indicator when content is scrollable (total > visible)

  Example title modifications:
  - Default: ` PHPUnit Tests `
  - With scroll: ` PHPUnit Tests [25/150] `

  Also update: render_fix_result, render_fix_all_results, render_pre_command_output to show scroll indicator when scrollable.
  </action>
  <verify>
  - `cargo check` passes
  - Visually verify: when output has many lines, title shows scroll position
  - PageDown/PageUp updates the indicator
  </verify>
  <done>
  - Output panel title shows scroll position when content is scrollable
  - User can see where they are in long output
  </done>
</task>

</tasks>

<verification>
1. `cargo check` - no compilation errors
2. `cargo test` - all existing tests pass
3. `cargo clippy` - no new warnings
4. Manual test: Run with a check that produces long output, verify:
   - PageDown scrolls through all content
   - PageUp scrolls back
   - Scroll indicator shows position
   - Selecting different check resets scroll to top
</verification>

<success_criteria>
- Users can scroll through entire output when it exceeds visible area
- Scroll position indicator visible in panel title
- No regressions in existing functionality
- All tests pass
</success_criteria>

<output>
After completion, create `.planning/quick/012-fix-check-output-panel-scrolling/012-SUMMARY.md`
</output>
