//! Result cache: skip checks whose inputs are unchanged since their last
//! passing run (shown as "cached ✓"; `--no-cache` turns reads off).
//!
//! Key per check ([`stamp_keys`], run by `select_checks`): check id +
//! resolved command + content of its matched files (plus the changed source
//! files behind its test discovery, so editing only the source re-runs its
//! tests) + hash of the config file text. Files are read under the key root,
//! where changed-file paths resolve (repo root for git paths, execution root
//! for `--files`). Checks without concrete files (always-run, run-all,
//! on-demand, skipped) or with an unreadable file get no key and are never
//! cached. A pass is stored only if its key still matches when it finishes
//! (files edited during the run are not cached).
//!
//! Only these inputs count: edits elsewhere (`Cargo.toml`, unchanged files
//! the changed ones use, a new base ref, the docker image, env) do not
//! invalidate a pass. `--no-cache`, or `r` in the TUI, runs it anyway.
//!
//! Store ([`ResultCache`]): `<git dir>/ci-tui/results.json` maps check id to
//! the key of its latest pass. A pass overwrites the entry, a failure removes
//! it, so the store holds at most one entry per check (config validation
//! keeps ids unique). Cache problems (not a git repo, unreadable / corrupt
//! file, write error) never fail a run: they only turn caching off.
//!
//! Hashing uses std's `DefaultHasher` (SipHash with fixed keys, stable across
//! runs): no extra dependency. A toolchain changing the algorithm only costs
//! one cache miss per check; 64 bits make a false hit negligible.

use crate::checks::{match_file_pattern, CheckToRun};
use crate::config::CiConfig;
use crate::git::ChangedFiles;
use crate::runner::{CheckResult, CheckStatus};
use std::borrow::Borrow;
use std::collections::HashMap;
use std::hash::{DefaultHasher, Hash, Hasher};
use std::io::{self, Write};
use std::path::{Path, PathBuf};

/// Cache directory inside the git dir
const CACHE_DIR: &str = "ci-tui";
/// Cache file inside [`CACHE_DIR`]
const CACHE_FILE: &str = "results.json";

/// Result cache key of a check ([`CheckToRun::cache_key`]), with what it
/// takes to recompute it when the run finishes
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CacheKey {
    /// Hash of all inputs; stored for a pass
    value: u64,
    /// Changed test-discovery sources hashed besides the check's files
    sources: Vec<String>,
    /// Config file hash the value covers
    config_hash: u64,
}

/// Stable hash of `bytes`
pub(crate) fn hash_bytes(bytes: &[u8]) -> u64 {
    let mut hasher = DefaultHasher::new();
    bytes.hash(&mut hasher);
    hasher.finish()
}

/// Set every check's [`CheckToRun::cache_key`] from the current contents of
/// its files under `root` (each file read once). Reads files: blocking.
pub(crate) fn stamp_keys(
    checks: &mut [CheckToRun],
    config: &CiConfig,
    changed_files: &ChangedFiles,
    root: &Path,
) {
    let mut hashes: HashMap<String, Option<Option<u64>>> = HashMap::new();
    let mut hashed = |path: &str| {
        *hashes
            .entry(path.to_string())
            .or_insert_with(|| file_hash(root, path))
    };
    for check in checks {
        let sources = discovery_sources(config, changed_files, check);
        let config_hash = config.source_hash;
        check.cache_key =
            key_value(check, &sources, config_hash, &mut hashed).map(|value| CacheKey {
                value,
                sources,
                config_hash,
            });
    }
}

/// Changed files matching the check's `test_discovery.source_pattern`
fn discovery_sources(
    config: &CiConfig,
    changed_files: &ChangedFiles,
    check: &CheckToRun,
) -> Vec<String> {
    let Some(discovery) = check
        .definition
        .triggers
        .as_ref()
        .and_then(|t| t.test_discovery.as_ref())
    else {
        return Vec::new();
    };
    match_file_pattern(config, changed_files, &discovery.source_pattern)
        .into_iter()
        .map(String::from)
        .collect()
}

/// Content hash of `path` under `root`: `Some(None)` when missing
/// (deleted), `None` when unreadable (directory, permissions)
fn file_hash(root: &Path, path: &str) -> Option<Option<u64>> {
    match std::fs::read(root.join(path)) {
        Ok(bytes) => Some(Some(hash_bytes(&bytes))),
        Err(e) if e.kind() == io::ErrorKind::NotFound => Some(None),
        Err(_) => None,
    }
}

