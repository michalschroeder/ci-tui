---
name: ci-tui-setup
description: Generate or update ci-tui.yaml from the project's existing CI tools.
disable-model-invocation: true
---

# ci-tui setup

Goal: a `ci-tui.yaml` that gives a fast pre-push signal by running the project's existing lint/format/test tools on changed files. It complements full CI; it does not replicate it.

Schema: https://github.com/michalschroeder/ci-tui/blob/master/docs/configuration.md
Full example: https://github.com/michalschroeder/ci-tui/blob/master/docs/examples/php-symfony.yaml
`ci-tui init` writes a commented starter.

## Steps

1. **Inventory.** Find every check the project already runs: CI files (`.github/workflows`, `.gitlab-ci.yml`, etc.), Makefile/justfile, package.json/composer.json scripts, pre-commit, linter configs. Done when each tool has: command, where it runs (compose service or host), file types covered, whether it takes file args, fix variant.
2. **Runner.** Pick the runner whose environment is closest to CI: `docker` when CI runs in containers, `local` when the toolchain runs natively.
3. **Plan.** Show the user a table: check → command → trigger → group. Ask before writing. Ask before overwriting an existing `ci-tui.yaml`.
4. **Write.**
   - Every command is one the project already runs. Suggest missing tools separately, outside the config.
   - Scope each command as narrowly as the tool still gives correct results: `{files}` for per-file tools, whole project for tools needing cross-file context (type checkers, builds).
   - `--fix` covers everything auto-fixable, so only human-judgment failures remain.
   - Order groups by time to first useful failure. Parallelize checks that share no state (DB, caches, files they write).
   - Test discovery fits the repo's test layout; a full-suite path stays one `t` keypress away (`on_demand: true`).
   - `ignore_patterns` covers tool- and package-manager-generated paths.
5. **Verify.** Run `ci-tui validate` until exit 0. Then `ci-tui --simple --files <one sample file per pattern>` and confirm the expected checks trigger. Any fix that changes the approved plan table goes back to the user. Done when validate passes and every check fires on at least one sample.
