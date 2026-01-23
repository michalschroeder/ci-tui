---
phase: quick
plan: 015
subsystem: ci-cd
tags: [release-please, github-actions, automation, versioning]

dependency_graph:
  requires: [quick-014]
  provides: [automated-releases, changelog-generation, version-bumping]
  affects: [release-workflow]

tech_stack:
  added: [release-please-action-v4]
  patterns: [conventional-commits, manifest-based-config]

key_files:
  created:
    - .github/workflows/release-please.yml
    - .release-please-manifest.json
    - release-please-config.json
    - CONTRIBUTING.md

decisions:
  - id: manifest-config
    choice: "Manifest-based Release Please configuration"
    reason: "More flexible than inline config, easier to customize changelog sections"
  - id: bump-pre-major
    choice: "bump-minor-pre-major and bump-patch-for-minor-pre-major enabled"
    reason: "While version < 1.0.0, breaking changes bump minor, features bump patch"
  - id: hidden-changelog-sections
    choice: "Hide chore, test, ci from changelog"
    reason: "Keep changelog focused on user-facing changes (feat, fix, perf, refactor, docs)"

metrics:
  duration: 2m
  completed: 2026-01-23
---

# Quick Task 015: Add Release Please for Automated Releases

**One-liner:** Release Please GitHub Action with Rust config for automated versioning, changelog, and release creation.

## Changes Made

### Task 1: Release Please workflow and configuration
- Created `.github/workflows/release-please.yml` triggered on master push
- Created `.release-please-manifest.json` tracking version 0.1.0
- Created `release-please-config.json` with Rust release type and changelog sections
- **Commit:** 80b6ec6

### Task 2: CONTRIBUTING.md documentation
- Documented conventional commit format and types
- Explained version bump rules (feat=minor, fix=patch)
- Described automated release workflow
- **Commit:** 7a5530d

## Deviations from Plan

None - plan executed exactly as written.

## Release Integration

The Release Please workflow integrates with the existing release pipeline:

```
Commits to master
       |
       v
Release Please opens/updates PR
       |
       v
Merge release PR
       |
       v
Release Please creates GitHub release with v*.*.* tag
       |
       v
release.yml workflow triggered
       |
       v
Docker image built and pushed to ghcr.io
```

## Configuration Details

**release-please-config.json:**
- `release-type: rust` - Updates Cargo.toml version
- `include-component-in-tag: false` - Tags are `v0.2.0` not `ci-tui-v0.2.0`
- `bump-minor-pre-major: true` - Breaking changes bump minor while < 1.0.0

**Changelog sections visible:**
- Features (feat)
- Bug Fixes (fix)
- Performance Improvements (perf)
- Code Refactoring (refactor)
- Documentation (docs)

**Hidden from changelog:**
- Miscellaneous Chores (chore)
- Tests (test)
- Continuous Integration (ci)

## Verification

- [x] All config files valid YAML/JSON
- [x] Workflow triggers on push to master
- [x] Tag format matches release.yml trigger (v*.*.*)
- [x] CONTRIBUTING.md documents release process
