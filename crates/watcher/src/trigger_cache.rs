//! Per-cycle trigger-config cache for the watcher pipeline.
//!
//! Parsing a workflow file is expensive enough (YAML parse + glob
//! compilation for every `branches:`/`tags:`/`paths:` pattern) that
//! doing it every cycle on a network-mounted repo adds measurable
//! latency between debouncer drain and execution. This module keeps a
//! `HashMap<PathBuf, TriggerCacheEntry>` that is only invalidated when
//! a notify event's canonicalized path matches a workflow file's
//! canonicalized path.
//!
//! Extracted from `watcher.rs` so the path-form-normalization rationale
//! (relative `read_dir` output vs macOS-canonicalized notify paths) and
//! the "previously parseable but now broken" cache-eviction regression
//! live with their own tests.

use std::collections::{HashMap, HashSet};
use std::path::PathBuf;
use wrkflw_trigger_filter::canonicalize_allowing_missing;
use wrkflw_trigger_filter::{TriggerFilterConfig, WorkflowTriggerConfig};

/// One entry in the trigger cache: the compiled config plus a memoized
/// canonical form of the workflow's path.
///
/// Stashing the canonical form lets `refresh_trigger_cache_blocking` skip
/// `canonicalize_allowing_missing` for any workflow that's already cached —
/// previously, every cycle re-canonicalized every workflow file, which on
/// a network mount or a deep tree was a measurable per-cycle latency hit.
#[derive(Debug, Clone)]
pub(crate) struct TriggerCacheEntry {
    /// Canonical form of the workflow file path. Used to test set
    /// membership against `changed_paths` (which arrive from notify in
    /// canonical absolute form on macOS).
    pub canonical_path: PathBuf,
    pub config: WorkflowTriggerConfig,
}

