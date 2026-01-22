---
id: quick-001
type: execute
autonomous: true
files_modified: [Makefile]
estimated_context: 20%
---

<objective>
Refactor Makefile to use consistent local cargo execution throughout, removing Docker-based cargo targets in favor of the established local tooling from Phase 1.

Purpose: The Makefile currently mixes Docker-based cargo execution with local cargo inconsistently. Phase 1 established cargo-nextest as the local test runner and the CI pipeline uses local cargo. The Docker-based cargo targets are redundant, slow (clippy reinstalls component each run), and confusing (the `ci` target mixes both approaches).

Output: Clean, organized Makefile with consistent local cargo approach for development and Docker targets only for image building/deployment.
</objective>

<context>
@Makefile
@.github/workflows/ci.yml (if exists - CI uses local cargo)

Key context:
- Phase 1 established cargo-nextest as primary test runner (3x faster)
- CI pipeline runs tests/lints locally, not in Docker
- Docker targets should be for building/deploying the CI-TUI Docker image, not for running cargo commands
- PROJECT_ROOT navigation for `dev` target is a remnant from when this was in a monorepo subdirectory
</context>

<tasks>

<task type="auto">
  <name>Task 1: Reorganize Makefile with local cargo approach</name>
  <files>Makefile</files>
  <action>
Rewrite Makefile with these changes:

1. REMOVE Docker-based cargo targets:
   - Remove `check` (docker rust:alpine cargo check)
   - Remove `fmt` (docker rust:latest cargo fmt)
   - Remove `clippy` (docker rust:latest with component install)
   - Remove `test` (docker rust:alpine cargo test)

2. RENAME local targets to standard names:
   - `test-local` -> `test` (cargo nextest run)
   - `fmt-check` stays as-is (check formatting)
   - Add `fmt` as local cargo fmt

3. UPDATE `ci` target:
   - Should call: `fmt-check`, `clippy`, `test`
   - Add `clippy` as local: `cargo clippy -- -D warnings`

4. REMOVE dead variables:
   - Remove PROJECT_ROOT (monorepo remnant)
   - Remove CONFIG_FILE (not used by current targets)

5. REMOVE `dev` target:
   - It uses PROJECT_ROOT which is a monorepo pattern
   - Use `cargo run` directly instead

6. ORGANIZE into sections with comments:
   ```makefile
   # === Development ===
   # (test, fmt, fmt-check, clippy, coverage targets)

   # === CI ===
   # (ci target that runs all checks)

   # === Docker Image ===
   # (build, run, push, pull, clean targets)
   ```

7. UPDATE help comments to reflect new organization

8. KEEP Docker image targets as-is:
   - build, build-no-cache, run, run-local, push, pull, clean
   - These are for building/deploying the CI-TUI Docker image itself
  </action>
  <verify>
    - `make help` shows organized targets
    - `make fmt-check` runs (cargo fmt -- --check)
    - `make clippy` runs (cargo clippy -- -D warnings)
    - `make test` runs (cargo nextest run)
    - `make ci` runs all three checks
  </verify>
  <done>
    Makefile uses consistent local cargo execution with clear organization. Docker targets only for image management.
  </done>
</task>

<task type="auto">
  <name>Task 2: Verify CI compatibility</name>
  <files>Makefile</files>
  <action>
Run the refactored Makefile targets to confirm they work:

1. Run `make fmt-check` - should pass (code is formatted)
2. Run `make clippy` - verify it runs (may have warnings, that's expected for Phase 2)
3. Run `make test` - should run nextest
4. Run `make ci` - should run all three in sequence

If any target fails due to missing tools, that's acceptable - the user's environment may not have all tools installed. The structure is correct.
  </action>
  <verify>
    Each command executes without Makefile syntax errors. Tool availability is separate from Makefile correctness.
  </verify>
  <done>
    Refactored Makefile targets execute correctly.
  </done>
</task>

</tasks>

<verification>
- [ ] `make help` shows clearly organized sections
- [ ] No Docker-based cargo targets remain
- [ ] `make ci` runs fmt-check, clippy, test (all local)
- [ ] Docker targets (build, run, push, pull) unchanged
- [ ] No references to PROJECT_ROOT or monorepo paths
</verification>

<success_criteria>
Makefile is clean, consistent, and uses local cargo for all development/CI tasks. Docker is only used for building the CI-TUI Docker image itself.
</success_criteria>

<output>
No summary file needed for quick tasks.
</output>
