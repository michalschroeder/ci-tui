---
phase: quick
plan: 003
type: execute
wave: 1
depends_on: []
files_modified:
  - run.sh
  - Makefile
autonomous: true

must_haves:
  truths:
    - "run.sh is removed from the repository"
    - "make run and make run-local targets are removed"
    - "Makefile still has all legitimate development targets (test, fmt, clippy, build, etc.)"
  artifacts:
    - path: "Makefile"
      provides: "Development and Docker build targets"
      contains: "build:"
  key_links: []
---

<objective>
Remove unused run.sh script and associated Makefile targets that were designed for end-users of ci-tui in other projects, not for ci-tui development itself.

Purpose: Clean up dead code. The run.sh script expects a different project structure (goes up 3 directories, expects tools/ci/ci-config.yaml) and is not relevant to ci-tui development. For developing ci-tui itself, `make build` + `cargo run` is the correct workflow.

Output: Cleaner Makefile focused on development tasks, no orphan run.sh script.
</objective>

<context>
@./Makefile
@./run.sh

Analysis:
- run.sh line 8: `PROJECT_ROOT="$(cd "$SCRIPT_DIR/../../.." && pwd)"` - expects script to be 3 levels deep in another project
- run.sh line 17: `CONFIG_FILE="${CI_TUI_CONFIG:-./tools/ci/ci-config.yaml}"` - path doesn't exist in ci-tui repo
- run.sh line 125: fetches `origin development` - not relevant to this repo's git structure
- `make run` just calls ./run.sh
- `make run-local` duplicates what developers can do with `make build && cargo run`
</context>

<tasks>

<task type="auto">
  <name>Task 1: Remove run.sh and update Makefile</name>
  <files>run.sh, Makefile</files>
  <action>
1. Delete run.sh from the repository

2. Update Makefile to remove:
   - `run` from .PHONY line
   - `run-local` from .PHONY line
   - The entire `run:` target block (lines 54-55)
   - The entire `run-local:` target block (lines 57-62)

3. Keep all other targets intact:
   - help, test, fmt, fmt-check, clippy, coverage, ci
   - build, build-no-cache, push, pull, clean

4. Ensure the Makefile still compiles (no dangling references)
  </action>
  <verify>
    - `ls run.sh` returns "No such file"
    - `make help` works and shows no run/run-local targets
    - `make build` still works
    - `grep -E "^run" Makefile` returns nothing
  </verify>
  <done>
    - run.sh deleted
    - Makefile has no run or run-local targets
    - All development targets still functional
  </done>
</task>

<task type="auto">
  <name>Task 2: Commit changes</name>
  <files>run.sh, Makefile</files>
  <action>
Stage the deletion of run.sh and modified Makefile, then commit with message:

```
chore(003): remove unused run.sh and run targets

run.sh was designed for end-users running ci-tui in other projects,
not for ci-tui development. It expected a different directory structure
(3 levels deep) and config paths that don't exist in this repo.

For ci-tui development, use: make build && cargo run -- --config <path>
```
  </action>
  <verify>
    - `git status` shows clean working tree
    - `git log -1 --oneline` shows the commit
  </verify>
  <done>Changes committed to git</done>
</task>

</tasks>

<verification>
- `make help` lists only legitimate development targets
- `make build` succeeds
- No run.sh in repository root
- Git history shows removal commit
</verification>

<success_criteria>
- run.sh removed
- Makefile cleaned of run/run-local targets
- All other Makefile targets work
- Changes committed
</success_criteria>

<output>
After completion, create `.planning/quick/003-review-and-remove-unused-run-sh-and-make/003-SUMMARY.md`
</output>
