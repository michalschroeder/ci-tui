---
name: implement-issue
description: Implement a GitHub issue by number: fetch, branch, delegate to senior-rust-engineer, verify, commit.
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

`git status` must be clean; if dirty, ask the user. Then `git switch master && git pull` and `git switch -c <type>/<slug>-$ARGUMENTS`, where `<type>` is the conventional commit type (`feat`, `fix`, `refactor`, `docs`, `chore`). Branch in place, no worktree.

## 3. Delegate

Dispatch the `senior-rust-engineer` subagent with the issue number and title, the brief verbatim, and the branch name. It has no conversation context: the brief is all it knows.

On `BLOCKED`: relay its question to the user, then continue the same agent via SendMessage with the answer.

## 4. Verify

Trust evidence, not the report. Run `make ci` yourself and read `git diff master`. For each criterion, point at the code and the test (or manual check) that satisfies it.

Any gap (criterion unmet, test missing, `make ci` red, scope creep beyond the brief): SendMessage the engineer with the specific gap and re-verify. Done when every criterion maps to passing evidence and `make ci` is green.

## 5. Commit

One conventional commit (`<type>: <summary>`, body ends `Closes #$ARGUMENTS`). Then report: criteria → evidence, files changed. Ask whether to push and open a PR.
