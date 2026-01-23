---
phase: quick
plan: 009
subsystem: execution
tags: [docker, execution, infrastructure]

requires:
  phases: []
  features: [docker-compose-execution]

provides:
  features: [docker-exec-run-execution]
  capabilities: [container-running-detection, fallback-execution]

affects:
  phases: []
  files: [src/runner.rs, src/simple.rs, src/config.rs, src/ui/mod.rs]

tech-stack:
  added: []
  patterns: [docker-exec-fallback]

key-files:
  created: []
  modified:
    - src/config.rs
    - src/runner.rs
    - src/simple.rs
    - src/ui/mod.rs

decisions:
  - decision: Use docker exec when container running, docker run when not
    rationale: Eliminates docker-compose dependency for check execution
    alternatives: [Always use docker run, Keep docker-compose]
    file: quick/009-PLAN.md
  - decision: Derive container name from project_dir + service by default
    rationale: Maintains backward compatibility with existing configs
    alternatives: [Require explicit container name, Use docker inspect]
    file: src/config.rs
  - decision: Strip -1 suffix from container name for image lookup
    rationale: Docker Compose convention maps container names to images
    alternatives: [Use docker inspect, Separate image config field]
    file: src/runner.rs

metrics:
  duration: 11m 51s
  completed: 2026-01-23

manual-qa-notes: |
  Manual testing recommended:
  1. With running container: verify checks use docker exec
  2. Without running container: verify checks use docker run
  3. Verify backward compatibility with existing configs
---

# Quick Task 009: Replace docker-compose exec with docker exec/run Summary

**One-liner:** Direct docker exec/run execution eliminates docker-compose dependency from check execution

## What Was Completed

### Tasks Executed

1. **Task 1: Update DockerConfig to support container name** (72e3a88)
   - Added optional `container` field to DockerConfig struct
   - Implemented `container_name()` method with fallback to project_dir + service derivation
   - Updated test config examples to document new field
   - Maintains backward compatibility

2. **Task 2: Replace docker compose exec with docker exec in runner.rs** (9b73d4c)
   - Added `is_container_running()` helper to check container state
   - Added `build_docker_exec_command()` for running containers
   - Added `build_docker_run_command()` for non-running containers
   - Updated `execute_docker_command()` to use exec/run pattern with automatic fallback
   - Updated `run_pre_command()` to use new execution pattern
   - Replaced `docker_project_dir` with `container_name` throughout runner.rs
   - Updated ui/mod.rs to use container_name from config

3. **Task 3: Update simple.rs to use docker exec/run** (a3859bf)
   - Added `is_container_running()` helper function
   - Updated `run_check()` to use docker exec/run pattern
   - Replaced docker_project_dir with container_name throughout simple.rs
   - Consistent implementation with runner.rs

### Success Metrics

- ✅ All `docker compose exec` and `docker compose run` calls replaced
- ✅ New config field `container` available for explicit container name
- ✅ Backward compatible: existing configs work via container name derivation
- ✅ All tests pass (66/66)
- ✅ No clippy warnings

## Technical Implementation

### Container Name Resolution

**DockerConfig.container_name():**
```rust
// Explicit container name (if provided)
container: "myproject-app-1"

// Or derived from project_dir + service
// project_dir: "./myproject", service: "app" → "myproject-app-1"
```

### Execution Flow

**Before:**
```bash
docker compose --project-directory=./infra exec -T app bash -c 'command'
```

**After (container running):**
```bash
docker exec myproject-app-1 bash -c 'command'
```

**After (container not running):**
```bash
docker run --rm -w /app myproject-app bash -c 'command'
```

### Container State Detection

```rust
fn is_container_running(container_name: &str) -> bool {
    docker inspect -f '{{.State.Running}}' container_name
    // Returns true if output is "true"
}
```

## Deviations from Plan

None - plan executed exactly as written.

## Decisions Made

1. **Docker exec/run fallback pattern**: Automatically detect container state and choose appropriate command, eliminating need for docker-compose
2. **Container name derivation**: Extract project name from last path component of project_dir, combine with service using compose naming convention
3. **Image name derivation**: Strip -1 suffix from container name for docker run (e.g., "myproject-app-1" → "myproject-app")
4. **Backward compatibility**: Existing configs continue to work without changes via automatic container name derivation

## Testing Notes

### Automated Testing

- All 66 existing tests pass
- No new clippy warnings
- Changes verified via cargo check, cargo test, cargo clippy

### Manual Testing Recommended

1. **With running container**: Verify checks execute via `docker exec`
2. **Without running container**: Verify checks execute via `docker run`
3. **Backward compatibility**: Test with existing config files (no container field)
4. **Explicit container name**: Test with new container field in config

## Next Phase Readiness

### Blockers

None

### Concerns

None - implementation is complete and tested

### Documentation Updates

- Config documentation should note the new optional `container` field
- Migration notes: existing configs work without changes

## Files Changed Summary

| File | Lines Changed | Purpose |
|------|---------------|---------|
| src/config.rs | +25 | Add container field and container_name() method |
| src/runner.rs | +129, -88 | Replace docker-compose with docker exec/run |
| src/simple.rs | +42, -14 | Replace docker-compose with docker exec/run |
| src/ui/mod.rs | +8, -8 | Update to use container_name |

## Commit References

1. **72e3a88** - feat(quick-009): add container name support to DockerConfig
2. **9b73d4c** - feat(quick-009): replace docker compose exec with docker exec/run
3. **a3859bf** - feat(quick-009): update simple.rs to use docker exec/run

---

**Status:** ✅ Complete
**Total Duration:** 11m 51s
**Quality:** All tests passing, no clippy warnings
