pub mod config;
pub mod error;
pub mod eval;
pub mod git;
pub mod model;
pub mod parser;
pub mod path_matcher;
pub mod ref_matcher;

pub use config::TriggerFilterConfig;
pub use error::TriggerFilterError;
pub use eval::evaluate_trigger;
pub use git::{find_repo_root_detailed, head_mtime, FindRepoRootError};
pub use model::{
    EventContext, EventFilter, GlobPattern, TriggerMatchResult, WorkflowTriggerConfig,
};
pub use parser::parse_trigger_config;

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::Mutex;
use std::time::SystemTime;

/// Read a workflow file from disk and parse its trigger configuration in
/// one step. Centralizes the "read + parse + compile globs" pipeline so
/// that `watcher`, the TUI, and the CLI all fail identically on the same
/// broken file.
///
/// This performs blocking file I/O via [`wrkflw_parser::workflow::parse_workflow`].
/// Call from a blocking context (or wrap in `spawn_blocking`) if invoked
/// from an async task that must not stall the reactor.
pub fn load_trigger_config(
    workflow_path: &Path,
) -> Result<WorkflowTriggerConfig, TriggerFilterError> {
    let workflow = wrkflw_parser::workflow::parse_workflow(workflow_path)
        .map_err(|e| TriggerFilterError::ParseError(e.to_string()))?;
    parse_trigger_config(&workflow, workflow_path.to_path_buf())
}

/// Bulk-load trigger configs for many workflow files, partitioning the
/// result into successes and per-file failure pairs.
///
/// This is the shape both the watcher and the TUI need: parse failures
/// must be surfaced to the user (rather than `filter_map(... .ok())`d
/// into invisibility), so any caller that wants to render "N failed to
/// parse" diagnostics gets the offending paths and reasons in one call.
///
/// Same blocking-I/O caveat as [`load_trigger_config`] — wrap in
/// `spawn_blocking` from async contexts.
pub fn load_trigger_configs(
    paths: &[PathBuf],
) -> (Vec<WorkflowTriggerConfig>, Vec<(PathBuf, String)>) {
    let mut configs = Vec::with_capacity(paths.len());
    let mut failures: Vec<(PathBuf, String)> = Vec::new();
    for path in paths {
        match load_trigger_config(path) {
            Ok(cfg) => configs.push(cfg),
            Err(e) => failures.push((path.clone(), e.to_string())),
        }
    }
    (configs, failures)
}

/// Evaluate multiple pre-parsed trigger configs against an event context.
///
/// Callers are expected to cache the [`WorkflowTriggerConfig`] values and
/// invalidate them only when the underlying workflow file changes. This
/// avoids re-running `parse_trigger_config` — and thus re-compiling every
/// glob pattern — on every cycle.
pub fn filter_trigger_configs(
    configs: &[&WorkflowTriggerConfig],
    context: &EventContext,
) -> Vec<TriggerMatchResult> {
    configs
        .iter()
        .map(|config| evaluate_trigger(config, context))
        .collect()
}

// ---------------------------------------------------------------------------
// Process-wide compiled-pattern cache
// ---------------------------------------------------------------------------

/// Entry in the global compiled-pattern cache. The mtime is the
/// invalidation key: any caller asking for the same path after a write
/// will observe a different mtime and re-parse. `u64` is the LRU
/// "last used" counter — we avoid dragging in a full LRU crate for a
/// hot-path cache whose typical hit ratio is >95%.
#[derive(Debug, Clone)]
struct CachedTriggerConfig {
    mtime: SystemTime,
    config: WorkflowTriggerConfig,
    last_used: u64,
}

