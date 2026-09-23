---
name: implement-issue
description: Implement a GitHub issue by number: fetch, branch, delegate to senior-rust-engineer, verify, open PR.
disable-model-invocation: true
argument-hint: <issue-number>
---

Implement GitHub issue #$ARGUMENTS.

## 1. Brief

Run `gh issue view $ARGUMENTS --comments`. Distill it into a **brief**:

- **Goal**: one sentence.
- **Acceptance criteria**: numbered, each checkable by a test or a concrete manual check. Source them from body and comments; mark any you infer `(inferred)`.
- **Out of scope**: what the issue excludes or defers.

Done when every criterion is checkable. If one cannot be made checkable, or the issue contradicts itself, ask the user and wait. A tracking/roadmap issue has no single change to make: ask which sub-item to implement.

## 2. Branch

`git status` must be clean; if dirty, ask the user. Then `git switch master && git pull` and `git switch -c <type>/<slug>-$ARGUMENTS`, where `<type>` is the conventional commit type (`feat`, `fix`, `perf`, `refactor`, `test`, `docs`, `ci`, `chore`). Branch in place, no worktree.

## 3. Delegate

Dispatch the `senior-rust-engineer` subagent with the issue number and title, the brief verbatim, and the branch name. It has no conversation context: the brief is all it knows.

On `BLOCKED`: relay its question to the user, then continue the same agent via SendMessage with the answer.

## 4. Verify

Trust evidence, not the report. Run `make ci` yourself, then read `git status` and `git diff master` (untracked files count). For each criterion, point at the code and the test (or manual check) that satisfies it.

Any gap (criterion unmet, test missing, `make ci` red, scope creep beyond the brief): SendMessage the engineer with the specific gap and re-verify. Done when every criterion maps to passing evidence and `make ci` is green. After 3 rounds still short: stop and ask the user.

## 5. Ship

1. Stage only the paths the change touched, by name. One conventional commit: `<type>[!]: <summary>`; `!` when the change breaks config, CLI, or output compatibility (Release Please bumps major).
2. `git push -u origin <branch>`.
3. Write the body to a scratchpad file, then `gh pr create --base master --title "<commit subject>" --body-file <file>`. Body:
   - **Summary**: 1–3 bullets, what changed.
   - **Acceptance criteria**: each criterion → its test or manual check.
   - **Test plan**: `make ci` green, plus any manual checks still pending as unchecked boxes.
   - `Closes #$ARGUMENTS`.

Done when the PR URL exists; report it. If push or `gh pr create` fails, stop and report the error; never force-push. If a PR already exists for the branch, report its URL instead.
