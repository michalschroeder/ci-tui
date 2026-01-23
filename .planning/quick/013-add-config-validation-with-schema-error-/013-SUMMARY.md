---
phase: quick
plan: 013
subsystem: config
completed: 2026-01-23
duration: 1m
status: complete
tags: [validation, config, error-handling, serde]

# Dependency graph
requires: []
provides:
  - "Strict config validation with descriptive error messages"
  - "Unknown field detection for all config structs"
  - "Test coverage for config validation errors"
affects: []

# Tech stack
tech-stack:
  added: []
  patterns:
    - "serde deny_unknown_fields for schema validation"

# Key files
key-files:
  created: []
  modified:
    - path: "src/config.rs"
      purpose: "Added deny_unknown_fields to all config structs and test coverage"
      lines-added: 81
      lines-removed: 1

# Decisions made
decisions:
  - id: deny-unknown-fields-all-structs
    what: "Apply deny_unknown_fields to all config structs"
    why: "Catch typos and invalid fields early with descriptive errors"
    alternatives: "Manual validation or custom deserializers"
    outcome: "serde_yaml provides good error messages by default"

# Metrics
metrics:
  duration: 1m
  tasks-completed: 2
  tests-added: 3
  commits: 1
---

# Quick Task 013: Add Config Validation With Schema Error Summary

**One-liner:** Config validation now rejects unknown fields with descriptive error messages showing field names and locations.

## What Changed

### Config Validation
Added `#[serde(deny_unknown_fields)]` attribute to all config structs:
- **Top-level:** CiConfig
- **Docker/Git:** DockerConfig, GitConfig
- **Checks:** GroupConfig, CheckDefinition, CheckTriggers
- **Test Discovery:** TestDiscoveryConfig, TestDiscoveryStrategy, PathMappingRule
- **Pre-commands:** PreCommand
- **File Patterns:** FilePattern

### Error Message Examples
Users now get clear errors when config has typos:

```
Error: unknown field `typo_field`, expected one of `version`, `docker`, `git`, ...
```

For nested fields:
```
Error: unknown field `unknown_docker_field`, expected one of `project_dir`, `service`, ...
```

### Test Coverage
Added three tests to verify error messages:
1. `test_unknown_field_rejected_top_level` - Top-level unknown field
2. `test_unknown_field_rejected_nested` - Unknown field in nested struct
3. `test_typo_produces_helpful_error` - Common typo scenario

## Implementation Details

### serde deny_unknown_fields
The attribute leverages serde_yaml's built-in error reporting, which:
- Shows the exact unknown field name
- Lists expected field names
- Includes YAML location information
- Works for both struct fields and enum variants

### Backward Compatibility
All existing valid configs continue to work. This only affects configs with:
- Typos in field names
- Extra/invalid fields
- Wrong nesting structure

## Verification

```bash
# All tests pass (69 tests)
make test
✓ test_unknown_field_rejected_top_level
✓ test_unknown_field_rejected_nested
✓ test_typo_produces_helpful_error

# Clippy clean
make clippy
✓ No warnings
```

## Deviations from Plan

None - plan executed exactly as written.

## Next Phase Readiness

**Blockers:** None
**Concerns:** None

This is a quality-of-life improvement that makes config errors easier to debug. All config structs now enforce strict schema validation.

## Commits

| Task | Description | Commit | Files |
|------|-------------|--------|-------|
| 1-2 | Add deny_unknown_fields and tests | f66485a | src/config.rs |