/// Process-wide LRU cache of compiled trigger configs, keyed by
/// `(absolute_path, mtime)`. Three hosts (CLI prefilter, watcher hot
/// loop, TUI diff-filter toggle) previously each re-parsed every
/// workflow on every invocation, re-compiling every glob pattern. This
/// cache collapses that work to one parse per (path, mtime) pair across
/// the entire process.
///
/// Size is bounded by `TriggerFilterConfig::pattern_cache_size` —
/// overflow evicts the least-recently-used entry. The lock is a
/// `std::sync::Mutex` because the critical section is bounded at
/// `O(cache_size)` in the worst case (linear LRU scan on eviction),
/// and the hit path is a single HashMap lookup.
static PATTERN_CACHE: Mutex<Option<PatternCache>> = Mutex::new(None);

#[derive(Debug)]
struct PatternCache {
    capacity: usize,
    tick: u64,
    entries: HashMap<PathBuf, CachedTriggerConfig>,
}

impl PatternCache {
    fn new(capacity: usize) -> Self {
        Self {
            capacity,
            tick: 0,
            entries: HashMap::new(),
        }
    }

    fn evict_lru(&mut self) {
        // Linear LRU — correct and cheap for the ~128-entry default.
        // If the capacity ever needs to scale, swap in `lru` crate.
        if let Some(victim) = self
            .entries
            .iter()
            .min_by_key(|(_, v)| v.last_used)
            .map(|(k, _)| k.clone())
        {
            self.entries.remove(&victim);
        }
    }
}

/// Load a trigger config via the process-wide LRU cache.
///
/// Falls back to an uncached parse when the configured capacity is
/// zero (the test-mode opt-out) or when the file's mtime cannot be
/// read. Callers that want a guaranteed fresh parse should call
/// [`load_trigger_config`] directly.
///
/// **Locking discipline.** The cache mutex is released BEFORE the
/// blocking YAML parse on a miss, so concurrent callers hitting
/// different files never serialize behind a single slow parse. A
/// previous shape held the lock across `load_trigger_config`, which
/// defeated the point of the cache on any multi-thread host: one
/// slow file made every other parse wait for it. Racing duplicate
/// parses on the *same* file are safe — both writers produce the
/// same value, and late-writer-wins is the simpler invariant.
pub fn load_trigger_config_cached(
    workflow_path: &Path,
    config: &TriggerFilterConfig,
) -> Result<WorkflowTriggerConfig, TriggerFilterError> {
    if config.pattern_cache_size == 0 {
        return load_trigger_config(workflow_path);
    }
    let mtime = match std::fs::metadata(workflow_path).and_then(|m| m.modified()) {
        Ok(t) => t,
        Err(_) => return load_trigger_config(workflow_path),
    };

    // Fast path: cache hit under lock. Release the lock before the
    // blocking parse on a miss — see the doc comment above.
    {
        let mut guard = match PATTERN_CACHE.lock() {
            Ok(g) => g,
            Err(p) => p.into_inner(),
        };
        let cache = guard.get_or_insert_with(|| PatternCache::new(config.pattern_cache_size));
        // Honour capacity changes across calls — a test that flips the
        // cache size off/on should see the new ceiling immediately.
        if cache.capacity != config.pattern_cache_size {
            cache.capacity = config.pattern_cache_size;
            while cache.entries.len() > cache.capacity {
                cache.evict_lru();
            }
        }
        cache.tick = cache.tick.wrapping_add(1);
        if let Some(entry) = cache.entries.get_mut(workflow_path) {
            if entry.mtime == mtime {
                entry.last_used = cache.tick;
                return Ok(entry.config.clone());
            }
        }
    }

    // Cache miss — parse WITHOUT holding the lock. This is the load-
    // bearing change: `load_trigger_config` does blocking file I/O +
    // YAML parse + glob compile. Holding `PATTERN_CACHE` across that
    // would serialize every other caller in the process.
    let parsed = load_trigger_config(workflow_path)?;

    // Re-acquire and insert. A concurrent caller may have populated
    // the entry while we were parsing; overwrite it with our fresh
    // value — same content, so late-writer-wins is safe.
    let mut guard = match PATTERN_CACHE.lock() {
        Ok(g) => g,
        Err(p) => p.into_inner(),
    };
    let cache = guard.get_or_insert_with(|| PatternCache::new(config.pattern_cache_size));
    let tick = cache.tick;
    cache.entries.insert(
        workflow_path.to_path_buf(),
        CachedTriggerConfig {
            mtime,
            config: parsed.clone(),
            last_used: tick,
        },
    );
    if cache.entries.len() > cache.capacity {
        cache.evict_lru();
    }
    Ok(parsed)
}

