//! `ci-tui init` / `ci-tui validate` subcommands: scaffold and check config files.

use crate::exit::ConfigError;
use anyhow::{Context, Result};
use std::io::{ErrorKind, Write};
use std::path::Path;

/// Default config path for `init` / `validate` when none is given.
pub const DEFAULT_CONFIG_FILE: &str = "ci-tui.yaml";

/// Starter config. First line points yaml-language-server editors at this
/// version's schema, uploaded to each release (see `ci-tui schema`).
pub(crate) const TEMPLATE: &str = concat!(
    "# yaml-language-server: $schema=https://github.com/michalschroeder/ci-tui/releases/download/v",
    env!("CARGO_PKG_VERSION"),
    "/ci-tui.schema.json\n",
    r#"# ci-tui configuration
# Full reference: https://github.com/michalschroeder/ci-tui/blob/master/docs/configuration.md
version: 2

# Where checks run: docker (default) or local (directly on the host).
# With `runner: local` the docker section is not needed:
# runner: local
# local:
#   shell: bash
#   env:
#     APP_ENV: testing

docker:
  # Directory with your compose file (for `docker compose --project-directory`)
  project_dir: .
  # Compose service checks run in (per check: `service:`). ci-tui asks
  # `docker compose ps` for its running container and `docker exec`s into it;
  # when it is not running, `docker compose run --rm` starts one
  service: app
  # Shell inside the container ("/bin/sh" for Alpine images)
  shell: bash
  # env:
  #   APP_ENV: testing
  # Without compose: run every check in a plain image instead (drop
  # project_dir / service, no per-check `service:`)
  # image: my-dev-image:latest
  # volume_mount: "${HOST_PWD}:/app"      # repo mount for `docker run`
  # work_dir: /app                        # workdir inside the container

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

# Optional default max runtime for checks / pre-commands (<n>s, <n>m or <n>h).
# Expired checks are killed and reported as "timed out". Unset = no limit.
# timeout: 15m

# Groups run sequentially (YAML order); checks inside a group with
# `parallel: true` run concurrently, at most `max_parallel` at a time
# (top-level or per group; default CPU count; `--jobs N` overrides top-level)
# max_parallel: 4

# TUI: ring the bell and send a desktop notification when a run finishes
# (also `--notify`)
# notify: true
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
        # timeout: 5m                         # per-check limit, overrides top-level timeout
        triggers:
          file_pattern: source
"#
);

/// Write the starter config to `path`. Refuses to overwrite an existing file.
/// Errors are [`ConfigError`]s (exit 2).
pub fn init(path: &Path, base_branch: &str) -> Result<()> {
    write_template(path, base_branch).map_err(|e| ConfigError(e).into())
}

/// Starter config with `git.base_branch` set to `base_branch`, serialized as
/// a YAML scalar (quoted when needed: `@trunk`, `#trunk`, `123`, `yes`).
fn template_for(base_branch: &str) -> String {
    let scalar = serde_yaml::to_string(base_branch).expect("a string serializes");
    TEMPLATE.replacen(
        "  base_branch: main\n",
        &format!("  base_branch: {}\n", scalar.trim_end()),
        1,
    )
}

/// Default branch of the repository in `dir`, for `git.base_branch`:
/// `origin/HEAD` when it resolves (not dangling), else an existing local
/// `main` / `master`, else the current branch, else `main` (also outside a
/// git repo). Refs are read in full and their namespace stripped: `--short`
/// keeps `remotes/` when a local branch `origin/<name>` exists.
pub fn detect_base_branch(dir: &Path) -> String {
    let git = |args: &[&str]| -> Option<String> {
        let out = std::process::Command::new("git")
            .args(args)
            .current_dir(dir)
            .output()
            .ok()?;
        let text = String::from_utf8(out.stdout).ok()?.trim().to_string();
        (out.status.success() && !text.is_empty()).then_some(text)
    };
    let resolves = |r: &str| git(&["rev-parse", "--verify", "--quiet", r]).is_some();
    let target = |symref: &str, prefix: &str| {
        git(&["symbolic-ref", symref]).and_then(|r| r.strip_prefix(prefix).map(str::to_string))
    };
    let origin_head = "refs/remotes/origin/HEAD";
    resolves(origin_head)
        .then(|| target(origin_head, "refs/remotes/origin/"))
        .flatten()
        .or_else(|| {
            ["main", "master"]
                .into_iter()
                .find(|b| resolves(&format!("refs/heads/{b}")))
                .map(str::to_string)
        })
        .or_else(|| target("HEAD", "refs/heads/"))
        .unwrap_or_else(|| "main".to_string())
}

