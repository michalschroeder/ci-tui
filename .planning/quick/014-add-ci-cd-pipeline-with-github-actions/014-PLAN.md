---
phase: quick
plan: 014
type: execute
wave: 1
depends_on: []
files_modified:
  - .github/workflows/release.yml
autonomous: true

must_haves:
  truths:
    - "Docker image builds automatically on version tags"
    - "Image is pushed to GitHub Container Registry (ghcr.io)"
    - "Image includes version metadata from git tag"
  artifacts:
    - path: ".github/workflows/release.yml"
      provides: "CD workflow for Docker image builds"
      contains: "ghcr.io"
  key_links:
    - from: ".github/workflows/release.yml"
      to: "Dockerfile"
      via: "docker/build-push-action"
      pattern: "build-push-action"
---

<objective>
Add CD (Continuous Deployment) workflow to build and publish Docker images to GitHub Container Registry on version tags.

Purpose: Enable automated releases - when a version tag (e.g., v0.1.0) is pushed, the Docker image is automatically built and published to ghcr.io, making it easy for users to pull and run CI-TUI.

Output: New `.github/workflows/release.yml` workflow file
</objective>

<execution_context>
@./.claude/get-shit-done/workflows/execute-plan.md
@./.claude/get-shit-done/templates/summary.md
</execution_context>

<context>
@.planning/STATE.md
@.github/workflows/ci.yml
@Dockerfile
@Makefile
</context>

<tasks>

<task type="auto">
  <name>Task 1: Create release workflow for Docker image publishing</name>
  <files>.github/workflows/release.yml</files>
  <action>
Create a GitHub Actions workflow that:

1. Triggers on version tags (pattern: `v*.*.*`)
2. Uses QEMU and Docker Buildx for multi-platform builds (linux/amd64, linux/arm64)
3. Logs into GitHub Container Registry using GITHUB_TOKEN
4. Extracts metadata (version tags, labels) from git tag
5. Builds and pushes Docker image with:
   - Tags: version tag (e.g., v0.1.0), latest, sha
   - Build args: CI_TUI_GIT_HASH and CI_TUI_BUILD_DATE (matching Makefile pattern)
   - OCI labels from docker/metadata-action

Use standard actions:
- docker/setup-qemu-action@v3
- docker/setup-buildx-action@v3
- docker/login-action@v3 (registry: ghcr.io)
- docker/metadata-action@v5 (for tags/labels)
- docker/build-push-action@v6

Image name: ghcr.io/${{ github.repository }}
  </action>
  <verify>
Verify workflow syntax: `cat .github/workflows/release.yml` shows valid YAML with:
- on.push.tags pattern
- GHCR login step
- build-push-action with platforms and push: true
  </verify>
  <done>
Release workflow exists that will build and push Docker images to ghcr.io when version tags are pushed
  </done>
</task>

<task type="auto">
  <name>Task 2: Update existing CI workflow to also build (but not push) on PRs</name>
  <files>.github/workflows/ci.yml</files>
  <action>
Add a new job to the existing CI workflow that builds the Docker image (without pushing) on PRs to catch Dockerfile issues early.

Add job named "build" that:
1. Runs on ubuntu-latest
2. Uses docker/setup-buildx-action@v3
3. Builds with docker/build-push-action@v6:
   - push: false (just verify build works)
   - load: true (load into local Docker for potential future testing)
   - Build args: CI_TUI_GIT_HASH=${{ github.sha }}, CI_TUI_BUILD_DATE from date command
   - Cache: type=gha for GitHub Actions cache

This ensures Dockerfile changes don't break the build before merge.
  </action>
  <verify>
`cat .github/workflows/ci.yml` shows new "build" job with:
- docker/build-push-action
- push: false
- load: true
  </verify>
  <done>
CI workflow includes Docker build verification on PRs, catching Dockerfile issues before merge
  </done>
</task>

</tasks>

<verification>
- [ ] `.github/workflows/release.yml` exists with valid YAML syntax
- [ ] Release workflow triggers on `v*.*.*` tags
- [ ] Release workflow pushes to ghcr.io
- [ ] CI workflow has build job that verifies Dockerfile on PRs
- [ ] Both workflows use consistent build args (CI_TUI_GIT_HASH, CI_TUI_BUILD_DATE)
</verification>

<success_criteria>
1. New release workflow created at `.github/workflows/release.yml`
2. Release workflow configured to build multi-platform images and push to GHCR on version tags
3. CI workflow updated with Docker build verification job
4. Workflows use standard docker/* actions following GitHub Actions best practices
</success_criteria>

<output>
After completion, create `.planning/quick/014-add-ci-cd-pipeline-with-github-actions/014-SUMMARY.md`
</output>
