---
phase: quick
plan: 008
type: execute
wave: 1
depends_on: []
files_modified:
  - .planning/quick/008-research-ci-tui-self-hosting-requirement/008-RESEARCH.md
autonomous: true

must_haves:
  truths:
    - "Research document explains current docker compose exec architecture"
    - "Research document identifies gap between current and self-hosting needs"
    - "Research document proposes solution approaches with tradeoffs"
  artifacts:
    - path: ".planning/quick/008-research-ci-tui-self-hosting-requirement/008-RESEARCH.md"
      provides: "Self-hosting requirements analysis"
      min_lines: 80
  key_links: []
---

<objective>
Research what changes are needed for CI-TUI to run CI checks on its own codebase (self-hosting).

Purpose: CI-TUI currently uses `docker compose exec` to run checks in running containers. The CI-TUI project itself uses simple `docker run` commands via Makefile targets. This research identifies the gap and proposes solutions.

Output: A research document with findings, options, and recommendations.
</objective>

<execution_context>
@./.claude/get-shit-done/workflows/execute-plan.md
@./.claude/get-shit-done/templates/summary.md
</execution_context>

<context>
Key findings from code analysis:

**Current Execution Model (runner.rs):**
- All commands executed via: `docker compose --project-directory={dir} exec -T {service} bash -c '{command}'`
- Requires running docker compose services (exec into existing container)
- Uses `project_dir` from config to locate docker-compose.yml
- Uses `service` name to identify which container to exec into

**CI-TUI Project's Makefile:**
- Uses `docker run --rm -v $(PWD):/build -w /build rust:latest ...` pattern
- No docker-compose.yml exists
- Ephemeral containers (run, not exec)
- Single `rust:latest` image, no named services

**Config Model (config.rs):**
- DockerConfig has: project_dir (required), service (defaults to "app"), env (HashMap)
- No support for `docker run` mode
- No way to specify image name (only service name)

**Gap Summary:**
1. runner.rs hardcodes `docker compose exec` - no `docker run` alternative
2. config.rs has no fields for docker image names
3. No mechanism to run ephemeral containers vs exec into existing ones
</context>

<tasks>

<task type="auto">
  <name>Task 1: Write research findings document</name>
  <files>.planning/quick/008-research-ci-tui-self-hosting-requirement/008-RESEARCH.md</files>
  <action>
Create a research document that covers:

1. **Current State Analysis**
   - How runner.rs executes commands (docker compose exec pattern)
   - What config.rs supports (project_dir, service, env)
   - How Makefile runs checks (docker run --rm pattern)

2. **Gap Analysis**
   - docker compose exec vs docker run (persistent vs ephemeral)
   - Service names vs image names
   - Project directory (for compose) vs volume mounts (for run)

3. **Solution Options**

   **Option A: Add `docker run` mode to runner**
   - Add new DockerMode enum (Compose | Run)
   - Add image field to DockerConfig (used when mode=Run)
   - Modify execute_docker_command to handle both modes
   - Pros: Clean separation, explicit config
   - Cons: More complex config, parallel code paths

   **Option B: Create minimal docker-compose.yml for ci-tui**
   - Add docker-compose.yml that defines a rust service
   - Keep existing runner.rs unchanged
   - Run `docker compose up -d` before ci-tui
   - Pros: No code changes, works today
   - Cons: Awkward for simple projects, requires running container

   **Option C: Add "exec mode" vs "run mode" per-check**
   - PreCommand already has `exec: bool` field (line 161 config.rs)
   - Extend this pattern to CheckDefinition
   - Pros: Flexible per-check control
   - Cons: Confusing semantics, doesn't solve image name issue

   **Option D: Support command prefix override**
   - Add optional `command_prefix` to DockerConfig
   - Default: `docker compose --project-directory={dir} exec -T {service}`
   - Override: `docker run --rm -v {project_root}:/build -w /build {image}`
   - Pros: Maximum flexibility
   - Cons: Requires users to understand Docker, error-prone

4. **Recommendation**
   - For self-hosting ci-tui specifically: Option B (docker-compose.yml)
   - For general improvement: Option A (proper docker run mode)
   - Reasoning and implementation effort estimates

5. **Example Config for Self-Hosting**
   - What a ci-tui.yaml would look like
   - What docker-compose.yml would need

  </action>
  <verify>File exists with all sections, min 80 lines, covers options A-D</verify>
  <done>Research document provides clear analysis of self-hosting requirements with actionable recommendations</done>
</task>

</tasks>

<verification>
- [ ] Research document exists at expected path
- [ ] Document covers current architecture
- [ ] Document identifies specific gaps
- [ ] Document proposes multiple solution options
- [ ] Document includes recommendation with rationale
</verification>

<success_criteria>
Research document created that enables informed decision about how to approach self-hosting, with enough detail to estimate implementation effort for each option.
</success_criteria>

<output>
After completion, create `.planning/quick/008-research-ci-tui-self-hosting-requirement/008-SUMMARY.md`
</output>
