//! End-to-end exit codes of the binary: 0 pass, 1 checks failed, 2 config
//! error, 3 git / environment error (see `ci_tui::exit`).

use ci_tui::exit;

mod common;
use common::configs::{docker_yaml, local_yaml, yaml_check, YAML_HEAD};
use common::{
    assert_code, commit, git, run_ci_tui, run_config, run_config_env, stderr, stdout, write,
};
use std::path::Path;
use std::process::Output;

#[test]
fn passing_checks_exit_0() {
    let (_tmp, out) = run_config(&local_yaml("'true'"), &["--simple", "--files", "a.rs"]);
    assert_code(&out, exit::SUCCESS);
}

#[test]
fn failing_check_exits_1() {
    let (_tmp, out) = run_config(&local_yaml("'false'"), &["--simple", "--files", "a.rs"]);
    assert_code(&out, exit::CHECKS_FAILED);
}

#[test]
fn unparsable_config_exits_2_with_message() {
    let (_tmp, out) = run_config("checks: [not, a, map", &["--list", "--files", "a.rs"]);
    assert_code(&out, exit::CONFIG_ERROR);
    let stderr = String::from_utf8_lossy(&out.stderr);
    assert!(stderr.contains("ci-tui.yaml"), "{stderr}");
}

#[test]
fn missing_config_file_exits_2() {
    let (_tmp, out) = run_config("", &["-c", "nope.yaml", "--list", "--files", "a.rs"]);
    assert_code(&out, exit::CONFIG_ERROR);
}

/// Docker-section errors are config errors (exit 2) naming the problem
#[rstest::rstest]
#[case::compose_without_project_dir(
    "docker:\n  shell: sh\n",
    "",
    "`docker.project_dir` is required"
)]
#[case::volume_mount_without_image(
    "docker:\n  project_dir: .\n  shell: sh\n  volume_mount: .:/app\n",
    "",
    "`docker.volume_mount` needs `docker.image`"
)]
#[case::work_dir_without_image(
    "docker:\n  project_dir: .\n  shell: sh\n  work_dir: /app\n",
    "",
    "`docker.work_dir` needs `docker.image`"
)]
#[case::user_without_image(
    "docker:\n  project_dir: .\n  shell: sh\n  user: host\n",
    "",
    "`docker.user` needs `docker.image`"
)]
#[case::project_dir_with_image(
    "docker:\n  image: img\n  project_dir: .\n  shell: sh\n",
    "",
    "`docker.project_dir` is not allowed with `docker.image`"
)]
#[case::docker_service_with_image(
    "docker:\n  image: img\n  service: app\n  shell: sh\n",
    "",
    "`docker.service` is not allowed with `docker.image`"
)]
#[case::service_with_image(
    "docker:\n  image: img\n  shell: sh\n",
    "        service: worker\n",
    "`g.c` sets `service`"
)]
#[case::removed_container(
    "docker:\n  image: img\n  shell: sh\n  container: c\n",
    "",
    "container"
)]
fn invalid_docker_section_exits_2(#[case] docker: &str, #[case] extra: &str, #[case] msg: &str) {
    let config = format!("{YAML_HEAD}{docker}{}{extra}", yaml_check("'true'"));
    let (_tmp, out) = run_config(&config, &["--list", "--files", "a.rs"]);
    assert_code(&out, exit::CONFIG_ERROR);
    let stderr = String::from_utf8_lossy(&out.stderr);
    assert!(stderr.contains(msg), "{stderr}");
}

#[test]
fn invalid_files_filter_regex_exits_2() {
    let config = docker_yaml("        triggers:\n          files_filter: '[invalid'\n");
    let (_tmp, out) = run_config(&config, &["--list", "--files", "a.rs"]);
    assert_code(&out, exit::CONFIG_ERROR);
    let stderr = String::from_utf8_lossy(&out.stderr);
    assert!(stderr.contains("`files_filter`"), "{stderr}");
}

#[test]
fn validate_invalid_config_exits_2() {
    let (_tmp, out) = run_config("checks: [not, a, map", &["validate"]);
    assert_code(&out, exit::CONFIG_ERROR);
}

