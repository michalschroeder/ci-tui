//! End-to-end config discovery (#155): without `--config`, `ci-tui.yaml` is
//! searched from cwd up to the git repo root and ci-tui runs from its dir;
//! `ci-tui.local.yaml` next to it overrides a few personal settings.

use ci_tui::exit;

mod common;
use common::configs::{docker_yaml, local_yaml};
use common::{assert_code, git, run_ci_tui_env, stderr, stdout};

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

#[test]
fn config_at_repo_root_found_from_subdir() {
    let tmp = repo(true, &[("ci-tui.yaml", &local_yaml("'true'"))]);
    let out = run_in(&tmp, "sub/deeper", &["--simple", "--files", "a.rs"]);
    assert_code(&out, exit::SUCCESS);
}

#[test]
fn nearest_config_wins() {
    let tmp = repo(
        true,
        &[
            ("ci-tui.yaml", &local_yaml("'false'")),
            ("sub/ci-tui.yaml", &local_yaml("'true'")),
        ],
    );
    let out = run_in(&tmp, "sub/deeper", &["--simple", "--files", "a.rs"]);
    assert_code(&out, exit::SUCCESS);
}

#[test]
fn config_above_repo_root_is_not_found() {
    let tmp = repo(true, &[]);
    std::fs::write(tmp.path().join("ci-tui.yaml"), local_yaml("'true'")).unwrap();
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
    let tmp = repo(false, &[("ci-tui.yaml", &local_yaml("'true'"))]);
    let out = run_in(&tmp, "sub", &["--list", "--files", "a.rs"]);
    assert_code(&out, exit::CONFIG_ERROR);
}

#[test]
fn files_are_relative_to_discovered_config_dir() {
    // Docker mode keeps `--files` as given; from a subdir they are rewritten
    // relative to the config's dir, as if ci-tui ran there
    let tmp = repo(true, &[("ci-tui.yaml", &docker_yaml(""))]);
    let out = run_in(&tmp, "sub", &["--list", "--files", "a.rs"]);
    assert_code(&out, exit::SUCCESS);
    assert!(stdout(&out).contains("  sub/a.rs\n"), "{}", stdout(&out));
}

#[test]
fn files_kept_as_given_in_config_dir() {
    let tmp = repo(true, &[("ci-tui.yaml", &docker_yaml(""))]);
    let out = run_in(&tmp, "", &["--list", "--files", "./a.rs"]);
    assert_code(&out, exit::SUCCESS);
    assert!(stdout(&out).contains("  ./a.rs\n"), "{}", stdout(&out));
}

#[test]
fn config_flag_skips_walk_up() {
    // Explicit --config: no discovery, ci-tui runs from cwd (files as given)
    let tmp = repo(true, &[("ci-tui.yaml", &docker_yaml(""))]);
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
    let tmp = repo(
        true,
        &[
            ("ci-tui.yaml", &local_yaml("'test \"$FROM_LOCAL\" = yes'")),
            (
                "ci-tui.local.yaml",
                "local:\n  env:\n    FROM_LOCAL: 'yes'\n",
            ),
        ],
    );
    let out = run_in(&tmp, "sub", &["--simple", "--files", "a.rs"]);
    assert_code(&out, exit::SUCCESS);
}

#[test]
fn invalid_local_overrides_exit_2_naming_the_file() {
    let tmp = repo(
        true,
        &[
            ("ci-tui.yaml", &local_yaml("'true'")),
            ("ci-tui.local.yaml", "checks: {}\n"),
        ],
    );
    let out = run_in(&tmp, "sub", &["--list", "--files", "a.rs"]);
    assert_code(&out, exit::CONFIG_ERROR);
    assert!(
        stderr(&out).contains("ci-tui.local.yaml"),
        "{}",
        stderr(&out)
    );
}

#[test]
fn validate_uses_discovered_config() {
    let tmp = repo(
        true,
        &[
            ("ci-tui.yaml", &local_yaml("'true'")),
            ("ci-tui.local.yaml", "checks: {}\n"),
        ],
    );
    let out = run_in(&tmp, "sub", &["validate"]);
    assert_code(&out, exit::CONFIG_ERROR);
    assert!(
        stderr(&out).contains("ci-tui.local.yaml"),
        "checks the discovered config with its overrides: {}",
        stderr(&out)
    );
}
