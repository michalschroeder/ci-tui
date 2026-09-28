//! Result cache: skip checks whose inputs are unchanged since their last
//! passing run (shown as "cached ✓"; `--no-cache` turns reads off).
//!
//! Key per check ([`stamp_keys`]): check id + resolved command + content of
//! its matched files (plus the changed source files behind its test
//! discovery, so editing only the source re-runs its tests) + hash of the
//! config file text. Checks without concrete files (always-run, run-all,
//! on-demand, skipped) get no key and are never cached.
//!
//! Store ([`ResultCache`]): `<git dir>/ci-tui/results.json` maps check id to
//! the key of its latest pass. A pass overwrites the entry, a failure removes
//! it, so the store holds at most one entry per check. Cache problems (not a
//! git repo, unreadable / corrupt file, write error) never fail a run: they
//! only turn caching off.
//!
//! Hashing uses std's `DefaultHasher` (SipHash with fixed keys, stable across
//! runs): no extra dependency. A toolchain changing the algorithm only costs
//! one cache miss per check; 64 bits make a false hit negligible.

use crate::checks::{CheckFiles, CheckToRun};
use crate::config::CiConfig;
use crate::git::ChangedFiles;
use crate::runner::{CheckResult, CheckStatus};
use std::collections::HashMap;
use std::hash::{DefaultHasher, Hash, Hasher};
use std::io;
use std::path::{Path, PathBuf};

/// Cache directory inside the git dir
const CACHE_DIR: &str = "ci-tui";
/// Cache file inside [`CACHE_DIR`]
const CACHE_FILE: &str = "results.json";

/// Stable hash of `bytes` (the config file text)
pub(crate) fn hash_bytes(bytes: &[u8]) -> u64 {
    let mut hasher = DefaultHasher::new();
    bytes.hash(&mut hasher);
    hasher.finish()
}

/// Set every check's [`CheckToRun::cache_key`] from the current file
/// contents under `root` (the command execution root). Reads files:
/// blocking.
pub fn stamp_keys(
    checks: &mut [CheckToRun],
    config: &CiConfig,
    changed_files: &ChangedFiles,
    root: &Path,
) {
    for check in checks {
        let sources = discovery_sources(config, changed_files, check);
        check.cache_key = key(check, &sources, config.source_hash, root);
    }
}

/// Changed files matching the check's `test_discovery.source_pattern`
fn discovery_sources<'a>(
    config: &CiConfig,
    changed_files: &'a ChangedFiles,
    check: &CheckToRun,
) -> Vec<&'a str> {
    check
        .definition
        .triggers
        .as_ref()
        .and_then(|t| t.test_discovery.as_ref())
        .and_then(|d| config.get_compiled_file_pattern(&d.source_pattern))
        .map(|re| changed_files.filter_by_pattern(re))
        .unwrap_or_default()
}

/// Cache key of `check`; `None` without concrete matched files
fn key(check: &CheckToRun, sources: &[&str], config_hash: u64, root: &Path) -> Option<u64> {
    let CheckFiles::Files(files) = &check.files else {
        return None;
    };
    if files.is_empty() {
        return None;
    }
    let mut hasher = DefaultHasher::new();
    (check.id(), &check.resolved_command, config_hash).hash(&mut hasher);
    for path in files
        .iter()
        .map(String::as_str)
        .chain(sources.iter().copied())
    {
        path.hash(&mut hasher);
        // `None` (deleted / unreadable) never equals any content
        std::fs::read(root.join(path)).ok().hash(&mut hasher);
    }
    Some(hasher.finish())
}

/// Key of the latest passing run per check id, persisted in the git dir.
#[derive(Debug, Default)]
pub struct ResultCache {
    /// Cache file; `None` = off (not a git repo, or a write failed)
    path: Option<PathBuf>,
    /// Serve unchanged checks from the cache (`false` with `--no-cache`)
    read: bool,
    /// Check id -> key of its latest pass
    entries: HashMap<String, u64>,
}

impl ResultCache {
    /// A cache that serves and records nothing (fix mode, tests)
    pub fn disabled() -> Self {
        Self::default()
    }

    /// Cache in the git dir of the repo containing `cwd` (a linked worktree
    /// has its own); disabled outside a repo. `read = false` still records.
    pub fn open(cwd: &Path, read: bool) -> Self {
        match crate::git::git_dir(cwd) {
            Ok(dir) => Self::at(dir.join(CACHE_DIR).join(CACHE_FILE), read),
            Err(_) => Self::disabled(),
        }
    }

    /// Cache stored at `path`. A missing, unreadable or corrupt file starts
    /// empty (overwritten on the next change).
    pub fn at(path: PathBuf, read: bool) -> Self {
        let entries = std::fs::read(&path)
            .ok()
            .and_then(|bytes| serde_json::from_slice(&bytes).ok())
            .unwrap_or_default();
        Self {
            path: Some(path),
            read,
            entries,
        }
    }

    /// True when `check` passed last time with the same key: serve it as
    /// [`CheckResult::cached`] instead of running it
    pub fn is_fresh(&self, check: &CheckToRun) -> bool {
        self.read
            && check
                .cache_key
                .is_some_and(|key| self.entries.get(check.id()) == Some(&key))
    }

