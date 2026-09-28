//! End-to-end result cache (#150): a check whose inputs are unchanged since
//! its last passing run is skipped and shown as cached (`--simple` mode).

use ci_tui::exit;

mod common;
use common::{git, run_ci_tui};

// Edge case: the binary reads a YAML file from disk, so configs stay raw YAML.
/// Local-mode config: `lint` (triggered by `.rs` files) appends to
/// `runs.log` and fails while a `fail` file exists; `always` has no triggers.
const CONFIG: &str = r#"version: 2
runner: local
local:
  shell: sh
git:
  base_branch: main
  fallback_branch: HEAD~1
file_patterns:
  rust:
    pattern: '\.rs$'
  rust_src:
    pattern: '^src/.*\.rs$'
checks:
  g:
    checks:
      lint:
        name: Lint
        command: "echo lint >> runs.log; cat {files} > /dev/null; test ! -e fail"
        fix_command: "echo fix >> runs.log"
        triggers:
          file_pattern: rust
      always:
        name: Always
        command: "echo always >> runs.log"
"#;

// Edge case: test discovery maps `src/{path}.rs` -> `tests/{path}_test.rs`.
/// `unit` runs only the discovered test files, never the source file itself
const DISCOVERY_CONFIG: &str = r#"version: 2
runner: local
local:
  shell: sh
git:
  base_branch: main
  fallback_branch: HEAD~1
file_patterns:
  rust_src:
    pattern: '^src/.*\.rs$'
checks:
  g:
    checks:
      unit:
        name: Unit
        command: "echo unit >> runs.log; cat {files} > /dev/null"
        triggers:
          test_discovery:
            source_pattern: rust_src
            strategies:
              - type: path_mapping
                rules:
                  - source: "src/{path}.rs"
                    tests:
                      - "tests/{path}_test.rs"
"#;

/// Tempdir with `ci-tui.yaml` = `config` and `a.rs`; a git repo if `git_init`
fn project(config: &str, git_init: bool) -> tempfile::TempDir {
    let tmp = tempfile::TempDir::new().unwrap();
    if git_init {
        git(tmp.path(), &["init", "-q"]);
    }
    write(tmp.path(), "ci-tui.yaml", config);
    write(tmp.path(), "a.rs", "fn a() {}\n");
    tmp
}

fn write(dir: &std::path::Path, path: &str, content: &str) {
    let path = dir.join(path);
    std::fs::create_dir_all(path.parent().unwrap()).unwrap();
    std::fs::write(path, content).unwrap();
}

/// `--simple` run on `files` (default `a.rs`) plus `extra` args
fn run(dir: &std::path::Path, extra: &[&str]) -> std::process::Output {
    let mut args = vec!["--simple"];
    if !extra.contains(&"--files") {
        args.extend(["--files", "a.rs"]);
    }
    args.extend(extra);
    run_ci_tui(dir, &args)
}

/// How often check `id` executed (lines it appended to `runs.log`)
fn runs(dir: &std::path::Path, id: &str) -> usize {
    std::fs::read_to_string(dir.join("runs.log"))
        .unwrap_or_default()
        .lines()
        .filter(|line| *line == id)
        .count()
}

fn stdout(out: &std::process::Output) -> String {
    String::from_utf8_lossy(&out.stdout).into_owned()
}

fn assert_code(out: &std::process::Output, code: i32) {
    assert_eq!(
        out.status.code(),
        Some(code),
        "stdout: {}\nstderr: {}",
        stdout(out),
        String::from_utf8_lossy(&out.stderr)
    );
}

#[test]
fn unchanged_check_is_cached_on_second_run() {
    let tmp = project(CONFIG, true);
    assert_code(&run(tmp.path(), &[]), exit::SUCCESS);
    assert!(tmp.path().join(".git/ci-tui/results.json").is_file());

    let out = run(tmp.path(), &[]);
    assert_code(&out, exit::SUCCESS);
    assert_eq!(runs(tmp.path(), "lint"), 1, "second run served from cache");
    let lint_line = stdout(&out)
        .lines()
        .find(|l| l.contains("lint"))
        .map(str::to_owned)
        .unwrap_or_default();
    assert!(
        lint_line.contains("✓") && lint_line.contains("cached"),
        "{lint_line:?}"
    );
    assert!(
        stdout(&out).contains("All 2 checks passed"),
        "{}",
        stdout(&out)
    );
}

#[test]
fn always_run_check_is_never_cached() {
    let tmp = project(CONFIG, true);
    run(tmp.path(), &[]);
    run(tmp.path(), &[]);
    assert_eq!(runs(tmp.path(), "always"), 2);
}

#[test]
fn editing_a_matched_file_reruns() {
    let tmp = project(CONFIG, true);
    run(tmp.path(), &[]);
    write(tmp.path(), "a.rs", "fn a() { changed() }\n");
    run(tmp.path(), &[]);
    assert_eq!(runs(tmp.path(), "lint"), 2);
}

#[test]
fn different_matched_files_rerun() {
    let tmp = project(CONFIG, true);
    write(tmp.path(), "b.rs", "fn b() {}\n");
    run(tmp.path(), &[]);
    run(tmp.path(), &["--files", "a.rs", "b.rs"]);
    assert_eq!(runs(tmp.path(), "lint"), 2);
}

