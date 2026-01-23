---
phase: quick
plan: 009
type: execute
wave: 1
depends_on: []
files_modified:
  - src/runner.rs
  - src/simple.rs
  - src/config.rs
autonomous: true

must_haves:
  truths:
    - "Checks execute via docker exec when container is running"
    - "Checks fall back to docker run when container is not running"
    - "Existing configs with project_dir continue to work (backward compatible)"
  artifacts:
    - path: "src/runner.rs"
      provides: "Docker exec/run execution logic"
      contains: "docker exec"
    - path: "src/config.rs"
      provides: "Container name configuration"
      contains: "container"
  key_links:
    - from: "src/runner.rs"
      to: "src/config.rs"
      via: "container name from DockerConfig"
      pattern: "docker\\.container"
---

<objective>
Replace docker compose exec with docker exec/run for simpler container execution

Purpose: Remove docker compose dependency from check execution, simplifying deployment and reducing overhead. Currently the app requires a docker-compose.yml to be present even when running in standalone container scenarios.

Output: Modified runner.rs and simple.rs that use `docker exec` (for running containers) with fallback to `docker run` (when container not running), plus updated config.rs to support container name configuration.
</objective>

<execution_context>
@./.claude/get-shit-done/workflows/execute-plan.md
@./.claude/get-shit-done/templates/summary.md
</execution_context>

<context>
@.planning/STATE.md
@src/runner.rs
@src/simple.rs
@src/config.rs
@CLAUDE.md
</context>

<tasks>

<task type="auto">
  <name>Task 1: Update DockerConfig to support container name</name>
  <files>src/config.rs</files>
  <action>
Update DockerConfig struct to add a `container` field:

1. Add optional `container: Option<String>` field to DockerConfig
   - This is the container name for `docker exec` (e.g., "myproject-app-1")
   - If not set, derive from project_dir + service (compose naming convention: `{project}-{service}-1`)

2. Add helper method `container_name(&self) -> String` to DockerConfig:
   - If `container` is Some, return it
   - Otherwise derive: extract project name from project_dir (last path component), combine with service
   - Example: project_dir="./myproject", service="app" -> "myproject-app-1"

3. Keep `project_dir` for backward compatibility (used to derive container name if not explicit)

4. Update the test config YAML examples to include the new field for documentation.
  </action>
  <verify>
    - `cargo check` passes
    - Existing tests in config.rs pass
  </verify>
  <done>DockerConfig has container field and container_name() method</done>
</task>

<task type="auto">
  <name>Task 2: Replace docker compose exec with docker exec in runner.rs</name>
  <files>src/runner.rs</files>
  <action>
Replace docker compose exec with docker exec, adding fallback to docker run:

1. Create helper function `is_container_running(container_name: &str) -> bool`:
   - Run `docker inspect -f '{{.State.Running}}' {container_name}`
   - Return true if output is "true", false otherwise

2. Create helper function `build_docker_exec_command(container_name, env, command) -> String`:
   - Format: `docker exec {env_flags} {container_name} bash -c '{command}'`
   - Env flags: `-e KEY='VALUE'` for each env var

3. Create helper function `build_docker_run_command(image_name, env, command, workdir) -> String`:
   - Format: `docker run --rm {env_flags} -w {workdir} {image_name} bash -c '{command}'`
   - Image name derived from container name (strip the -1 suffix for image lookup, or use container inspect)

4. Update `execute_docker_command()`:
   - Get container_name from config
   - Check if container is running
   - If running: use docker exec
   - If not running: use docker run with image from container inspect (or fall back to container name as image)
   - Remove `docker_project_dir` parameter (no longer needed)

5. Update `run_pre_command()`:
   - Same logic as execute_docker_command
   - Use docker exec when running, docker run when not

6. Update function signatures:
   - `run_docker_check`: replace docker_project_dir with container_name
   - Public functions: update to use container_name from config

Note: The -T flag from docker compose exec is not needed for docker exec (it's already non-TTY by default).
  </action>
  <verify>
    - `cargo check` passes
    - `cargo clippy` has no new warnings
  </verify>
  <done>runner.rs uses docker exec with docker run fallback, no docker compose dependency</done>
</task>

<task type="auto">
  <name>Task 3: Update simple.rs to use docker exec/run</name>
  <files>src/simple.rs</files>
  <action>
Update simple.rs to match the new docker execution pattern:

1. Import the helper functions from runner.rs (or duplicate if they're private)
   - Consider making `is_container_running`, `build_docker_exec_command`, `build_docker_run_command` public

2. Update `run_check()` function:
   - Currently uses `docker compose run --rm php sh -c "..."`
   - Replace with: check if container running -> docker exec, else docker run
   - Use the check's service to get container name

3. Update function signatures:
   - Replace `docker_project_dir: &str` parameter with container_name or config reference
   - Update callers: run_sequential, run_parallel

4. Update the main `run()` function to get container_name from config.
  </action>
  <verify>
    - `cargo check` passes
    - `cargo test` passes
    - `cargo clippy` has no new warnings
  </verify>
  <done>simple.rs uses docker exec/run, consistent with runner.rs</done>
</task>

</tasks>

<verification>
Run full test suite and lints:
```bash
make test
make clippy
```

Manual verification (if docker available):
1. With running container: check uses `docker exec`
2. Without running container: check uses `docker run`
</verification>

<success_criteria>
- All `docker compose exec` and `docker compose run` calls replaced
- New config field `container` available for explicit container name
- Backward compatible: existing configs work (container name derived from project_dir + service)
- All tests pass
- No new clippy warnings
</success_criteria>

<output>
After completion, create `.planning/quick/009-replace-docker-compose-exec-with-docker-/009-SUMMARY.md`
</output>
