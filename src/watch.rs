//! `--watch` (TUI): saving files re-runs the checks they affect.
//!
//! A `notify` watcher on the repo root feeds a thread that debounces bursts
//! of saves into one batch ([`next_batch`]), keeps the repo-relative paths
//! worth checking ([`keep_paths`]: not `.git/` / `.ci-tui/`, not matched by
//! `ignore_patterns`, not gitignored) and sends the checks they select to
//! run automatically ([`affected_checks`]) to the TUI, which re-runs them
//! like `r`.

use crate::checks::{select_checks, CheckToRun, Decision};
use crate::config::CiConfig;
use crate::git::{ChangedFiles, CLI_FILES_BASE_REF};
use crate::test_discovery::run_with_timeout;
use anyhow::{Context, Result};
use notify::event::{EventKind, ModifyKind};
use notify::{RecursiveMode, Watcher};
use std::collections::HashSet;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::mpsc::{Receiver, RecvTimeoutError};
use std::sync::Arc;
use std::time::{Duration, Instant};
use tokio::sync::mpsc::UnboundedSender;

/// Quiet window closing a batch: saves within it re-run checks once
const DEBOUNCE: Duration = Duration::from_millis(300);
/// Longest a batch waits for quiet, so constant writes elsewhere (a build
/// into a gitignored dir) cannot hold back saves
const MAX_BATCH_WAIT: Duration = Duration::from_secs(2);
/// Repo-relative dirs never watched for saves: git's own writes and
/// ci-tui's (result cache under `.git/`, logs) would re-trigger checks
const ALWAYS_IGNORED: [&str; 2] = [".git/", ".ci-tui/"];
/// Longest `git check-ignore` may take before the batch counts as not ignored
const GIT_TIMEOUT: Duration = Duration::from_secs(10);

/// Running watcher; dropping it stops watching (and its batch thread)
pub struct Watch {
    _watcher: notify::RecommendedWatcher,
}

/// Watch the repo containing `cwd` (else `cwd`) recursively, sending each
/// batch's [`affected_checks`] to `tx`. Errors when the watcher cannot
/// start (e.g. inotify watch limit reached).
pub fn start(
    cwd: &Path,
    config: Arc<CiConfig>,
    exec_root: PathBuf,
    tx: UnboundedSender<Vec<CheckToRun>>,
) -> Result<Watch> {
    let root = crate::git::repo_root(cwd).unwrap_or_else(|_| cwd.to_path_buf());
    // Real path: watchers may report events under it (macOS FSEvents)
    let root = root.canonicalize().unwrap_or(root);
    let (path_tx, path_rx) = std::sync::mpsc::channel();
    let mut watcher = notify::recommended_watcher(move |event: notify::Result<notify::Event>| {
        // Watch errors (e.g. a vanished dir) only lose those events
        if let Some(event) = event.ok().filter(|e| is_save(&e.kind)) {
            event.paths.into_iter().for_each(|p| drop(path_tx.send(p)));
        }
    })
    .context("--watch: failed to create file watcher")?;
    watcher
        .watch(&root, RecursiveMode::Recursive)
        .with_context(|| format!("--watch: failed to watch {}", root.display()))?;
    std::thread::Builder::new()
        .name("watch".to_string())
        .spawn(move || watch_loop(&path_rx, &root, &config, &exec_root, &tx))
        .context("--watch: failed to start watch thread")?;
    Ok(Watch { _watcher: watcher })
}

/// Batch thread: until the watcher or the TUI is gone, send each batch's
/// affected checks
fn watch_loop(
    paths: &Receiver<PathBuf>,
    root: &Path,
    config: &CiConfig,
    exec_root: &Path,
    tx: &UnboundedSender<Vec<CheckToRun>>,
) {
    while let Some(batch) = next_batch(paths, DEBOUNCE, MAX_BATCH_WAIT) {
        let files = keep_paths(root, batch, config);
        let checks = affected_checks(config, files, exec_root);
        if !checks.is_empty() && tx.send(checks).is_err() {
            return;
        }
    }
}

