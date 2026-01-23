---
phase: quick-005
plan: 01
type: execute
wave: 1
depends_on: []
files_modified:
  - build.rs
  - Makefile
autonomous: true

must_haves:
  truths:
    - "Footer shows build datetime with time component (HH:MM), not just date"
    - "Both local cargo builds and Docker builds show datetime"
  artifacts:
    - path: "build.rs"
      provides: "Date format for local builds"
      contains: "%H:%M"
    - path: "Makefile"
      provides: "Date format for Docker builds"
      contains: "%H:%M"
  key_links: []
---

<objective>
Update build info to show full datetime (YYYY-MM-DD HH:MM) instead of just date (YYYY-MM-DD).

Purpose: Makes it easier to identify exactly when a build was created, especially when multiple builds happen on the same day.
Output: Footer displays datetime like "d73bafc (built 2026-01-23 14:32)" instead of "d73bafc (built 2026-01-23)"
</objective>

<execution_context>
@./.claude/get-shit-done/workflows/execute-plan.md
@./.claude/get-shit-done/templates/summary.md
</execution_context>

<context>
@build.rs (line 31 - date format for local builds)
@Makefile (line 43 - BUILD_DATE variable for Docker builds)
@src/ui/dashboard.rs (line 902 - where BUILD_DATE is displayed in footer)
</context>

<tasks>

<task type="auto">
  <name>Task 1: Update date format to include time</name>
  <files>build.rs, Makefile</files>
  <action>
    In build.rs line 31:
    - Change `["+%Y-%m-%d"]` to `["+%Y-%m-%d %H:%M"]`

    In Makefile line 43:
    - Change `date +%Y-%m-%d` to `date "+%Y-%m-%d %H:%M"`
    - Note: quotes needed in Makefile because of the space
  </action>
  <verify>
    Run `cargo build` and check the binary shows time in footer:
    ```
    cargo build --release
    ./target/release/ci-tui --version 2>&1 || true
    ```
    Check build.rs and Makefile have the updated format strings.
  </verify>
  <done>
    - build.rs uses format `+%Y-%m-%d %H:%M`
    - Makefile BUILD_DATE uses format `+%Y-%m-%d %H:%M`
    - Local builds show datetime with time component
  </done>
</task>

</tasks>

<verification>
1. `grep -n "%H:%M" build.rs Makefile` shows both files have the time format
2. `cargo build --release` completes successfully
3. Running the binary shows datetime in footer (visible when TUI starts)
</verification>

<success_criteria>
- Build datetime displays as "YYYY-MM-DD HH:MM" format
- Both local (cargo) and Docker builds include time
- No regressions in build process
</success_criteria>

<output>
After completion, create `.planning/quick/005-show-full-datetime-in-build-footer/005-SUMMARY.md`
</output>