#[test]
fn config_change_reruns() {
    let tmp = project(CONFIG, true);
    run(tmp.path(), &[]);
    write(tmp.path(), "ci-tui.yaml", &format!("{CONFIG}# edited\n"));
    run(tmp.path(), &[]);
    assert_eq!(runs(tmp.path(), "lint"), 2);
}

#[test]
fn editing_only_the_discovery_source_reruns() {
    let tmp = project(DISCOVERY_CONFIG, true);
    write(tmp.path(), "src/a.rs", "fn a() {}\n");
    write(tmp.path(), "tests/a_test.rs", "fn t() {}\n");
    let files = ["--files", "src/a.rs"];
    run(tmp.path(), &files);
    run(tmp.path(), &files);
    assert_eq!(runs(tmp.path(), "unit"), 1, "unchanged: cached");

    write(tmp.path(), "src/a.rs", "fn a() { changed() }\n");
    run(tmp.path(), &files);
    assert_eq!(runs(tmp.path(), "unit"), 2, "source edited: rerun");
}

#[test]
fn no_cache_flag_runs_every_check_but_still_records_passes() {
    let tmp = project(CONFIG, true);
    run(tmp.path(), &["--no-cache"]);
    let out = run(tmp.path(), &["--no-cache"]);
    assert_code(&out, exit::SUCCESS);
    assert!(!stdout(&out).contains("cached"), "{}", stdout(&out));
    assert_eq!(runs(tmp.path(), "lint"), 2);

    run(tmp.path(), &[]);
    assert_eq!(
        runs(tmp.path(), "lint"),
        2,
        "pass recorded under --no-cache"
    );
}

#[test]
fn failed_run_is_not_cached() {
    let tmp = project(CONFIG, true);
    write(tmp.path(), "fail", "");
    assert_code(&run(tmp.path(), &[]), exit::CHECKS_FAILED);
    assert_code(&run(tmp.path(), &[]), exit::CHECKS_FAILED);
    assert_eq!(runs(tmp.path(), "lint"), 2);
}

#[test]
fn failure_after_cached_pass_invalidates_entry() {
    let tmp = project(CONFIG, true);
    run(tmp.path(), &[]);
    write(tmp.path(), "fail", "");
    assert_code(&run(tmp.path(), &["--no-cache"]), exit::CHECKS_FAILED);
    std::fs::remove_file(tmp.path().join("fail")).unwrap();

    assert_code(&run(tmp.path(), &[]), exit::SUCCESS);
    assert_eq!(runs(tmp.path(), "lint"), 3, "entry dropped by the failure");
}

#[test]
fn outside_git_repo_runs_without_cache() {
    let tmp = project(CONFIG, false);
    assert_code(&run(tmp.path(), &[]), exit::SUCCESS);
    let out = run(tmp.path(), &[]);
    assert_code(&out, exit::SUCCESS);
    assert_eq!(runs(tmp.path(), "lint"), 2);
}

#[test]
fn corrupt_cache_file_is_ignored_and_replaced() {
    let tmp = project(CONFIG, true);
    write(tmp.path(), ".git/ci-tui/results.json", "{not json");
    assert_code(&run(tmp.path(), &[]), exit::SUCCESS);
    assert_code(&run(tmp.path(), &[]), exit::SUCCESS);
    assert_eq!(runs(tmp.path(), "lint"), 1);
}

#[test]
fn unwritable_cache_dir_does_not_fail_the_run() {
    let tmp = project(CONFIG, true);
    // A file where the cache dir should be: every write fails
    write(tmp.path(), ".git/ci-tui", "");
    let out = run(tmp.path(), &[]);
    assert_code(&out, exit::SUCCESS);
    assert_eq!(runs(tmp.path(), "lint"), 1);
}

#[test]
fn linked_worktree_caches_in_its_git_dir() {
    let outer = tempfile::TempDir::new().unwrap();
    let main = outer.path().join("main");
    std::fs::create_dir(&main).unwrap();
    git(&main, &["init", "-q"]);
    write(&main, "ci-tui.yaml", CONFIG);
    write(&main, "a.rs", "fn a() {}\n");
    git(&main, &["add", "."]);
    let identity = ["-c", "user.name=t", "-c", "user.email=t@t"];
    git(
        &main,
        &[&identity[..], &["commit", "-q", "-m", "init"]].concat(),
    );
    git(&main, &["worktree", "add", "-q", "../wt"]);

    let wt = outer.path().join("wt");
    run(&wt, &[]);
    run(&wt, &[]);
    assert_eq!(runs(&wt, "lint"), 1);
    assert!(main.join(".git/worktrees/wt/ci-tui/results.json").is_file());
}

#[test]
fn fix_mode_ignores_cache() {
    let tmp = project(CONFIG, true);
    run(tmp.path(), &[]);
    let out = run_ci_tui(tmp.path(), &["--fix", "--files", "a.rs"]);
    assert_code(&out, exit::SUCCESS);
    assert_eq!(runs(tmp.path(), "fix"), 1);
}