/// A file content / name change: create, modify (not metadata only), remove
fn is_save(kind: &EventKind) -> bool {
    match kind {
        EventKind::Modify(ModifyKind::Metadata(_)) => false,
        EventKind::Create(_) | EventKind::Modify(_) | EventKind::Remove(_) => true,
        _ => false,
    }
}

/// Next batch from `rx`: blocks for a first item, then takes items until
/// none came for `quiet` (or `max_wait` passed since the first). `None`
/// once `rx` is disconnected and drained.
fn next_batch<T>(rx: &Receiver<T>, quiet: Duration, max_wait: Duration) -> Option<Vec<T>> {
    let mut batch = vec![rx.recv().ok()?];
    let deadline = Instant::now() + max_wait;
    loop {
        let wait = quiet.min(deadline.saturating_duration_since(Instant::now()));
        match rx.recv_timeout(wait) {
            Ok(item) => batch.push(item),
            Err(RecvTimeoutError::Timeout | RecvTimeoutError::Disconnected) => return Some(batch),
        }
    }
}

/// Saved `paths` as sorted, deduped repo-relative files worth checking:
/// under `root`, not in `.git/` / `.ci-tui/`, not matched by
/// `ignore_patterns`, not directories, and not gitignored. Cheap filters
/// first: a build can save thousands of paths per batch.
fn keep_paths(root: &Path, paths: Vec<PathBuf>, config: &CiConfig) -> Vec<String> {
    let mut files: Vec<String> = paths
        .into_iter()
        .filter_map(|path| Some(path.strip_prefix(root).ok()?.to_string_lossy().into_owned()))
        .filter(|file| !file.is_empty() && !ALWAYS_IGNORED.iter().any(|d| file.starts_with(d)))
        .collect();
    files.sort();
    files.dedup();
    files.retain(|file| !config.should_ignore_file(file) && !root.join(file).is_dir());
    if files.is_empty() {
        return files;
    }
    let ignored = gitignored(root, &files);
    files.retain(|file| !ignored.contains(file));
    files
}

/// `files` (repo-relative) that git ignores in the repo at `root`; tracked
/// files never are. Empty when git fails (e.g. not a repo) or times out.
fn gitignored(root: &Path, files: &[String]) -> HashSet<String> {
    let mut command = Command::new("git");
    crate::utils::own_process_group(&mut command)
        .args(["check-ignore", "--stdin", "-z"])
        .current_dir(root);
    let input: String = files.iter().map(|file| format!("{file}\0")).collect();
    // Exit 1 = none ignored; output is empty then
    let Ok(output) = run_with_timeout(command, &input, GIT_TIMEOUT) else {
        return HashSet::new();
    };
    output
        .stdout
        .split('\0')
        .filter(|path| !path.is_empty())
        .map(str::to_string)
        .collect()
}