fn write_template(path: &Path, base_branch: &str) -> Result<()> {
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
    if let Err(e) = file.write_all(template_for(base_branch).as_bytes()) {
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
    fn test_template_local_variant_is_valid() {
        let start = TEMPLATE.find("docker:\n").unwrap();
        let end = TEMPLATE.find("\ngit:").unwrap();
        let local = format!("{}runner: local{}", &TEMPLATE[..start], &TEMPLATE[end..]);
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("ci-tui.yaml");
        std::fs::write(&path, local).unwrap();
        validate(&path).expect("local-mode template must parse");
    }

    #[test]
    fn test_init_writes_file() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("ci-tui.yaml");
        init(&path, "main").unwrap();
        assert_eq!(std::fs::read_to_string(&path).unwrap(), TEMPLATE);
    }

    use crate::watch::tests::in_git_hook;

    /// Real git in `dir`. Callers skip under [`in_git_hook`]: an inherited
    /// `GIT_DIR` / `GIT_WORK_TREE` would point these mutations (and
    /// `detect_base_branch`) at the outer repository.
    fn git(dir: &Path, args: &[&str]) {
        let ok = std::process::Command::new("git")
            .args(["-c", "user.name=t", "-c", "user.email=t@t"])
            .args(args)
            .current_dir(dir)
            .status()
            .unwrap()
            .success();
        assert!(ok, "git {args:?}");
    }

    /// Repo on `branch` with one empty commit
    fn repo(branch: &str) -> tempfile::TempDir {
        let dir = tempfile::tempdir().unwrap();
        git(dir.path(), &["init", "-q", "-b", branch]);
        git(dir.path(), &["commit", "-q", "--allow-empty", "-m", "one"]);
        dir
    }

    /// Points `origin/HEAD` at `refs/remotes/origin/<target>`
    fn set_origin_head(dir: &Path, target: &str) {
        let target = format!("refs/remotes/origin/{target}");
        git(dir, &["symbolic-ref", "refs/remotes/origin/HEAD", &target]);
    }

    #[rstest::rstest]
    #[case::plain("develop")]
    #[case::at("@trunk")]
    #[case::hash("#trunk")]
    #[case::anchor("&trunk")]
    #[case::quote("'trunk")]
    #[case::number("123")]
    #[case::bool_like("yes")]
    fn test_init_base_branch_round_trips(#[case] branch: &str) {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("ci-tui.yaml");
        init(&path, branch).unwrap();
        let config = crate::config::load_config(&path).expect("written config must load");
        assert_eq!(config.git.base_branch, branch);
    }

    #[test]
    fn test_detect_base_branch_master_without_remote() {
        if in_git_hook() {
            return;
        }
        let dir = repo("master");
        assert_eq!(detect_base_branch(dir.path()), "master");
    }

    #[test]
    fn test_detect_base_branch_prefers_origin_head() {
        if in_git_hook() {
            return;
        }
        let dir = repo("master");
        git(dir.path(), &["checkout", "-q", "-b", "feature"]);
        git(
            dir.path(),
            &["update-ref", "refs/remotes/origin/develop", "HEAD"],
        );
        set_origin_head(dir.path(), "develop");
        assert_eq!(detect_base_branch(dir.path()), "develop");
    }

    #[test]
    fn test_detect_base_branch_skips_dangling_origin_head() {
        if in_git_hook() {
            return;
        }
        let dir = repo("master");
        set_origin_head(dir.path(), "deleted");
        assert_eq!(detect_base_branch(dir.path()), "master");
    }

    #[test]
    fn test_detect_base_branch_origin_head_beside_local_origin_branch() {
        if in_git_hook() {
            return;
        }
        // Local branch `origin/develop` makes `symbolic-ref --short` print
        // `remotes/origin/develop`
        let dir = repo("master");
        git(
            dir.path(),
            &["update-ref", "refs/remotes/origin/develop", "HEAD"],
        );
        git(dir.path(), &["branch", "origin/develop"]);
        set_origin_head(dir.path(), "develop");
        assert_eq!(detect_base_branch(dir.path()), "develop");
    }

    #[test]
    fn test_detect_base_branch_prefers_local_default_over_feature() {
        if in_git_hook() {
            return;
        }
        let dir = repo("master");
        git(dir.path(), &["checkout", "-q", "-b", "feature"]);
        assert_eq!(detect_base_branch(dir.path()), "master");
    }

    #[test]
    fn test_detect_base_branch_current_branch_fallback() {
        if in_git_hook() {
            return;
        }
        let dir = repo("origin/trunk");
        assert_eq!(detect_base_branch(dir.path()), "origin/trunk");
    }

    #[test]
    fn test_detect_base_branch_outside_repo_is_main() {
        if in_git_hook() {
            return;
        }
        let dir = tempfile::tempdir().unwrap();
        assert_eq!(detect_base_branch(dir.path()), "main");
    }

    #[test]
    fn test_init_refuses_to_overwrite() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("ci-tui.yaml");
        std::fs::write(&path, "existing").unwrap();
        let err = init(&path, "main").unwrap_err().to_string();
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

    #[test]
    fn test_validate_rejects_bad_timeout() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("ci-tui.yaml");
        let bad = TEMPLATE.replace(
            "        triggers:",
            "        timeout: 10x\n        triggers:",
        );
        std::fs::write(&path, bad).unwrap();
        let err = format!("{:#}", validate(&path).unwrap_err());
        assert!(
            err.contains("checks.quality.checks.lint: `timeout`"),
            "got: {err}"
        );
    }

    #[test]
    fn test_template_first_line_is_versioned_schema_url() {
        let first = TEMPLATE.lines().next().unwrap();
        assert_eq!(
            first,
            format!(
                "# yaml-language-server: $schema=https://github.com/michalschroeder/ci-tui/releases/download/v{}/ci-tui.schema.json",
                env!("CARGO_PKG_VERSION")
            )
        );
    }

    #[test]
    fn test_template_documents_timeout() {
        assert!(TEMPLATE.contains("timeout:"));
    }
}
