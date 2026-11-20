//! Changed files of a config in a repo subdirectory (#340): git lists them
//! relative to the execution root. Docker mode (exec root = the config's
//! dir): only files under it, project-relative. Local mode (exec root = the
//! repo root): the whole repo, repo-relative.

use ci_tui::cache::ResultCache;
use ci_tui::checks::select_checks;
use ci_tui::config::load_config;
use ci_tui::exit;
use ci_tui::git::{detect_changes, ChangedFiles};
use ci_tui::runner::{CheckResult, CheckStatus};
use rstest::rstest;
use std::path::{Path, PathBuf};

mod common;
use common::{assert_code, commit, git, in_git_hook, run_in, stderr, stdout, write};

// Edge case: docker-mode config in `svc/` matching project-relative sources
const DOCKER_CONFIG: &str = r#"version: 2
docker: { project_dir: ., service: app, shell: sh }
git: { base_branch: main, fallback_branch: HEAD~1 }
file_patterns:
  rs: { pattern: '^src/.*\.rs$' }
checks:
  g:
    checks:
      c: { name: C, command: "lint {files}", triggers: { file_pattern: rs } }
"#;

// Edge case: local-mode config in `svc/` matching every Rust file
const LOCAL_CONFIG: &str = r#"version: 2
runner: local
local: { shell: sh }
git: { base_branch: main, fallback_branch: HEAD~1 }
file_patterns:
  rs: { pattern: '\.rs$' }
checks:
  g:
    checks:
      c: { name: C, command: "lint {files}", triggers: { file_pattern: rs } }
"#;

/// `diff.relative=true` in the user's git config
const DIFF_RELATIVE: [(&str, &str); 3] = [
    ("GIT_CONFIG_COUNT", "1"),
    ("GIT_CONFIG_KEY_0", "diff.relative"),
    ("GIT_CONFIG_VALUE_0", "true"),
];

/// `<tmp>/repo` on branch `feature` off `main`, `svc/ci-tui.yaml` = `config`.
/// Committed on `feature`: `svc/src/a.rs`, `other/c.rs`; uncommitted edits:
/// `svc/src/b.rs`, `root.rs`; untracked: `svc/src/new.rs`, `other/new.rs`
fn svc_repo(config: &str) -> tempfile::TempDir {
    let tmp = tempfile::TempDir::new().unwrap();
    let repo = tmp.path().join("repo");
    std::fs::create_dir(&repo).unwrap();
    git(&repo, &["init", "-q", "-b", "main"]);
    write(&repo, "svc/ci-tui.yaml", config);
    for file in ["svc/src/b.rs", "other/o.rs", "root.rs"] {
        write(&repo, file, "base\n");
    }
    git(&repo, &["add", "."]);
    commit(&repo, "base");
    git(&repo, &["checkout", "-q", "-b", "feature"]);
    write(&repo, "svc/src/a.rs", "a\n");
    write(&repo, "other/c.rs", "c\n");
    git(&repo, &["add", "."]);
    commit(&repo, "feature");
    write(&repo, "svc/src/b.rs", "edited\n");
    write(&repo, "root.rs", "edited\n");
    write(&repo, "svc/src/new.rs", "new\n");
    write(&repo, "other/new.rs", "new\n");
    tmp
}

/// `<tmp>/repo/<dir>`
fn dir(tmp: &tempfile::TempDir, dir: &str) -> PathBuf {
    tmp.path().join("repo").join(dir)
}

/// `--list` stdout of a run in `<tmp>/repo/<cwd>`, asserting exit 0
fn list(tmp: &tempfile::TempDir, cwd: &str, extra: &[&str], env: &[(&str, &str)]) -> String {
    let out = run_in(tmp, cwd, &[&["--list"], extra].concat(), env);
    assert_code(&out, exit::SUCCESS);
    stdout(&out)
}

/// The `--list` block naming `files` as the changed files
fn changed_block(files: &[&str]) -> String {
    let lines: String = files.iter().map(|f| format!("  {f}\n")).collect();
    format!("Changed files ({}):\n{lines}", files.len())
}

#[rstest]
#[case::config_dir("svc", &[])]
#[case::below_config_dir("svc/src", &[])]
#[case::diff_relative_config("svc", &DIFF_RELATIVE)]
fn docker_subdir_lists_project_relative_files_under_it(
    #[case] cwd: &str,
    #[case] env: &[(&str, &str)],
) {
    let tmp = svc_repo(DOCKER_CONFIG);

    let out = list(&tmp, cwd, &[], env);

    let files = ["src/a.rs", "src/b.rs", "src/new.rs"];
    assert!(out.contains(&changed_block(&files)), "{out}");
    assert!(
        out.contains("file_pattern `rs` matched: src/a.rs, src/b.rs, src/new.rs"),
        "{out}"
    );
    for outside in ["other/", "root.rs", "svc/"] {
        assert!(!out.contains(outside), "{outside} listed: {out}");
    }
}