/// Checks the saved `files` select to run automatically (not on-demand or
/// skipped); none for no files. Blocking: test discovery reads files.
pub fn affected_checks(config: &CiConfig, files: Vec<String>, exec_root: &Path) -> Vec<CheckToRun> {
    if files.is_empty() {
        return Vec::new();
    }
    let saved = ChangedFiles {
        files,
        base_ref: CLI_FILES_BASE_REF.to_string(),
    };
    let selected = select_checks(config, &saved, exec_root, None);
    let runs = |check: &CheckToRun| check.decision() == Decision::Run;
    selected.checks.into_iter().filter(runs).collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::mpsc;

    // Edge case: `lint` runs on PHP files, `yaml` on YAML files; vendor/ ignored
    fn config() -> CiConfig {
        serde_yaml::from_str(
            r#"
version: 2
runner: local
git: { base_branch: main, fallback_branch: HEAD~1 }
ignore_patterns: ['^vendor/']
file_patterns:
  php: { pattern: '\.php$' }
  yaml: { pattern: '\.ya?ml$' }
checks:
  fast:
    checks:
      lint: { name: Lint, command: "lint {files}", triggers: { file_pattern: php } }
      yaml: { name: Yaml, command: "yaml {files}", triggers: { file_pattern: yaml } }
"#,
        )
        .unwrap()
    }

    /// `keep_paths` of `paths` under a fake root (no git: nothing gitignored)
    fn kept(paths: &[&str]) -> Vec<String> {
        let root = Path::new("/nonexistent-ci-tui-watch");
        let paths = paths.iter().map(|p| root.join(p)).collect();
        keep_paths(root, paths, &config())
    }

    #[test]
    fn test_burst_of_saves_is_one_batch() {
        let (tx, rx) = mpsc::channel();
        let (done_tx, done_rx) = mpsc::channel::<()>();
        let sender = std::thread::spawn(move || {
            for i in 0..5 {
                tx.send(i).unwrap();
                std::thread::sleep(Duration::from_millis(20));
            }
            // Still connected: the batch must end on the quiet window
            let _ = done_rx.recv();
        });
        let quiet = Duration::from_millis(200);

        let batch = next_batch(&rx, quiet, Duration::from_secs(10));

        assert_eq!(batch, Some(vec![0, 1, 2, 3, 4]));
        drop(done_tx);
        sender.join().unwrap();
        assert_eq!(next_batch(&rx, quiet, Duration::from_secs(10)), None);
    }

    #[test]
    fn test_saves_after_quiet_window_are_next_batch() {
        let (tx, rx) = mpsc::channel();
        let quiet = Duration::from_millis(50);
        tx.send(1).unwrap();
        let sender = std::thread::spawn(move || {
            std::thread::sleep(Duration::from_millis(300));
            tx.send(2).unwrap();
        });

        assert_eq!(
            next_batch(&rx, quiet, Duration::from_secs(10)),
            Some(vec![1])
        );
        assert_eq!(
            next_batch(&rx, quiet, Duration::from_secs(10)),
            Some(vec![2])
        );
        sender.join().unwrap();
    }

    #[test]
    fn test_constant_saves_flush_at_max_wait() {
        let (tx, rx) = mpsc::channel();
        let stop = Arc::new(std::sync::atomic::AtomicBool::new(false));
        let stopped = Arc::clone(&stop);
        let sender = std::thread::spawn(move || {
            while !stopped.load(std::sync::atomic::Ordering::Relaxed) {
                let _ = tx.send(0);
                std::thread::sleep(Duration::from_millis(10));
            }
        });

        let started = Instant::now();
        let batch = next_batch(&rx, Duration::from_millis(100), Duration::from_millis(300));

        assert!(batch.is_some_and(|b| !b.is_empty()));
        assert!(
            started.elapsed() < Duration::from_secs(2),
            "flushed at max wait"
        );
        stop.store(true, std::sync::atomic::Ordering::Relaxed);
        sender.join().unwrap();
    }

    #[test]
    fn test_keep_paths_repo_relative_sorted_deduped() {
        assert_eq!(
            kept(&["src/B.php", "src/A.php", "src/B.php"]),
            ["src/A.php", "src/B.php"]
        );
    }

    #[test]
    fn test_keep_paths_drops_paths_outside_root() {
        let root = Path::new("/nonexistent-ci-tui-watch");
        let paths = vec![PathBuf::from("/elsewhere/a.php"), root.join("a.php")];
        assert_eq!(keep_paths(root, paths, &config()), ["a.php"]);
    }

    #[rstest::rstest]
    #[case::git_dir(".git/index")]
    #[case::git_cache(".git/ci-tui/results.json")]
    #[case::ci_tui_logs(".ci-tui/logs/lint.log")]
    #[case::ci_tui_gitignore(".ci-tui/.gitignore")]
    fn test_keep_paths_drops_git_and_ci_tui_dirs(#[case] path: &str) {
        assert!(kept(&[path]).is_empty(), "{path} kept");
    }

    #[test]
    fn test_keep_paths_keeps_dotfiles_like_git_dirs() {
        assert_eq!(
            kept(&[".gitignore", ".ci-tui.yaml"]),
            [".ci-tui.yaml", ".gitignore"]
        );
    }

    #[test]
    fn test_keep_paths_drops_ignore_patterns() {
        assert_eq!(kept(&["vendor/lib/A.php", "src/A.php"]), ["src/A.php"]);
    }

    #[test]
    fn test_keep_paths_drops_directories() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::create_dir(dir.path().join("src")).unwrap();
        let paths = vec![dir.path().join("src"), dir.path().join("src/A.php")];
        assert_eq!(keep_paths(dir.path(), paths, &config()), ["src/A.php"]);
    }

    /// Git env vars set by hooks (e.g. pre-commit) that redirect git to the
    /// outer repo instead of the test's tempdir repo
    const OUTER_GIT_ENV: [&str; 4] = ["GIT_DIR", "GIT_INDEX_FILE", "GIT_WORK_TREE", "GIT_PREFIX"];

    #[test]
    fn test_gitignored_reads_gitignore_not_tracked_files() {
        if OUTER_GIT_ENV.iter().any(|v| std::env::var_os(v).is_some()) {
            eprintln!("skipped: outer git env set (running inside a git hook)");
            return;
        }
        let repo = tempfile::tempdir().unwrap();
        let git = |args: &[&str]| {
            let status = Command::new("git")
                .args(args)
                .current_dir(repo.path())
                .status()
                .expect("git must be installed");
            assert!(status.success(), "git {args:?} failed");
        };
        git(&["init", "-q"]);
        std::fs::write(repo.path().join(".gitignore"), "build/\n*.log\n").unwrap();
        std::fs::write(repo.path().join("tracked.log"), "").unwrap();
        git(&["add", "-f", "tracked.log"]);
        let files = ["build/out.php", "a.log", "tracked.log", "src/A.php"].map(String::from);

        let ignored = gitignored(repo.path(), &files);

        let expected = HashSet::from(["build/out.php".to_string(), "a.log".to_string()]);
        assert_eq!(ignored, expected);
        let paths = files.iter().map(|f| repo.path().join(f)).collect();
        assert_eq!(
            keep_paths(repo.path(), paths, &config()),
            ["src/A.php", "tracked.log"]
        );
    }

    #[test]
    fn test_gitignored_outside_repo_is_empty() {
        let dir = tempfile::tempdir().unwrap();
        assert!(gitignored(dir.path(), &["a.php".to_string()]).is_empty());
    }

    fn ids(checks: &[CheckToRun]) -> Vec<&str> {
        checks.iter().map(CheckToRun::id).collect()
    }

    #[test]
    fn test_affected_checks_only_those_the_saves_select() {
        let dir = tempfile::tempdir().unwrap();
        let files = vec!["src/A.php".to_string()];

        let checks = affected_checks(&config(), files, dir.path());

        assert_eq!(ids(&checks), ["lint"], "yaml not triggered");
        assert_eq!(checks[0].files.paths(), ["src/A.php"]);
    }

    #[test]
    fn test_affected_checks_none_without_files() {
        let dir = tempfile::tempdir().unwrap();
        assert!(affected_checks(&config(), Vec::new(), dir.path()).is_empty());
    }

    #[test]
    fn test_start_fails_on_missing_root_with_env_exit_code() {
        let (tx, _rx) = tokio::sync::mpsc::unbounded_channel();
        let missing = Path::new("/nonexistent-ci-tui-watch");

        let err = start(missing, Arc::new(config()), missing.to_path_buf(), tx)
            .err()
            .expect("watching a missing dir fails");

        assert!(format!("{err:#}").contains("--watch"), "{err:#}");
        assert_eq!(crate::exit::code_for(&err), crate::exit::ENV_ERROR);
    }

    #[test]
    fn test_start_sends_affected_checks_for_a_save() {
        let dir = tempfile::tempdir().unwrap();
        let (tx, mut rx) = tokio::sync::mpsc::unbounded_channel();
        let root = dir.path().to_path_buf();
        let _watch = start(&root, Arc::new(config()), root.clone(), tx).unwrap();

        std::fs::write(dir.path().join("A.php"), "<?php").unwrap();

        let deadline = Instant::now() + Duration::from_secs(10);
        let checks = loop {
            if let Ok(checks) = rx.try_recv() {
                break checks;
            }
            assert!(Instant::now() < deadline, "no batch sent");
            std::thread::sleep(Duration::from_millis(20));
        };
        assert_eq!(ids(&checks), ["lint"]);
    }
}
