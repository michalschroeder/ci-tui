---
phase: quick
plan: 011
type: execute
wave: 1
depends_on: []
files_modified:
  - src/config.rs
  - src/runner.rs
  - ci-tui.yaml
autonomous: true

must_haves:
  truths:
    - "CI-TUI can run checks using standalone docker run with volume mounts"
    - "Existing docker-compose based configs continue to work unchanged"
    - "ci-tui.yaml defines fmt, clippy, test checks matching Makefile"
  artifacts:
    - path: "src/config.rs"
      provides: "DockerConfig with image, volume_mount, work_dir fields"
      contains: "image: Option<String>"
    - path: "src/runner.rs"
      provides: "build_docker_run_command using config fields"
      contains: "volume_mount"
    - path: "ci-tui.yaml"
      provides: "Self-hosting config for CI-TUI project"
      contains: "rust:latest"
  key_links:
    - from: "src/runner.rs"
      to: "src/config.rs"
      via: "DockerConfig fields"
      pattern: "config\\.docker\\.(image|volume_mount|work_dir)"
---

<objective>
Enable CI-TUI to self-host by adding standalone Docker run support with volume mounts.

Purpose: Currently build_docker_run_command derives image from container name and uses hardcoded /app workdir with no volume mounting. This works for docker-compose setups but fails for standalone docker run where the code isn't in the image.

Output: Extended DockerConfig with image/volume_mount/work_dir fields, updated runner to use them, and ci-tui.yaml for self-hosting.
</objective>

<execution_context>
@./.claude/get-shit-done/workflows/execute-plan.md
@./.claude/get-shit-done/templates/summary.md
</execution_context>

<context>
@.planning/PROJECT.md
@.planning/STATE.md
@src/config.rs
@src/runner.rs
@Makefile
</context>

<tasks>

<task type="auto">
  <name>Task 1: Extend DockerConfig with standalone run fields</name>
  <files>src/config.rs</files>
  <action>
Add three new optional fields to DockerConfig struct:

1. `image: Option<String>` - Docker image for standalone run (e.g., "rust:latest")
   - If set, use this image directly instead of deriving from container name
   - Add serde default attribute

2. `volume_mount: Option<String>` - Volume mount string (e.g., ".:/build")
   - Full volume specification including source:dest
   - Add serde default attribute

3. `work_dir: Option<String>` - Working directory inside container (e.g., "/build")
   - Overrides the default /app workdir
   - Add serde default attribute

Add getter methods to DockerConfig:
- `image_name(&self) -> String` - Returns explicit image if set, otherwise derives from container_name() by stripping -1 suffix (existing logic)
- `volume_args(&self) -> Option<String>` - Returns "-v {volume_mount}" if volume_mount is set, None otherwise
- `working_dir(&self) -> &str` - Returns work_dir if set, otherwise "/app"

