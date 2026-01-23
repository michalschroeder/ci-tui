---
phase: quick
plan: 008
subsystem: architecture
tags: [docker, self-hosting, research, execution-model]
requires: []
provides:
  - Self-hosting requirements analysis
  - Solution options with tradeoffs
  - Implementation roadmap
affects: []
tech-stack:
  added: []
  patterns: []
decisions:
  - id: quick-008-docker-run-needed
    title: "CI-TUI needs docker run mode for self-hosting"
    rationale: "Current docker compose exec model requires running services, but CI-TUI itself uses ephemeral docker run commands"
    alternatives:
      - "Add docker-compose.yml to CI-TUI (Option B - immediate)"
      - "Implement docker run mode in runner (Option A - long-term)"
    status: analyzed
key-files:
  created:
    - .planning/quick/008-research-ci-tui-self-hosting-requirement/008-RESEARCH.md
  modified: []
metrics:
  duration: 2m
  completed: 2026-01-23
---

# Quick Task 008: Research CI-TUI Self-Hosting Requirement Summary

**One-liner:** Analyzed docker compose exec vs docker run gap, recommending docker-compose.yml for immediate self-hosting and proper run mode for long-term flexibility

## Objective

Research what changes are needed for CI-TUI to run CI checks on its own codebase (self-hosting), given the architectural mismatch between docker compose exec (current) and docker run (needed).

## What Was Done

### Task 1: Write Research Findings Document

**Completed:** ✅

Created comprehensive 755-line research document analyzing:

1. **Current State Analysis**
   - How runner.rs executes all commands via `docker compose exec`
   - What config.rs supports (project_dir, service, env)
   - How CI-TUI Makefile uses `docker run --rm` pattern

2. **Gap Analysis**
   - Execution model mismatch (persistent vs ephemeral containers)
   - Service names vs image names
   - Missing configuration fields (image, volumes, workdir)
   - No alternative execution path in runner.rs

3. **Four Solution Options**

   **Option A: Add docker run mode to runner**
   - Add DockerMode enum (Compose | Run)
   - Add image/volume/workdir fields to DockerConfig
   - Modify execute_docker_command to handle both modes
   - Effort: 6 hours
   - Best for: Long-term flexibility

   **Option B: Create minimal docker-compose.yml**
   - Add docker-compose.yml with rust service + sleep infinity
   - Add ci-tui.yaml matching Makefile checks
   - Zero code changes, works immediately
   - Effort: 1 hour
   - Best for: Immediate self-hosting

   **Option C: Per-check exec flag**
   - Extend PreCommand.exec pattern to checks
   - Confusing semantics, scattered config
   - Not recommended

   **Option D: Command prefix override**
   - Add command_prefix field for power users
   - Too low-level, error-prone
   - Not recommended

4. **Recommendations**
   - Immediate: Option B (docker-compose.yml)
   - Long-term: Option A (proper run mode)
   - Rationale: B unblocks dogfooding now, A enables broader adoption later

5. **Example Configs**
   - Complete docker-compose.yml + ci-tui.yaml for Option B
   - Complete ci-tui.yaml with mode=run for Option A
   - Usage workflows for both approaches

**Files:**
- Created: `.planning/quick/008-research-ci-tui-self-hosting-requirement/008-RESEARCH.md` (755 lines)
- Commit: `05aa45e`

## Decisions Made

### Decision: Two-Phase Self-Hosting Strategy

**Context:** CI-TUI needs to dogfood itself, but current architecture requires docker compose services while CI-TUI project uses simple docker run commands.

**Decision:** Implement in two phases:
1. Phase 1 (immediate): Add docker-compose.yml to CI-TUI project (1 hour)
2. Phase 2 (future): Implement docker run mode in runner (6 hours)

**Rationale:**
- Phase 1 unblocks self-hosting immediately with zero code changes
- Validates existing compose architecture works
- Phase 2 enables broader adoption for simple projects
- Two-phase reduces risk (validate with compose before investing in run mode)

**Alternatives Considered:**
- Implement run mode first: Higher risk, 6x longer time to value
- Command prefix override: Too error-prone for primary solution
- Per-check exec flag: Confusing semantics, scattered config

**Impact:**
- Enables CI-TUI developers to dogfood their own tool
- Validates tool works for Rust projects (not just PHP/TypeScript)
- Identifies usability gaps through real usage
- Sets foundation for supporting single-image projects

## Technical Details

### Key Architectural Findings

**Current Execution Model (runner.rs):**
```rust
// Lines 434-449: All commands use this pattern
format!(
    "docker compose --project-directory={} exec -T {} bash -c '{}'",
    docker_project_dir, service, command
)
```

**No Alternative Exists:**
- Pre-commands use compose exec (lines 326-340)
- Check execution uses compose exec (lines 434-449)
- Fix commands use compose exec (line 523)
- Custom commands use compose exec (line 543)

