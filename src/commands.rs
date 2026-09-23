//! `ci-tui init` / `ci-tui validate` subcommands: scaffold and check config files.

use anyhow::{Context, Result};
use std::io::{ErrorKind, Write};
use std::path::Path;

/// Default config path for `init` / `validate` when none is given.
pub const DEFAULT_CONFIG_FILE: &str = "ci-tui.yaml";

pub(crate) const TEMPLATE: &str = r#"# ci-tui configuration
# Full reference: https://github.com/michalschroeder/ci-tui/blob/master/docs/configuration.md
version: 2

docker:
  # Directory containing docker-compose.yml (for `docker compose --project-directory`)
  project_dir: .
  # Compose service to exec checks in; container name derives as
  # {project}-{service}-1 (project = $COMPOSE_PROJECT_NAME, else project_dir name)
  service: app
  # container: myproject-app-1           # override if the derived name is wrong (e.g. compose `name:`)
  # Shell inside the container ("/bin/sh" for Alpine images)
  shell: bash
  # image: my-dev-image:latest            # for standalone `docker run` fallback
  # volume_mount: "${HOST_PWD}:/app"      # mount for `docker run`
  # work_dir: /app                        # workdir inside container
  # env:
  #   APP_ENV: testing

git:
  # Changed files are detected against origin/{base_branch}, then {base_branch},
  # then {fallback_branch}
  base_branch: main
  fallback_branch: HEAD~1

# Named regex patterns; checks reference these by key in triggers
file_patterns:
  source:
    pattern: '\.(rs|php|py|ts|go)$'
    color: yellow

# Files excluded from change detection
ignore_patterns:
  - '\.md$'
  - '^target/'

# Groups run sequentially (YAML order); checks inside a group with
# `parallel: true` run concurrently
checks:
  quality:
    name: Code Quality
    parallel: true
    checks:
      lint:
        name: Lint
        # {files} expands to the changed files matching the trigger.
        # Placeholder fails on purpose until replaced with a real command.
        command: echo "replace me — e.g. cargo clippy -- -D warnings" {files} && false
        # fix_command: echo "optional autofix — runs with --fix / 'x' key"
        triggers:
          file_pattern: source
"#;

/// Write the starter config to `path`. Refuses to overwrite an existing file.
pub fn init(path: &Path) -> Result<()> {
    let mut file = std::fs::OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(path)
        .map_err(|e| match e.kind() {
            ErrorKind::AlreadyExists => {
                anyhow::anyhow!("{} already exists — not overwriting", path.display())
            }
            _ => anyhow::Error::new(e).context(format!("failed to write {}", path.display())),
        })?;
    if let Err(e) = file.write_all(TEMPLATE.as_bytes()) {
        // Remove the partial file so a retry is not refused as "already exists".
        let _ = std::fs::remove_file(path);
        return Err(e).with_context(|| format!("failed to write {}", path.display()));
    }
    Ok(())
}

/// Validate a config file: parses + compiles patterns via `load_config`.
/// Errors already name the file (read / parse / invalid) via `load_config`.
pub fn validate(path: &Path) -> Result<()> {
    crate::config::load_config(path)?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_template_is_valid_config() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("ci-tui.yaml");
        std::fs::write(&path, TEMPLATE).unwrap();
        validate(&path).expect("init template must always parse");
    }

    #[test]
    fn test_init_writes_file() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("ci-tui.yaml");
        init(&path).unwrap();
        assert_eq!(std::fs::read_to_string(&path).unwrap(), TEMPLATE);
    }

    #[test]
    fn test_init_refuses_to_overwrite() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("ci-tui.yaml");
        std::fs::write(&path, "existing").unwrap();
        let err = init(&path).unwrap_err().to_string();
        assert!(err.contains("exists"), "got: {err}");
        assert_eq!(std::fs::read_to_string(&path).unwrap(), "existing");
    }

    #[test]
    fn test_validate_rejects_invalid_config() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("bad.yaml");
        std::fs::write(&path, "version: 2\nnot_a_field: true\n").unwrap();
        let err = format!("{:#}", validate(&path).unwrap_err());
        assert!(err.contains("not_a_field"), "got: {err}");
    }
}