/// Bulk cached variant of [`load_trigger_configs`] that pushes every
/// entry through the LRU. Same error-partitioning shape as the
/// uncached version so callers drop-in-replace without touching their
/// diagnostic rendering.
pub fn load_trigger_configs_cached(
    paths: &[PathBuf],
    config: &TriggerFilterConfig,
) -> (Vec<WorkflowTriggerConfig>, Vec<(PathBuf, String)>) {
    let mut configs = Vec::with_capacity(paths.len());
    let mut failures: Vec<(PathBuf, String)> = Vec::new();
    for path in paths {
        match load_trigger_config_cached(path, config) {
            Ok(cfg) => configs.push(cfg),
            Err(e) => failures.push((path.clone(), e.to_string())),
        }
    }
    (configs, failures)
}

/// Drop every entry from the process-wide pattern cache. Used by
/// tests and by long-lived hosts that need to react to an out-of-band
/// signal that every workflow may have changed (e.g. a `git pull`).
pub fn clear_pattern_cache() {
    if let Ok(mut guard) = PATTERN_CACHE.lock() {
        *guard = None;
    }
}

/// Auto-detect event context from the current git state.
///
/// Fetches the current branch, tag, and changed files (vs `diff_base`) from git.
///
/// `cwd` selects the working directory git operates in; pass `None` to use the
/// process CWD. Long-running consumers (e.g. the watcher) should always pass
/// their explicit repo root so they don't accidentally query the wrong repo.
///
/// **Branch handling:** detached HEAD returns `Ok` with `branch: None` (the
/// underlying [`git::get_current_branch`] surfaces detached HEAD as `Ok(None)`,
/// not an error). A *real* git error — e.g. permission denied on `.git/HEAD`,
/// corrupt repo — propagates as `Err`. Previously this code collapsed both
/// cases to `branch: None`, which masked real failures.
///
/// **Note:** for `pull_request`/`pull_request_target` events, this does NOT
/// populate `base_branch` — there's no way to infer the PR target from a
/// local checkout. Callers should pass it explicitly via the higher-level
/// API or the `--base-branch` CLI flag.
pub async fn auto_detect_context(
    event_name: &str,
    diff_base: &str,
    cwd: Option<&Path>,
) -> Result<EventContext, TriggerFilterError> {
    // Run the three independent git queries concurrently.
    let (branch_res, tag_res, changed_res) = tokio::join!(
        git::get_current_branch(cwd),
        git::get_current_tag(cwd),
        git::get_changed_files_with_warnings(diff_base, cwd),
    );

    let (changed_files, warnings) = changed_res?;
    Ok(EventContext {
        event_name: event_name.to_string(),
        branch: branch_res?,
        base_branch: None,
        tag: tag_res?,
        changed_files,
        // We actually ran `git diff` against `diff_base`, so even an
        // empty result is an *authoritative* "nothing changed". The
        // diagnostic layer uses this to stop suggesting `--diff` when
        // the user already passed one.
        changed_files_explicit: true,
        activity_type: None,
        warnings,
    })
}

/// Like [`auto_detect_context`] but also resolves the diff base via
/// [`git::get_default_diff_base`] when the caller has no preference.
///
/// Fails with a `GitError` if no reasonable diff base can be detected — the
/// caller should surface that so the user can pass `--diff-base` explicitly
/// instead of silently getting a filter that matches every workflow.
///
/// `verbose` is forwarded to [`git::get_default_diff_base`] so the
/// "diff base = HEAD on dirty tree" explanatory log only fires when the
/// caller wants it. The CLI opts in via its `--verbose` flag; the TUI
/// and any long-lived host pass `false` so a hot-path toggle doesn't
/// flood the log pane.
pub async fn auto_detect_context_default_base(
    event_name: &str,
    cwd: Option<&Path>,
    verbose: bool,
) -> Result<EventContext, TriggerFilterError> {
    let diff_base = git::get_default_diff_base(cwd, verbose).await?;
    auto_detect_context(event_name, &diff_base, cwd).await
}

