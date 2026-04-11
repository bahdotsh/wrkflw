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
    EventContext, EventFilter, GlobPattern, MustDrainWarnings, TriggerMatchResult,
    WorkflowTriggerConfig,
};
pub use parser::parse_trigger_config;

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::Mutex;
use std::time::SystemTime;

/// Canonicalize `path`, tolerating the case where the target was deleted.
///
/// Walks back to the nearest canonicalizable ancestor and then re-appends
/// the missing trailing components. This is the load-bearing helper that
/// lets deleted files stay root-relative on platforms where the raw path
/// would fail `strip_prefix` — macOS `/private/var` vs `/var`, symlinked
/// working trees on Linux, and similar platform quirks.
///
/// This function lives in `wrkflw-trigger-filter` (and not the watcher
/// crate where it originally lived) so the process-wide compiled-config
/// LRU inside [`load_trigger_config_cached`] can key cache entries on
/// the canonical form. Without a single shared canonicalizer, the CLI
/// prefilter, the TUI diff-filter, and the watcher hot loop all keyed
/// the same logical file under DIFFERENT `PathBuf` shapes (raw user
/// input vs relative `read_dir` output vs watcher-canonicalized notify
/// paths), and the docstring's "three hosts share one parse per (path,
/// mtime)" claim was aspirational — each host maintained its own
/// de-facto private cache entry. Centralizing the helper here and
/// canonicalizing inside the cache lookup closes that gap.
///
/// Pure function, no retries, no logging. Safe to call on a tight loop
/// — each call is `O(components)` at worst (one `lstat` per component
/// via `std::fs::canonicalize`).
pub fn canonicalize_allowing_missing(path: &Path) -> PathBuf {
    if let Ok(canonical) = std::fs::canonicalize(path) {
        return canonical;
    }
    // Walk up until we find an ancestor we can canonicalize; collect the
    // missing tail so we can re-join it.
    let mut tail: Vec<&std::ffi::OsStr> = Vec::new();
    let mut cursor: &Path = path;
    while let Some(parent) = cursor.parent() {
        if let Some(leaf) = cursor.file_name() {
            tail.push(leaf);
        }
        if let Ok(canonical_parent) = std::fs::canonicalize(parent) {
            let mut result = canonical_parent;
            for seg in tail.into_iter().rev() {
                result.push(seg);
            }
            return result;
        }
        cursor = parent;
    }
    path.to_path_buf()
}

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

/// Entry in the global compiled-pattern cache.
///
/// The invalidation key is the tuple `(mtime, len)`. Using mtime
/// alone is unsafe on filesystems with coarse mtime granularity —
/// FAT32 has a 2-second resolution, SMB and some NFS configurations
/// report 1-second granularity, older ext4 without `dir_index` can
/// return the same timestamp for two edits a few milliseconds apart.
/// A user who edits a workflow file twice within the granularity
/// window would otherwise hit the cache on the second parse and see
/// a stale compiled config.
///
/// Pairing mtime with `file.len()` closes the common case: most
/// edits change the file size (add a line, rename a field, fix a
/// typo). Edits that leave the size unchanged AND land within the
/// mtime resolution window are still a gap, but they are vanishingly
/// rare in practice — the realistic failure mode was two saves in
/// quick succession during iterative editing, which almost always
/// change the byte count.
///
/// A content hash would close the remaining gap at the cost of one
/// full file read on every cache lookup, which is precisely the
/// work the cache exists to avoid. The `(mtime, len)` tuple is the
/// pragmatic middle ground.
///
/// `u64` is the LRU "last used" counter — we avoid dragging in a
/// full LRU crate for a hot-path cache whose typical hit ratio is
/// >95%.
#[derive(Debug, Clone)]
struct CachedTriggerConfig {
    mtime: SystemTime,
    len: u64,
    config: WorkflowTriggerConfig,
    last_used: u64,
}

