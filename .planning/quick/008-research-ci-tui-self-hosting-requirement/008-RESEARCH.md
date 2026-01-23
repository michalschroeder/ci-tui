# Self-Hosting CI-TUI: Research & Analysis

**Author:** Claude
**Date:** 2026-01-23
**Purpose:** Identify what changes are needed for CI-TUI to run CI checks on its own codebase

---

## Executive Summary

CI-TUI currently requires running Docker Compose services to exec into, but the CI-TUI project itself uses ephemeral `docker run` commands via Makefile. This research identifies the architectural gap and proposes four solution approaches, recommending Option B (minimal docker-compose.yml) for immediate self-hosting and Option A (proper docker run mode) for long-term flexibility.

**Key Finding:** The gap is fundamental - runner.rs hardcodes `docker compose exec` with no alternative execution mode.

---

## 1. Current State Analysis

### 1.1 How runner.rs Executes Commands

**All commands use the same pattern** (lines 326-340, 434-449 in runner.rs):

```rust
format!(
    "docker compose --project-directory={} exec -T {} bash -c '{}'",
    docker_project_dir,
    service,
    command
)
```

**Key characteristics:**
- **exec** mode: Runs in existing container (requires container to be already running)
- **-T flag**: No TTY allocation (non-interactive)
- **project-directory**: Points to directory containing docker-compose.yml
- **service**: Named service from docker-compose.yml (e.g., "php", "app", "db")
- **bash -c**: Wraps command for proper shell execution

**No alternative execution mode exists** - this pattern is used for:
- Pre-commands (lines 325-340)
- Check execution (lines 434-449)
- Fix commands (line 523)
- Custom commands (line 543)

### 1.2 What config.rs Supports

**DockerConfig struct** (lines 49-58):

```rust
pub struct DockerConfig {
    pub project_dir: String,           // Required: docker compose project directory
    pub service: String,                // Default: "app"
    pub env: HashMap<String, String>,   // Environment variables
}
```

**Missing fields for ephemeral containers:**
- No `image` field (only `service` name for compose)
- No `mode` enum (compose vs run)
- No `volumes` configuration (run mode needs explicit volume mounts)
- No `working_dir` configuration (run mode needs explicit work directory)

**Service override capability exists:**
- CheckDefinition has optional `service` field (line 95)
- PreCommand has optional `service` field (line 159)
- Falls back to DockerConfig.service default

### 1.3 How CI-TUI Makefile Runs Checks

**Ephemeral container pattern** (lines 18, 21, 27):

```makefile
docker run --rm -v $(PWD):/build -w /build $(RUST_IMAGE) sh -c "..."
```

**Key characteristics:**
- **run --rm**: Creates new container, auto-removes on exit
- **Volume mount**: `-v $(PWD):/build` mounts project directory
- **Working directory**: `-w /build` sets container working directory
- **Image direct**: Uses `rust:latest` image directly (no docker-compose.yml)
- **No pre-running services**: Container lifecycle is per-command

**Examples:**
- `make test`: Installs cargo-nextest, runs tests
- `make clippy`: Installs clippy component, runs lints
- `make fmt`: Installs rustfmt component, formats code

---

## 2. Gap Analysis

### 2.1 Execution Model Mismatch

| Aspect | CI-TUI Expects | CI-TUI Project Uses |
|--------|----------------|---------------------|
| Container lifecycle | Persistent (exec into running) | Ephemeral (run, auto-remove) |
| Container management | docker compose | docker run |
| Target specification | Service name | Image name |
| Volume mounting | Configured in docker-compose.yml | Explicit `-v` flag |
| Working directory | Configured in docker-compose.yml | Explicit `-w` flag |

### 2.2 Specific Technical Gaps

**Gap 1: No docker run execution path**
- runner.rs line 326-449: All execution hardcoded to `docker compose exec`
- No code path for `docker run --rm`

**Gap 2: No image name configuration**
- DockerConfig only has `service` field (compose service name)
- Can't specify `rust:latest` or other direct image names