Update the test config YAML and tests to include these new optional fields (can be empty in existing tests since they're optional).
  </action>
  <verify>cargo check passes, existing tests pass with cargo test</verify>
  <done>DockerConfig has image, volume_mount, work_dir fields with getter methods</done>
</task>

<task type="auto">
  <name>Task 2: Update build_docker_run_command to use config fields</name>
  <files>src/runner.rs</files>
  <action>
Modify build_docker_run_command function signature and implementation:

Current signature:
```rust
fn build_docker_run_command(
    container_name: &str,
    env: &std::collections::HashMap<String, String>,
    command: &str,
) -> String
```

New signature - add DockerConfig reference:
```rust
fn build_docker_run_command(
    container_name: &str,
    docker_config: &DockerConfig,
    env: &std::collections::HashMap<String, String>,
    command: &str,
) -> String
```

Update implementation:
1. Use `docker_config.image_name()` instead of deriving from container_name
2. Add volume flag: if `docker_config.volume_args()` returns Some, include it
3. Use `docker_config.working_dir()` instead of hardcoded "/app"

Build command format (with volume):
```
docker run --rm -v {volume_mount} -w {work_dir} {env_flags} {image} bash -c '{command}'
```

Update all callers of build_docker_run_command in runner.rs:
1. `execute_docker_command` - pass docker config (will need to accept &DockerConfig or Arc<DockerConfig>)
2. `run_pre_command` - pass docker config

This requires threading DockerConfig through:
- CheckRunner already has `config: Arc<CiConfig>`, use `&self.config.docker`
- `run_docker_check` needs docker_config parameter
- `execute_docker_command` needs docker_config parameter
- Public functions `run_single_check`, `run_fix_command`, `run_check_with_command` need docker_config parameter

Update function signatures:
- `execute_docker_command(..., docker_config: &DockerConfig)`
- `run_docker_check(..., docker_config: &DockerConfig, ...)`
- `pub async fn run_single_check(..., docker_config: &DockerConfig, ...)`
- `pub async fn run_fix_command(..., docker_config: &DockerConfig, ...)`
- `pub async fn run_check_with_command(..., docker_config: &DockerConfig, ...)`

Update callers in ui/app.rs or wherever these public functions are called to pass docker_config.
  </action>
  <verify>cargo check passes, cargo test passes</verify>
  <done>build_docker_run_command uses DockerConfig fields for image, volume, workdir</done>
</task>

<task type="auto">
  <name>Task 3: Create ci-tui.yaml self-hosting config</name>
  <files>ci-tui.yaml</files>
  <action>
Create ci-tui.yaml in project root with config for self-hosting CI-TUI:

```yaml
version: 2

docker:
  project_dir: .
  service: rust
  image: rust:latest
  volume_mount: ".:/build"
  work_dir: /build

git:
  base_branch: master
  fallback_branch: HEAD~1

file_patterns:
  rust:
    pattern: '\.rs$'
    color: yellow
  toml:
    pattern: '\.toml$'
    color: blue
  yaml:
    pattern: '\.ya?ml$'
    color: green

ignore_patterns:
  - '\.md$'
  - '\.github/'
  - 'target/'

checks:
  quality:
    name: Code Quality
    parallel: true
    checks:
      fmt:
        name: Format Check
        command: rustup component add rustfmt && cargo fmt -- --check
        fix_command: rustup component add rustfmt && cargo fmt
        triggers:
          file_pattern: rust

      clippy:
        name: Clippy Lints
        command: rustup component add clippy && cargo clippy -- -D warnings
        triggers:
          file_pattern: rust

  tests:
    name: Tests
    checks:
      test:
        name: Unit Tests
        command: cargo install cargo-nextest --locked 2>/dev/null || true && cargo nextest run
        triggers:
          file_pattern: rust
```

Notes:
- Uses rust:latest image (matches Makefile RUST_IMAGE)
- Volume mounts current dir to /build (matches Makefile)
- work_dir set to /build (matches Makefile -w flag)
- fmt and clippy in parallel quality group
- test in sequential tests group
- Commands match Makefile targets (with component installs inline)
  </action>
  <verify>cargo run -- --config ci-tui.yaml --help works (config parses)</verify>
  <done>ci-tui.yaml exists and parses correctly</done>
</task>

</tasks>

<verification>
1. cargo check - compiles without errors
2. cargo test - all existing tests pass
3. cargo run -- --config ci-tui.yaml --help - config parses
4. Optional: cargo run -- --config ci-tui.yaml (with some .rs file changed) - runs checks in docker
</verification>

<success_criteria>
- DockerConfig extended with image, volume_mount, work_dir optional fields
- build_docker_run_command uses these fields when set
- Backward compatibility: existing configs without new fields work unchanged
- ci-tui.yaml created with working self-hosting config
- All tests pass
</success_criteria>

<output>
After completion, create `.planning/quick/011-create-ci-tui-config-for-self-hosting/011-SUMMARY.md`
</output>
