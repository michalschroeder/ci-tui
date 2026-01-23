---
phase: quick
plan: 011
subsystem: docker-execution
tags: [docker, config, self-hosting, volume-mount]
completed: 2026-01-23
duration: 8m
type: feature

requires:
  - quick-009: "Docker exec/run detection logic"
  - quick-010: "Removed docker-compose dependency from image"

provides:
  - Standalone docker run support with volume mounts
  - Self-hosting config for CI-TUI project
  - Image/volume/workdir configuration options

affects:
  - Any config needing standalone docker run (non-compose setups)

tech-stack:
  added: []
  patterns:
    - "Docker volume mounting for code access"
    - "Configurable image/workdir per project"

key-files:
  created:
    - ci-tui.yaml: "Self-hosting config with rust:latest image"
  modified:
    - src/config.rs: "Extended DockerConfig with image/volume_mount/work_dir fields"
    - src/runner.rs: "Updated build_docker_run_command to use config fields"
    - src/ui/mod.rs: "Threaded docker_config through event handlers"

decisions:
  - decision: "Add image/volume_mount/work_dir as optional fields"
    rationale: "Maintains backward compatibility while enabling standalone run"
    alternatives: ["Required fields with migration", "Separate config struct"]
    commit: 78c32e2

  - decision: "Derive image name from container_name if not explicit"
    rationale: "Preserves existing behavior for docker-compose setups"
    alternatives: ["Always require explicit image", "Error if not set"]
    commit: 78c32e2

  - decision: "Default work_dir to /app for backward compatibility"
    rationale: "Existing configs expect /app as working directory"
    alternatives: ["/", "No default (require explicit)"]
    commit: 78c32e2
---

# Quick Task 011: Create CI-TUI Config for Self-Hosting

**One-liner:** Added standalone docker run support with volume mounts and created self-hosting config using rust:latest image.

## What Was Done

Extended CI-TUI to support standalone docker run scenarios (no docker-compose) by adding configurable image, volume mount, and working directory to DockerConfig.

### Tasks Completed

**Task 1: Extend DockerConfig with standalone run fields**
- Added `image: Option<String>` for explicit Docker image specification
- Added `volume_mount: Option<String>` for volume mount string (e.g., ".:/build")
- Added `work_dir: Option<String>` for container working directory
- Implemented getter methods: `image_name()`, `volume_args()`, `working_dir()`
- All fields optional for backward compatibility
- Commit: 78c32e2

**Task 2: Update build_docker_run_command to use config fields**
- Modified `build_docker_run_command` signature to accept `&DockerConfig`
- Updated implementation to use `image_name()`, `volume_args()`, `working_dir()`
- Threaded `docker_config` through execution pipeline:
  - `execute_docker_command` → accepts docker_config
  - `run_docker_check` → passes docker_config
  - `run_single_check`, `run_fix_command`, `run_check_with_command` → updated signatures
- Updated UI event handlers to include docker_config in EventChannels
- Commit: 295746e

**Task 3: Create ci-tui.yaml self-hosting config**
- Created ci-tui.yaml in project root
- Configured with:
  - `image: rust:latest` (matches Makefile)
  - `volume_mount: ".:/build"` (mounts project into container)
  - `work_dir: /build` (runs commands in mounted directory)
- Defined checks: fmt, clippy (parallel), test (sequential)
- Commands match Makefile targets with inline component installs
- Commit: e42634c

## Technical Details

### Docker Run Command Construction

**Before (hardcoded):**
```bash
docker run --rm -w /app myproject-app bash -c 'command'
```

**After (configurable):**
```bash
docker run --rm -v .:/build -w /build rust:latest bash -c 'command'
```

### Config Schema Extension

```yaml
docker:
  image: rust:latest        # Optional: explicit image (derives from container if not set)
  volume_mount: ".:/build"  # Optional: volume mount specification
  work_dir: /build          # Optional: working directory (defaults to /app)
```

### Backward Compatibility

Existing configs without new fields work unchanged:
- `image_name()` derives from container name (existing logic)
- `volume_args()` returns None if not set (no -v flag added)
- `working_dir()` returns "/app" if not set (existing default)

## Verification

1. ✅ cargo check - compiles without errors
2. ✅ cargo nextest run - all 66 tests pass
3. ✅ ci-tui --config ci-tui.yaml --help - config parses correctly
4. ✅ Backward compatibility - existing configs unaffected

## Deviations from Plan

None - plan executed exactly as written.

## Next Phase Readiness

**Self-hosting capability complete:**
- CI-TUI can now check itself using standalone docker run
- No docker-compose required for CI-TUI project checks
- Volume mounting enables code access without image rebuilds

**Ready for:**
- Phase 2: Code quality baseline (can use ci-tui.yaml for checks)
- Additional standalone docker run scenarios
- Projects without docker-compose infrastructure

## Metrics

**Commits:** 3
- 78c32e2: feat(quick-011): extend DockerConfig with standalone run fields
- 295746e: feat(quick-011): update build_docker_run_command to use DockerConfig fields
- e42634c: feat(quick-011): create ci-tui.yaml self-hosting config

**Files Changed:**
- src/config.rs: +37 lines (new fields + getters)
- src/runner.rs: +33 lines / -21 deletions (updated command building)
- src/ui/mod.rs: +21 lines / -13 deletions (threaded docker_config)
- ci-tui.yaml: +55 lines (new file)

**Duration:** 8m 29s
**Tests:** 66 passed, 0 failed