/// Build an event context using an explicit two-ref diff range.
///
/// Used by the CLI when both `--diff-base` and `--diff-head` are provided.
pub async fn context_from_diff_range(
    event_name: &str,
    base_ref: &str,
    head_ref: &str,
    cwd: Option<&Path>,
) -> Result<EventContext, TriggerFilterError> {
    let (branch_res, tag_res, changed_res) = tokio::join!(
        git::get_current_branch(cwd),
        git::get_current_tag(cwd),
        git::get_changed_files_between(base_ref, head_ref, cwd),
    );

    Ok(EventContext {
        event_name: event_name.to_string(),
        branch: branch_res?,
        base_branch: None,
        tag: tag_res?,
        changed_files: changed_res?,
        // Explicit two-ref diff range — the caller asked for a diff,
        // so an empty result is authoritative.
        changed_files_explicit: true,
        activity_type: None,
        warnings: Vec::new(),
    })
}

/// Build an event context with pre-supplied changed files.
///
/// Useful when the caller already knows the changed files (e.g. from `--changed-files`
/// CLI flag or from filesystem watcher events).
pub async fn context_from_changed_files(
    event_name: &str,
    changed_files: Vec<String>,
    cwd: Option<&Path>,
) -> Result<EventContext, TriggerFilterError> {
    let (branch_res, tag_res) =
        tokio::join!(git::get_current_branch(cwd), git::get_current_tag(cwd),);

    Ok(EventContext {
        event_name: event_name.to_string(),
        branch: branch_res?,
        base_branch: None,
        tag: tag_res?,
        changed_files,
        // Caller supplied the list explicitly — even `vec![]` is a
        // deliberate "nothing changed", not "I didn't bother to
        // check". The watcher uses this path with the set of files
        // that fired a notify event, so every call site here means
        // "authoritative".
        changed_files_explicit: true,
        activity_type: None,
        warnings: Vec::new(),
    })
}

/// Validate a user-supplied changed-file path (from `--changed-files`
/// or a similar host). Rejects absolute paths and any entry containing
/// `..` components, since both violate the "repo-relative POSIX path"
/// contract the evaluator assumes — a non-relative entry would silently
/// fail every `paths:` glob.
///
/// The returned normalized string uses `/` separators on every
/// platform so a user on Windows passing `src\foo.rs` gets the same
/// matching behavior as a user on Linux.
pub fn normalize_user_changed_file(raw: &str) -> Result<String, TriggerFilterError> {
    let trimmed = raw.trim();
    if trimmed.is_empty() {
        return Err(TriggerFilterError::ParseError(
            "--changed-files entries must be non-empty repo-relative paths".to_string(),
        ));
    }
    // Reject embedded NUL bytes up front. NUL is not a valid filename
    // byte on any supported platform (Unix forbids it in pathnames;
    // Windows uses NUL-terminated APIs), and letting one through would
    // only produce downstream confusion in glob matching or subprocess
    // argv handling. Consistent with the rest of the boundary-input
    // validation in this function.
    if trimmed.contains('\0') {
        return Err(TriggerFilterError::ParseError(format!(
            "--changed-files entry contains a NUL byte, which is not a valid \
             path component on any supported platform (raw: {:?})",
            raw
        )));
    }
    // Cheap textual checks cover the whole set of invalid shapes we
    // care about without pulling in a full path-canonicalization
    // helper (which would touch the filesystem — defeating the point
    // of validating user input up front).
    if trimmed.starts_with('/') || trimmed.starts_with('\\') {
        return Err(TriggerFilterError::ParseError(format!(
            "--changed-files entry '{}' is absolute; use repo-relative paths so `paths:` \
             globs can match against the same form GitHub Actions would see",
            trimmed
        )));
    }
    // Windows drive letter detection — `C:\foo` or `C:/foo` are both
    // absolute even though they don't start with `/`. Catch the
    // common drive-letter-plus-colon shape up front.
    if trimmed.len() >= 2 {
        let bytes = trimmed.as_bytes();
        if bytes[1] == b':' && bytes[0].is_ascii_alphabetic() {
            return Err(TriggerFilterError::ParseError(format!(
                "--changed-files entry '{}' looks like a drive-letter absolute path; \
                 pass repo-relative paths instead",
                trimmed
            )));
        }
    }
    let normalized = trimmed.replace('\\', "/");
    for component in normalized.split('/') {
        if component == ".." {
            return Err(TriggerFilterError::ParseError(format!(
                "--changed-files entry '{}' contains `..`; only in-tree repo-relative \
                 paths are allowed",
                raw
            )));
        }
    }
    Ok(normalized)
}