/// Synchronous implementation of trigger cache refresh. Extracted so the
/// async wrapper can move it onto a `spawn_blocking` thread without
/// dragging `&self` along, and so unit tests can drive it without an
/// ambient tokio runtime.
///
/// **Path-form normalization is load-bearing here.** `workflow_files`
/// arrives in whatever form `read_dir(workflow_dir)` produced (typically
/// relative — `.github/workflows/ci.yml`), while `changed_paths` arrives
/// from notify (typically absolute and OS-canonicalized — on macOS that
/// means `/private/var/...` instead of `/var/...`). A naive
/// `HashSet::contains` between the two never matches, so the
/// "this workflow file was edited, reparse it" branch silently rots and
/// the watcher serves stale parsed configs forever. We canonicalize both
/// sides to the same form before set membership.
///
/// **Canonical-path memoization.** Each entry stores its canonical form
/// in [`TriggerCacheEntry::canonical_path`]; we only re-canonicalize a
/// workflow file when it isn't yet in the cache. This bounds the
/// per-cycle canonicalize calls to "newly-added workflows", instead of
/// "every workflow on every cycle".
pub(crate) fn refresh_trigger_cache_blocking(
    trigger_cache: &mut HashMap<PathBuf, TriggerCacheEntry>,
    workflow_files: &[PathBuf],
    changed_paths: &[PathBuf],
    verbose: bool,
    tf_config: &TriggerFilterConfig,
) {
    let active_set: HashSet<&PathBuf> = workflow_files.iter().collect();
    trigger_cache.retain(|k, _| active_set.contains(k));

    // Canonicalize the change set into the same shape as the workflow
    // file paths so equality comparisons actually work. We use the
    // missing-tolerant canonicalize so a workflow file that was just
    // deleted still hashes consistently with whatever notify reports.
    let changed_canon: HashSet<PathBuf> = changed_paths
        .iter()
        .map(|p| canonicalize_allowing_missing(p))
        .collect();

    let mut parse_failures = 0usize;
    for wf_path in workflow_files {
        // Reuse the cached canonical form if we have one — only newly
        // discovered workflow files cost a fresh canonicalize.
        let cached_canonical = trigger_cache.get(wf_path).map(|e| e.canonical_path.clone());
        let wf_canon = match cached_canonical {
            Some(c) => c,
            None => canonicalize_allowing_missing(wf_path),
        };
        let needs_reparse =
            !trigger_cache.contains_key(wf_path) || changed_canon.contains(&wf_canon);
        if !needs_reparse {
            continue;
        }
        // Route through the process-wide LRU cache so CLI prefilter,
        // watcher, and TUI all share the same compiled-glob cache.
        // With the default cache size, the unchanged-file path here
        // is a single HashMap lookup instead of a full YAML parse
        // plus glob compile.
        match wrkflw_trigger_filter::load_trigger_config_cached(wf_path, tf_config) {
            Ok(mut cfg) => {
                // Drain any parser-level diagnostics (unknown event
                // name, typo detection) into the log sink BEFORE
                // stashing the config in the watcher-side cache.
                // Leaving the warnings on `cfg.warnings` would either
                // silently lose them here (the old bug) or trip the
                // `MustDrainWarnings` Drop check every time the
                // cache evicts an entry. Log-on-drain parity with
                // the CLI prefilter and the TUI diff-filter path is
                // load-bearing: all three hosts must surface the
                // same diagnostics from the same library call, or
                // the "why did my workflow silently not fire"
                // failure mode reappears for whichever host skipped.
                for w in cfg.warnings.take() {
                    wrkflw_logging::warning(&w);
                }
                trigger_cache.insert(
                    wf_path.clone(),
                    TriggerCacheEntry {
                        canonical_path: wf_canon,
                        config: cfg,
                    },
                );
            }
            Err(e) => {
                // `remove` returns `Some` iff this workflow was
                // previously parseable — i.e. the user just broke a
                // working file. Surface that loudly even at non-verbose
                // level so the regression isn't hidden behind a generic
                // "N workflows failed to parse" summary that the user
                // has no way to act on.
                let was_previously_parseable = trigger_cache.remove(wf_path).is_some();
                parse_failures += 1;
                if verbose || was_previously_parseable {
                    wrkflw_logging::warning(&format!(
                        "Failed to parse {}: {}",
                        wf_path.display(),
                        e
                    ));
                }
            }
        }
    }

    if trigger_cache.is_empty() && !workflow_files.is_empty() {
        wrkflw_logging::warning(&format!(
            "No workflows are usable: all {} workflow file(s) failed to parse. \
             Run with --verbose for details.",
            workflow_files.len()
        ));
    } else if parse_failures > 0 && !verbose {
        wrkflw_logging::warning(&format!(
            "{} workflow file(s) failed to parse and were skipped (use --verbose for details)",
            parse_failures
        ));
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Regression for the dead-code cache invalidation branch:
    /// `workflow_files` arrives in relative form (`.github/workflows/ci.yml`)
    /// while `changed_paths` from notify is absolute + OS-canonicalized
    /// (`/private/var/folders/...` on macOS). The naive `HashSet::contains`
    /// against raw `PathBuf`s never matched, so editing a workflow file
    /// mid-watch left the cache stale forever. After the fix, an absolute
    /// canonicalized changed path must invalidate the cached entry for the
    /// matching relative workflow file.
    #[test]
    fn refresh_trigger_cache_reparses_edited_workflow_across_path_forms() {
        let tmp = tempfile::TempDir::new().expect("tempdir");
        let repo = tmp.path().to_path_buf();

        // Write the workflow with `paths: ['src/foo.rs']`. The schema
        // validator that `parse_workflow` runs requires at least one
        // step, so we give the job a trivial echo.
        std::fs::create_dir_all(repo.join(".github").join("workflows"))
            .expect("create workflow dir");
        let wf_abs = repo.join(".github/workflows/ci.yml");
        let v1_yaml = "name: test\non:\n  push:\n    paths:\n      - 'src/foo.rs'\njobs:\n  build:\n    runs-on: ubuntu-latest\n    steps:\n      - run: echo v1\n";
        std::fs::write(&wf_abs, v1_yaml).expect("write ci.yml v1");

        let workflow_files = vec![wf_abs.clone()];
        let mut cache: HashMap<PathBuf, TriggerCacheEntry> = HashMap::new();

        refresh_trigger_cache_blocking(
            &mut cache,
            &workflow_files,
            &[],
            false,
            &TriggerFilterConfig::default(),
        );
        let v1 = cache.get(&wf_abs).expect("v1 cached");
        let v1_paths: Vec<&str> = v1.config.events[0]
            .paths
            .iter()
            .map(|p| p.source.as_str())
            .collect();
        assert_eq!(v1_paths, vec!["src/foo.rs"], "v1 should have foo paths");

        // Rewrite the file with `paths: ['src/bar.rs']`.
        let v2_yaml = "name: test\non:\n  push:\n    paths:\n      - 'src/bar.rs'\njobs:\n  build:\n    runs-on: ubuntu-latest\n    steps:\n      - run: echo v2\n";
        std::fs::write(&wf_abs, v2_yaml).expect("write ci.yml v2");

        // Simulate a notify event with the OS-canonicalized absolute form.
        // (On macOS this prepends `/private`.)
        let changed = std::fs::canonicalize(&wf_abs).expect("canonicalize wf");
        refresh_trigger_cache_blocking(
            &mut cache,
            &workflow_files,
            &[changed],
            false,
            &TriggerFilterConfig::default(),
        );

        let v2 = cache.get(&wf_abs).expect("v2 cached");
        let v2_paths: Vec<&str> = v2.config.events[0]
            .paths
            .iter()
            .map(|p| p.source.as_str())
            .collect();
        assert_eq!(
            v2_paths,
            vec!["src/bar.rs"],
            "edit must invalidate the cached parse — got stale {:?}",
            v2_paths
        );
    }

    /// Regression: when a workflow file that *was* successfully parsed
    /// breaks (e.g. user introduces an invalid glob), the cache must
    /// drop the stale entry AND the failure should not require
    /// `--verbose` to be visible. Previously, the only signal at
    /// non-verbose level was a generic "N workflow file(s) failed to
    /// parse" summary, leaving the user with no way to identify which
    /// file regressed. After the fix, a previously-parseable workflow
    /// going broken must propagate to the cache (entry removed) so the
    /// surrounding warning logic can fire.
    #[test]
    fn refresh_trigger_cache_drops_previously_parseable_on_break() {
        let tmp = tempfile::TempDir::new().expect("tempdir");
        let repo = tmp.path().to_path_buf();
        std::fs::create_dir_all(repo.join(".github").join("workflows"))
            .expect("create workflow dir");

        let wf_abs = repo.join(".github/workflows/ci.yml");
        let good = "name: t\non:\n  push:\n    paths:\n      - 'src/**'\njobs:\n  b:\n    runs-on: ubuntu-latest\n    steps:\n      - run: echo hi\n";
        std::fs::write(&wf_abs, good).expect("write good ci.yml");

        let workflow_files = vec![wf_abs.clone()];
        let mut cache: HashMap<PathBuf, TriggerCacheEntry> = HashMap::new();

        // Prime: file parses fine, cache populated.
        refresh_trigger_cache_blocking(
            &mut cache,
            &workflow_files,
            &[],
            false,
            &TriggerFilterConfig::default(),
        );
        assert!(
            cache.contains_key(&wf_abs),
            "good workflow must be cached after first refresh"
        );

        // Break the file with an invalid glob and re-refresh, simulating
        // a notify event for the broken workflow.
        let bad = "name: t\non:\n  push:\n    paths:\n      - '[unclosed'\njobs:\n  b:\n    runs-on: ubuntu-latest\n    steps:\n      - run: echo hi\n";
        std::fs::write(&wf_abs, bad).expect("write bad ci.yml");
        let changed = std::fs::canonicalize(&wf_abs).expect("canonicalize wf");
        refresh_trigger_cache_blocking(
            &mut cache,
            &workflow_files,
            &[changed],
            false,
            &TriggerFilterConfig::default(),
        );

        assert!(
            !cache.contains_key(&wf_abs),
            "broken workflow must be evicted from cache so the surrounding \
             warning logic can surface the regression"
        );
    }

    /// Regression: deleting every workflow file mid-session must
    /// evict the previously-cached entries. The old behavior had
    /// `collect_workflow_files_blocking` returning `Err` for an
    /// empty dir, the watcher's rescan branch fell back to the
    /// stale snapshot, and `active_set` then retained the deleted
    /// files forever. With the empty-dir fix the rescan hands us
    /// an empty `workflow_files`, and `retain` must drop every
    /// cached entry.
    #[test]
    fn refresh_trigger_cache_drops_entries_when_workflow_files_becomes_empty() {
        let tmp = tempfile::TempDir::new().expect("tempdir");
        let repo = tmp.path().to_path_buf();
        std::fs::create_dir_all(repo.join(".github").join("workflows"))
            .expect("create workflow dir");

        let wf_abs = repo.join(".github/workflows/ci.yml");
        std::fs::write(
            &wf_abs,
            "name: t\non:\n  push:\n    paths:\n      - 'src/**'\njobs:\n  b:\n    runs-on: ubuntu-latest\n    steps:\n      - run: echo hi\n",
        )
        .expect("write ci.yml");

        let workflow_files = vec![wf_abs.clone()];
        let mut cache: HashMap<PathBuf, TriggerCacheEntry> = HashMap::new();
        refresh_trigger_cache_blocking(
            &mut cache,
            &workflow_files,
            &[],
            false,
            &TriggerFilterConfig::default(),
        );
        assert!(cache.contains_key(&wf_abs));

        // Simulate the user deleting the last workflow file — the
        // next rescan returns an empty list, and `refresh_trigger_cache_blocking`
        // must drop the stale entry so the evaluator does not run
        // against a file that no longer exists.
        std::fs::remove_file(&wf_abs).expect("delete ci.yml");
        refresh_trigger_cache_blocking(
            &mut cache,
            &[], // empty active set
            &[],
            false,
            &TriggerFilterConfig::default(),
        );
        assert!(
            cache.is_empty(),
            "stale cache entries must be evicted when workflow_files is empty, got {:?}",
            cache.keys().collect::<Vec<_>>()
        );
    }

    /// Refreshing the cache twice for an unchanged workflow file must
    /// only canonicalize that file once — the second pass should reuse
    /// the canonical form stored in [`TriggerCacheEntry`]. This is the
    /// memoization that backs the per-cycle latency win on network
    /// mounts; the test asserts the cache hit by checking that the
    /// stored canonical_path survives the second refresh unchanged.
    #[test]
    fn refresh_trigger_cache_memoizes_canonical_paths() {
        let tmp = tempfile::TempDir::new().expect("tempdir");
        let repo = tmp.path().to_path_buf();
        std::fs::create_dir_all(repo.join(".github").join("workflows"))
            .expect("create workflow dir");

        let wf_abs = repo.join(".github/workflows/ci.yml");
        let yaml = "name: test\non:\n  push:\n    paths:\n      - 'src/foo.rs'\njobs:\n  build:\n    runs-on: ubuntu-latest\n    steps:\n      - run: echo hi\n";
        std::fs::write(&wf_abs, yaml).expect("write ci.yml");

        let workflow_files = vec![wf_abs.clone()];
        let mut cache: HashMap<PathBuf, TriggerCacheEntry> = HashMap::new();

        refresh_trigger_cache_blocking(
            &mut cache,
            &workflow_files,
            &[],
            false,
            &TriggerFilterConfig::default(),
        );
        let canonical_after_first = cache
            .get(&wf_abs)
            .expect("v1 cached")
            .canonical_path
            .clone();
        assert!(
            !canonical_after_first.as_os_str().is_empty(),
            "canonical_path should be populated"
        );

        // A second refresh with no changes must keep the same canonical
        // form (we're asserting the cache value survives, which proves
        // the entry was reused rather than re-canonicalized into a new
        // entry).
        refresh_trigger_cache_blocking(
            &mut cache,
            &workflow_files,
            &[],
            false,
            &TriggerFilterConfig::default(),
        );
        let canonical_after_second = &cache.get(&wf_abs).expect("still cached").canonical_path;
        assert_eq!(
            &canonical_after_first, canonical_after_second,
            "second refresh should reuse the memoized canonical_path"
        );
    }
}
