//! `--watch` (TUI): saving files re-runs the checks they affect.
//!
//! A `notify` watcher on each dir of the repo worth watching ([`walk`]: not
//! gitignored, not `.git/` / `.ci-tui/`, not a nested repo; dirs created
//! later are added as they appear) feeds a thread that debounces bursts of
//! saves into one batch ([`next_batch`]), keeps the repo-relative paths
//! worth checking ([`keep_paths`]: also not editor temp files, not matched
//! by `ignore_patterns`, not gitignored) and sends the checks they select
//! to run automatically ([`affected_checks`]) with the saved paths
//! ([`WatchBatch`]) to the TUI, which re-runs them like `r`.

use crate::checks::{select_checks, CheckToRun, Decision};
use crate::config::CiConfig;
use crate::git::{ChangedFiles, CLI_FILES_BASE_REF};
use crate::test_discovery::run_with_timeout;
use anyhow::{Context, Result};
use ignore::{DirEntry, WalkBuilder};
use notify::event::{EventKind, ModifyKind};
use notify::{RecommendedWatcher, RecursiveMode, Watcher};
use std::collections::HashSet;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::mpsc::Receiver;
use std::sync::{Arc, Mutex, PoisonError, Weak};
use std::time::{Duration, Instant};
use tokio::sync::mpsc::UnboundedSender;

/// Quiet window closing a batch: saves within it re-run checks once
const DEBOUNCE: Duration = Duration::from_millis(300);
/// Longest a batch waits for quiet, so constant writes elsewhere cannot
/// hold back saves
const MAX_BATCH_WAIT: Duration = Duration::from_secs(2);
/// Repo-relative dirs never watched for saves: git's own writes and
/// ci-tui's (result cache under `.git/`, logs) would re-trigger checks
const ALWAYS_IGNORED: [&str; 2] = [".git/", ".ci-tui/"];
/// Longest `git check-ignore` may take; the batch is dropped after it
const GIT_TIMEOUT: Duration = Duration::from_secs(10);

/// Running watcher; dropping it stops watching (and its batch thread)
pub struct Watch {
    _watcher: Arc<Mutex<RecommendedWatcher>>,
}

/// Checks a batch of saves selects, for the TUI
#[derive(Debug)]
pub struct WatchBatch {
    pub checks: Vec<CheckToRun>,
    /// The batch's kept saved files, repo-relative (sorted, deduped): their
    /// content tells a check's own writes apart
    /// ([`crate::ui::app::App::queue_watch`])
    pub saved: Vec<String>,
}

/// Watched tree: the repo root, else the cwd
struct Root {
    /// Canonical path
    path: PathBuf,
    /// `path` is a git repo: gitignored paths are neither watched nor checked
    git: bool,
}

/// Watch the repo containing `cwd` (else `cwd`), sending each batch's
/// [`affected_checks`] to `tx`. Errors when the watcher cannot start (e.g.
/// inotify watch limit reached).
pub fn start(
    cwd: &Path,
    config: Arc<CiConfig>,
    exec_root: PathBuf,
    tx: UnboundedSender<WatchBatch>,
) -> Result<Watch> {
    let (path, git) = match crate::git::repo_root(cwd) {
        Ok(root) => (root, true),
        Err(_) => (cwd.to_path_buf(), false),
    };
    // Real path: watchers may report events under it (macOS FSEvents)
    let path = path.canonicalize().unwrap_or(path);
    let root = Root { path, git };
    let (event_tx, event_rx) = std::sync::mpsc::channel();
    let mut watcher = notify::recommended_watcher(move |event: notify::Result<notify::Event>| {
        // Watch errors (e.g. a vanished dir) only lose those events
        if let Some(event) = event.ok().filter(|e| is_save(&e.kind)) {
            event.paths.into_iter().for_each(|p| drop(event_tx.send(p)));
        }
    })
    .context("--watch: failed to create file watcher")?;
    let failed = || format!("--watch: failed to watch {}", root.path.display());
    watcher
        .watch(&root.path, RecursiveMode::NonRecursive)
        .with_context(failed)?;
    let (dirs, _) = walk(&root, &root.path);
    add_watches(&mut watcher, &dirs).with_context(failed)?;
    let watcher = Arc::new(Mutex::new(watcher));
    let weak = Arc::downgrade(&watcher);
    std::thread::Builder::new()
        .name("watch".to_string())
        .spawn(move || watch_loop(&event_rx, &root, &weak, &config, &exec_root, &tx))
        .context("--watch: failed to start watch thread")?;
    Ok(Watch { _watcher: watcher })
}