/// Bulk validate user-supplied changed-file entries. Returns the
/// normalized list on success, or the first error with enough context
/// for the CLI to print it verbatim.
pub fn normalize_user_changed_files(raw: &[String]) -> Result<Vec<String>, TriggerFilterError> {
    raw.iter().map(|s| normalize_user_changed_file(s)).collect()
}

// Note: the tests for the deleted `filter_workflows` (which parsed +
// evaluated in one shot) used to live here. They have been removed
// alongside the function — equivalent coverage already exists in
// `eval.rs` (match/no-match across event types and filter combos) and
// `parser.rs::invalid_glob_pattern_surfaces_as_parse_error` (the
// "broken glob surfaces a parse error" contract). The cached path
// `filter_trigger_configs` is exercised by every consumer in the
// workspace.

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::TempDir;

    #[test]
    fn load_trigger_configs_partitions_successes_and_failures() {
        // Mixed batch: one valid workflow, one with broken YAML, one
        // with a malformed glob. The bulk loader must return the
        // successful config and a per-file failure entry for each
        // broken file — the silent-drop pattern that
        // `.filter_map(... .ok())` produced was exactly the failure
        // mode that drove the original "lying about which workflows
        // would run" incident.
        let tmp = TempDir::new().expect("tempdir");
        let root = tmp.path();

        let good = root.join("good.yml");
        std::fs::write(
            &good,
            "name: good\non: push\njobs:\n  build:\n    runs-on: ubuntu-latest\n    steps:\n      - run: echo hi\n",
        )
        .unwrap();

        let bad_yaml = root.join("bad_yaml.yml");
        std::fs::write(&bad_yaml, "name: bad\non: [unterminated\n").unwrap();

        let bad_glob = root.join("bad_glob.yml");
        std::fs::write(
            &bad_glob,
            "name: bad_glob\non:\n  push:\n    paths:\n      - '[unclosed'\njobs:\n  build:\n    runs-on: ubuntu-latest\n    steps:\n      - run: echo hi\n",
        )
        .unwrap();

        let paths = vec![good.clone(), bad_yaml.clone(), bad_glob.clone()];
        let (configs, failures) = load_trigger_configs(&paths);

        assert_eq!(configs.len(), 1, "exactly one workflow should parse");
        assert_eq!(configs[0].workflow_path, good);

        assert_eq!(failures.len(), 2, "both broken files must surface");
        let failed_paths: Vec<&PathBuf> = failures.iter().map(|(p, _)| p).collect();
        assert!(failed_paths.contains(&&bad_yaml));
        assert!(failed_paths.contains(&&bad_glob));
    }

    #[test]
    fn load_trigger_config_cached_reuses_parse_across_calls() {
        // Clean slate so prior tests in the same process do not
        // contaminate the LRU's tick/last_used counters.
        clear_pattern_cache();
        let tmp = TempDir::new().expect("tempdir");
        let wf = tmp.path().join("ci.yml");
        std::fs::write(
            &wf,
            "name: ci\non: push\njobs:\n  build:\n    runs-on: ubuntu-latest\n    steps:\n      - run: echo hi\n",
        )
        .unwrap();
        let cfg = TriggerFilterConfig::default();
        let a = load_trigger_config_cached(&wf, &cfg).unwrap();
        let b = load_trigger_config_cached(&wf, &cfg).unwrap();
        assert_eq!(a.workflow_name, b.workflow_name);
        assert_eq!(a.workflow_path, b.workflow_path);
    }

    #[test]
    fn load_trigger_config_cached_invalidates_on_mtime_change() {
        clear_pattern_cache();
        let tmp = TempDir::new().expect("tempdir");
        let wf = tmp.path().join("ci.yml");
        std::fs::write(
            &wf,
            "name: first\non: push\njobs:\n  build:\n    runs-on: ubuntu-latest\n    steps:\n      - run: echo hi\n",
        )
        .unwrap();
        let cfg = TriggerFilterConfig::default();
        let first = load_trigger_config_cached(&wf, &cfg).unwrap();
        assert_eq!(first.workflow_name, "first");

        // Bump mtime by rewriting — on fast filesystems the mtime
        // resolution is coarser than the test runtime, so sleep just
        // long enough to guarantee a distinct mtime value. 20ms is
        // well under any realistic filesystem granularity.
        std::thread::sleep(std::time::Duration::from_millis(20));
        std::fs::write(
            &wf,
            "name: second\non: push\njobs:\n  build:\n    runs-on: ubuntu-latest\n    steps:\n      - run: echo hi\n",
        )
        .unwrap();
        let second = load_trigger_config_cached(&wf, &cfg).unwrap();
        assert_eq!(
            second.workflow_name, "second",
            "cache must re-parse when the file mtime changes"
        );
    }

    #[test]
    fn pattern_cache_size_zero_disables_caching() {
        clear_pattern_cache();
        let tmp = TempDir::new().expect("tempdir");
        let wf = tmp.path().join("ci.yml");
        std::fs::write(
            &wf,
            "name: ci\non: push\njobs:\n  build:\n    runs-on: ubuntu-latest\n    steps:\n      - run: echo hi\n",
        )
        .unwrap();
        let cfg = TriggerFilterConfig::default().with_pattern_cache_size(0);
        // Both calls must succeed; we can't directly observe that
        // caching is disabled, but the code path (early-return
        // without touching the static cache) is exercised here.
        let _ = load_trigger_config_cached(&wf, &cfg).unwrap();
        let _ = load_trigger_config_cached(&wf, &cfg).unwrap();
    }

    #[test]
    fn normalize_user_changed_file_rejects_absolute_and_parent_refs() {
        assert!(normalize_user_changed_file("/etc/passwd").is_err());
        assert!(normalize_user_changed_file("../outside").is_err());
        assert!(normalize_user_changed_file("src/../etc/passwd").is_err());
        assert!(normalize_user_changed_file("C:\\Windows\\system32").is_err());
        assert!(normalize_user_changed_file("").is_err());
        assert!(normalize_user_changed_file("   ").is_err());
        // Legit cases survive, with backslashes flipped to forward.
        assert_eq!(
            normalize_user_changed_file("src/main.rs").unwrap(),
            "src/main.rs"
        );
        assert_eq!(
            normalize_user_changed_file("src\\main.rs").unwrap(),
            "src/main.rs"
        );
    }

    #[test]
    fn normalize_user_changed_file_rejects_nul_bytes() {
        // NUL is not a valid filename byte on any supported platform;
        // a user passing one through `--changed-files` is almost
        // certainly a bug in whatever generated the list. Fail fast
        // with a pointer at the input instead of letting glob matching
        // silently misbehave.
        let err = normalize_user_changed_file("src/main\0.rs").unwrap_err();
        let msg = err.to_string();
        assert!(msg.contains("NUL"), "got: {}", msg);
    }
}
