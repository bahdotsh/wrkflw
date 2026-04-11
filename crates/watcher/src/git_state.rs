//! TTL-bounded cache for `(branch, tag)` git state used by the watcher
//! hot loop.
//!
//! Extracted from `watcher.rs` so the cache + its refresh policy live
//! in one place. `WorkflowWatcher` owns a [`GitStateCache`] instead of
//! an inline `Mutex<Option<CachedGitState>>`, which keeps the struct
//! definition out of the reactor file and lets the staleness contract
//! be read without stepping through 2k lines of orchestration.
//!
//! The watcher reads `(branch, tag)` once per cycle. During a
//! file-save storm a single debounced cycle used to shell out
//! `git rev-parse` + `git describe` per event — this cache is what
//! makes the hot loop cheap. See [`GitStateCache::get`] for the
//! staleness rules.

use std::path::Path;
use std::sync::Mutex;
use std::time::Instant;

/// Snapshot of the last-fetched git state (branch + tag) with the
/// wall-clock instant it was fetched at. Reused across cycles inside
/// `TriggerFilterConfig::git_state_ttl` so a file-save storm doesn't
/// trigger one `git rev-parse` + one `git describe` per event.
///
/// `head_mtime` is the mtime of `.git/HEAD` at fetch time. We
/// re-`stat` it on every lookup and treat the cache as stale if the
/// value changed — a `git checkout` inside the TTL window bumps
/// `.git/HEAD`'s mtime, so the next cycle's branch/tag fetch is
/// guaranteed to run instead of handing back a value from the
/// pre-checkout working tree. Without this key, a fast `checkout +
/// save` sequence silently evaluated branch filters against the
/// previous branch for up to one TTL.
#[derive(Debug, Clone)]
pub(crate) struct CachedGitState {
    fetched_at: Instant,
    head_mtime: Option<std::time::SystemTime>,
    branch: Option<String>,
    tag: Option<String>,
}

/// TTL-bounded cache wrapping a `Mutex<Option<CachedGitState>>`.
///
/// `Mutex` rather than `RwLock` because the critical section is
/// trivially short (one clone of an `Option<String>`), and because the
/// write path runs at most once per TTL so contention is effectively
/// zero. Uses `std::sync::Mutex` so the guard is cheap to acquire
/// without involving the tokio runtime.
#[derive(Debug)]
pub(crate) struct GitStateCache {
    inner: Mutex<Option<CachedGitState>>,
}

impl GitStateCache {
    pub(crate) fn new() -> Self {
        Self {
            inner: Mutex::new(None),
        }
    }

    /// Test-only accessor that returns the cached `fetched_at` instant
    /// if the cache is populated. The `cached_git_state_reuses_within_ttl`
    /// test uses it to assert that two successive hits share a single
    /// fetch instant (i.e. the second call did not refresh). Hidden
    /// behind `cfg(test)` so the production path has no observable
    /// surface on the cache's wall-clock state.
    #[cfg(test)]
    pub(crate) fn peek_fetched_at(&self) -> Option<Instant> {
        let guard = match self.inner.lock() {
            Ok(g) => g,
            Err(p) => p.into_inner(),
        };
        guard.as_ref().map(|s| s.fetched_at)
    }

    /// Return cached `(branch, tag)` if still fresh within
    /// `config.git_state_ttl`; otherwise re-fetch both via concurrent
    /// git subprocess calls and update the cache.
    ///
    /// Errors out of git are propagated so the caller can build a
    /// `WatchEvent` with an `error` payload — the previous code path
    /// silently collapsed git failures to `branch: None`, which made
    /// every `branches:` filter deterministically reject and produced
    /// a session-long stream of "0 triggered" reports with no
    /// explanation.
    pub(crate) async fn get(
        &self,
        config: &wrkflw_trigger_filter::TriggerFilterConfig,
        repo_root: &Path,
    ) -> Result<(Option<String>, Option<String>), wrkflw_trigger_filter::TriggerFilterError> {
        let ttl = config.git_state_ttl;
        let cwd = Some(repo_root);
        let current_head_mtime = wrkflw_trigger_filter::git::head_mtime(cwd);

        // Cheap lock: just check freshness and clone out if hit.
        //
        // Mutex poisoning is handled via `into_inner()` inside a
        // `match` — the guard from the poisoned branch is a
        // `MutexGuard` too, so both arms feed a single usage below.
        //
        // Two independent staleness tests:
        //   1. Wall-clock TTL (bounds worst-case staleness even if
        //      `.git/HEAD` didn't move — e.g. a fresh clone with no
        //      committed HEAD yet, or a platform where mtime is
        //      coarser than the test expects).
        //   2. HEAD mtime divergence — catches `git checkout` within
        //      the TTL. Only compared when BOTH the cached and the
        //      freshly-stat'd mtime are `Some`; one-sided `None` is
        //      treated as "don't know, fall back to the TTL alone"
        //      so the happy path still short-circuits on platforms
        //      that don't expose a usable modified() time.
        {
            let guard = match self.inner.lock() {
                Ok(g) => g,
                Err(p) => p.into_inner(),
            };
            if let Some(state) = guard.as_ref() {
                let ttl_ok = state.fetched_at.elapsed() < ttl;
                let head_ok = match (state.head_mtime, current_head_mtime) {
                    (Some(cached), Some(current)) => cached == current,
                    _ => true,
                };
                if ttl_ok && head_ok {
                    return Ok((state.branch.clone(), state.tag.clone()));
                }
            }
        }

        // Cache miss — capture HEAD mtime BEFORE the git calls. This is
        // load-bearing for the cache's staleness contract: if a `git
        // checkout` lands between `get_current_branch` and the final
        // store, we want the stored mtime to reflect the *pre-checkout*
        // state that produced the branch/tag values we just read. The
        // next `GitStateCache::get` call will then stat the (newer)
        // post-checkout mtime, observe a mismatch against the stored
        // value, and force a refresh.
        //
        // An earlier draft captured `head_mtime` after the git calls so
        // the stored value reflected the post-checkout state. That
        // "looked right" but produced the opposite bug: the cache would
        // happily serve the pre-checkout branch/tag for the full TTL
        // window because the stored mtime already matched whatever the
        // next call would observe. The regression is pinned by
        // `cached_git_state_invalidates_when_checkout_races_git_reads`.
        //
        // Racing writers: we accept that a concurrent refresher may
        // overwrite with its own fetch. Both branches produce the same
        // value on the steady-state, so late-writer-wins is safe; this
        // avoids a compare-and-set dance on the hot path.
        let fetched_at = Instant::now();
        let head_mtime = wrkflw_trigger_filter::git::head_mtime(cwd);

        let (branch_res, tag_res) = tokio::join!(
            wrkflw_trigger_filter::git::get_current_branch(cwd),
            wrkflw_trigger_filter::git::get_current_tag(cwd),
        );
        let branch = branch_res?;
        let tag = tag_res?;

        let mut guard = match self.inner.lock() {
            Ok(g) => g,
            Err(p) => p.into_inner(),
        };
        *guard = Some(CachedGitState {
            fetched_at,
            head_mtime,
            branch: branch.clone(),
            tag: tag.clone(),
        });
        Ok((branch, tag))
    }
}
