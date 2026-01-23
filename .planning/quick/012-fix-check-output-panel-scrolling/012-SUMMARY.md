---
phase: quick
plan: 012
subsystem: ui
tags: [ratatui, scroll, tui]

# Dependency graph
requires:
  - phase: quick-007
    provides: Hotkey infrastructure for UI interactions
provides:
  - Dynamic scroll bounds based on actual visible area height
  - Scroll position indicator in output panel titles
affects: [ui-improvements]

# Tech tracking
tech-stack:
  added: []
  patterns:
    - "Dynamic UI state updated during render for scroll calculations"
    - "Scroll indicators show [current/total] when content exceeds visible area"

key-files:
  created: []
  modified:
    - src/ui/app.rs
    - src/ui/dashboard.rs
    - src/ui/mod.rs

key-decisions:
  - "Track output_visible_lines in App state and update during render"
  - "Calculate scroll bounds using actual visible area instead of hardcoded value"
  - "Show scroll position indicator only when content is scrollable"

patterns-established:
  - "Update dynamic UI metrics during render cycle for responsive calculations"
  - "Scroll indicator format: '[line/total]' appended to panel title"

# Metrics
duration: 12min
completed: 2026-01-23
---

# Quick Task 012: Fix Check Output Panel Scrolling Summary

**Dynamic scroll bounds with visible area tracking and [line/total] position indicators in output panel titles**

## Performance

- **Duration:** 12 min
- **Started:** 2026-01-23T10:15:03Z
- **Completed:** 2026-01-23T10:27:11Z
- **Tasks:** 2
- **Files modified:** 3

## Accomplishments
- Output panel scrolling now works correctly for large output exceeding visible area
- Users can scroll through entire content instead of being limited by hardcoded bounds
- Scroll position indicator visible when content is scrollable
- All existing tests updated and passing

## Task Commits

Each task was committed atomically:

1. **Task 1: Add output_area_height tracking and fix scroll bounds** - `1ef7a34` (feat)
   - Added output_visible_lines field to App struct (defaults to 20)
   - Updated scroll_down to use actual visible lines instead of hardcoded 5
   - Changed dashboard::render signature to &mut App for state update

2. **Task 2: Add scroll position indicator to output panel title** - `8840214` (feat)
   - Show [line/total] format when content exceeds visible area
   - Applied to check output, fix result, fix-all results, and pre-command output
   - Only displays when content is scrollable (total > visible)

3. **Test fix for dynamic visible lines** - `e59c0f4` (test)
   - Updated test_scroll_down_and_up to work with dynamic scroll calculation

## Files Created/Modified
- `src/ui/app.rs` - Added output_visible_lines field, set_output_visible_lines method, updated scroll_down logic
- `src/ui/dashboard.rs` - Changed render signatures to &mut App, added scroll indicator calculations to all output renderers
- `src/ui/mod.rs` - Updated terminal.draw call to pass &mut app

## Decisions Made

**Track visible lines in App state instead of passing as parameter:**
- Rationale: Simpler API, state is updated once during render and used by scroll methods
- Alternative considered: Pass visible_lines as parameter to scroll_down
- Chosen approach integrates better with existing App state management

**Update visible lines during render_output:**
- Rationale: Accurate area height available at render time, before scroll calculations
- Calculate as area.height - 2 to account for borders
- Default of 20 handles initial state before first render

**Show scroll indicator only when content exceeds visible area:**
- Rationale: Avoid cluttering titles when scrolling isn't needed
- Format: ` CheckName [42/150] ` shows current line / total lines
- Applied consistently across all output renderers

## Deviations from Plan

None - plan executed exactly as written.

## Issues Encountered

**Test failure after Task 1:**
- Issue: test_scroll_down_and_up failed because it had 10 lines of output with default 20 visible lines
- Resolution: Added `app.set_output_visible_lines(5)` to test to create scrollable scenario
- Committed in: e59c0f4 (test fix)

## User Setup Required

None - no external service configuration required.

## Next Phase Readiness

- Output panel scrolling fully functional for large outputs
- Scroll position indicator provides user feedback during navigation
- No known issues or blockers
- Ready for Phase 2 planning

---
*Phase: quick*
*Completed: 2026-01-23*