/// Key value of `check` with `file_hash` per input file; `None` without
/// concrete files or with an unreadable one
fn key_value(
    check: &CheckToRun,
    sources: &[String],
    config_hash: u64,
    mut file_hash: impl FnMut(&str) -> Option<Option<u64>>,
) -> Option<u64> {
    let files = check.files.paths();
    if files.is_empty() {
        return None;
    }
    let mut hasher = DefaultHasher::new();
    (check.id(), &check.resolved_command, config_hash).hash(&mut hasher);
    for path in files.iter().chain(sources) {
        (path, file_hash(path)?).hash(&mut hasher);
    }
    Some(hasher.finish())
}

/// Key of the latest passing run per check id, persisted in the git dir.
#[derive(Debug, Default)]
pub struct ResultCache {
    /// Cache file; `None` = off (not a git repo, or a write failed)
    path: Option<PathBuf>,
    /// Where key files are read (see [`Self::key_root`]); `None` = off
    root: Option<PathBuf>,
    /// Serve unchanged checks from the cache (`false` with `--no-cache`)
    read: bool,
    /// Check id -> key of its latest pass
    entries: HashMap<String, u64>,
}

impl ResultCache {
    /// A cache that keys, serves and records nothing (fix mode, tests)
    pub fn disabled() -> Self {
        Self::default()
    }

    /// Cache in the git dir of the repo containing `cwd` (a linked worktree
    /// has its own), reading key files under `root`; disabled outside a
    /// repo. `read = false` still records.
    pub fn open(cwd: &Path, root: PathBuf, read: bool) -> Self {
        match crate::git::git_dir(cwd) {
            Ok(dir) => Self::at(dir.join(CACHE_DIR).join(CACHE_FILE), root, read),
            Err(_) => Self::disabled(),
        }
    }

    /// Cache stored at `path`. A missing, unreadable or corrupt file starts
    /// empty (overwritten on the next change).
    pub fn at(path: PathBuf, root: PathBuf, read: bool) -> Self {
        let entries = std::fs::read(&path)
            .ok()
            .and_then(|bytes| serde_json::from_slice(&bytes).ok())
            .unwrap_or_default();
        Self {
            path: Some(path),
            root: Some(root),
            read,
            entries,
        }
    }

    /// Root the changed-file paths resolve against, for `select_checks` to
    /// key checks; `None` when disabled (no keys, nothing cached)
    pub fn key_root(&self) -> Option<&Path> {
        self.root.as_deref()
    }

    /// True when `check` passed last time with the same key: serve it as
    /// [`CheckResult::cached`] instead of running it
    pub fn is_fresh(&self, check: &CheckToRun) -> bool {
        self.read
            && check
                .cache_key
                .as_ref()
                .is_some_and(|key| self.entries.get(check.id()) == Some(&key.value))
    }

    /// Split `checks` into (fresh: serve as cached, the rest: run)
    pub fn split_fresh<C: Borrow<CheckToRun>>(
        &self,
        checks: impl IntoIterator<Item = C>,
    ) -> (Vec<C>, Vec<C>) {
        checks.into_iter().partition(|c| self.is_fresh(c.borrow()))
    }

    /// [`Self::record`] for the check in `checks` that produced `result`
    pub fn record_for(&mut self, checks: &[CheckToRun], result: &CheckResult) -> io::Result<()> {
        match checks.iter().find(|c| c.id() == result.check_id) {
            Some(check) => self.record(check, result),
            None => Ok(()),
        }
    }

    /// Record a finished run of `check`: a pass stores its key (replacing the
    /// previous one) if its files still hash to it, a failure drops the
    /// entry; cached / cancelled results change nothing. Saves on change. A
    /// write error is returned once, then the cache is off for the rest of
    /// the run.
    pub fn record(&mut self, check: &CheckToRun, result: &CheckResult) -> io::Result<()> {
        let Some(path) = &self.path else {
            return Ok(());
        };
        if result.cached {
            return Ok(());
        }
        let id = check.id();
        let changed = match &check.cache_key {
            Some(key) if result.status == CheckStatus::Passed => {
                self.still_matches(check, key)
                    && self.entries.insert(id.to_string(), key.value) != Some(key.value)
            }
            _ if result.status.is_failure() => self.entries.remove(id).is_some(),
            _ => false,
        };
        if !changed {
            return Ok(());
        }
        let written = write_atomic(path, &serde_json::to_vec(&self.entries)?);
        if written.is_err() {
            self.path = None;
        }
        written
    }