#[test]
fn validate_valid_config_exits_0() {
    let (_tmp, out) = run_config(&local_yaml("'true'"), &["validate"]);
    assert_code(&out, exit::SUCCESS);
}

/// Exit 3 with `error` on stderr, nothing blaming the base ref: not the
/// "could not resolve" message, neither hint
fn assert_git_error(out: &Output, error: &str) {
    assert_code(out, exit::ENV_ERROR);
    let stderr = stderr(out);
    assert!(stderr.contains(error), "{stderr}");
    for blame in ["could not resolve", "No base ref", "exists locally"] {
        assert!(!stderr.contains(blame), "{stderr}");
    }
}

/// Git repo on `branch` with the config (base `main`, fallback `HEAD~1`) and
/// no commits
fn new_repo(branch: &str) -> tempfile::TempDir {
    let tmp = tempfile::TempDir::new().unwrap();
    git(tmp.path(), &["init", "-q", "-b", branch]);
    write(tmp.path(), "ci-tui.yaml", &local_yaml("'true'"));
    tmp
}

/// Commit everything in `dir` plus new `file`
fn commit_file(dir: &Path, file: &str) {
    write(dir, file, "change\n");
    git(dir, &["add", "."]);
    commit(dir, file);
}

/// Repo on `other` with two commits, sharing no history with `main`'s one
fn unrelated_repo() -> tempfile::TempDir {
    let tmp = new_repo("main");
    let dir = tmp.path();
    commit_file(dir, "main.txt");
    git(dir, &["checkout", "-q", "--orphan", "other"]);
    commit_file(dir, "first.txt");
    commit_file(dir, "second.txt");
    tmp
}

#[test]
fn git_detection_outside_repo_exits_3() {
    let (_tmp, out) = run_config(&local_yaml("'true'"), &["--list"]);
    assert_git_error(
        &out,
        "Error: git command failed: fatal: not a git repository",
    );
}

#[test]
fn missing_git_exits_3() {
    let env = [("PATH", "")];
    let (_tmp, out) = run_config_env(&local_yaml("'true'"), &["--list"], &env);
    assert_git_error(&out, "Error: Failed to run git command");
}

#[test]
fn missing_base_refs_fall_back_to_head_parent() {
    let tmp = new_repo("trunk");
    commit_file(tmp.path(), "first.txt");
    commit_file(tmp.path(), "second.txt");

    let out = run_ci_tui(tmp.path(), &["--list"]);

    assert_code(&out, exit::SUCCESS);
    let stdout = stdout(&out);
    assert!(stdout.contains("Base ref: HEAD~1 (fallback:"), "{stdout}");
}

#[test]
fn no_existing_base_ref_exits_3_with_hint() {
    let tmp = new_repo("trunk");
    let out = run_ci_tui(tmp.path(), &["--list"]);
    assert_code(&out, exit::ENV_ERROR);
    let stderr = stderr(&out);
    assert!(stderr.contains("could not resolve"), "{stderr}");
    assert!(
        stderr.contains("(tried: origin/main, main, HEAD~1)"),
        "{stderr}"
    );
    assert!(stderr.contains("No base ref could be resolved"), "{stderr}");
}

#[test]
fn no_merge_base_exits_3_without_fallback() {
    let tmp = unrelated_repo();
    let out = run_ci_tui(tmp.path(), &["--list"]);
    assert_git_error(&out, "no merge base");
}

#[test]
fn base_ref_at_missing_object_exits_3_without_fallback() {
    let tmp = new_repo("trunk");
    commit_file(tmp.path(), "first.txt");
    commit_file(tmp.path(), "second.txt");
    write(
        tmp.path(),
        ".git/refs/remotes/origin/main",
        "1234567890123456789012345678901234567890\n",
    );

    let out = run_ci_tui(tmp.path(), &["--list"]);

    assert_git_error(&out, "bad object");
}

#[test]
fn base_text_search_ref_resolves() {
    let tmp = new_repo("trunk");
    commit_file(tmp.path(), "first.txt");
    commit_file(tmp.path(), "second.txt");

    let out = run_ci_tui(tmp.path(), &["--list", "--base", ":/first.txt"]);

    assert_code(&out, exit::SUCCESS);
}