    /// Record a finished run of `check`: a pass stores its key (replacing the
    /// previous one), a failure drops the entry; cached / cancelled results
    /// change nothing. Saves on change. A write error is returned once, then
    /// the cache is off for the rest of the run.
    pub fn record(&mut self, check: &CheckToRun, result: &CheckResult) -> io::Result<()> {
        if self.path.is_none() || result.cached {
            return Ok(());
        }
        let id = check.id();
        let changed = match check.cache_key {
            Some(key) if result.status == CheckStatus::Passed => {
                self.entries.insert(id.to_string(), key) != Some(key)
            }
            _ if result.status.is_failure() => self.entries.remove(id).is_some(),
            _ => false,
        };
        if changed {
            self.save()
        } else {
            Ok(())
        }
    }

    /// Write all entries (temp file + rename, so readers never see a partial
    /// file); on error the cache turns off
    fn save(&mut self) -> io::Result<()> {
        let Some(path) = &self.path else {
            return Ok(());
        };
        let written = write_atomic(path, &serde_json::to_vec(&self.entries)?);
        if written.is_err() {
            self.path = None;
        }
        written
    }
}

fn write_atomic(path: &Path, bytes: &[u8]) -> io::Result<()> {
    if let Some(dir) = path.parent() {
        std::fs::create_dir_all(dir)?;
    }
    let tmp = path.with_extension("json.tmp");
    std::fs::write(&tmp, bytes)?;
    std::fs::rename(&tmp, path)
}

#[cfg(test)]
mod tests {
    use super::*;
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

    fn keyed(id: &str, command: &str, root: &Path) -> CheckToRun {
        let mut c = check(id, command, files(&["a.rs"]));
        c.cache_key = key(&c, &[], 1, root);
        c
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
        assert_ne!(k(&base, &[], 1), original, "matched file deleted");
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
    fn stamp_keys_covers_discovery_sources() {
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
        let stamped = || {
            let mut checks = crate::checks::determine_checks(&config, &changed, dir.path());
            stamp_keys(&mut checks, &config, &changed, dir.path());
            assert_eq!(checks[0].files, files(&["tests/a_test.rs"]));
            checks[0].cache_key.expect("keyed")
        };
        let before = stamped();
        std::fs::write(dir.path().join("src/a.rs"), "a changed").unwrap();
        assert_ne!(stamped(), before);
    }

    #[test]
    fn recorded_pass_is_fresh_after_reload() {
        let dir = root();
        let path = dir.path().join("git/ci-tui/results.json");
        let lint = keyed("lint", "lint a.rs", dir.path());
        let mut cache = ResultCache::at(path.clone(), true);
        assert!(!cache.is_fresh(&lint));

        cache
            .record(&lint, &result("lint", CheckStatus::Passed))
            .unwrap();
        assert!(cache.is_fresh(&lint));
        assert!(ResultCache::at(path.clone(), true).is_fresh(&lint));
        assert!(!ResultCache::at(path, false).is_fresh(&lint), "--no-cache");
    }

    #[test]
    fn new_pass_overwrites_the_single_entry_per_check() {
        let dir = root();
        let path = dir.path().join("results.json");
        let mut cache = ResultCache::at(path.clone(), true);
        let old = keyed("lint", "lint a.rs", dir.path());
        let new = keyed("lint", "lint --strict a.rs", dir.path());
        cache
            .record(&old, &result("lint", CheckStatus::Passed))
            .unwrap();
        cache
            .record(&new, &result("lint", CheckStatus::Passed))
            .unwrap();

        let reloaded = ResultCache::at(path, true);
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
        let mut cache = ResultCache::at(path.clone(), true);
        cache
            .record(&lint, &result("lint", status.clone()))
            .unwrap();
        assert!(!path.exists(), "failure alone writes nothing");

        cache
            .record(&lint, &result("lint", CheckStatus::Passed))
            .unwrap();
        cache.record(&lint, &result("lint", status)).unwrap();
        assert!(!cache.is_fresh(&lint));
        assert!(!ResultCache::at(path, true).is_fresh(&lint));
    }

    #[test]
    fn unkeyed_cancelled_and_cached_results_are_not_stored() {
        let dir = root();
        let path = dir.path().join("results.json");
        let mut cache = ResultCache::at(path.clone(), true);
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
    fn corrupt_file_starts_empty() {
        let dir = root();
        let path = dir.path().join("results.json");
        std::fs::write(&path, "{not json").unwrap();
        let lint = keyed("lint", "lint a.rs", dir.path());
        let mut cache = ResultCache::at(path.clone(), true);
        assert!(!cache.is_fresh(&lint));
        cache
            .record(&lint, &result("lint", CheckStatus::Passed))
            .unwrap();
        assert!(ResultCache::at(path, true).is_fresh(&lint));
    }

    #[test]
    fn write_error_is_reported_once_then_cache_is_off() {
        let dir = root();
        // A file where the cache dir should be
        std::fs::write(dir.path().join("ci-tui"), "").unwrap();
        let mut cache = ResultCache::at(dir.path().join("ci-tui/results.json"), true);
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
        cache
            .record(&lint, &result("lint", CheckStatus::Passed))
            .unwrap();
        assert!(!cache.is_fresh(&lint));
    }
}