#[test]
fn docker_subdir_with_changes_only_outside_lists_none() {
    let tmp = svc_repo(DOCKER_CONFIG);
    let repo = dir(&tmp, "");
    git(&repo, &["checkout", "-q", "--", "svc"]);
    std::fs::remove_file(repo.join("svc/src/new.rs")).unwrap();

    // Against HEAD: nothing committed; `root.rs`, `other/new.rs` outside svc
    let out = list(&tmp, "svc", &["--base", "HEAD"], &[]);

    assert!(out.contains("Changed files (0): none\n"), "{out}");
}

#[test]
fn docker_subdir_files_placeholder_gets_project_relative_paths() {
    if in_git_hook() {
        return;
    }
    let tmp = svc_repo(DOCKER_CONFIG);
    let svc = dir(&tmp, "svc");
    let config = load_config(&svc.join("ci-tui.yaml")).unwrap();

    let changed = detect_changes(&svc, &config.git).unwrap();
    let checks = select_checks(&config, &changed, &svc, false).checks;

    assert_eq!(checks.len(), 1);
    assert_eq!(
        checks[0].resolved_command,
        "lint src/a.rs src/b.rs src/new.rs"
    );
}

#[test]
fn docker_subdir_staged_lists_project_relative_files_under_it() {
    let tmp = svc_repo(DOCKER_CONFIG);
    let repo = dir(&tmp, "");
    write(&repo, "svc/src/gone.rs", "gone\n");
    git(
        &repo,
        &["add", "svc/src/b.rs", "svc/src/gone.rs", "root.rs"],
    );
    std::fs::remove_file(repo.join("svc/src/gone.rs")).unwrap();

    let out = list(&tmp, "svc", &["--staged"], &[]);

    assert!(out.contains(&changed_block(&["src/b.rs"])), "{out}");
}

#[rstest]
#[case::plain(&[])]
#[case::diff_relative_config(&DIFF_RELATIVE)]
fn local_subdir_lists_repo_wide_repo_relative_files(#[case] env: &[(&str, &str)]) {
    let tmp = svc_repo(LOCAL_CONFIG);

    let out = list(&tmp, "svc", &[], env);

    let files = [
        "other/c.rs",
        "svc/src/a.rs",
        "root.rs",
        "svc/src/b.rs",
        "other/new.rs",
        "svc/src/new.rs",
    ];
    assert!(out.contains(&changed_block(&files)), "{out}");
}

#[test]
fn local_subdir_staged_lists_repo_relative_files() {
    let tmp = svc_repo(LOCAL_CONFIG);
    git(&dir(&tmp, ""), &["add", "svc/src/b.rs", "root.rs"]);

    let out = list(&tmp, "svc", &["--staged"], &[]);

    assert!(
        out.contains(&changed_block(&["root.rs", "svc/src/b.rs"])),
        "{out}"
    );
}

#[test]
fn docker_subdir_missing_base_keeps_hint_and_exit_code() {
    let tmp = svc_repo(DOCKER_CONFIG);

    let out = run_in(&tmp, "svc", &["--list", "--base", "nosuch"], &[]);

    assert_code(&out, exit::ENV_ERROR);
    assert!(
        stderr(&out).contains("Check that `nosuch` exists locally"),
        "{}",
        stderr(&out)
    );
}

/// Record a pass of `svc`'s only check over its project-relative changed
/// files, keyed under `svc` (the exec root) like a run there
fn record_pass(svc: &Path) {
    let config = load_config(&svc.join("ci-tui.yaml")).unwrap();
    let changed = ChangedFiles {
        files: ["src/a.rs", "src/b.rs", "src/new.rs"]
            .map(String::from)
            .to_vec(),
        base_ref: "main".to_string(),
    };
    let checks = select_checks(&config, &changed, svc, true).checks;
    assert!(checks[0].cache_key.is_some(), "files resolve under svc");
    let passed = CheckResult {
        status: CheckStatus::Passed,
        ..CheckResult::pending("c")
    };
    ResultCache::open(svc, true)
        .record(&checks[0], &passed)
        .unwrap();
}

#[test]
fn docker_subdir_cache_keys_read_files_under_the_project_dir() {
    if in_git_hook() {
        return;
    }
    let tmp = svc_repo(DOCKER_CONFIG);
    record_pass(&dir(&tmp, "svc"));

    let out = list(&tmp, "svc", &[], &[]);

    assert!(out.contains("  cached     c  C\n"), "{out}");
}