#[test]
fn shallow_clone_without_merge_base_exits_3() {
    let source = new_repo("main");
    let dir = source.path();
    commit_file(dir, "base.txt");
    git(dir, &["checkout", "-q", "-b", "feature"]);
    commit_file(dir, "feature.txt");
    git(dir, &["checkout", "-q", "main"]);
    commit_file(dir, "main.txt");

    let clone = tempfile::TempDir::new().unwrap();
    let url = format!("file://{}", dir.display());
    let args = [
        "clone",
        "-q",
        "--depth",
        "1",
        "--no-single-branch",
        "--branch",
        "feature",
        url.as_str(),
        ".",
    ];
    git(clone.path(), &args);

    let out = run_ci_tui(clone.path(), &["--list"]);
    assert_git_error(&out, "no merge base");
}

#[test]
fn unknown_base_ref_exits_3() {
    let tmp = tempfile::TempDir::new().unwrap();
    git(tmp.path(), &["init", "-q"]);
    std::fs::write(tmp.path().join("ci-tui.yaml"), local_yaml("'true'")).unwrap();
    let out = run_ci_tui(tmp.path(), &["--list", "--base", "no-such-ref"]);
    assert_code(&out, exit::ENV_ERROR);
    let stderr = stderr(&out);
    assert!(stderr.contains("could not resolve"), "{stderr}");
    assert!(stderr.contains("no-such-ref"), "{stderr}");
    assert!(
        stderr.contains("Check that `no-such-ref` exists locally"),
        "{stderr}"
    );
}

#[test]
fn base_without_merge_base_exits_3_without_hint() {
    let tmp = unrelated_repo();
    let out = run_ci_tui(tmp.path(), &["--list", "--base", "main"]);
    assert_git_error(&out, "no merge base");
}

#[test]
fn list_in_docker_mode_does_not_probe_docker() {
    // No daemon needed: --list executes nothing
    let (_tmp, out) = run_config(&docker_yaml(""), &["--list", "--files", "a.rs"]);
    assert_code(&out, exit::SUCCESS);
}

#[test]
fn only_on_demand_checks_do_not_probe_docker() {
    // `rust`-triggered check, no `.rs` changed: selected on-demand only
    let config = docker_yaml("        triggers:\n          file_pattern: rust\n").replace(
        "file_patterns: {}",
        "file_patterns:\n  rust:\n    pattern: '\\.rs$'",
    );
    let (_tmp, out) = run_config(&config, &["--simple", "--files", "README.md"]);
    assert_code(&out, exit::SUCCESS);
}

#[test]
fn simple_never_runs_an_on_demand_check() {
    // #267: `on_demand: true` without triggers would always run; its
    // failing command must not decide the exit code
    let config = format!("{}        on_demand: true\n", local_yaml("'false'"));
    let (_tmp, out) = run_config(&config, &["--simple", "--files", "a.rs"]);
    assert_code(&out, exit::SUCCESS);
}

#[test]
fn fix_without_fix_commands_does_not_probe_docker() {
    let (_tmp, out) = run_config(&docker_yaml(""), &["--fix", "--files", "a.rs"]);
    assert_code(&out, exit::SUCCESS);
}

/// Local config whose check `c` fails even after its (passing) fix
fn still_failing_after_fix() -> String {
    format!("{}        fix_command: 'true'\n", local_yaml("'false'"))
}

#[test]
fn fix_exits_1_when_verification_fails() {
    let (_tmp, out) = run_config(&still_failing_after_fix(), &["--fix", "--files", "a.rs"]);
    assert_code(&out, exit::CHECKS_FAILED);
    let stdout = String::from_utf8_lossy(&out.stdout);
    assert!(stdout.contains("still failing"), "{stdout}");
}

#[test]
fn fix_with_no_verify_exits_0() {
    let args = ["--fix", "--no-verify", "--files", "a.rs"];
    let (_tmp, out) = run_config(&still_failing_after_fix(), &args);
    assert_code(&out, exit::SUCCESS);
}

#[test]
fn init_over_existing_config_exits_2() {
    let (_tmp, out) = run_config(&local_yaml("'true'"), &["init"]);
    assert_code(&out, exit::CONFIG_ERROR);
    let stderr = String::from_utf8_lossy(&out.stderr);
    assert!(stderr.contains("already exists"), "{stderr}");
}