**Config Limitations (config.rs):**
```rust
// Lines 49-58: Only supports compose model
pub struct DockerConfig {
    pub project_dir: String,           // Compose project directory
    pub service: String,                // Compose service name
    pub env: HashMap<String, String>,   // Environment variables
    // Missing: image, mode, volumes, workdir
}
```

**Why This Matters:**
- Target users have multi-service apps (needs compose)
- CI-TUI itself is single-image app (needs run)
- Supporting both patterns enables broader adoption

### Implementation Effort Comparison

| Option | Development | Testing | Docs | Total | Risk |
|--------|-------------|---------|------|-------|------|
| B: docker-compose.yml | 0.5h | 0h | 0.5h | 1h | Low |
| A: docker run mode | 3.5h | 1.5h | 1h | 6h | Medium |
| C: per-check exec | 2h | 1h | 0.5h | 3.5h | High |
| D: command prefix | 0.5h | 0.25h | 0.25h | 1h | High |

## Deviations from Plan

None - plan executed exactly as written.

## Verification Results

✅ Research document exists at expected path
✅ Document covers current architecture (runner.rs, config.rs, Makefile)
✅ Document identifies specific gaps (4 major gaps documented)
✅ Document proposes multiple solution options (4 options with full analysis)
✅ Document includes recommendation with rationale (two-phase approach)
✅ Document exceeds minimum 80 lines (755 lines)

## Next Phase Readiness

**Blockers:** None

**Concerns:** None

**Recommendations:**

1. **Implement Option B First (Priority: High)**
   - Create docker-compose.yml for CI-TUI
   - Create ci-tui.yaml matching Makefile checks
   - Validate self-hosting workflow
   - Low effort (1 hour), immediate value

2. **Dogfood for Validation (Priority: High)**
   - CI-TUI developers use ci-tui daily
   - Identify usability gaps
   - Collect feedback before implementing Option A
   - Validates compose path works correctly

3. **Consider Option A Later (Priority: Medium)**
   - Implement after Option B proves successful
   - Enables targeting simple projects as first-class users
   - Clean architecture with explicit DockerMode enum
   - 6 hours effort when ready

4. **Document Trade-offs (Priority: Low)**
   - When to use compose vs run mode
   - Decision tree for users choosing pattern
   - Examples for both common scenarios

## Reflection

### What Went Well

1. **Thorough Code Analysis**
   - Examined runner.rs execution flow in detail
   - Identified all docker command construction points
   - Found config.rs limitations clearly

2. **Comprehensive Option Analysis**
   - Evaluated 4 distinct approaches
   - Provided concrete examples for each
   - Estimated effort realistically

3. **Practical Recommendations**
   - Two-phase approach balances risk and value
   - Option B unblocks immediate need (1 hour)
   - Option A provides long-term flexibility (6 hours)

4. **Example Configs**
   - Complete, working examples for both options
   - Clear usage workflows
   - Notes on caching and tool installation

### What Could Be Improved

1. **Performance Testing**
   - Should benchmark docker run vs exec for repeated commands
   - How much does tool installation overhead matter?
   - Is CARGO_HOME caching effective?

2. **Multi-Image Projects**
   - Focused on single-image case (CI-TUI)
   - Didn't explore mixed mode (some checks compose, some run)
   - May need hybrid approach for complex projects

3. **Migration Path**
   - Didn't address compose-only users migrating to run mode
   - Backward compatibility strategy unclear
   - Deprecation timeline for compose-only?

### Lessons Learned

1. **Dogfooding Reveals Design Gaps**
   - CI-TUI designed for complex apps (compose)
   - CI-TUI itself is simple app (run)
   - Tool's own needs don't match target user needs
   - Self-hosting forces broadening target scope

2. **Architecture Should Support Range**
   - Single execution model limits adoption
   - Simple projects have different needs than complex
   - Supporting both patterns enables broader use cases

3. **Incremental Validation Reduces Risk**
   - Option B validates existing architecture (1 hour)
   - Option A builds on proven foundation (6 hours)
   - Two-phase approach better than big-bang implementation

## Related Files

**Research Output:**
- `.planning/quick/008-research-ci-tui-self-hosting-requirement/008-RESEARCH.md` (755 lines)

**Code References (No Changes):**
- `src/runner.rs` (lines 326-340, 434-449, 523)
- `src/config.rs` (lines 49-58, 90-106, 152-166)
- `Makefile` (lines 18, 21, 27)

**Future Implementation:**
- `docker-compose.yml` (Option B - immediate)
- `ci-tui.yaml` (Option B - immediate)
- `src/config.rs` + `src/runner.rs` (Option A - future)

---

**Status:** ✅ Complete
**Duration:** 2 minutes
**Commit:** 05aa45e
