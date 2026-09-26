//! JSON Schema for `ci-tui.yaml`, derived via `schemars` from the config structs.
//!
//! Printed by `ci-tui schema`, committed at `schema/ci-tui.schema.json` and attached to each
//! release; the `init` template points editors at it via a
//! `# yaml-language-server: $schema=...` line.

/// Config JSON Schema as pretty-printed JSON with a trailing newline.
pub fn generate() -> String {
    let schema = schemars::schema_for!(crate::config::RawCiConfig);
    let json = serde_json::to_string_pretty(&schema).expect("schema serializes to JSON");
    format!("{json}\n")
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::Path;

    /// Repo-relative path of the committed schema.
    const SCHEMA_FILE: &str = "schema/ci-tui.schema.json";

    fn manifest_path(rel: &str) -> std::path::PathBuf {
        Path::new(env!("CARGO_MANIFEST_DIR")).join(rel)
    }

    fn validator() -> jsonschema::Validator {
        let schema = schemars::schema_for!(crate::config::RawCiConfig);
        jsonschema::validator_for(schema.as_value())
            .expect("generated schema must be a valid JSON Schema")
    }

    /// Schema errors for `yaml` (empty = valid), YAML converted via serde_json.
    fn schema_errors(yaml: &str) -> Vec<String> {
        let value: serde_json::Value = serde_yaml::from_str(yaml).unwrap();
        validator()
            .iter_errors(&value)
            .map(|e| format!("{e} at {}", e.instance_path()))
            .collect()
    }

    fn load_config_accepts(yaml: &str) -> Result<(), String> {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("ci-tui.yaml");
        std::fs::write(&path, yaml).unwrap();
        crate::commands::validate(&path).map_err(|e| format!("{e:#}"))
    }

    #[test]
    fn test_committed_schema_matches_generated() {
        let committed = std::fs::read_to_string(manifest_path(SCHEMA_FILE)).unwrap_or_default();
        assert!(
            committed == generate(),
            "{SCHEMA_FILE} is out of date with the config structs; regenerate with \
             `make schema` (or `cargo run -q -- schema > {SCHEMA_FILE}`)"
        );
    }

    #[rstest::rstest]
    #[case::init_template(crate::commands::TEMPLATE.to_string())]
    #[case::repo_config(std::fs::read_to_string(manifest_path("ci-tui.yaml")).unwrap())]
    #[case::php_example(
        std::fs::read_to_string(manifest_path("docs/examples/php-symfony.yaml")).unwrap()
    )]
    #[case::local_runner_without_docker(
        "version: 2\nrunner: local\ngit: {base_branch: main, fallback_branch: HEAD~1}\n\
         file_patterns: {}\nchecks: {}\n"
            .to_string()
    )]
    fn test_valid_config_passes_schema(#[case] yaml: String) {
        load_config_accepts(&yaml).expect("fixture must load");
        let errors = schema_errors(&yaml);
        assert!(errors.is_empty(), "{errors:#?}");
    }

    const MINIMAL: &str = "version: 2\n\
        docker: {project_dir: ., shell: bash}\n\
        git: {base_branch: main, fallback_branch: HEAD~1}\n\
        file_patterns: {src: {pattern: '\\.rs$'}}\n\
        checks:\n  g:\n    checks:\n      c:\n        name: C\n        command: 'true'\n";

    #[rstest::rstest]
    #[case::unknown_top_level_field("not_a_field: true\n")]
    #[case::unknown_check_field("        colour: red\n")]
    #[case::wrong_type_parallel("    parallel: yes please\n")]
    #[case::bad_timeout("        timeout: 10x\n")]
    #[case::zero_timeout("        timeout: 0s\n")]
    #[case::bad_runner("runner: podman\n")]
    #[case::unknown_strategy(
        "        triggers:\n          test_discovery:\n            source_pattern: src\n            \
         strategies:\n              - type: magic\n"
    )]
    fn test_invalid_config_fails_schema(#[case] extra: &str) {
        // Appended: indentation picks the level (top / group `g` / check `c`).
        let yaml = format!("{MINIMAL}{extra}");
        assert!(
            load_config_accepts(&yaml).is_err(),
            "load_config accepted:\n{yaml}"
        );
        assert!(!schema_errors(&yaml).is_empty(), "schema accepted:\n{yaml}");
    }

    #[test]
    fn test_docker_section_required_unless_local() {
        let yaml = MINIMAL.replace("docker: {project_dir: ., shell: bash}\n", "");
        assert!(load_config_accepts(&yaml).is_err());
        assert!(!schema_errors(&yaml).is_empty(), "schema accepted:\n{yaml}");
    }
}