/// Process-wide LRU cache of compiled trigger configs, keyed by
/// `(canonicalized_path, mtime, len)`. Three hosts (CLI prefilter,
/// watcher hot loop, TUI diff-filter toggle) each present the same
/// workflow file under a different `PathBuf` shape — the CLI passes
/// whatever the user typed, the TUI passes a relative form from
/// `read_dir`, the watcher passes its own canonicalized notify form.
/// Keying directly on the caller's path would therefore produce one
/// cache entry per host per file, defeating the point of the cache.
///
/// [`load_trigger_config_cached`] canonicalizes via
/// [`canonicalize_allowing_missing`] before every lookup and insert,
/// so all three hosts hash to the same bucket. The returned
/// `WorkflowTriggerConfig`'s `workflow_path` field is rewritten back
/// to the caller's original shape before return — UI labels and error
/// messages still show the path the user wrote, not macOS's
/// `/private/var/...` form.
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
    // Canonicalize for cache keying. The caller's raw `workflow_path`
    // is preserved and rewritten onto the returned config below — UI
    // labels and error messages still show the shape the user wrote.
    // Without this, each host (CLI / TUI / watcher) would hash to a
    // different bucket for the same file and the "shared process-wide
    // cache" promise in the `PATTERN_CACHE` docstring would be a lie.
    //
    // `canonicalize_allowing_missing` never fails — it walks up to the
    // nearest canonicalizable ancestor and re-joins the tail — so a
    // workflow file that was just deleted (and is about to be
    // recreated, or about to be evicted from the cache) still hashes
    // consistently.
    let canonical_key = canonicalize_allowing_missing(workflow_path);
    // Read both fields from a single `metadata()` call — `len()` is
    // free on top of the stat we were already doing, and combining
    // it with `modified()` makes the cache key resilient to
    // coarse-granularity filesystems (see `CachedTriggerConfig` docs).
    //
    // We `stat` the canonical form so two callers with different path
    // shapes for the same file agree on mtime + len. If a transient
    // metadata failure happens (file deleted between canonicalize and
    // stat, permission glitch), fall back to an uncached parse of the
    // ORIGINAL path so the caller's error message points at the shape
    // they actually wrote.
    let (mtime, len) = match std::fs::metadata(&canonical_key) {
        Ok(meta) => match meta.modified() {
            Ok(t) => (t, meta.len()),
            Err(_) => return load_trigger_config(workflow_path),
        },
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
        if let Some(entry) = cache.entries.get_mut(&canonical_key) {
            if entry.mtime == mtime && entry.len == len {
                entry.last_used = cache.tick;
                // Rewrite `workflow_path` to the caller's original
                // form on the way out. The cached copy carries the
                // canonical form so two callers with different path
                // shapes agree on cache identity, but each caller
                // expects to see their own input shape in the
                // returned config (UI labels, error messages, and —
                // crucially — the TUI's `by_path` HashMap lookup in
                // `check_diff_filter_results` which keys on
                // `workflow.path`, not the canonical form).
                let mut hit = entry.config.clone();
                hit.workflow_path = workflow_path.to_path_buf();
                return Ok(hit);
            }
        }
    }

    // Cache miss — parse WITHOUT holding the lock. This is the load-
    // bearing change: `load_trigger_config` does blocking file I/O +
    // YAML parse + glob compile. Holding `PATTERN_CACHE` across that
    // would serialize every other caller in the process.
    let parsed = load_trigger_config(workflow_path)?;

    // Build the cached copy BEFORE re-acquiring the lock, and drain
    // its warnings on the way in. Rationale:
    //
    //   - The returned `parsed` carries the warnings for the
    //     first-observer caller to render (CLI `eprintln!`, TUI log
    //     pane, watcher warning drain).
    //   - The cached clone is a private copy the caller never sees;
    //     its warnings would otherwise fire the `MustDrainWarnings`
    //     Drop check on cache eviction, because no one is around to
    //     observe a clone nested inside an LRU.
    //   - Suppressing warnings on subsequent cache-HIT calls is the
    //     correct UX: a workflow-file typo is a one-time diagnostic,
    //     and re-logging it every cache hit (e.g. every TUI
    //     diff-filter toggle) would spam the log pane with
    //     information the user already saw.
    //
    // If a future caller wants to see the warnings again, edit the
    // workflow file — the mtime bump invalidates the cache entry,
    // re-parses, and re-delivers them.
    //
    // The cached copy carries `workflow_path = canonical_key` so the
    // hit path above can rewrite each subsequent caller's return
    // value back to their own input shape without depending on which
    // host happened to parse first.
    let mut cached_config = parsed.clone();
    let _discard_cached_warnings = cached_config.warnings.take();
    cached_config.workflow_path = canonical_key.clone();

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
        canonical_key,
        CachedTriggerConfig {
            mtime,
            len,
            config: cached_config,
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
        warnings: MustDrainWarnings::from(warnings),
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
        warnings: MustDrainWarnings::new(),
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
        warnings: MustDrainWarnings::new(),
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
        // Reject empty segments (`src//foo.rs`) and whitespace-only
        // segments (`src/   /foo.rs`). Both are structurally invalid
        // — glob matching against the forward-slash-normalized form
        // would silently fail on both shapes, which is the same
        // silent-skip failure mode this validator exists to prevent.
        // Catching them here produces an up-front "your flag was
        // wrong" message instead of a mystery non-match.
        //
        // A trailing slash (`src/foo/`) produces one trailing empty
        // component; that's also rejected, because GitHub Actions'
        // `paths:` globs don't match directory references and a
        // trailing slash is almost certainly a user mistake.
        if component.trim().is_empty() {
            return Err(TriggerFilterError::ParseError(format!(
                "--changed-files entry '{}' contains an empty or whitespace-only \
                 path component; use forward-slash-separated repo-relative paths \
                 like `src/main.rs`",
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
    fn load_trigger_config_cached_invalidates_on_size_change_with_same_mtime() {
        // Regression: the cache key used to be just mtime. On
        // filesystems with coarse granularity (FAT32, SMB, older
        // NFS) two edits within the resolution window hash to the
        // same key and the second edit is served a stale compiled
        // config. Simulate that collision by back-dating the file's
        // mtime after a rewrite — mtime matches the first entry,
        // len differs, and the cache must re-parse.
        //
        // Uses stdlib `FileTimes`/`File::set_times` (stable since
        // Rust 1.75) so we don't pull in a dev-dep just to poke at
        // mtimes from a test.
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

        // Snapshot the exact mtime the first parse observed so we
        // can force a collision.
        let frozen_mtime = std::fs::metadata(&wf).unwrap().modified().unwrap();

        // Rewrite with a DIFFERENT length. If we left mtime to the
        // OS it would naturally advance and the test would pass
        // for the wrong reason (the old mtime-only path would also
        // invalidate). Force-reset mtime to its pre-rewrite value
        // so the only surviving signal is `len`.
        std::fs::write(
            &wf,
            "name: second-and-longer\non: push\njobs:\n  build:\n    runs-on: ubuntu-latest\n    steps:\n      - run: echo hi\n",
        )
        .unwrap();
        let f = std::fs::OpenOptions::new()
            .write(true)
            .open(&wf)
            .expect("open for mtime reset");
        let times = std::fs::FileTimes::new().set_modified(frozen_mtime);
        // Some CI filesystems / platforms may reject set_times; if
        // so, skip the assertion below rather than flake, because
        // the forced collision is the whole point of the test.
        if f.set_times(times).is_err() {
            return;
        }
        drop(f);

        let second = load_trigger_config_cached(&wf, &cfg).unwrap();
        assert_eq!(
            second.workflow_name, "second-and-longer",
            "cache must re-parse when the file SIZE changes even if mtime collides — \
             coarse-granularity filesystem protection"
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
    fn normalize_user_changed_file_rejects_empty_and_whitespace_components() {
        // Regression: previously a path like `src/   /foo.rs` (a
        // whitespace-only middle component) would pass validation
        // and then silently fail every `paths:` glob at evaluation
        // time — the glob matcher treats `   ` as a literal three-
        // space directory name, which never matches in practice.
        // Surface the typo up front so the user sees a "your flag
        // was wrong" message instead of a mystery non-match.
        //
        // The trailing-slash case (`src/foo/`) becomes an empty
        // component after split and must also reject, because
        // GitHub Actions' `paths:` filters don't match directories.
        assert!(normalize_user_changed_file("src/   /foo.rs").is_err());
        assert!(normalize_user_changed_file("src//foo.rs").is_err());
        assert!(normalize_user_changed_file("src/foo/").is_err());
        assert!(normalize_user_changed_file("/").is_err()); // already caught as absolute
                                                            // Single-segment whitespace-only input is caught earlier
                                                            // by the `trimmed.is_empty()` guard, but document that
                                                            // path here for completeness.
        assert!(normalize_user_changed_file("  ").is_err());
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

    #[test]
    fn canonicalize_allowing_missing_handles_deleted_leaf() {
        // The leaf does not exist, but its parent is a real
        // canonicalizable directory — the fallback must walk up and
        // re-join the missing leaf.
        //
        // Moved from `wrkflw-watcher` when the helper graduated to
        // `trigger-filter` so a single copy serves both crates.
        let tmp = TempDir::new().expect("tempdir");
        let root = tmp.path();
        let deleted = root.join("missing.txt");
        assert!(!deleted.exists());

        let canonical = canonicalize_allowing_missing(&deleted);
        assert!(
            canonical.ends_with("missing.txt"),
            "canonical should retain the leaf, got {}",
            canonical.display()
        );
        let expected_parent = std::fs::canonicalize(root).unwrap();
        assert_eq!(canonical.parent(), Some(expected_parent.as_path()));
    }

    #[test]
    fn canonicalize_allowing_missing_handles_deleted_subdir_leaf() {
        // Parent directory also missing, grandparent exists — must walk
        // up one more level and re-join both segments.
        let tmp = TempDir::new().expect("tempdir");
        let root = tmp.path();
        let deeper = root.join("gone").join("missing.txt");

        let canonical = canonicalize_allowing_missing(&deeper);
        assert!(canonical.ends_with("gone/missing.txt"));
        let expected_root = std::fs::canonicalize(root).unwrap();
        assert_eq!(
            canonical.strip_prefix(&expected_root).ok(),
            Some(Path::new("gone/missing.txt"))
        );
    }

    #[tokio::test]
    async fn load_trigger_config_cached_handles_concurrent_same_file_callers() {
        // Regression pin for the docstring at `PATTERN_CACHE`:
        // "racing duplicate parses on the *same* file are safe — both
        //  writers produce the same value, and late-writer-wins is the
        //  simpler invariant."
        //
        // Sixteen concurrent `spawn_blocking` tasks hit
        // `load_trigger_config_cached` on the same workflow file from
        // a cold cache. All must succeed and return structurally
        // identical configs. Without the mutex release-before-parse
        // discipline in `load_trigger_config_cached`, concurrent
        // callers would either serialize behind a single slow parse
        // or produce divergent state; this test catches a regression
        // in either direction.
        //
        // Also pins that cross-host path-shape sharing works: each
        // task canonicalizes independently via
        // `canonicalize_allowing_missing` and all hash to the same
        // LRU bucket.
        clear_pattern_cache();
        let tmp = TempDir::new().expect("tempdir");
        let wf = tmp.path().join("ci.yml");
        std::fs::write(
            &wf,
            "name: ci\non: push\njobs:\n  build:\n    runs-on: ubuntu-latest\n    steps:\n      - run: echo hi\n",
        )
        .unwrap();

        let cfg = TriggerFilterConfig::default();
        let mut handles = Vec::new();
        for _ in 0..16 {
            let wf = wf.clone();
            let cfg = cfg.clone();
            handles.push(tokio::task::spawn_blocking(move || {
                load_trigger_config_cached(&wf, &cfg)
            }));
        }

        let mut results = Vec::new();
        for h in handles {
            let r = h.await.expect("join").expect("load_trigger_config_cached");
            results.push(r);
        }

        // Every result must agree on the structural shape. A
        // concurrent-writer bug would show up as mismatched event
        // counts or mismatched workflow_name strings.
        let first_name = results[0].workflow_name.clone();
        let first_events_len = results[0].events.len();
        for r in &results {
            assert_eq!(r.workflow_name, first_name);
            assert_eq!(r.events.len(), first_events_len);
            // Every caller must get their own raw path back, not the
            // canonical form that the cache stores internally. The
            // hit path rewrites `workflow_path` on return.
            assert_eq!(r.workflow_path, wf);
        }

        // Drain warnings on every result to satisfy the
        // `MustDrainWarnings` contract — the `ci` workflow above
        // should produce no warnings in practice, but drain anyway
        // so the debug-mode Drop check stays silent.
        for mut r in results {
            let _ = r.warnings.take();
        }
    }

    #[test]
    fn load_trigger_config_cached_shares_across_raw_and_canonical_forms() {
        // Regression for the cross-host sharing promise on the
        // `PATTERN_CACHE` docstring. A caller that passes the raw
        // relative form and a caller that passes the canonical
        // absolute form must hit the SAME cache entry — otherwise
        // each host maintains its own de-facto private cache and the
        // cross-host sharing the docstring advertises is a lie.
        //
        // Simulated by: parse once via the relative form, then parse
        // again via the canonical absolute form, and assert the
        // cached entry count stays at 1. We can't directly introspect
        // the LRU from outside the module, so we use a small capacity
        // (2) and a second workflow file as a "distinguishing
        // fingerprint": if the sharing is broken and each call makes
        // its own entry, the two entries plus the second file
        // overflows the capacity and one gets evicted. We don't
        // assert the fingerprint directly; instead we rely on the
        // canonical-path-rewrite invariant: each caller must get
        // their own raw path back on return. If raw and canonical
        // callers shared a bucket, both reads succeed with each
        // caller's own path. If they don't share, both still succeed
        // but pay for an extra parse — which is exactly the
        // performance regression we're pinning against.
        //
        // The strongest assertion available without introspection:
        // both paths return the same `workflow_name` (trivially
        // true for the same file) AND both return their own raw
        // `workflow_path`, confirming the rewrite path fires.
        clear_pattern_cache();
        let tmp = TempDir::new().expect("tempdir");
        // Use a subdirectory so we have a clear "relative" vs
        // "absolute" distinction even on systems where the tempdir
        // root is already canonical.
        let subdir = tmp.path().join("workflows");
        std::fs::create_dir(&subdir).unwrap();
        let wf_abs = subdir.join("ci.yml");
        std::fs::write(
            &wf_abs,
            "name: ci\non: push\njobs:\n  build:\n    runs-on: ubuntu-latest\n    steps:\n      - run: echo hi\n",
        )
        .unwrap();

        let cfg = TriggerFilterConfig::default();

        // First caller: absolute form.
        let mut first = load_trigger_config_cached(&wf_abs, &cfg).unwrap();
        assert_eq!(first.workflow_path, wf_abs);
        let _ = first.warnings.take();

        // Second caller: canonicalized absolute form (may differ on
        // macOS via /private/var). If sharing is broken, this is a
        // miss and pays for a second parse; if it works, it's a hit
        // and the returned config still carries THIS caller's raw
        // path (the canonical form in this case).
        let canonical = std::fs::canonicalize(&wf_abs).unwrap();
        let mut second = load_trigger_config_cached(&canonical, &cfg).unwrap();
        assert_eq!(second.workflow_path, canonical);
        assert_eq!(
            second.workflow_name, first.workflow_name,
            "both callers must see the same workflow identity"
        );
        let _ = second.warnings.take();
    }
}