**Gap 3: No volume/workdir configuration**
- Assumes docker-compose.yml handles volume mounts
- Assumes docker-compose.yml handles working directory
- `docker run` needs explicit flags: `-v $(PWD):/build -w /build`

**Gap 4: No tool installation strategy**
- Makefile installs cargo tools on-demand: `cargo install cargo-nextest`
- Would need to either:
  - Pre-install in custom image
  - Install per-command (like Makefile does)
  - Use persistent container with tools installed

### 2.3 Why This Matters

**Self-hosting = dogfooding:**
- CI-TUI developers should use CI-TUI for development
- Demonstrates tool works for Rust projects (not just PHP/TypeScript)
- Finds usability issues real users would encounter
- Validates design works for simple projects (not just complex multi-service apps)

**Current state:**
- Can't create ci-tui.yaml without docker-compose.yml
- Adding docker-compose.yml feels awkward for single-image Rust project
- Would diverge from simple Makefile-based workflow

---

## 3. Solution Options

### Option A: Add `docker run` Mode to Runner

**Implementation:**

Add new execution mode to config:

```yaml
# config.rs additions
pub struct DockerConfig {
    pub project_dir: String,
    pub service: String,
    pub env: HashMap<String, String>,
    pub mode: DockerMode,              // NEW
    pub image: Option<String>,         // NEW: for mode=Run
    pub volume_mount: Option<String>,  // NEW: for mode=Run
    pub work_dir: Option<String>,      // NEW: for mode=Run
}

pub enum DockerMode {
    Compose,  // docker compose exec (default)
    Run,      // docker run --rm
}
```

Modify execute_docker_command to handle both:

```rust
async fn execute_docker_command(...) -> CheckResult {
    let docker_cmd = match config.docker.mode {
        DockerMode::Compose => {
            format!("docker compose --project-directory={} exec -T {} bash -c '{}'",
                    project_dir, service, command)
        }
        DockerMode::Run => {
            let image = config.docker.image.as_ref()
                .ok_or("image required for mode=Run")?;
            let volume = config.docker.volume_mount.as_deref()
                .unwrap_or("$(PWD):/build");
            let workdir = config.docker.work_dir.as_deref()
                .unwrap_or("/build");
            format!("docker run --rm -v {} -w {} {} bash -c '{}'",
                    volume, workdir, image, command)
        }
    };
    // ... rest of execution
}
```

**Example ci-tui.yaml:**

```yaml
version: 2

docker:
  mode: run
  image: rust:latest
  volume_mount: "$(PWD):/build"
  work_dir: /build

git:
  base_branch: master
  fallback_branch: HEAD~1

file_patterns:
  rust:
    pattern: '\.rs$'
    color: orange

checks:
  quality:
    parallel: true
    checks:
      fmt:
        name: Format check
        command: rustup component add rustfmt && cargo fmt -- --check
        triggers:
          file_pattern: rust
      clippy:
        name: Clippy lints
        command: rustup component add clippy && cargo clippy -- -D warnings
        triggers:
          file_pattern: rust

  tests:
    checks:
      test:
        name: Run tests
        command: cargo install cargo-nextest --locked && cargo nextest run
        triggers:
          file_pattern: rust
```

**Pros:**
- Clean separation of concerns (compose vs run are different modes)
- Explicit configuration (mode, image, volumes all visible)
- No docker-compose.yml needed for simple projects
- Enables CI-TUI for single-image projects (most CLI tools, simple services)
- Config clearly communicates execution model

**Cons:**
- More complex config schema (4 new fields)
- Parallel code paths in runner (increases maintenance)
- Need to handle edge cases (what if mode=Run but service specified?)
- More testing surface area (need tests for both modes)
- Backward compatibility considerations (default to Compose mode)

**Effort estimate:** 4-6 hours
- Config schema changes: 30 min
- Runner modifications: 2 hours
- Error handling: 1 hour
- Tests: 1.5 hours
- Documentation: 1 hour

---

### Option B: Create Minimal docker-compose.yml for CI-TUI

**Implementation:**

Add docker-compose.yml to ci-tui project:

