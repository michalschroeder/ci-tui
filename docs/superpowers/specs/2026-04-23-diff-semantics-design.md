# Design — Correct git diff semantics

## Problem

`src/git.rs` detects changed files using `git diff --name-only <base>`. Three correctness bugs share this code path:

1. **Two-dot vs three-dot diff.** `git diff <base>` compares the working tree directly to `<base>`. When `<base>` has advanced since the branch was cut, commits landed on base show up as "your" changes. The intent is merge-base semantics: "what did *I* change relative to where I branched from?"
2. **Deleted files included.** `--name-only` lists deletions. Downstream checks run commands against paths that no longer exist → spurious failures.
3. **Silent empty fallback.** When all three base-ref candidates fail (`origin/<base>`, `<base>`, `<fallback>`), `detect_changes` returns `Ok(ChangedFiles { files: [], base_ref: "HEAD" })`. The user sees "no changes" when git actually couldn't resolve anything.

Bundled together because they touch the same two functions.

Out of scope: untracked files (separate spec), pattern robustness (separate spec).

## Change detection model

A changed file is either:

1. **Committed on my branch since divergence**
   `git diff --name-only --diff-filter=ACMR --merge-base <base> HEAD`
2. **Modified locally — staged or unstaged**
   `git diff --name-only --diff-filter=ACMR HEAD`

Union the two sets preserving insertion order. Committed-first ordering. Dedup by path.

Rationale:

- **Merge-base**, not two-dot diff: prevents base-side commits from polluting "my" changes.
- **`--diff-filter=ACMR`** (Added/Copied/Modified/Renamed): drops deletions. Renames surface as the new path.
- **Union**: current behavior checks uncommitted work; keeping that is important for the mid-edit workflow.

`ChangedFiles.base_ref` continues to hold the ref name that resolved (e.g., `origin/master`). Used only for UI display.

## Error handling

`detect_changes` tries `origin/<base>`, `<base>`, `<fallback>` in order. For each ref:

- Run the committed-diff call. If it fails (ref doesn't exist, no common ancestor), move to the next ref.
- On success, run the uncommitted-diff call (does not depend on the base ref), union, return.

When all three refs fail:

- `detect_changes` returns `Err` whose message names every ref that was tried.
- `main.rs` catches this at the top-level and prints a friendly message:
  > `No base ref could be resolved against the current repository. If this is intentional, bypass git with --files <paths...>.`
  Then exits non-zero.

The uncommitted diff is only evaluated after a base ref resolves. We don't surface WIP with no meaningful baseline.

## Testing

Unit tests in `src/git.rs` (inline fixtures, per CLAUDE.md library-test convention):

- Two git calls issued — committed diff first, then uncommitted. Union preserves order.
- Dedup across the two — a path in both appears once, at its committed-first position.
- `--diff-filter=ACMR` present in both arg vectors.
- `--merge-base` present in the committed-diff arg vector.
- Merge-base failure on ref #1 falls through to ref #2 in `detect_changes`.
- All refs fail → `Err` with a message containing all three ref names.

Manual smokes in the verification checklist (real git, not automated):

- Branch where base has advanced → base-side changes do NOT appear.
- Local deletion → deleted path does NOT appear.
- Invalid `base_branch` config → clear error + `--files` hint, not empty.

Any existing integration tests asserting a single `git diff` call or `base_ref == "HEAD"` as a sentinel need updating — `rg -n "detect_changes|base_ref" src tests` during implementation.

## Files expected to change

- `src/git.rs` — diff command rework, error on all-refs-failed, updated + new tests
- `src/main.rs` — catch the specific error, print friendly hint
- `tests/**` — any caller asserting the old single-call or silent-empty behavior

## Verification checklist

- [ ] `make fmt` clean
- [ ] `make ci` passes
- [ ] Unit tests cover: two-call union, dedup, `--diff-filter=ACMR`, `--merge-base`, ref fallthrough, all-refs-failed error
- [ ] Manual: base has advanced → base-side files absent
- [ ] Manual: tracked file deleted locally → absent
- [ ] Manual: invalid `base_branch` → friendly error mentioning `--files`

## Out of scope

- Untracked files (separate spec)
- Pattern matching / dedup / validation (separate spec)
- Config flag to disable the uncommitted half or the merge-base behavior
