//! Raw YAML configs for tests that run the binary (it reads config files
//! from disk).
// Compiled into every integration test, not all use these
#![allow(dead_code)]

/// Config head shared by the fixtures below
pub const HEAD: &str = "version: 2
git:
  base_branch: main
  fallback_branch: HEAD~1
file_patterns: {}
";

/// One always-run check `c` running `command`
pub fn check(command: &str) -> String {
    format!("checks:\n  g:\n    checks:\n      c:\n        name: C\n        command: {command}\n")
}

/// Local-mode config with one always-run check running `command`
pub fn local_config(command: &str) -> String {
    format!("{HEAD}runner: local\n{}", check(command))
}

/// Docker-mode config; `extra` adds fields to check `c`. Fine for tests that
/// run no command in docker (no daemon needed).
pub fn docker_config(extra: &str) -> String {
    format!(
        "{HEAD}docker:\n  project_dir: .\n  service: app\n  shell: sh\n{}{extra}",
        check("'true'")
    )
}
