---
name: implement-issue
description: Implement a GitHub issue by number: fetch, branch, delegate to senior-rust-engineer, verify, open PR.
disable-model-invocation: true
argument-hint: <issue-number>
---

Implement GitHub issue #N, where N is the first token of `$ARGUMENTS` (the rest is extra guidance from the user; use it in the brief, never in branch names, gh commands or `Closes`).

Pipe `make` output only under `set -o pipefail`; read diffs with `git diff --no-ext-diff`.

## 1. Brief

Run `gh issue view N --json title,body,comments`. Read the code the issue touches and the CLAUDE.md lines for those modules. Distill a **brief**:

- **Goal**: one sentence.
- **Context**: code anchors (file:fn) and pitfalls the engineer needs; any `LIMITATION` / `TODO` / `Future work` comment in the touched code (`rg`), since it may already name the right design. Each claim about an external tool's behaviour (git, docker, compose, terminal) checked by running the tool.
- **Acceptance criteria**: numbered, each checkable by a test or a concrete manual check. Keep the issue's own numbers, limits and examples verbatim. Mark any you infer `(inferred)`.
- **Interactions**: for each sibling path the change can reach, one criterion or an explicit "unaffected because …": cache key, group `pre_commands`, `--fix` / verify, simple / `--format` reports, `--list`, `--watch`, `--only` / `--group`, `max_parallel` / `parallel: false`, cancel / retry / `R` refresh, failure / timeout / empty states.
- **Docs**: every user-facing surface to update, named: README sections, `docs/`, CLAUDE.md module line, schema (`make schema`), `commands.rs` init template, TUI help overlay / footer.
- **Out of scope**: what the issue excludes or defers, and why the feature still works without it.

Done when every criterion is checkable and every interaction is answered. An `(inferred)` item that decides behaviour (what the user sees, a default, a compatibility break, a choice between designs the issue offers): ask the user in one AskUserQuestion call (recommended option first) and wait; record answers as `(confirmed)`. Same for a self-contradicting issue. A tracking/roadmap issue has no single change to make: ask which sub-item to implement.

## 2. Branch

Leave plan mode first. `git status` must be clean; if dirty, ask the user. Then `git switch master && git pull` and `git switch -c <type>/<slug>-N`, where `<type>` is the conventional commit type (`feat`, `fix`, `perf`, `refactor`, `test`, `docs`, `ci`, `chore`). Branch in place, no worktree.

## 3. Delegate

Dispatch the `senior-rust-engineer` subagent with the issue number and title, the brief verbatim (inline or as a scratchpad file path), and the branch name. It has no conversation context: the brief is all it knows. Its result arrives only as a real task notification; wait for it.

On `BLOCKED`: relay its question to the user, then continue the same agent via SendMessage with the answer. SendMessage unavailable: dispatch a fresh engineer with the brief, its last report, and the answer.

## 4. Verify

You are the adversary: the engineer's report is a claim, the diff is the evidence. Run `make ci` yourself, read `git status` and the full `git diff master` (untracked files count), then:

1. **Criteria**: each criterion and interaction maps to code plus a test (or manual check). Each test of new behaviour has a quoted **red** in the report: an assertion failure on the old behaviour (compile errors and panics in setup don't count); a test pinning unchanged behaviour is marked `guard`. A wiring or precedence criterion is tested through the real path (loader, `main` flow, runner), not a hand-built stand-in.
2. **Review**: run the `code-review` skill on the branch diff, then the `simplify` skill report-only (you route its fixes). Every finding is a gap or a rejection with evidence. A finding that changes behaviour, not just structure, is a new criterion: it needs a red test, and a swap of external-tool semantics is checked across every state the old code handled (stopped, paused, missing, older version).
3. **Notes**: every risk or deviation in the engineer's Notes is a gap or a question for the user.
4. **Docs**: `rg` each new or renamed flag, key, config field and module across the Docs list; each hit is current.
5. **Real run**: a change to `src/ui/`, docker placement, the image (`Dockerfile`, `docker/`), a CLI flag, or performance runs the built binary once against a real config (TUI under `tmux` or `script`); record what you saw. Unrunnable here → a manual check in the PR body, named as pending.

Gaps go to the engineer via SendMessage as one numbered list (one test per fix); re-verify the new diff. Done when every criterion and interaction maps to passing evidence, review findings are closed, and `make ci` is green. After 3 rounds still short: stop and ask the user.

## 5. Ship

1. `git branch --show-current` is the step 2 branch. Stage only the paths the change touched, by name. One conventional commit: `<type>[!]: <summary>`; `!` when the change breaks config, CLI, or output compatibility (Release Please bumps major).
2. `git push -u origin <branch>`.
3. Write the body to a scratchpad file from the final diff, then `gh pr create --base master --title "<commit subject>" --body-file <file>`. Body:
   - **Summary**: 1–3 bullets, what changed.
   - **Acceptance criteria**: each criterion → its test or manual check.
   - **Test plan**: `make ci` green, real run result, plus any manual checks still pending as unchecked boxes.
   - `Closes #N`.
4. `gh pr checks --watch`; a red check is a gap (back to step 4).

Done when the PR URL exists and checks are green; report it. If push or `gh pr create` fails, stop and report the error; never force-push. If a PR already exists for the branch, report its URL instead.
