---
phase: quick
plan: 015
type: execute
wave: 1
depends_on: []
files_modified:
  - .github/workflows/release-please.yml
  - .release-please-manifest.json
  - release-please-config.json
autonomous: true

must_haves:
  truths:
    - "Release Please workflow triggers on pushes to master"
    - "Release Please creates release PRs with changelog from conventional commits"
    - "When release PR is merged, Release Please creates a GitHub release with tag"
    - "The tag triggers the existing release.yml workflow for Docker image publishing"
  artifacts:
    - path: ".github/workflows/release-please.yml"
      provides: "Release Please GitHub Action workflow"
    - path: ".release-please-manifest.json"
      provides: "Version tracking for Release Please"
    - path: "release-please-config.json"
      provides: "Release Please configuration for Rust/Cargo"
  key_links:
    - from: ".github/workflows/release-please.yml"
      to: ".github/workflows/release.yml"
      via: "tag creation triggers release workflow"
      pattern: "v\\*\\.\\*\\.\\*"
---

<objective>
Add Release Please GitHub Action to automate version bumping, changelog generation, and release creation.

Purpose: Enable automated releases via conventional commits - when commits land on master, Release Please opens/updates a release PR. When merged, it creates a GitHub release with a tag that triggers the existing Docker image publishing workflow.

Output: Working Release Please configuration that integrates with existing release.yml
</objective>

<execution_context>
@./.claude/get-shit-done/workflows/execute-plan.md
@./.claude/get-shit-done/templates/summary.md
</execution_context>

<context>
@.github/workflows/release.yml (existing release workflow triggered by v*.*.* tags)
@Cargo.toml (current version: 0.1.0)
</context>

<tasks>

<task type="auto">
  <name>Task 1: Create Release Please workflow and configuration</name>
  <files>
    .github/workflows/release-please.yml
    .release-please-manifest.json
    release-please-config.json
  </files>
  <action>
Create Release Please GitHub Action workflow at `.github/workflows/release-please.yml`:
- Trigger on push to master branch
- Use googleapis/release-please-action@v4
- Configure for Rust/Cargo package type
- Use manifest-based configuration for flexibility
- Grant contents: write and pull-requests: write permissions

Create `.release-please-manifest.json`:
- Track current version from Cargo.toml (0.1.0)
- Map root directory "." to the version

Create `release-please-config.json`:
- Set release-type to "rust"
- Configure changelog sections for conventional commit types (feat, fix, refactor, etc.)
- Set include-component-in-tag to false (single package, no need for component prefix)
- Enable versioning-strategy to "default" for semver

Key configuration points:
- release-type: "rust" so it updates Cargo.toml version
- tag-name: "v${version}" to match release.yml trigger pattern
- bump-minor-pre-major: true (while in 0.x.x, breaking changes bump minor not major)
  </action>
  <verify>
    - Files exist: ls -la .github/workflows/release-please.yml .release-please-manifest.json release-please-config.json
    - Workflow YAML is valid: python3 -c "import yaml; yaml.safe_load(open('.github/workflows/release-please.yml'))"
    - Manifest JSON is valid: python3 -c "import json; json.load(open('.release-please-manifest.json'))"
    - Config JSON is valid: python3 -c "import json; json.load(open('release-please-config.json'))"
  </verify>
  <done>
    - release-please.yml workflow exists and triggers on master push
    - Manifest tracks version 0.1.0
    - Config uses rust release type with v${version} tag format
  </done>
</task>

<task type="auto">
  <name>Task 2: Document release process in README or CONTRIBUTING</name>
  <files>
    CONTRIBUTING.md
  </files>
  <action>
Create CONTRIBUTING.md with release process documentation:

1. Conventional Commits section:
   - Explain commit message format (feat:, fix:, refactor:, etc.)
   - Note that commits drive changelog generation
   - List which types trigger releases (feat = minor, fix = patch)
   - Mention BREAKING CHANGE footer for major bumps

2. Release Process section:
   - Explain Release Please creates/updates PR automatically
   - PR accumulates changes since last release
   - Merging PR triggers GitHub release creation
   - GitHub release tag triggers Docker image publishing

3. Version Bumping:
   - feat: bumps minor version
   - fix: bumps patch version
   - BREAKING CHANGE: bumps major (or minor while < 1.0)

Keep it concise - developers need to know the commit format and that releases are automated.
  </action>
  <verify>
    - File exists: test -f CONTRIBUTING.md && echo "exists"
    - Contains conventional commits section: grep -q "Conventional Commits" CONTRIBUTING.md
    - Contains release process section: grep -q "Release Process" CONTRIBUTING.md
  </verify>
  <done>
    - CONTRIBUTING.md exists with conventional commits guide
    - Release process is documented
  </done>
</task>

</tasks>

<verification>
1. All three Release Please config files exist and are valid YAML/JSON
2. Workflow triggers on push to master
3. Tag format (v${version}) matches release.yml trigger pattern (v*.*.*)
4. CONTRIBUTING.md documents the release workflow
</verification>

<success_criteria>
- Release Please workflow configured for Rust project
- Manifest-based configuration ready for first release
- Tag format matches existing release.yml trigger
- Process documented for contributors
</success_criteria>

<output>
After completion, create `.planning/quick/015-add-release-please-for-automated-release/015-SUMMARY.md`
</output>
