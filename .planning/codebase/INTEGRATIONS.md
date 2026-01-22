# External Integrations

**Analysis Date:** 2026-01-22

## Version Control

**Git:**
- Integration: Command-line git operations for change detection
- Change detection: Compares against configurable base branches
  - Primary: `origin/{base_branch}` (remote tracking)
  - Secondary: `{base_branch}` (local branch)
  - Fallback: `{fallback_branch}` (configured fallback)
- Purpose: Determines which files have changed to trigger appropriate checks
- Implementation: `src/git.rs` - Git CLI invocation via `tokio::process::Command`
- Git feature: Safely mounts git repositories via `git config --global --add safe.directory '*'` (allows any mounted directory)

## Container Orchestration

**Docker & Docker Compose:**
- Integration: Primary execution environment for all CI checks
- Check execution: Runs via `docker compose exec` commands
- Location: Configured via `config.docker.project_dir` in YAML config
- Service selection: Each check specifies Docker service to run in (defaults to configured service)
- Environment variables: Passed via `-e KEY=VALUE` flags to docker compose exec
- Working directory: Mounted project root at `/app` within container
- Container access: Requires mounted Docker socket (`/var/run/docker.sock`) for Docker-in-Docker capability
- Implementation: `src/runner.rs`
  - `CheckRunner` struct manages execution
  - `run_docker_check()` constructs and executes docker compose commands
  - Pre-commands: Support setup commands (e.g., database initialization) that run before checks
  - Command construction: Uses shell escaping to safely pass complex commands

**Docker Compose Command Pattern:**
```bash
docker compose --project-directory={project_dir} exec -T {service} {env_flags} bash -c '{command}'
```
- `-T` flag: No pseudo-TTY (non-interactive execution)
- `bash -c`: Ensures shell variable expansion inside container
- Single quotes with escaping: Prevents shell variable expansion on host

## Data Storage

**File System:**
- Configuration: Read from YAML file on disk
- Project source: Mounted as read-only for analysis (git operations, file pattern matching)
- Caching: No persistent cache between runs
- Output: Streamed to TUI or console, no output persistence

**No Persistent Database:**
- Application is stateless between invocations
- Check results exist only in memory during execution
- UI state (selected check, filter view) is in-memory only

## Authentication & Identity

**Not Applicable:**
- No API authentication required
- No user management
- Git authentication: Inherited from host environment (SSH keys, credentials)
- Docker authentication: Inherited from host Docker CLI configuration

## Monitoring & Observability

**Error Tracking:**
- Not integrated with external error tracking services
- Errors handled via `color-eyre` for formatted output
- `thiserror` for structured error types

**Logging:**
- Console output: Direct to stdout/stderr
- Output streaming: ANSI-to-TUI conversion for colored output in terminal
- Check output: Captured and displayed in real-time via ratatui TUI
- Stderr filtering: Docker Compose warnings filtered to reduce noise

**System Monitoring:**
- sysinfo 0.32 - System information (included but usage not core to execution)

## CI/CD & Deployment

**Hosting:**
- Docker containers (self-hosted or cloud)
- Alpine Linux 3.21 runtime

**Registry:**
- Default registry: `ghcr.io/lendable` (GitHub Container Registry)
- Configurable via `CI_TUI_REGISTRY` environment variable
- Configuration: `run.sh` handles pull/build/push operations

**CI Pipeline:**
- Not integrated with external CI systems
- Standalone tool invoked within existing CI workflows
- Simple mode (`--simple` flag) for non-interactive CI pipeline execution
- Auto-detection: Disables TUI if stdout is not a terminal (for CI pipelines)

**Deployment Script:**
- `run.sh` - Bash script that orchestrates Docker image handling
  - Registry pull-first strategy (with local build fallback)
  - Build support with BuildKit
  - Push capability for registry publishing
  - Configuration: Reads `CI_TUI_CONFIG` environment variable for config file path

## Environment Configuration

**Configuration File:**
- Path: Specified via `--config` CLI argument (required)
- Format: YAML
- Location typically: `./tools/ci/ci-config.yaml` (convention, not enforced)

**Environment Variables (Docker Execution):**
- Global: Defined in `config.docker.env` YAML section
- Per-check: Defined in `checks[group][check].env` YAML section
- Per-pre-command: Defined in `checks[group].pre_commands[].env` YAML section
- Precedence: Check env overrides global env; per-check values override global

**Deployment Environment Variables:**
- `CI_TUI_REGISTRY` - Docker registry for image pull/push (default: `ghcr.io/lendable`)
- `CI_TUI_VERSION` - Image tag (default: `latest`)
- `CI_TUI_CONFIG` - Path to configuration file (default: `./tools/ci/ci-config.yaml`)

**No Secrets Management:**
- Secrets passed via environment variables
- No integration with external secret managers
- Sensitive values (API keys, credentials) passed through Docker environment

## Webhooks & Callbacks

**Incoming:**
- Not applicable - Tool is CLI-based, not server-based

**Outgoing:**
- Not applicable - Tool does not emit external webhooks
- Event streaming: Internal mpsc channels for TUI event communication only

## Git Integration Details

**Change Detection Process:**
- Compares current HEAD against configured base references
- Returns list of changed file paths (relative to repository root)
- File extensions extracted for pattern matching
- Ignore patterns applied (regex-based exclusion)

**Test Discovery Integration:**
- Path mapping: Maps source file changes to related test files using regex patterns
- Grep search: Searches test directories for patterns matching changed file names/content
- Placeholder expansion: Supports `{basename}`, `{filename}`, `{extension}`, `{dirname}`, `{path}`

---

*Integration audit: 2026-01-22*
