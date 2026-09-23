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
- Sequential check groups, parallel checks within a group, group `pre_commands` (e.g. DB init)
- Test discovery via `path_mapping` and `grep_search` strategies
- On-demand checks (`t` key): unmatched triggers, or `test_discovery` checks marked `on_demand: true` that found no related tests
- `--fix` mode runs each check's `fix_command` (formatters, etc.)
- `--simple` console mode for CI pipelines — auto-selected when stdout is not a TTY
- `--files` to bypass git detection and check specific paths
- Falls back to `docker run` when the compose container isn't up — without `docker.volume_mount` this runs the image's baked-in code, not your working tree
- `runner: local` runs checks directly on the host when you don't use Docker
- `ci-tui init` scaffolds a commented starter config; `ci-tui validate` checks one (unknown fields, bad regexes, triggers naming undefined patterns)

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
| `j`/`k`, `↓`/`↑` | Select check |
| `PgUp`/`PgDn` | Scroll output |
| `f` | Toggle failed-only filter |
| `a` | Show all checks |
| `r` | Retry selected check |
| `R` | Re-detect changes and rerun all |
| `t` | Trigger on-demand check |
| `A` | Run selected check on all files |
| `x` | Fix selected check |
| `X` | Fix all checks |
| `c` | Copy check command to clipboard (OSC 52 terminals) |
| `e` | Toggle full command display |

While a status message (e.g. "Command copied") is shown, the first keypress only dismisses it — press again to trigger the action.

## CI / scripting usage

```bash
ci-tui --simple            # plain console output, exit code reflects results
ci-tui --fix               # run fix commands only
ci-tui --files src/a.rs    # bypass git detection
ci-tui validate            # lint the config in CI (non-zero exit if invalid)
```

Both TUI and `--simple` exit `1` if any check failed, so `ci-tui && git push` is safe. In the TUI, checks still pending when you quit don't count as failures.

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
