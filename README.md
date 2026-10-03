# ci-tui

**Local CI that only runs what your diff touches — in your real CI containers, with a live terminal UI.**

Stop pushing to find out CI is red. `ci-tui` detects your changed files against the base branch, figures out which checks and which *tests* are relevant, and runs them in the same Docker containers your CI uses (or directly on the host with `runner: local`) — live per-check status in a TUI, output shown as each check completes.

## Why

The usual loop is: push → wait for CI → red → fix → push again. Pre-commit hooks help, but they run on your host toolchain (≠ CI environment) and they don't know which *tests* your change affects.

`ci-tui` closes both gaps:

- **Change-aware**: file-pattern triggers decide which checks run for this diff.
- **Test discovery**: maps changed sources to their tests (`src/Foo.php` → `tests/FooTest.php`) or greps test dirs for references — so it runs the *related* tests, not the whole suite.
- **CI parity**: checks execute via `docker exec` into your compose service's container. Green here predicts green in CI.

## Features

- Live TUI (ratatui) with per-check status, output shown on completion, and timing
- Jump to failure: when the run ends the first failed check is selected and opened at its first error line; error lines are highlighted (per-check `error_pattern` regex overrides the default)
- Sequential check groups, parallel checks within a group, group `pre_commands` (e.g. DB init)
- Test discovery via `path_mapping` and `grep_search` strategies
- On-demand checks (`t` key): unmatched triggers, or `test_discovery` checks marked `on_demand: true` that found no related tests
- `--fix` mode runs each check's `fix_command` (formatters, etc.), then re-runs each check whose fix passed to confirm it passes now (`--no-verify` skips this; also for `x` / `X` in the TUI)
- `--simple` console mode for CI pipelines — auto-selected when stdout is not a TTY
- `--format json|junit` (implies `--simple`) prints a JSON or JUnit XML report of the run on stdout instead of the text (warnings stay on stderr); `--output <file>` writes it to the file and keeps the text on stdout (an existing file is removed at start, so a run that stops early leaves no stale report; if writing it fails, failed checks still exit `1`). JSON: `version: 1`, `base_ref`, `changed_files`, `duration_ms`, `summary` (`total` / `passed` / `failed` / `cached`) and `checks[]` (`id`, `name`, `group`, `status` e.g. `timed_out`, `duration_ms`, `cached`, ANSI-stripped `output` (empty when cached) / `error_output`, `fix_command` or `null`). JUnit: one `<testsuite>` per group, one `<testcase>` per check (`<failure>` with the output for failed / timed out, `<skipped/>` for cancelled; `tests` / `failures` / `errors` / `skipped` counts). Not with `--list` / `--fix` / `--watch` / `--exit-on-finish`
- Under GitHub Actions (`GITHUB_ACTIONS=true`), text output folds each group into `::group::` / `::endgroup::` and annotates each failed check with `::error title=<id>::` plus its output (not when stdout holds a `--format` report)
- `--files` to bypass git detection and check specific paths
- `--base <ref>` to diff against any ref (stacked branch, tag) instead of `git.base_branch`
- `--staged` checks only files staged in the git index (`git diff --cached`) — for pre-commit hooks; checks still read the working tree, so unstaged edits in staged files are included
- `--only <id>[,<id>]` / `--group <id>[,<id>]` run a subset of checks for fast iteration (TUI, `--simple`, `--fix`, `--list`); unknown ids error with the valid ones listed
- Result cache: a check whose inputs (id, resolved command, content of its matched files and test-discovery sources, config file except `notify`, comments and formatting) are unchanged since its last passing run is skipped and shown as `✓ cached` (counts as passed). Stored per check in `.git/ci-tui/` (per linked worktree); checks without concrete files (always-run, run-all, on-demand) are never cached, `r` / `R` / `t` in the TUI always run (a skipped group's `pre_commands` run first); `--list` marks cached checks. Only those inputs count: edits elsewhere (`Cargo.toml`, unchanged files the changed ones use, a new base ref, the docker image, env) don't invalidate a pass — use `--no-cache` then. `--no-cache` runs everything (passes are still recorded). Check ids must be unique across groups
- `--list` (alias `--dry-run`) prints changed files, base ref, and which checks would run / wait on-demand / skip and why — executes nothing
- Falls back to `docker run` when the compose container isn't up — without `docker.volume_mount` this runs the image's baked-in code, not your working tree
- `runner: local` runs checks directly on the host when you don't use Docker
- `--no-color` (or a non-empty `NO_COLOR` env var, per [no-color.org](https://no-color.org)) disables color in the TUI, `--simple` and `--fix` output
- `--notify` (or top-level `notify: true`) rings the bell and sends a desktop notification (OSC 9; tmux needs `allow-passthrough on`; GNU screen works as is) when a TUI run finishes; `--exit-on-finish` quits the TUI then, exiting `0` / `1` like `--simple`
- `--watch` (TUI only) keeps the TUI running and re-runs the checks a file save affects, like `r` (git refresh first; a running one restarts, one the current run still runs is left to it). Re-runs wait while a fix or its verification runs and keep a group's concurrency (`parallel: false` one at a time, else `max_parallel`); a check is not re-run for files it wrote itself (saves during its run, up to 1 s after). Watches the repo's dirs except gitignored ones, `.git/`, `.ci-tui/` and nested repos / submodules (new dirs are picked up); saves are debounced (300 ms quiet, 2 s cap), and files matching `ignore_patterns`, gitignored files and editor temp files (vim swap / `4913`, `*~`, emacs `.#*` / `#*#`) are skipped. A save selects checks like a run on just the saved files, so always-run checks (no `triggers`) re-run on every save
- `--no-stats` (TUI only; ignored by `--simple` / `--fix` / `--list`) starts the TUI with the CPU/MEM panel hidden (`m` toggles it); terminals smaller than 60x21 (60x15 without stats) show a "terminal too small" notice instead of a clipped layout
- `ci-tui init` scaffolds a commented starter config; `ci-tui validate` checks one (unknown fields, bad regexes, triggers naming undefined patterns)
- JSON Schema for editor autocomplete / validation: `ci-tui schema`, [`schema/ci-tui.schema.json`](schema/ci-tui.schema.json), and a `ci-tui.schema.json` asset on each release

## Install

From source:

```bash
git clone https://github.com/michalschroeder/ci-tui && cd ci-tui
cargo install --path .
```

`runner: local` needs ci-tui installed on the host (from source / cargo), not the Docker image, since checks use the host toolchain.

Docker image (published to GHCR on each release):

```bash
docker pull ghcr.io/michalschroeder/ci-tui:latest

# needs the docker socket (to run checks) and your project mounted at /app;
# workdir /app derives compose project name "app" unless overridden
docker run --rm -it \
  -v /var/run/docker.sock:/var/run/docker.sock \
  -v "$PWD":/app \
  -e COMPOSE_PROJECT_NAME=<your-compose-project> \
  ghcr.io/michalschroeder/ci-tui:latest
```

## Quickstart

1. Scaffold a config in your project root and edit its `checks` section (full reference: [docs/configuration.md](docs/configuration.md)):

```bash
ci-tui init          # writes a commented ci-tui.yaml (never overwrites)
ci-tui validate      # checks it without running anything
```

The generated file starts with a `# yaml-language-server: $schema=...` line pointing at this version's release schema, so editors using yaml-language-server (VS Code YAML extension, Neovim, Helix, JetBrains) autocomplete and validate it. For an existing config, add that line yourself (see [Schema Validation](docs/configuration.md#schema-validation)).

Or let an AI agent generate it from your existing CI tools: copy [skills/ci-tui-setup](skills/ci-tui-setup/SKILL.md) into your project's `.claude/skills/` and run `/ci-tui-setup` (other agents: paste the file as a prompt).

The starter's placeholder check fails on purpose until you replace its `command`. A real config looks like this:

```yaml
version: 2

docker:
  project_dir: .
  service: app
  shell: bash
  volume_mount: .:/app  # used by the docker-run fallback if the compose container isn't up

git:
  base_branch: main
  fallback_branch: HEAD~1

file_patterns:
  rust:
    pattern: '\.rs$'
    color: yellow

checks:
  quality:
    name: Code Quality
    parallel: true
    checks:
      fmt:
        name: Format Check
        command: cargo fmt -- --check {files}
        fix_command: cargo fmt -- {files}
        triggers:
          file_pattern: rust
      clippy:
        name: Clippy Lints
        command: cargo clippy -- -D warnings
        triggers:
          file_pattern: rust
```

2. Run it:

```bash
ci-tui                     # uses ./ci-tui.yaml
ci-tui --config other.yaml # or any path
```

Changed files are detected against `origin/{base_branch}`, then `{base_branch}`, then `{fallback_branch}`.

## Keybindings

| Key | Action |
|-----|--------|
| `q` / `Ctrl-C` | Quit |
| `j`/`k`, `↓`/`↑` | Select check or group header |
| `Space` / `Enter` | Fold / unfold selected group (fully passed groups fold automatically) |
| `PgUp`/`PgDn` | Scroll output |
| `f` | Toggle failed-only filter |
| `a` | Show all checks |
| `r` | Retry selected check (also cancelled ones) |
| `s` | Cancel selected running check (others keep running; not a failure) |
| `R` | Re-detect changes and rerun all |
| `t` | Trigger on-demand check |
| `A` | Run selected check on all files |
| `x` | Fix selected check, then re-run it (unless `--no-verify`) |
| `X` | Fix all failed checks, then re-run those whose fix passed (unless `--no-verify`) |
| `c` | Copy check command to clipboard (OSC 52 terminals) |
| `e` | Toggle full command display |
| `o` | Open selected check's output in `$PAGER` (default `less`; TUI suspended until it exits) |
| `O` | Open selected check's output in `$EDITOR` (default `vi`) |
| `w` | Save selected check's output to `.ci-tui/logs/<check>.log` (plain text, overwritten) |
| `m` | Show / hide CPU/MEM stats panel |

Cancel, timeout, quit and rerun kill the check's whole process tree, and `docker kill` the container of a `docker run` fallback. Limitation: for `docker exec` into a running container only the local client is killed; the command keeps running inside the container until it exits.

Info messages (e.g. "Command copied") auto-dismiss after 3s; errors stay until a keypress; progress messages ("Refreshing changed files...") stay until the work finishes. Any keypress dismisses the message and still performs its action.

## CI / scripting usage

```bash
ci-tui --simple            # plain console output, exit code reflects results
ci-tui --fix               # run fix commands, re-run fixed checks to verify
ci-tui --fix --no-verify   # run fix commands only
ci-tui --files src/a.rs    # bypass git detection
ci-tui --base v1.2.0       # diff against a ref instead of git.base_branch
ci-tui --staged -s         # staged files only (pre-commit hook)
ci-tui --list              # explain check selection, run nothing (alias --dry-run)
ci-tui --only fmt,clippy   # only these check ids (--group <id> for whole groups)
ci-tui -s --no-color       # no ANSI escapes (same as NO_COLOR=1)
ci-tui -s --no-cache       # run every check, even ones cached as unchanged
ci-tui --format json > report.json            # JSON report only on stdout
ci-tui --format junit --output junit.xml      # JUnit file, text on stdout
ci-tui --notify --exit-on-finish  # TUI: notify when done, quit with the exit code
ci-tui --watch             # TUI: re-run affected checks on every file save
ci-tui validate            # lint the config in CI (non-zero exit if invalid)
```

The exit code tells scripts why a run failed, so `ci-tui && git push` is safe:

| Code | Meaning |
|------|---------|
| `0` | All selected checks passed (also `--list`, and runs with nothing to check) |
| `1` | A check failed (TUI, `--simple`; in the TUI also a group pre-command), or a fix command or its verification re-run failed (`--fix`) |
| `2` | Config error: missing / unparsable / invalid config (also `ci-tui validate`), `ci-tui init` refusing to overwrite, bad flag value (e.g. unknown `--only` id) |
| `3` | Git or environment error: base ref / `--staged` detection failed, Docker unreachable or not answering within 10s when non-on-demand checks / fix commands would run in docker mode, `--watch` file watcher failed to start (e.g. inotify watch limit), other runtime errors |
| `130` | Interrupted (Ctrl-C) |

In the TUI, checks still pending when you quit don't count as failures.

Simple mode is auto-enabled when stdout is not a terminal, so `ci-tui` works as a CI runner. This repo dogfoods it: its own [`ci-tui.yaml`](ci-tui.yaml) runs fmt, clippy and tests in the dev image (`make build-dev`, then `HOST_PWD=$PWD ci-tui`).

## How check selection works

1. `git diff` against the base ref lists changed files, plus uncommitted and untracked files (minus `ignore_patterns`).
2. A check with no `triggers` key always runs. A check with a `triggers` block but neither `file_pattern` nor `test_discovery` set never runs.
3. `triggers.file_pattern` is matched against the list; if it's set and nothing matches, the check becomes `t`-triggerable instead.
4. `triggers.test_discovery` maps changed sources to test files; if none are found: `on_demand: true` makes it wait for you to press `t` (never runs in `--simple`), otherwise it runs the full command if the command has no `{files}`, else it's skipped. `on_demand` only has an effect here — it's ignored on checks without `test_discovery`.

Test discovery is heuristic (path conventions + grep) — it does not do dependency analysis, so a passing run is a fast pre-push signal, not a replacement for full CI. See [limitations](docs/configuration.md#test-discovery-limitations).

## Development

```bash
make build-dev   # build the dev image (once)
make fmt         # auto-format
make ci          # fmt-check + clippy + tests, in Docker
```

## License

Licensed under either of [Apache License 2.0](LICENSE-APACHE) or [MIT License](LICENSE-MIT) at your option.
