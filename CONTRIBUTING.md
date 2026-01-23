# Contributing to CI-TUI

## Conventional Commits

This project uses [Conventional Commits](https://www.conventionalcommits.org/) for automated changelog generation and semantic versioning.

### Commit Message Format

```
<type>(<scope>): <description>

[optional body]

[optional footer(s)]
```

### Commit Types

| Type       | Description                          | Version Bump |
|------------|--------------------------------------|--------------|
| `feat`     | New feature                          | Minor        |
| `fix`      | Bug fix                              | Patch        |
| `perf`     | Performance improvement              | Patch        |
| `refactor` | Code change (no feature/fix)         | None         |
| `docs`     | Documentation only                   | None         |
| `test`     | Adding/updating tests                | None         |
| `chore`    | Maintenance tasks                    | None         |
| `ci`       | CI/CD changes                        | None         |

### Breaking Changes

For breaking changes, add `BREAKING CHANGE:` in the commit footer:

```
feat(config): change default timeout to 30s

BREAKING CHANGE: default timeout changed from 60s to 30s
```

While version is < 1.0.0, breaking changes bump the minor version.

## Release Process

Releases are fully automated via [Release Please](https://github.com/googleapis/release-please):

1. **Commits land on master** - Each commit with a conventional type is tracked
2. **Release PR created/updated** - Release Please opens a PR with:
   - Version bump based on commit types
   - Generated CHANGELOG.md entries
   - Updated Cargo.toml version
3. **Merge the release PR** - When ready to release, merge the PR
4. **GitHub Release created** - Release Please creates a tagged release
5. **Docker image published** - The tag triggers the release workflow which builds and pushes the Docker image to ghcr.io

### Manual Intervention

You don't need to:
- Update version numbers manually
- Write changelog entries
- Create git tags
- Trigger Docker builds

Just write good commit messages following conventional commits format.
