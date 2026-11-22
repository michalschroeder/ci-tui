//! `ci-tui init` through the binary: `git.base_branch` detected from the
//! repo the config is written into, printed, and the written file valid.

use ci_tui::exit;

mod common;
use common::{assert_code, commit, git, run_ci_tui, stdout, write};
use std::path::Path;

/// Repo at `dir` on `branch` with one commit of `a.rs`
fn repo_at(dir: &Path, branch: &str) {
    std::fs::create_dir_all(dir).unwrap();
    git(dir, &["init", "-q", "-b", branch]);
    write(dir, "a.rs", "a\n");
    git(dir, &["add", "a.rs"]);
    commit(dir, "one");
}

/// `init` printed `branch` and wrote it to `path`, which validates
fn assert_init_wrote(out: &std::process::Output, path: &Path, branch: &str) {
    assert_code(out, exit::SUCCESS);
    let stdout = stdout(out);
    assert!(
        stdout.contains(&format!("(git.base_branch: {branch})")),
        "{stdout}"
    );
    let config = ci_tui::config::load_config(path).expect("written config must load");
    assert_eq!(config.git.base_branch, branch);
}

/// The issue's repro: a single-commit `master` repo without a remote
#[test]
fn init_on_master_without_remote_writes_master_and_run_resolves_it() {
    let tmp = tempfile::TempDir::new().unwrap();
    let repo = tmp.path().join("repo");
    repo_at(&repo, "master");

    let out = run_ci_tui(&repo, &["init"]);
    assert_init_wrote(&out, &repo.join("ci-tui.yaml"), "master");

    // Only the new ci-tui.yaml changed: no check selected, base ref resolves
    let out = run_ci_tui(&repo, &["--simple", "--no-color"]);
    assert_code(&out, exit::SUCCESS);
    let stdout = stdout(&out);
    assert!(stdout.contains("vs master"), "{stdout}");
}

/// Run from a `main` repo, writing into a separate `master` repo: the
/// branch comes from the config's repo, not the caller's
#[rstest::rstest]
#[case::positional(&["init", "../target/ci-tui.yaml"])]
#[case::config_flag(&["--config", "../target/ci-tui.yaml", "init"])]
fn init_detects_branch_of_the_config_repo(#[case] args: &[&str]) {
    let tmp = tempfile::TempDir::new().unwrap();
    let caller = tmp.path().join("caller");
    let target = tmp.path().join("target");
    repo_at(&caller, "main");
    repo_at(&target, "master");

    let out = run_ci_tui(&caller, args);
    assert_init_wrote(&out, &target.join("ci-tui.yaml"), "master");
    assert!(!caller.join("ci-tui.yaml").exists());
}
