---
phase: quick
plan: 013
type: execute
wave: 1
depends_on: []
files_modified:
  - src/config.rs
autonomous: true
must_haves:
  truths:
    - "Unknown fields in config YAML produce descriptive error messages"
    - "Error messages indicate which field is unknown and at what location"
    - "All existing valid configs continue to work"
  artifacts:
    - path: "src/config.rs"
      provides: "Config structs with deny_unknown_fields"
      contains: "deny_unknown_fields"
  key_links:
    - from: "src/config.rs"
      to: "serde_yaml"
      via: "deny_unknown_fields attribute"
      pattern: "deny_unknown_fields"
---

<objective>
Add strict config validation that rejects unknown fields with descriptive error messages.

Purpose: Users who typo field names in config YAML should get clear errors explaining which field is unrecognized, rather than silently having their config ignored.

Output: Config structs with `#[serde(deny_unknown_fields)]` attribute that produce clear error messages.
</objective>

<execution_context>
@./.claude/get-shit-done/workflows/execute-plan.md
@./.claude/get-shit-done/templates/summary.md
</execution_context>

<context>
@src/config.rs
</context>

<tasks>

<task type="auto">
  <name>Task 1: Add deny_unknown_fields to all config structs</name>
  <files>src/config.rs</files>
  <action>
Add `#[serde(deny_unknown_fields)]` attribute to all config structs in config.rs:

1. CiConfig - top-level config
2. FilePattern - file pattern definitions
3. DockerConfig - docker configuration
4. GitConfig - git branch configuration
5. GroupConfig - check group configuration
6. CheckDefinition - individual check config
7. CheckTriggers - trigger configuration
8. TestDiscoveryConfig - test discovery settings
9. PathMappingRule - path mapping rules
10. PreCommand - pre-command configuration

For enum TestDiscoveryStrategy, add `#[serde(deny_unknown_fields)]` to each variant's struct fields (PathMapping and GrepSearch are internally tagged enums with struct-like content).

The attribute goes directly after `#[derive(Debug, Deserialize)]` or similar derive lines, like:
```rust
#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct CiConfig {
```

Note: serde_yaml provides good error messages by default with deny_unknown_fields, showing field name and location. No additional library needed.
  </action>
  <verify>
Create a test config with an unknown field and verify parsing fails with descriptive error:
```bash
cd /home/ms/projects/ci-tui && cargo test test_unknown_field_rejected --no-fail-fast 2>&1 | head -50
```
  </verify>
  <done>All config structs have deny_unknown_fields attribute</done>
</task>

<task type="auto">
  <name>Task 2: Add tests for unknown field rejection</name>
  <files>src/config.rs</files>
  <action>
Add tests to the existing `mod tests` section in config.rs:

1. `test_unknown_field_rejected_top_level` - Test that unknown field at top level produces error containing field name
2. `test_unknown_field_rejected_nested` - Test that unknown field in nested struct (e.g., docker section) produces descriptive error
3. `test_typo_produces_helpful_error` - Test common typo scenario (e.g., "base_branc" instead of "base_branch") shows clear error

Each test should:
- Create YAML with intentional unknown/typo field
- Parse with serde_yaml::from_str
- Assert Err result
- Assert error message contains the unknown field name

Example structure:
```rust
#[test]
fn test_unknown_field_rejected_top_level() {
    let yaml = r#"
version: 2
docker:
  project_dir: .
git:
  base_branch: main
  fallback_branch: HEAD~1
file_patterns: {}
checks: {}
typo_field: oops
"#;
    let result: Result<CiConfig, _> = serde_yaml::from_str(yaml);
    assert!(result.is_err());
    let err = result.unwrap_err().to_string();
    assert!(err.contains("typo_field"), "Error should mention unknown field: {}", err);
}
```
  </action>
  <verify>
```bash
cd /home/ms/projects/ci-tui && cargo test test_unknown_field -- --nocapture 2>&1
```
  </verify>
  <done>Tests verify unknown fields produce descriptive errors with field names</done>
</task>

</tasks>

<verification>
```bash
# All tests pass
cd /home/ms/projects/ci-tui && cargo test 2>&1 | tail -20

# Clippy clean
cd /home/ms/projects/ci-tui && cargo clippy -- -D warnings 2>&1 | tail -10
```
</verification>

<success_criteria>
- All config structs have #[serde(deny_unknown_fields)] attribute
- Unknown fields in config produce errors mentioning the field name
- All existing tests still pass (no regression)
- Clippy clean
</success_criteria>

<output>
After completion, create `.planning/quick/013-add-config-validation-with-schema-error-/013-SUMMARY.md`
</output>
