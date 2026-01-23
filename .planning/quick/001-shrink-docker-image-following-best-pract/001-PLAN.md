---
phase: quick-001
plan: 01
type: execute
wave: 1
depends_on: []
files_modified:
  - .dockerignore
  - Dockerfile
autonomous: true

must_haves:
  truths:
    - "Docker build context excludes target directory and other build artifacts"
    - "Docker image size is reduced or unchanged (cannot increase)"
  artifacts:
    - path: ".dockerignore"
      provides: "Build context exclusions"
      contains: "target"
    - path: "Dockerfile"
      provides: "Optimized container build"
  key_links: []
---

<objective>
Optimize Docker image build by adding .dockerignore and verifying current best practices are in place.

Purpose: Reduce build context transfer time (currently sending 3.5GB target/ dir) and verify image is as small as practical given runtime dependencies.
Output: .dockerignore file, potentially optimized Dockerfile, documented image size baseline.
</objective>

<execution_context>
@./.claude/get-shit-done/workflows/execute-plan.md
@./.claude/get-shit-done/templates/summary.md
</execution_context>

<context>
@.planning/STATE.md
@Dockerfile
@Cargo.toml
@CLAUDE.md
</context>

<analysis>
## Current State

- **Image size:** 117MB
- **Binary (ci-tui):** 3.6MB - already optimized with LTO, strip, panic=abort
- **Docker CLI:** 28.4MB - required for `docker compose exec`
- **Docker Compose:** 59.5MB - required for `docker compose exec`
- **Git:** 2.9MB - required for change detection (per CLAUDE.md)
- **Alpine base + libs:** ~22MB

## What's Already Optimized

The Dockerfile already follows best practices:
- Multi-stage build (builder + runtime stages)
- Alpine base image (smallest practical with package manager)
- Dependency caching with dummy src (layer optimization)
- BuildKit cache mounts for cargo registry
- Release profile with LTO, strip, codegen-units=1, panic=abort

## Optimization Opportunities

1. **Add .dockerignore** (HIGH IMPACT on build time)
   - Currently no .dockerignore exists
   - target/ directory is 3.5GB being sent to Docker daemon
   - Build context transfer is slow and wasteful

2. **Consider scratch/distroless** (REJECTED)
   - Would save ~8MB (Alpine overhead)
   - But requires static linking of OpenSSL/libc
   - git and docker-cli need glibc or musl runtime
   - Not worth the complexity for 8MB

3. **UPX compression** (REJECTED)
   - Would compress binary from 3.6MB to ~1.5MB
   - Adds startup decompression latency
   - Not worth the tradeoff for 2MB

## Conclusion

The image is already well-optimized. The main improvement is adding .dockerignore
to speed up builds, not reduce final image size. Runtime dependencies (docker CLI,
compose, git) are irreducible at ~93MB.
</analysis>

<tasks>

<task type="auto">
  <name>Task 1: Add .dockerignore for build context optimization</name>
  <files>.dockerignore</files>
  <action>
Create .dockerignore with standard Rust exclusions:

```
# Build artifacts (3.5GB+)
target/

# IDE and editor files
.idea/
.vscode/
*.swp
*.swo
*~

# Git directory (not needed in build)
.git/

# Documentation and planning
*.md
.planning/
docs/

# CI configuration
.github/
.gitlab-ci.yml
.travis.yml

# Test fixtures that aren't needed for build
tests/fixtures/

# Misc
*.log
.DS_Store
Thumbs.db
```

This prevents the 3.5GB target/ directory from being sent to Docker daemon,
dramatically speeding up builds.
  </action>
  <verify>
Run `docker build` and observe that build context is now small (KB not GB):
```bash
docker build --no-cache -t ci-tui:test . 2>&1 | head -5
```
Should show "transferring context" with KB/MB size, not GB.
  </verify>
  <done>.dockerignore exists and build context is under 100KB</done>
</task>

<task type="auto">
  <name>Task 2: Verify image size and document baseline</name>
  <files>None (verification only)</files>
  <action>
Build the image and compare size to baseline (117MB).

1. Build fresh image with .dockerignore:
   ```bash
   docker build -t ci-tui:optimized .
   ```

2. Compare sizes:
   ```bash
   docker images ci-tui --format "{{.Tag}}: {{.Size}}"
   ```

3. Document findings:
   - Build context transfer time improvement
   - Final image size (should be ~117MB, unchanged)
   - Breakdown of major size contributors for future reference

The image size itself won't decrease significantly because the large components
(docker-cli at 28MB, docker-compose at 60MB, git at 3MB) are required runtime
dependencies. The optimization is build speed, not image size.
  </action>
  <verify>
```bash
docker images ci-tui:optimized --format "{{.Size}}"
```
Image size should be approximately 117MB (within 5MB of original).
  </verify>
  <done>Image builds successfully, size verified as optimal for given runtime dependencies</done>
</task>

</tasks>

<verification>
- [ ] .dockerignore file exists in repository root
- [ ] .dockerignore excludes target/, .git/, and other build artifacts
- [ ] Docker build completes successfully
- [ ] Build context transfer is fast (under 1MB transferred)
- [ ] Final image size is ~117MB (no regression)
</verification>

<success_criteria>
- .dockerignore reduces build context from 3.5GB to under 1MB
- Docker image builds successfully
- Image size remains approximately 117MB (runtime dependencies are irreducible)
- Build time noticeably faster due to smaller context transfer
</success_criteria>

<output>
After completion, create `.planning/quick/001-shrink-docker-image-following-best-pract/001-SUMMARY.md`
</output>
