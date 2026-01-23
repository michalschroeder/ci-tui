---
phase: quick
plan: 006
subsystem: ui
tags: [rust, ratatui, git, hotkey, refresh]

# Dependency graph
requires:
  - phase: quick-005
    provides: UI hotkey infrastructure and patterns
provides:
  - Dynamic git state refresh on check retry
  - Updated check determination before execution
  - Intelligent fallback handling for git errors
affects: []

# Tech tracking
tech-stack:
  added: []
  patterns:
    - "Git state refresh pattern for single-check operations"
    - "Check ID-based matching after re-determination"

key-files:
  created: []
  modified:
    - src/ui/mod.rs

key-decisions:
  - "Refresh git state before retrying single check to detect new file changes"
  - "Fall back to existing check state if git refresh fails (graceful degradation)"
  - "Update both app.changed_files and app.checks to keep UI state consistent"

patterns-established:
  - "Single-check retry pattern: refresh git → re-determine checks → find by ID → update state → run"

# Metrics
duration: 7min
completed: 2026-01-23
---

# Quick Task 006: r-hotkey Refresh Git Changes Before Retry Summary

**r-hotkey now refreshes git state before retrying checks, ensuring tests run against the latest file changes**

## Performance

- **Duration:** 7 min
- **Started:** 2026-01-23T00:22:36Z
- **Completed:** 2026-01-23T00:29:22Z
- **Tasks:** 2 (implemented as single cohesive change)
- **Files modified:** 2

## Accomplishments
- r-hotkey refreshes git changes before retrying selected check
- Check runs with updated file list from fresh git state
- UI state (file count, check files) updates to reflect refreshed git state
- Graceful error handling: falls back to existing check if git refresh fails
- User-friendly message if check no longer applicable after git refresh

## Task Commits

Each task was committed atomically:

1. **Task 1: Update r-hotkey to refresh git and re-determine check** - `c02b7fe` (feat)
2. **Task 2: Update App state with refreshed check info** - included in `c02b7fe` (feat)
3. **Formatting cleanup** - `6336815` (style)

_Note: Tasks 1 and 2 were implemented together as they are tightly coupled - updating the check requires updating the app state._

## Files Created/Modified
- `src/ui/mod.rs` - Updated r-hotkey handler to refresh git state before retry, find matching check by ID, update app state, handle error cases
- `src/ui/app.rs` - Auto-formatted test code (no functional changes)

## Decisions Made

**1. Reuse 'R' handler pattern for git refresh**
- The 'R' (shift+r) handler already had a clean pattern for refreshing git state
- Applied the same approach (get_changed_files → apply_ignore_patterns → determine_checks) but for single-check retry
- Rationale: Consistency in git refresh behavior across different hotkeys

**2. Fall back to existing check on git errors**
- If get_changed_files fails (e.g., git not available, corrupted repo), continue with existing check
- Rationale: Better to let user retry with stale state than prevent retry entirely

**3. Show status message if check no longer applicable**
- After git refresh, check may no longer match any files (e.g., files reverted or committed)
- Display "Check no longer applicable after git refresh" instead of silently doing nothing
- Rationale: Clear feedback helps users understand why retry didn't happen

**4. Update both app.changed_files and app.checks**
- Updating only app.checks would leave UI footer showing stale file counts
- Updating both ensures UI accurately reflects current git state
- Rationale: Consistent UI state prevents user confusion

## Deviations from Plan

None - plan executed exactly as written.

## Issues Encountered

None - implementation was straightforward following the existing 'R' handler pattern.

## Next Phase Readiness

This completes the quick task. The r-hotkey now provides intelligent retry behavior that adapts to git changes, improving developer workflow when fixing issues and retrying checks.

---
*Quick Task: 006*
*Completed: 2026-01-23*
