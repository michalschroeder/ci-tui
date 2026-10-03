# CLAUDE.md

This file provides guidance to Claude Code (claude.ai/code) when working with code in this repository.

## Project Overview

CI-TUI is a terminal UI application for running CI checks on changed files. It detects git changes, determines which checks to run based on file patterns, and executes them in Docker containers (or on the host with `runner: local`), reporting progress via lifecycle events.

## Code Validation (REQUIRED)

**IMPORTANT:** After writing or modifying code, ALWAYS run these commands to validate.

Requires the dev image — build it once with `make build-dev`.

### Step 1: Auto-fix (before commits)

```bash
make fmt
```

This runs `cargo fmt` to fix formatting issues automatically.

### Step 2: Fast loop while iterating (dogfood)

```bash
HOST_PWD=$PWD ci-tui --simple --no-color
```

Runs this repo's own `ci-tui.yaml` (fmt, clippy, unit, integration, doc tests in the dev image) only for files changed vs `master` (incl. untracked), skipping checks unchanged since their last pass (`✓ cached`). Exit 0 pass, 1 checks failed, 2 config, 3 docker/git.
- Narrow: `--group quality|tests`, `--only fmt,clippy,unit,integration,doctests`
- `--list` shows what would run and why; `--no-cache` forces every selected check
- `--fix` runs `cargo fmt` on changed files and re-runs fmt to verify
- `--format json` for a machine-readable report (failed check output in `checks[].output`)
- Needs `ci-tui` >= 0.6.0 on PATH (installed binary, not the code under change): `cargo install --path . --locked --target-dir /tmp/ci-tui-install` (repo `target/release` is root-owned). Not installed → skip to step 3

### Step 3: Final gate

```bash
make ci
```

This runs all CI checks in Docker (using the dev image), regardless of what changed:
- `cargo fmt --check` - Code formatting check
- `cargo clippy -- -D warnings` - Linter checks
- `cargo nextest run` - Test suite

**Workflow:** `make fmt`, iterate with `ci-tui --simple`, then `make ci`. `make fmt` and `make ci` must pass before any commit or push (ci-tui runs nothing for docs-only changes, so it never replaces `make ci`).

These are the ONLY commands Claude should use to validate code changes. Do not use `cargo test`, `cargo clippy`, or other commands directly.

### Failure Handling

If validation commands fail or do not pass after **3 attempts**, STOP and ask the user:

1. **Continue with 3 more retries?** - Keep trying to fix the issue
2. **Need more context?** - User provides additional information about the problem
3. **Stop here?** - Abandon the current approach and discuss alternatives

Do not keep retrying indefinitely. After 3 failed attempts, always pause and ask for guidance.

## Other Development Commands

```bash
# Build dev image (required for make ci/fmt/test/clippy)
make build-dev

# Build production Docker image locally
make build

# Run individual checks
make test        # Run tests only
make fmt-check   # Check formatting without fixing
make clippy      # Run clippy only

# Run with local cargo (requires Rust installed)
cargo run -- --config <path-to-config.yaml>
cargo run -- -c <config> --files src/main.rs src/lib.rs  # bypass git, check specific files
```

## Versioning (Release Please)