/// Batch thread: until the watcher or the TUI is gone, watch new dirs and
/// send each batch's affected checks
fn watch_loop(
    events: &Receiver<PathBuf>,
    root: &Root,
    watcher: &Weak<Mutex<RecommendedWatcher>>,
    config: &CiConfig,
    exec_root: &Path,
    tx: &UnboundedSender<WatchBatch>,
) {
    while let Some(mut batch) = next_batch(events, DEBOUNCE, MAX_BATCH_WAIT) {
        let Some(watcher) = watcher.upgrade() else {
            return;
        };
        let found = watch_new_dirs(&watcher, root, &batch);
        drop(watcher);
        batch.extend(found);
        let saved = keep_paths(root, batch, config);
        let checks = affected_checks(config, saved.clone(), exec_root);
        if !checks.is_empty() && tx.send(WatchBatch { checks, saved }).is_err() {
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

/// Dirs under `dir` (itself included) worth watching, and the files in
/// them. Below it, a dir is skipped (with its subtree) when gitignored (in
/// a repo; nested `.gitignore`s count), `.git/` / `.ci-tui/`, or a nested
/// repo / submodule ([`watchable`]).
fn walk(root: &Root, dir: &Path) -> (Vec<PathBuf>, Vec<PathBuf>) {
    let base = root.path.clone();
    let (dirs, files): (Vec<DirEntry>, Vec<DirEntry>) = WalkBuilder::new(dir)
        .hidden(false)
        .ignore(false)
        .parents(root.git)
        .git_ignore(root.git)
        .git_exclude(root.git)
        .git_global(root.git)
        .filter_entry(move |entry| !is_dir(entry) || watchable(&base, entry.path()))
        .build()
        // Unreadable entries only lose their saves
        .filter_map(Result::ok)
        .partition(is_dir);
    let paths = |entries: Vec<DirEntry>| entries.into_iter().map(DirEntry::into_path).collect();
    (paths(dirs), paths(files))
}

fn is_dir(entry: &DirEntry) -> bool {
    entry.file_type().is_some_and(|t| t.is_dir())
}

/// Dir `path` below `base` may be watched: not `.git/` / `.ci-tui/`, not a
/// nested repo or submodule (it has a `.git` entry)
fn watchable(base: &Path, path: &Path) -> bool {
    let Some(rel) = path
        .strip_prefix(base)
        .ok()
        .filter(|r| !r.as_os_str().is_empty())
    else {
        return false;
    };
    let rel = format!("{}/", rel.to_string_lossy());
    !ALWAYS_IGNORED.iter().any(|d| rel.starts_with(d)) && !path.join(".git").exists()
}

/// Watch each of `dirs` non-recursively; one gone meanwhile is skipped
fn add_watches(watcher: &mut RecommendedWatcher, dirs: &[PathBuf]) -> notify::Result<()> {
    let mut paths = watcher.paths_mut();
    for dir in dirs {
        match paths.add(dir, RecursiveMode::NonRecursive) {
            Err(e) if matches!(e.kind, notify::ErrorKind::PathNotFound) => {}
            result => result?,
        }
    }
    paths.commit()
}

/// Watch the dirs `batch` created (and their subdirs) like the initial
/// ones ([`walk`]); returns the files already in them, as saved when the
/// dir appeared: they were written before the watch (`mkdir -p` + write,
/// checkout)
fn watch_new_dirs(
    watcher: &Mutex<RecommendedWatcher>,
    root: &Root,
    batch: &[PathBuf],
) -> Vec<PathBuf> {
    let mut seen = HashSet::new();
    let mut found = Vec::new();
    for dir in batch {
        let new = dir.is_dir() && seen.insert(dir) && watchable(&root.path, dir);
        if !new || dir_gitignored(root, dir) {
            continue;
        }
        let (dirs, files) = walk(root, dir);
        let mut watcher = watcher.lock().unwrap_or_else(PoisonError::into_inner);
        // A failed watch (watch limit) only loses saves there
        let _ = add_watches(&mut watcher, &dirs);
        found.extend(files);
    }
    found
}

/// Dir `dir` is gitignored; when git fails it is not (watched: its saved
/// files still go through [`keep_paths`])
fn dir_gitignored(root: &Root, dir: &Path) -> bool {
    let Ok(rel) = dir.strip_prefix(&root.path) else {
        return false;
    };
    let rel = format!("{}/", rel.to_string_lossy());
    gitignored(root, &[rel]).is_some_and(|ignored| !ignored.is_empty())
}

/// Next batch from `rx`: blocks for a first item, then takes items until
/// none came for `quiet` or `max_wait` passed since the first (also while
/// items keep coming). `None` once `rx` is disconnected and drained.
fn next_batch<T>(rx: &Receiver<T>, quiet: Duration, max_wait: Duration) -> Option<Vec<T>> {
    let mut batch = vec![rx.recv().ok()?];
    let deadline = Instant::now() + max_wait;
    let left = || {
        deadline
            .checked_duration_since(Instant::now())
            .filter(|d| !d.is_zero())
    };
    while let Some(left) = left() {
        match rx.recv_timeout(quiet.min(left)) {
            Ok(item) => batch.push(item),
            Err(_) => break,
        }
    }
    Some(batch)
}

/// Saved `paths` as sorted, deduped repo-relative files worth checking:
/// under `root`, not in `.git/` / `.ci-tui/`, not editor temp files, not
/// matched by `ignore_patterns`, not directories, and not gitignored (none
/// when git fails in a repo: better a lost batch than re-running on
/// ignored files). Cheap filters first: a build can save thousands of
/// paths per batch.
fn keep_paths(root: &Root, paths: Vec<PathBuf>, config: &CiConfig) -> Vec<String> {
    let mut files: Vec<String> = paths
        .into_iter()
        .filter_map(|path| {
            Some(
                path.strip_prefix(&root.path)
                    .ok()?
                    .to_string_lossy()
                    .into_owned(),
            )
        })
        .filter(|file| !file.is_empty() && !ALWAYS_IGNORED.iter().any(|d| file.starts_with(d)))
        .filter(|file| !is_editor_temp(file))
        .collect();
    files.sort();
    files.dedup();
    files.retain(|file| !config.should_ignore_file(file) && !root.path.join(file).is_dir());
    if files.is_empty() {
        return files;
    }
    let Some(ignored) = gitignored(root, &files) else {
        return Vec::new();
    };
    files.retain(|file| !ignored.contains(file));
    files
}

/// Editor temp file written beside a save (by file name): vim's `4913`
/// probe, swap files (`*.swp`, `*.swx`), backups (`*~`), emacs lock (`.#*`)
/// and auto-save (`#*#`) files
fn is_editor_temp(file: &str) -> bool {
    let name = file.rsplit('/').next().unwrap_or(file);
    let autosave = name.len() > 1 && name.starts_with('#') && name.ends_with('#');
    name == "4913"
        || name.ends_with(".swp")
        || name.ends_with(".swx")
        || name.ends_with('~')
        || name.starts_with(".#")
        || autosave
}

/// `files` (repo-relative; `dir/` for a dir) that git ignores in the repo
/// at `root`; tracked files never are. None outside a repo. `None` when git
/// fails (exit other than 0 / 1) or times out.
fn gitignored(root: &Root, files: &[String]) -> Option<HashSet<String>> {
    if !root.git {
        return Some(HashSet::new());
    }
    let mut command = Command::new("git");
    crate::utils::own_process_group(&mut command)
        .args(["check-ignore", "--stdin", "-z"])
        .current_dir(&root.path);
    let input: String = files.iter().map(|file| format!("{file}\0")).collect();
    let output = run_with_timeout(command, &input, GIT_TIMEOUT).ok()?;
    // Exit 1 = none ignored; output is empty then
    if !matches!(output.code, Some(0 | 1)) {
        return None;
    }
    let ignored = output.stdout.split('\0').filter(|path| !path.is_empty());
    Some(ignored.map(str::to_string).collect())
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

    /// Watched tree at `path`; `git`: a repo
    fn root(path: &Path, git: bool) -> Root {
        Root {
            path: path.to_path_buf(),
            git,
        }
    }

    /// `keep_paths` of `paths` under a fake root (no git: nothing gitignored)
    fn kept(paths: &[&str]) -> Vec<String> {
        let root = root(Path::new("/nonexistent-ci-tui-watch"), false);
        let paths = paths.iter().map(|p| root.path.join(p)).collect();
        keep_paths(&root, paths, &config())
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

    /// Saves queued faster than the batch drains them (a build) still end
    /// the batch at `max_wait`, not once drained
    #[test]
    fn test_flood_of_saves_flushes_at_max_wait() {
        const QUEUED: usize = 1_000_000;
        let (tx, rx) = mpsc::channel();
        (0..QUEUED).for_each(|i| tx.send(i).unwrap());

        let started = Instant::now();
        let batch = next_batch(&rx, Duration::from_secs(10), Duration::from_millis(1)).unwrap();

        assert!(batch.len() < QUEUED, "stopped at max wait, not drained");
        assert!(started.elapsed() < Duration::from_secs(5));
        drop(tx);
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
        let root = root(Path::new("/nonexistent-ci-tui-watch"), false);
        let paths = vec![PathBuf::from("/elsewhere/a.php"), root.path.join("a.php")];
        assert_eq!(keep_paths(&root, paths, &config()), ["a.php"]);
    }

    #[rstest::rstest]
    #[case::git_dir(".git/index")]
    #[case::git_cache(".git/ci-tui/results.json")]
    #[case::ci_tui_logs(".ci-tui/logs/lint.log")]
    #[case::ci_tui_gitignore(".ci-tui/.gitignore")]
    fn test_keep_paths_drops_git_and_ci_tui_dirs(#[case] path: &str) {
        assert!(kept(&[path]).is_empty(), "{path} kept");
    }

    #[rstest::rstest]
    #[case::vim_probe("src/4913")]
    #[case::vim_swap("src/.A.php.swp")]
    #[case::vim_swap_x("src/.A.php.swx")]
    #[case::backup("src/A.php~")]
    #[case::emacs_lock("src/.#A.php")]
    #[case::emacs_autosave("src/#A.php#")]
    fn test_keep_paths_drops_editor_temp_files(#[case] path: &str) {
        assert_eq!(kept(&[path, "src/A.php"]), ["src/A.php"], "{path} kept");
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
        let root = root(dir.path(), false);
        assert_eq!(keep_paths(&root, paths, &config()), ["src/A.php"]);
    }

    /// Git env vars set by hooks (e.g. pre-commit) that redirect git to the
    /// outer repo instead of the test's tempdir repo
    const OUTER_GIT_ENV: [&str; 4] = ["GIT_DIR", "GIT_INDEX_FILE", "GIT_WORK_TREE", "GIT_PREFIX"];

    /// Running inside a git hook: tempdir repos would see the outer one
    fn in_git_hook() -> bool {
        let set = OUTER_GIT_ENV.iter().any(|v| std::env::var_os(v).is_some());
        if set {
            eprintln!("skipped: outer git env set (running inside a git hook)");
        }
        set
    }

    /// Tempdir git repo with `.gitignore` `gitignore`
    fn repo(gitignore: &str) -> tempfile::TempDir {
        let repo = tempfile::tempdir().unwrap();
        let status = Command::new("git")
            .args(["init", "-q"])
            .current_dir(repo.path())
            .status()
            .expect("git must be installed");
        assert!(status.success(), "git init failed");
        std::fs::write(repo.path().join(".gitignore"), gitignore).unwrap();
        repo
    }

    #[test]
    fn test_gitignored_reads_gitignore_not_tracked_files() {
        if in_git_hook() {
            return;
        }
        let repo = repo("build/\n*.log\n");
        std::fs::write(repo.path().join("tracked.log"), "").unwrap();
        let status = Command::new("git")
            .args(["add", "-f", "tracked.log"])
            .current_dir(repo.path())
            .status()
            .unwrap();
        assert!(status.success());
        let files = ["build/out.php", "a.log", "tracked.log", "src/A.php"].map(String::from);
        let root = root(repo.path(), true);

        let ignored = gitignored(&root, &files);

        let expected = HashSet::from(["build/out.php".to_string(), "a.log".to_string()]);
        assert_eq!(ignored, Some(expected));
        let paths = files.iter().map(|f| repo.path().join(f)).collect();
        assert_eq!(
            keep_paths(&root, paths, &config()),
            ["src/A.php", "tracked.log"]
        );
    }

    /// Outside a repo nothing is gitignored, `.gitignore` or not
    #[test]
    fn test_gitignored_outside_repo_is_empty() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(dir.path().join(".gitignore"), "*.php\n").unwrap();
        let root = root(dir.path(), false);

        assert_eq!(
            gitignored(&root, &["a.php".to_string()]),
            Some(HashSet::new())
        );
        let paths = vec![dir.path().join("a.php")];
        assert_eq!(keep_paths(&root, paths, &config()), ["a.php"]);
    }

    /// `git check-ignore` failing in a repo (here: the repo is gone) drops
    /// the batch instead of keeping gitignored files
    #[test]
    fn test_keep_paths_drops_batch_when_git_fails_in_repo() {
        if in_git_hook() {
            return;
        }
        let dir = tempfile::tempdir().unwrap(); // not a repo: git exits 128
        let root = root(dir.path(), true);

        assert_eq!(gitignored(&root, &["a.php".to_string()]), None);
        let paths = vec![dir.path().join("a.php")];
        assert!(keep_paths(&root, paths, &config()).is_empty());
    }

    /// Repo-relative dirs [`walk`] finds under `root`
    fn walked(root: &Root) -> Vec<String> {
        let (dirs, _) = walk(root, &root.path);
        let mut dirs: Vec<String> = dirs
            .iter()
            .map(|d| {
                d.strip_prefix(&root.path)
                    .unwrap()
                    .to_string_lossy()
                    .into_owned()
            })
            .collect();
        dirs.sort();
        dirs
    }

    /// Dirs `src/lib`, `target/debug` (gitignored), `.ci-tui/logs`, `sub`
    /// (nested repo) and `docs` (gitignored in `src/.gitignore`) in `dir`
    fn make_tree(dir: &Path) {
        for d in [
            "src/lib",
            "src/docs",
            "target/debug",
            ".ci-tui/logs",
            "sub/.git",
        ] {
            std::fs::create_dir_all(dir.join(d)).unwrap();
        }
        std::fs::write(dir.join("src/.gitignore"), "docs/\n").unwrap();
    }

    #[test]
    fn test_walk_skips_gitignored_git_ci_tui_and_nested_repos() {
        if in_git_hook() {
            return;
        }
        let repo = repo("target/\n");
        make_tree(repo.path());

        assert_eq!(walked(&root(repo.path(), true)), ["", "src", "src/lib"]);
    }

    /// Outside a repo `.gitignore` does not apply
    #[test]
    fn test_walk_outside_repo_skips_only_ci_tui_and_nested_repos() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(dir.path().join(".gitignore"), "target/\n").unwrap();
        make_tree(dir.path());

        let expected = ["", "src", "src/docs", "src/lib", "target", "target/debug"];
        assert_eq!(walked(&root(dir.path(), false)), expected);
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

    /// Next batch from `rx` (within 10 s)
    fn recv_batch(rx: &mut tokio::sync::mpsc::UnboundedReceiver<WatchBatch>) -> WatchBatch {
        let deadline = Instant::now() + Duration::from_secs(10);
        loop {
            if let Ok(batch) = rx.try_recv() {
                return batch;
            }
            assert!(Instant::now() < deadline, "no batch sent");
            std::thread::sleep(Duration::from_millis(20));
        }
    }

    /// Files of the batch's only check
    fn batch_files(batch: &WatchBatch) -> Vec<String> {
        assert_eq!(ids(&batch.checks), ["lint"]);
        batch.checks[0].files.paths().to_vec()
    }

    #[test]
    fn test_start_sends_affected_checks_for_a_save() {
        let dir = tempfile::tempdir().unwrap();
        let (tx, mut rx) = tokio::sync::mpsc::unbounded_channel();
        let root = dir.path().to_path_buf();
        let _watch = start(&root, Arc::new(config()), root.clone(), tx).unwrap();

        std::fs::write(dir.path().join("A.php"), "<?php").unwrap();

        let batch = recv_batch(&mut rx);
        assert_eq!(batch_files(&batch), ["A.php"]);
        assert_eq!(batch.saved, ["A.php"]);
    }

    /// A dir created after start is watched, its subdirs too; files written
    /// before its watch existed (`mkdir -p` + write) count as saved
    #[test]
    fn test_start_watches_new_dirs() {
        let dir = tempfile::tempdir().unwrap();
        let (tx, mut rx) = tokio::sync::mpsc::unbounded_channel();
        let root = dir.path().to_path_buf();
        let _watch = start(&root, Arc::new(config()), root.clone(), tx).unwrap();

        std::fs::create_dir_all(root.join("a/b")).unwrap();
        std::fs::write(root.join("a/b/A.php"), "<?php").unwrap();
        assert_eq!(batch_files(&recv_batch(&mut rx)), ["a/b/A.php"]);

        std::fs::write(root.join("a/b/B.php"), "<?php").unwrap();
        assert_eq!(batch_files(&recv_batch(&mut rx)), ["a/b/B.php"]);
    }

    /// Saves in a gitignored dir (not watched) send nothing
    #[test]
    fn test_start_ignores_saves_in_gitignored_dir() {
        if in_git_hook() {
            return;
        }
        let repo = repo("target/\n");
        std::fs::create_dir_all(repo.path().join("target")).unwrap();
        let (tx, mut rx) = tokio::sync::mpsc::unbounded_channel();
        let root = repo.path().to_path_buf();
        let _watch = start(&root, Arc::new(config()), root.clone(), tx).unwrap();

        std::fs::write(root.join("target/A.php"), "<?php").unwrap();
        std::thread::sleep(DEBOUNCE * 3);
        std::fs::write(root.join("B.php"), "<?php").unwrap();

        assert_eq!(batch_files(&recv_batch(&mut rx)), ["B.php"]);
    }
}
