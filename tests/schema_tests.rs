//! End-to-end `ci-tui schema`: prints the generated config JSON Schema, needs no config.

#[test]
fn binary_schema_prints_generated_schema_without_config() {
    let tmp = tempfile::TempDir::new().unwrap();
    let out = std::process::Command::new(env!("CARGO_BIN_EXE_ci-tui"))
        .current_dir(tmp.path())
        .arg("schema")
        .output()
        .unwrap();
    assert!(
        out.status.success(),
        "{}",
        String::from_utf8_lossy(&out.stderr)
    );
    assert_eq!(
        String::from_utf8(out.stdout).unwrap(),
        ci_tui::schema::generate()
    );
}