This project uses [Release Please](https://github.com/googleapis/release-please) for automated versioning and changelog generation.

**How it works:**
1. Push commits to `master` with [Conventional Commits](https://www.conventionalcommits.org/) format
2. CI runs → Release Please creates/updates a release PR
3. Merging the release PR triggers version bump, CHANGELOG.md update, and Docker image build

**Version bumps based on commit types:**
- `feat:` → Minor version bump (0.1.x → 0.2.0)
- `fix:` → Patch version bump (0.1.8 → 0.1.9)
- `feat!:` or `BREAKING CHANGE:` → Major version bump (0.x → 1.0.0)

**Do NOT:**
- Manually create git tags (Release Please manages them)
- Edit `Cargo.toml` version field directly
- Edit `CHANGELOG.md` directly

**Config files:**
- `release-please-config.json` - Release Please configuration
- `.release-please-manifest.json` - Current version tracking

## Test Fixtures

Test configs are centralized in `tests/common/configs.rs`:

- **Fixtures**: Use `minimal_config()`, `php_project_config()`, `rust_project_config()` for common scenarios
- **Builder**: Use `ConfigBuilder::new().with_*().build()` for custom configs
- **Inline**: Keep truly unique edge-case configs inline with `// Edge case: ...` comment

New tests should require <10 lines of config setup.

### Test Architecture Note

**Integration tests** (`tests/*.rs`): Import shared fixtures from `tests/common/configs.rs`

**Library tests** (`src/*.rs #[cfg(test)]`): Use inline structured fixtures. Cannot import from `tests/common/` due to cargo fmt limitations in Docker - the `#[path = "..."]` directive breaks when cargo fmt runs inside containers. This is a known Rust tooling limitation.

The inline fixtures in `src/config.rs` and `src/checks.rs` mirror the structure of `tests/common/configs.rs` but are defined locally. When adding new library tests, follow the existing inline fixture pattern in that module.

## Architecture

### Core Flow
1. **main.rs** - CLI entry point using clap. Loads config, detects git changes (or accepts `--files` to bypass git), determines checks, launches UI or simple mode
2. **config.rs** - YAML config parsing with `CiConfig` as the root type. Uses `IndexMap` to preserve YAML key ordering for group execution order
3. **git.rs** - Git change detection comparing against base branch (tries `origin/{base}`, `{base}`, fallback in order)
4. **checks/** - `determine_checks()` matches changed files against file patterns and test discovery rules to build `CheckToRun` list (`mod.rs` + `determine.rs`)
5. **runner.rs** - `CheckRunner` executes checks via `ExecTarget` (docker exec / docker run, or host shell in local mode) with event streaming through mpsc channels
6. **ui/mod.rs** - Main TUI event loop using ratatui. Keyboard input runs on dedicated OS thread for responsiveness under high CPU load

### Key Design Patterns

**Check Execution Groups**: Groups defined in config execute sequentially. Within a group, `parallel: true` runs checks concurrently. Groups can have `pre_commands` (e.g., DB init) that run before checks.

**Test Discovery**: Two strategies for finding related tests when source files change:
- `path_mapping`: Maps source paths to test paths (e.g., `src/{path}.php` → `tests/{path}Test.php`)
- `grep_search`: Searches test directories for content patterns with placeholders (`{basename}`, `{filename}`, etc.)

**On-Demand Checks**: Checks marked `on_demand: true` don't run automatically - user triggers with 't' key. Used for expensive tests when no specific test files are found.

**Event-Driven UI**: Main loop uses `tokio::select!` (biased, keyboard first) over runner/input/stats/task channels (plus a live-timer tick while a check runs, and `--watch` batches), redrawing when `needs_redraw` is set.

### Module Responsibilities

- **config.rs**: All config structs (`CiConfig`, `GroupConfig`, `CheckDefinition`, `TestDiscoveryConfig`), YAML deserialization, pattern compilation caching
- **checks/**: `CheckToRun` struct (`mod.rs`), `determine_checks()` logic (`determine.rs`), `group_checks()` for execution grouping
- **runner.rs**: `CheckRunner`, `CheckResult`, `CheckStatus` enum, `ExecTarget` (docker/local dispatch) command execution, `RunnerEvent` variants
- **test_discovery.rs**: `find_related_tests()`, path mapping, grep search, placeholder expansion
- **ui/app.rs**: `App` state struct with all UI state (selected check, results, filters, etc.)
- **ui/dashboard.rs**: Rendering logic using ratatui widgets
- **ui/external.rs**: `o`/`O` output in `$PAGER`/`$EDITOR` (plain text in private session temp dir; main loop stops keyboard thread, runs viewer on blocking pool, keeps draining events) and `w` log save to `.ci-tui/logs/<id>.log` (writes `.ci-tui/.gitignore`). Helper processes use `utils::own_process_group` so pager Ctrl-C spares them
- **cache.rs**: Result cache (#150) — `select_checks(.., key_root)` sets `CheckToRun::cache_key` (id + command + matched/discovery-source file contents + config hash; `None` without concrete files or with a missing / unreadable one; discovered tests read under the exec root). `ResultCache` in `<git dir>/ci-tui/results.json`: latest pass per check, stored only if the key still matches at finish. Each change re-reads the file under a lock (concurrent runs). Initial run serves `CheckResult::cached`; TUI records on a background thread (`Recorder`, not for `a` all-files runs); single runs of a skipped group run its `pre_commands` first, wait while the runner owes them (`App::claim_group_setup`). Details in module doc
- **color.rs**: `--color auto|always|never` (`ColorChoice`; `--no-color` = `never`, conflicts with `--color`, checked in `cli::color_with_no_color` too) decision `should_color(choice, stdout_tty, NO_COLOR)`: `auto` off when stdout not a TTY or non-empty `NO_COLOR`, `always` beats `NO_COLOR`; console modes print via `cprintln!` (strips ANSI CSI + OSC when off, process-wide switch set in main); TUI resets cell fg/bg after drawing (`App::color`)
- **ui/notify.rs**: End-of-run notification (`TuiOptions::notify` = config `notify` or `--notify`; `notify` is left out of the cache config hash): BEL + OSC 9 (DCS passthrough under tmux `$TMUX` / screen `$STY`) with the dashboard's finished-status text. The main loop sends it once per full run (`report_run_end`) when the run settled (`run_settled`: runner's `AllFinished`, not `App::busy` with retries / fixes / `R` refresh, no viewer open), then `--exit-on-finish` quits (`exit_due`; exit code `App::exit_code`, 1 also for a failed pre-command)
- **watch.rs**: `--watch` (TUI only; conflicts with `--simple` / `--list` / `--fix` / `--exit-on-finish`) — `notify` watcher, non-recursive per dir (`walk`, `ignore` crate: repo root (canonical; cwd outside a repo) minus gitignored dirs, `.git/`, `.ci-tui/`, nested repos / submodules; dirs created later watched by the batch thread, their existing files count as saved), started in `ui::run` before the terminal (failure → exit 3). Its thread debounces saves (`next_batch`: 300 ms quiet, 2 s cap even under a flood), keeps repo-relative paths (`keep_paths`: drops `.git/`, `.ci-tui/`, editor temp files, dirs, `ignore_patterns`, `git check-ignore`; a failing check-ignore in a repo drops the batch), sends `WatchBatch` (`affected_checks`: `select_checks` on the saved set, `Decision::Run` only, plus first / last save `Instant`). Main loop: `Message::Watch` → `App::queue_watch` (skips a finished check whose saves all fell in its latest run + 1 s: own writes; `RunState::run_spans`); `start_watch_runs` only on `Message::Watch`, task events (not output chunks) and runner check / pre-command / group / all finished: nothing while `App::fixing` (fix, fix-all, verification), runner-owed (pending in unfinished run / queued) kept until the runner takes it (then dropped, not cancelled; still pending at run end → started), running cancelled once (`WatchRun::cancel_sent`) and kept until its result lands (no stale overwrite), group full (`App::group_slot_free`: `parallel: false` → 1, else `group_parallel_limit`) or setup-busy kept queued, else `retry_with_refresh(.., auto_only)` with the saved check (refresh fails → that runs; refreshed on-demand / skipped → shown, not run); unlisted check dropped (app lists every check the config can select). Header shows `watching`
- **cli.rs**: clap `Cli` / `Command` definitions, `init`/`validate` config-path resolution (main.rs stays thin)
- **commands.rs**: `init` / `validate` subcommands — starter config template (round-trip tested against `load_config`; first line links the versioned release schema)
- **schema.rs**: `ci-tui schema` — config JSON Schema via `schemars` (root `RawCiConfig`); committed at `schema/ci-tui.schema.json`, drift-guarded by a test (`make schema` regenerates), uploaded as a release asset
- **exit.rs**: Process exit code constants (0 pass, 1 checks failed, 2 config, 3 git/env, 130 Ctrl-C); `ConfigError` marker + `code_for()` map errors escaping `run()` to 2 or 3
- **filter.rs**: `--only` / `--group` — narrows `CiConfig` right after load (so TUI refresh, simple, fix, list share the subset); unknown ids → exit 2 listing valid ones; empty groups dropped (no `pre_commands`); triggers still apply
- **fix.rs**: Auto-fix mode (`--fix` flag) — runs fix commands for matched checks; after all fixes, passing fixes' checks that `select_checks` selected as non-on-demand re-run via `CheckRunner::run_checks` (`--no-verify` skips). TUI `x`/`X` verify one at a time, no git refresh (`verify_fixes` in ui/mod.rs)
- **list.rs**: `--list` / `--dry-run` — per-check run/on-demand/skipped decision plus reason, from the same `Selection` evaluation `determine_checks` uses; executes nothing
- **preflight.rs**: Docker startup checks. Fatal: `docker_reachable()` — `docker version` probe (10s timeout) when non-on-demand checks / fix commands would run in docker mode, Err → exit 3. Non-fatal: `docker_warnings()` when changed files won't resolve in the containers selected checks use (exec: repo-relative path vs container WORKDIR; `docker run` fallback without `volume_mount` or mounted away from `docker.work_dir`)
- **simple.rs**: Non-TUI console output mode for CI pipelines; takes `report::ReportOptions`
- **report.rs**: `--format text|json|junit` (json / junit imply simple; conflict with `--list` / `--fix` / `--watch` / `--exit-on-finish`, checked in `cli::report_flag_error`) and `--output <file>` (needs json / junit; then stdout keeps text; `ReportOptions::new` removes an old file up front). `Report::json` (`version: 1`, `status_name` snake_case statuses) / `Report::junit` (hand-rolled XML, `xml()` escapes + drops invalid control chars), written by `ReportOptions::write`; `simple::run` returns the exit code (failed checks → 1 even if the write fails; a panicked parallel check task counts as failed). `GITHUB_ACTIONS=true` + text on stdout → `::group::` per group, `::error title=<id>::` per failed check in the summary
- **utils/**: Shared helpers — `docker.rs` (Docker command building), `time.rs` (duration formatting)

## Config File Structure

The tool expects a YAML config with:
- `runner`: `docker` (default) or `local` — where checks execute
- `docker.project_dir`: Path for `docker compose --project-directory` (required in docker mode)
- `docker.service`: Default container service name (docker mode)
- `local.shell` / `local.env`: Shell and env vars for `runner: local` (docker section optional/ignored in this mode)
- `git.base_branch` / `git.fallback_branch`: For change detection
- `file_patterns`: Named regex patterns with optional colors
- `checks`: Groups containing check definitions with triggers
- `ignore_patterns`: Regex patterns for files to exclude from change detection
- `max_parallel`: Cap on concurrent checks in `parallel: true` groups (top-level and per group, >= 1; group can only lower it; default CPU count)
- `--jobs N` / `-j N`: CLI override of top-level `max_parallel`
- `notify`: TUI bell + OSC 9 desktop notification when a full run finishes (default false; `--notify` also enables). `--exit-on-finish`: TUI quits after the run, exit 0 / 1
- `--format json|junit` / `--output <file>` (CLI only): simple-mode reports (see report.rs)
- `--watch` (CLI only, no config key): TUI re-runs checks affected by file saves until quit (see watch.rs)