    /// True when `check`'s files still hash to `key` (not edited since it
    /// was stamped, e.g. while the check ran)
    fn still_matches(&self, check: &CheckToRun, key: &CacheKey) -> bool {
        let Some(root) = &self.root else {
            return false;
        };
        let now = key_value(check, &key.sources, key.config_hash, |p| file_hash(root, p));
        now == Some(key.value)
    }
}

/// Write via a unique temp file + rename: readers (another ci-tui run in the
/// same worktree) never see a partial file
fn write_atomic(path: &Path, bytes: &[u8]) -> io::Result<()> {
    let dir = path.parent().unwrap_or(Path::new("."));
    std::fs::create_dir_all(dir)?;
    let mut tmp = tempfile::NamedTempFile::new_in(dir)?;
    tmp.write_all(bytes)?;
    tmp.persist(path)?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::checks::CheckFiles;
    use crate::config::CheckDefinition;
    use std::collections::HashMap;

    // Inline fixtures: library tests cannot import tests/common (see CLAUDE.md)
    fn check(id: &str, command: &str, files: CheckFiles) -> CheckToRun {
        CheckToRun {
            id: id.into(),
            group: "g".into(),
            definition: CheckDefinition {
                name: id.into(),
                command: command.into(),
                service: None,
                container: None,
                fix_command: None,
                triggers: None,
                on_demand: false,
                env: HashMap::new(),
                timeout: None,
                error_pattern: None,
            },
            service: None,
            files,
            resolved_command: command.into(),
            resolved_fix_command: None,
            cache_key: None,
        }
    }

    fn files(paths: &[&str]) -> CheckFiles {
        CheckFiles::Files(paths.iter().map(|p| p.to_string()).collect())
    }

    /// Tempdir root holding `a.rs`
    fn root() -> tempfile::TempDir {
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(dir.path().join("a.rs"), "fn a() {}").unwrap();
        dir
    }

    /// Key value of `c` with `sources` and config hash `config`, files under `root`
    fn key(c: &CheckToRun, sources: &[&str], config: u64, root: &Path) -> Option<u64> {
        let sources: Vec<String> = sources.iter().map(|s| s.to_string()).collect();
        key_value(c, &sources, config, |p| file_hash(root, p))
    }

    fn keyed(id: &str, command: &str, root: &Path) -> CheckToRun {
        let mut c = check(id, command, files(&["a.rs"]));
        c.cache_key = key(&c, &[], 1, root).map(|value| CacheKey {
            value,
            sources: vec![],
            config_hash: 1,
        });
        c
    }

    fn cache_at(path: PathBuf, root: &Path) -> ResultCache {
        ResultCache::at(path, root.to_path_buf(), true)
    }

    fn result(id: &str, status: CheckStatus) -> CheckResult {
        CheckResult {
            status,
            ..CheckResult::pending(id)
        }
    }

    #[test]
    fn key_is_stable_for_identical_inputs() {
        let dir = root();
        let c = check("lint", "lint a.rs", files(&["a.rs"]));
        assert!(key(&c, &[], 1, dir.path()).is_some());
        assert_eq!(key(&c, &[], 1, dir.path()), key(&c, &[], 1, dir.path()));
    }

    #[test]
    fn key_changes_with_each_component() {
        let dir = root();
        let base = check("lint", "lint a.rs", files(&["a.rs"]));
        let k = |c: &CheckToRun, sources: &[&str], config: u64| key(c, sources, config, dir.path());
        let original = k(&base, &[], 1);

        let other_id = check("other", "lint a.rs", files(&["a.rs"]));
        assert_ne!(k(&other_id, &[], 1), original, "check id");
        let other_cmd = check("lint", "lint --strict a.rs", files(&["a.rs"]));
        assert_ne!(k(&other_cmd, &[], 1), original, "resolved command");
        assert_ne!(k(&base, &[], 2), original, "config hash");

        std::fs::write(dir.path().join("src.rs"), "fn s() {}").unwrap();
        let with_source = k(&base, &["src.rs"], 1);
        assert_ne!(with_source, original, "discovery source added");
        std::fs::write(dir.path().join("src.rs"), "fn s() { changed() }").unwrap();
        assert_ne!(k(&base, &["src.rs"], 1), with_source, "source content");

        std::fs::write(dir.path().join("a.rs"), "fn a() { changed() }").unwrap();
        assert_ne!(k(&base, &[], 1), original, "matched file content");
        std::fs::remove_file(dir.path().join("a.rs")).unwrap();
        let deleted = k(&base, &[], 1);
        assert!(deleted.is_some(), "a deleted file still keys");
        assert_ne!(deleted, original, "matched file deleted");
    }

    #[rstest::rstest]
    #[case::always_run(CheckFiles::Files(vec![]))]
    #[case::run_all(CheckFiles::RunAll)]
    #[case::on_demand(CheckFiles::OnDemand)]
    #[case::skipped(CheckFiles::SkippedNoMatch)]
    fn no_key_without_concrete_files(#[case] files: CheckFiles) {
        let dir = root();
        assert_eq!(key(&check("c", "c", files), &[], 1, dir.path()), None);
    }

    #[test]
    fn no_key_with_an_unreadable_file() {
        // Edge case: a directory (e.g. a submodule path) cannot be read as a file
        let dir = root();
        std::fs::create_dir(dir.path().join("sub")).unwrap();
        let c = check("lint", "lint", files(&["a.rs", "sub"]));
        assert_eq!(key(&c, &[], 1, dir.path()), None);
    }

    /// Config with a test-discovery check `unit` (`src/{path}.rs` ->
    /// `tests/{path}_test.rs`) and a `src/a.rs` change, files under a tempdir
    fn discovery_setup() -> (CiConfig, ChangedFiles, tempfile::TempDir) {
        // Edge case: a test-discovery check whose files are only the tests
        let config: CiConfig = serde_yaml::from_str(
            r#"
version: 2
runner: local
git: { base_branch: main, fallback_branch: HEAD~1 }
file_patterns:
  rust_src: { pattern: '^src/.*\.rs$' }
checks:
  g:
    checks:
      unit:
        name: Unit
        command: t {files}
        triggers:
          test_discovery:
            source_pattern: rust_src
            strategies:
              - type: path_mapping
                rules: [{ source: "src/{path}.rs", tests: ["tests/{path}_test.rs"] }]
"#,
        )
        .unwrap();
        let dir = tempfile::tempdir().unwrap();
        for (path, content) in [("src/a.rs", "a"), ("tests/a_test.rs", "t")] {
            std::fs::create_dir_all(dir.path().join(path).parent().unwrap()).unwrap();
            std::fs::write(dir.path().join(path), content).unwrap();
        }
        let changed = ChangedFiles {
            files: vec!["src/a.rs".into()],
            base_ref: "main".into(),
        };
        (config, changed, dir)
    }

    #[test]
    fn stamp_keys_covers_discovery_sources() {
        let (config, changed, dir) = discovery_setup();
        let stamped = || {
            let root = Some(dir.path());
            let checks = crate::checks::select_checks(&config, &changed, dir.path(), root).checks;
            assert_eq!(checks[0].files, files(&["tests/a_test.rs"]));
            checks[0].cache_key.clone().expect("keyed").value
        };
        let before = stamped();
        std::fs::write(dir.path().join("src/a.rs"), "a changed").unwrap();
        assert_ne!(stamped(), before);
    }

    #[test]
    fn select_checks_without_key_root_stamps_nothing() {
        let (config, changed, dir) = discovery_setup();
        let checks = crate::checks::select_checks(&config, &changed, dir.path(), None).checks;
        assert_eq!(checks[0].cache_key, None);
    }

    #[test]
    fn recorded_pass_is_fresh_after_reload() {
        let dir = root();
        let path = dir.path().join("git/ci-tui/results.json");
        let lint = keyed("lint", "lint a.rs", dir.path());
        let mut cache = cache_at(path.clone(), dir.path());
        assert!(!cache.is_fresh(&lint));

        cache
            .record(&lint, &result("lint", CheckStatus::Passed))
            .unwrap();
        assert!(cache.is_fresh(&lint));
        assert!(cache_at(path.clone(), dir.path()).is_fresh(&lint));
        let no_cache = ResultCache::at(path, dir.path().to_path_buf(), false);
        assert!(!no_cache.is_fresh(&lint), "--no-cache");
    }

    #[test]
    fn pass_after_files_changed_since_stamping_is_not_stored() {
        let dir = root();
        let path = dir.path().join("results.json");
        let lint = keyed("lint", "lint a.rs", dir.path());
        // Edited while the check ran: the pass may be for other content
        std::fs::write(dir.path().join("a.rs"), "fn a() { edited() }").unwrap();
        let mut cache = cache_at(path.clone(), dir.path());
        cache
            .record(&lint, &result("lint", CheckStatus::Passed))
            .unwrap();
        assert!(!path.exists());

        std::fs::write(dir.path().join("a.rs"), "fn a() {}").unwrap();
        assert!(!cache.is_fresh(&lint), "undoing the edit serves nothing");
    }

    #[test]
    fn new_pass_overwrites_the_single_entry_per_check() {
        let dir = root();
        let path = dir.path().join("results.json");
        let mut cache = cache_at(path.clone(), dir.path());
        let old = keyed("lint", "lint a.rs", dir.path());
        let new = keyed("lint", "lint --strict a.rs", dir.path());
        cache
            .record(&old, &result("lint", CheckStatus::Passed))
            .unwrap();
        cache
            .record(&new, &result("lint", CheckStatus::Passed))
            .unwrap();

        let reloaded = cache_at(path, dir.path());
        assert_eq!(reloaded.entries.len(), 1);
        assert!(reloaded.is_fresh(&new) && !reloaded.is_fresh(&old));
    }

    #[rstest::rstest]
    #[case::failed(CheckStatus::Failed)]
    #[case::timed_out(CheckStatus::TimedOut)]
    fn failure_drops_entry_and_is_never_stored(#[case] status: CheckStatus) {
        let dir = root();
        let path = dir.path().join("results.json");
        let lint = keyed("lint", "lint a.rs", dir.path());
        let mut cache = cache_at(path.clone(), dir.path());
        cache
            .record(&lint, &result("lint", status.clone()))
            .unwrap();
        assert!(!path.exists(), "failure alone writes nothing");

        cache
            .record(&lint, &result("lint", CheckStatus::Passed))
            .unwrap();
        cache.record(&lint, &result("lint", status)).unwrap();
        assert!(!cache.is_fresh(&lint));
        assert!(!cache_at(path, dir.path()).is_fresh(&lint));
    }

    #[test]
    fn unkeyed_cancelled_and_cached_results_are_not_stored() {
        let dir = root();
        let path = dir.path().join("results.json");
        let mut cache = cache_at(path.clone(), dir.path());
        let unkeyed = check("always", "always", files(&[]));
        let lint = keyed("lint", "lint a.rs", dir.path());
        cache
            .record(&unkeyed, &result("always", CheckStatus::Passed))
            .unwrap();
        cache
            .record(&lint, &result("lint", CheckStatus::Cancelled))
            .unwrap();
        cache.record(&lint, &CheckResult::cached("lint")).unwrap();
        assert!(!path.exists());
        assert!(!cache.is_fresh(&unkeyed));
    }

    #[test]
    fn split_fresh_separates_cached_from_runnable() {
        let dir = root();
        let mut cache = cache_at(dir.path().join("results.json"), dir.path());
        let (lint, other) = (
            keyed("lint", "lint a.rs", dir.path()),
            keyed("other", "other a.rs", dir.path()),
        );
        cache
            .record(&lint, &result("lint", CheckStatus::Passed))
            .unwrap();
        let (fresh, run) = cache.split_fresh([&lint, &other]);
        assert_eq!(fresh[0].id(), "lint");
        assert_eq!(run[0].id(), "other");
    }

    #[test]
    fn corrupt_file_starts_empty() {
        let dir = root();
        let path = dir.path().join("results.json");
        std::fs::write(&path, "{not json").unwrap();
        let lint = keyed("lint", "lint a.rs", dir.path());
        let mut cache = cache_at(path.clone(), dir.path());
        assert!(!cache.is_fresh(&lint));
        cache
            .record(&lint, &result("lint", CheckStatus::Passed))
            .unwrap();
        assert!(cache_at(path, dir.path()).is_fresh(&lint));
    }

    #[test]
    fn write_error_is_reported_once_then_cache_is_off() {
        let dir = root();
        // A file where the cache dir should be
        std::fs::write(dir.path().join("ci-tui"), "").unwrap();
        let mut cache = cache_at(dir.path().join("ci-tui/results.json"), dir.path());
        let lint = keyed("lint", "lint a.rs", dir.path());
        let other = keyed("other", "other a.rs", dir.path());
        assert!(cache
            .record(&lint, &result("lint", CheckStatus::Passed))
            .is_err());
        assert!(cache
            .record(&other, &result("other", CheckStatus::Passed))
            .is_ok());
    }

    #[test]
    fn disabled_cache_serves_and_stores_nothing() {
        let dir = root();
        let lint = keyed("lint", "lint a.rs", dir.path());
        let mut cache = ResultCache::disabled();
        assert_eq!(cache.key_root(), None);
        cache
            .record(&lint, &result("lint", CheckStatus::Passed))
            .unwrap();
        assert!(!cache.is_fresh(&lint));
    }
}