```yaml
# docker-compose.yml
version: '3.8'

services:
  rust:
    image: rust:latest
    volumes:
      - .:/build
    working_dir: /build
    # Keep container running for exec
    command: sleep infinity
```

Add ci-tui.yaml config:

```yaml
# ci-tui.yaml
version: 2

docker:
  project_dir: .
  service: rust

git:
  base_branch: master
  fallback_branch: HEAD~1

file_patterns:
  rust:
    pattern: '\.rs$'
    color: orange

checks:
  quality:
    parallel: true
    checks:
      fmt:
        name: Format check
        command: rustup component add rustfmt && cargo fmt -- --check
        triggers:
          file_pattern: rust
      clippy:
        name: Clippy lints
        command: rustup component add clippy && cargo clippy -- -D warnings
        triggers:
          file_pattern: rust

  tests:
    checks:
      test:
        name: Run tests
        command: cargo install cargo-nextest --locked && cargo nextest run
        triggers:
          file_pattern: rust
```

Workflow:

```bash
# Start service once
docker compose up -d

# Run ci-tui (uses exec into running container)
./target/release/ci-tui --config ci-tui.yaml

# Stop when done
docker compose down
```

**Pros:**
- **Zero code changes** - works with current implementation
- **Works today** - can be implemented immediately
- **Proven pattern** - uses existing compose path
- Simple mental model (docker-compose.yml = environment)
- Service persists between checks (faster if tools cached)

**Cons:**
- **Awkward for simple projects** - docker-compose.yml overkill for single image
- **Container management burden** - need to start/stop service
- **Doesn't match Makefile pattern** - Makefile uses ephemeral containers
- `sleep infinity` is a hack to keep container running
- Container consumes resources between ci-tui runs

**Effort estimate:** 1 hour
- docker-compose.yml: 15 min
- ci-tui.yaml: 30 min
- Documentation: 15 min

---

### Option C: Add "exec Mode" vs "Run Mode" Per-Check

**Implementation:**

Extend PreCommand.exec pattern to CheckDefinition:

```rust
// config.rs
pub struct CheckDefinition {
    pub name: String,
    pub command: String,
    pub service: Option<String>,
    pub exec: bool,  // NEW: true = exec, false = run
    // ... rest of fields
}
```

**Config example:**

```yaml
checks:
  tests:
    checks:
      test:
        name: Run tests
        command: cargo nextest run
        exec: false  # Use docker run instead of exec
```

But this doesn't solve the fundamental issues:
- Still no way to specify image name (only service name)
- Still no way to specify volume mounts
- Still no way to specify working directory

**Would need additional fields:**

```rust
pub struct CheckDefinition {
    pub exec: bool,
    pub image: Option<String>,         // For exec=false
    pub volume_mount: Option<String>,  // For exec=false
    pub work_dir: Option<String>,      // For exec=false
    // ...
}
```

**Pros:**
- Flexible per-check control (mix exec and run in same config)
- PreCommand already has exec field precedent (line 162)

**Cons:**
- **Confusing semantics** - exec field means different things in different contexts
- **Repetitive config** - need image/volume/workdir on every exec=false check
- **Scattered configuration** - docker settings at check level, not global
- **Doesn't match PreCommand.exec** - PreCommand.exec means "use exec instead of run" for compose, different meaning here
- **Complex validation** - need to validate image/volume present when exec=false

**Effort estimate:** 3-4 hours (but not recommended)

---

### Option D: Support Command Prefix Override

**Implementation:**

Add escape hatch for power users:

```rust
// config.rs
pub struct DockerConfig {
    pub project_dir: String,
    pub service: String,
    pub env: HashMap<String, String>,
    pub command_prefix: Option<String>,  // NEW: override default docker command
}
```

**Config example:**

```yaml
docker:
  command_prefix: "docker run --rm -v $(PWD):/build -w /build rust:latest bash -c"
```

Runner would use command_prefix if present:

```rust
let docker_cmd = if let Some(prefix) = &config.docker.command_prefix {
    format!("{} '{}'", prefix, command)
} else {
    // Default docker compose exec pattern
    format!("docker compose --project-directory={} exec -T {} bash -c '{}'", ...)
};
```

**Pros:**
- **Maximum flexibility** - users can use any docker pattern
- **Minimal code changes** - just string substitution
- **Covers edge cases** - users can handle scenarios we haven't thought of
- Quick to implement (1 hour)

**Cons:**
- **Requires Docker expertise** - users need to understand docker run flags
- **Error-prone** - easy to get quoting wrong, forget --rm, etc.
- **No validation** - can't validate prefix is correct
- **Poor error messages** - failures happen at docker level, not ci-tui level
- **Leaky abstraction** - exposes implementation detail (bash -c)
- **Harder to evolve** - if we change runner, breaks user prefixes

**Effort estimate:** 1 hour (but not recommended for primary solution)

---

## 4. Recommendation

### For Immediate Self-Hosting: Option B

**Recommendation:** Create minimal docker-compose.yml for ci-tui project.

**Rationale:**
- **Works immediately** (zero code changes)
- **Unblocks dogfooding** (ci-tui developers can use ci-tui)
- **Validates existing architecture** (proves compose path works)
- **Low risk** (no code changes to test)
- **Can coexist with Makefile** (both workflows work)

**Trade-off acceptance:**
- Yes, docker-compose.yml is awkward for single-image project
- But it's a 15-line file vs 6 hours of development
- Pattern already works for target users (multi-service apps)

**Implementation plan:**
1. Add docker-compose.yml (rust service with sleep infinity)
2. Add ci-tui.yaml matching existing Makefile checks
3. Add docs: "Start docker compose up -d before running ci-tui"
4. Consider this a stepping stone to Option A

### For Long-Term: Option A

**Recommendation:** Implement proper `docker run` mode after validating Option B works.

**Rationale:**
- **Better UX for simple projects** (no compose file needed)
- **Matches user expectations** (simple projects use docker run)
- **Clean architecture** (explicit mode enum)
- **Enables broader adoption** (works for CLI tools, simple services)

**When to implement:**
- After Option B proves self-hosting works
- When we have bandwidth for proper implementation (4-6 hours)
- When we want to target single-image projects as primary users

**Implementation phases:**
1. Add DockerMode enum and new fields to config
2. Modify runner to handle both modes
3. Add comprehensive tests (both modes)
4. Update docs with examples for both patterns
5. Consider deprecation path for compose-only (probably never needed)

### Not Recommended

**Option C (per-check exec flag):** Confusing semantics, scattered config
**Option D (command prefix override):** Too low-level, error-prone

---

## 5. Example Configs for Self-Hosting

### Option B: With docker-compose.yml (Immediate)

**docker-compose.yml:**

```yaml
version: '3.8'

services:
  rust:
    image: rust:latest
    volumes:
      - .:/build
    working_dir: /build
    command: sleep infinity
    environment:
      # Cache cargo registry between runs
      CARGO_HOME: /build/.cargo
```

**ci-tui.yaml:**

```yaml
version: 2

docker:
  project_dir: .
  service: rust

git:
  base_branch: master
  fallback_branch: HEAD~1

file_patterns:
  rust:
    pattern: '\.rs$'
    color: orange
  toml:
    pattern: '\.toml$'
    color: blue

ignore_patterns:
  - '^target/'
  - '^\.git/'

checks:
  format:
    checks:
      fmt-check:
        name: Format check
        command: rustup component add rustfmt && cargo fmt -- --check
        fix_command: rustup component add rustfmt && cargo fmt
        triggers:
          file_pattern: rust

  lint:
    checks:
      clippy:
        name: Clippy lints
        command: rustup component add clippy && cargo clippy -- -D warnings
        triggers:
          file_pattern: rust

  test:
    checks:
      nextest:
        name: Run tests
        command: cargo install cargo-nextest --locked && cargo nextest run
        triggers:
          file_pattern: rust
```

**Usage:**

