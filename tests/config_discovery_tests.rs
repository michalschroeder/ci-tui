//! End-to-end config discovery (#155): without `--config`, `ci-tui.yaml` is
//! searched from cwd up to the git repo root and ci-tui runs from its dir;
//! `ci-tui.local.yaml` next to it overrides a few personal settings.

use ci_tui::exit;

mod common;
use common::{assert_code, git, run_ci_tui_env, stdout};

// Edge case: the binary reads YAML files from disk, so configs stay raw YAML.
/// Config head shared by the fixtures below
const HEAD: &str = "version: 2
git:
  base_branch: main
  fallback_branch: HEAD~1
file_patterns: {}
";

/// Local-mode config with one always-run check running `command`
fn local_config(command: &str) -> String {
    format!(
        "{HEAD}runner: local\nchecks:\n  g:\n    checks:\n      c:\n        name: C\n        command: {command}\n"
    )
}

/// Docker-mode config; tests only `--list`, so no daemon is needed
fn docker_config() -> String {
    format!(
        "{HEAD}docker:\n  project_dir: .\n  shell: sh\nchecks:\n  g:\n    checks:\n      c:\n        name: C\n        command: 'true'\n"
    )
}

/// `<tmp>/repo` (a git repo unless `git_init` is false) with `sub/deeper`;
/// `configs` are (repo-relative path, content) files to write
fn repo(git_init: bool, configs: &[(&str, &str)]) -> tempfile::TempDir {
    let tmp = tempfile::TempDir::new().unwrap();
    let repo = tmp.path().join("repo");
    std::fs::create_dir_all(repo.join("sub/deeper")).unwrap();
    if git_init {
        git(&repo, &["init", "-q"]);
    }
    for (path, content) in configs {
        std::fs::write(repo.join(path), content).unwrap();
    }
    tmp
}

/// Run the binary in `<tmp>/repo/<dir>` with git discovery capped at `<tmp>`
fn run_in(tmp: &tempfile::TempDir, dir: &str, args: &[&str]) -> std::process::Output {
    let ceiling = tmp.path().to_str().unwrap();
    let cwd = tmp.path().join("repo").join(dir);
    run_ci_tui_env(&cwd, args, &[("GIT_CEILING_DIRECTORIES", ceiling)])
}

fn stderr(out: &std::process::Output) -> String {
    String::from_utf8_lossy(&out.stderr).into_owned()
}

#[test]
fn config_at_repo_root_found_from_subdir() {
    let tmp = repo(true, &[("ci-tui.yaml", &local_config("'true'"))]);
    let out = run_in(&tmp, "sub/deeper", &["--simple", "--files", "a.rs"]);
    assert_code(&out, exit::SUCCESS);
}

#[test]
fn nearest_config_wins() {
    let configs = [
        ("ci-tui.yaml", local_config("'false'")),
        ("sub/ci-tui.yaml", local_config("'true'")),
    ];
    let configs: Vec<_> = configs.iter().map(|(p, c)| (*p, c.as_str())).collect();
    let tmp = repo(true, &configs);
    let out = run_in(&tmp, "sub/deeper", &["--simple", "--files", "a.rs"]);
    assert_code(&out, exit::SUCCESS);
}

#[test]
fn config_above_repo_root_is_not_found() {
    let tmp = repo(true, &[]);
    std::fs::write(tmp.path().join("ci-tui.yaml"), local_config("'true'")).unwrap();
    let out = run_in(&tmp, "sub", &["--list", "--files", "a.rs"]);
    assert_code(&out, exit::CONFIG_ERROR);
    assert!(
        stderr(&out).contains("or its parents up to the repo root"),
        "{}",
        stderr(&out)
    );
}

#[test]
fn outside_repo_only_cwd_is_searched() {
    let tmp = repo(false, &[("ci-tui.yaml", &local_config("'true'"))]);
    let out = run_in(&tmp, "sub", &["--list", "--files", "a.rs"]);
    assert_code(&out, exit::CONFIG_ERROR);
}

#[test]
fn files_are_relative_to_discovered_config_dir() {
    // Docker mode keeps `--files` as given; from a subdir they are rewritten
    // relative to the config's dir, as if ci-tui ran there
    let tmp = repo(true, &[("ci-tui.yaml", &docker_config())]);
    let out = run_in(&tmp, "sub", &["--list", "--files", "a.rs"]);
    assert_code(&out, exit::SUCCESS);
    assert!(stdout(&out).contains("  sub/a.rs\n"), "{}", stdout(&out));
}

#[test]
fn files_kept_as_given_in_config_dir() {
    let tmp = repo(true, &[("ci-tui.yaml", &docker_config())]);
    let out = run_in(&tmp, "", &["--list", "--files", "./a.rs"]);
    assert_code(&out, exit::SUCCESS);
    assert!(stdout(&out).contains("  ./a.rs\n"), "{}", stdout(&out));
}

#[test]
fn config_flag_skips_walk_up() {
    // Explicit --config: no discovery, ci-tui runs from cwd (files as given)
    let tmp = repo(true, &[("ci-tui.yaml", &docker_config())]);
    let out = run_in(
        &tmp,
        "sub",
        &["-c", "../ci-tui.yaml", "--list", "--files", "a.rs"],
    );
    assert_code(&out, exit::SUCCESS);
    assert!(stdout(&out).contains("  a.rs\n"), "{}", stdout(&out));
}

#[test]
fn local_overrides_of_discovered_config_are_merged() {
    // The override makes the check read an env var only it sets
    let configs = [
        ("ci-tui.yaml", local_config("'test \"$FROM_LOCAL\" = yes'")),
        (
            "ci-tui.local.yaml",
            "local:\n  env:\n    FROM_LOCAL: 'yes'\n".to_string(),
        ),
    ];
    let configs: Vec<_> = configs.iter().map(|(p, c)| (*p, c.as_str())).collect();
    let tmp = repo(true, &configs);
    let out = run_in(&tmp, "sub", &["--simple", "--files", "a.rs"]);
    assert_code(&out, exit::SUCCESS);
}

#[test]
fn invalid_local_overrides_exit_2_naming_the_file() {
    let configs = [
        ("ci-tui.yaml", local_config("'true'")),
        ("ci-tui.local.yaml", "checks: {}\n".to_string()),
    ];
    let configs: Vec<_> = configs.iter().map(|(p, c)| (*p, c.as_str())).collect();
    let tmp = repo(true, &configs);
    let out = run_in(&tmp, "sub", &["--list", "--files", "a.rs"]);
    assert_code(&out, exit::CONFIG_ERROR);
    assert!(
        stderr(&out).contains("ci-tui.local.yaml"),
        "{}",
        stderr(&out)
    );
}