```bash
# One-time setup
docker compose up -d

# Run ci-tui (many times during development)
cargo run -- --config ci-tui.yaml

# When done for the day
docker compose down
```

**Notes:**
- CARGO_HOME set to cache installed tools (nextest, etc.)
- Tool installation happens on first run, cached after
- Container stays running between ci-tui invocations (faster startup)

---

### Option A: With mode=run (Future)

**ci-tui.yaml:**

```yaml
version: 2

docker:
  mode: run
  image: rust:latest
  volume_mount: "$(PWD):/build"
  work_dir: /build
  env:
    CARGO_HOME: /build/.cargo

git:
  base_branch: master
  fallback_branch: HEAD~1

file_patterns:
  rust:
    pattern: '\.rs$'
    color: orange
  toml:
    pattern: '\.toml$'
    color: blue

ignore_patterns:
  - '^target/'
  - '^\.git/'

checks:
  format:
    checks:
      fmt-check:
        name: Format check
        command: rustup component add rustfmt && cargo fmt -- --check
        fix_command: rustup component add rustfmt && cargo fmt
        triggers:
          file_pattern: rust

  lint:
    checks:
      clippy:
        name: Clippy lints
        command: rustup component add clippy && cargo clippy -- -D warnings
        triggers:
          file_pattern: rust

  test:
    checks:
      nextest:
        name: Run tests
        command: cargo install cargo-nextest --locked && cargo nextest run
        triggers:
          file_pattern: rust
```

**Usage:**

```bash
# Just run (no docker compose needed)
cargo run -- --config ci-tui.yaml
```

**Notes:**
- No docker-compose.yml needed
- Each check runs in ephemeral container (like Makefile)
- CARGO_HOME still cached via volume mount
- Matches existing Makefile workflow mental model

---

## 6. Implementation Effort Summary

| Option | Development | Testing | Docs | Total | Risk |
|--------|-------------|---------|------|-------|------|
| B: docker-compose.yml | 0.5h | 0h | 0.5h | 1h | Low |
| A: docker run mode | 3.5h | 1.5h | 1h | 6h | Medium |
| C: per-check exec | 2h | 1h | 0.5h | 3.5h | High |
| D: command prefix | 0.5h | 0.25h | 0.25h | 1h | High |

**Recommendation:** Start with Option B (1h), validate self-hosting works, then implement Option A (6h) when ready to support simple projects as first-class users.

---

## 7. Next Steps

**Phase 1 - Immediate (Option B):**
1. Create docker-compose.yml for ci-tui
2. Create ci-tui.yaml mapping to Makefile checks
3. Test self-hosting workflow
4. Document: "Run docker compose up -d first"

**Phase 2 - Future (Option A):**
1. Design DockerMode enum and config fields
2. Implement runner changes with both modes
3. Write comprehensive tests (both modes)
4. Create examples for both patterns
5. Update docs with decision guidance

**Phase 3 - Validation:**
1. Dogfood: CI-TUI developers use ci-tui daily
2. Collect feedback on UX
3. Identify missing features
4. Refine based on real usage

---

## Appendices

### A. Related Code References

**runner.rs:**
- Lines 326-340: Pre-command execution (docker compose exec)
- Lines 434-449: Check execution (docker compose exec)
- Line 523: Fix command execution (docker compose exec)

**config.rs:**
- Lines 49-58: DockerConfig struct
- Lines 90-106: CheckDefinition struct
- Lines 152-166: PreCommand struct (has exec field)

**Makefile:**
- Line 18: test target (docker run pattern)
- Line 27: clippy target (docker run pattern)
- Line 21: fmt target (docker run pattern)

### B. Key Architectural Insights

**Why docker compose exec?**
- Target users have multi-service apps (app + db + redis)
- Need persistent services for integration tests
- exec is faster than run for repeated commands

**Why docker run --rm for CI-TUI?**
- CI-TUI is single binary, single image
- No dependencies, no services
- Ephemeral containers match Makefile workflow

**The tension:**
- CI-TUI designed for complex apps (compose)
- CI-TUI itself is simple app (run)
- Resolution: Support both patterns
