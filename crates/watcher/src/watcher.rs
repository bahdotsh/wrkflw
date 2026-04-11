use crate::debouncer::Debouncer;
use crate::error::WatchError;
use crate::event_kind::is_relevant_event_kind;
use crate::ignore::{build_ignore_set, should_ignore_path};
use crate::paths::{canonicalize_allowing_missing, display_workflow_path, normalize_separators};
use crate::setup::{collect_workflow_files_blocking, setup_watches};
use crate::shutdown::ShutdownSignal;
use crate::trigger_cache::{refresh_trigger_cache_blocking, TriggerCacheEntry};
use futures::stream::{self, StreamExt};
use futures::FutureExt;
use notify::{Event, RecommendedWatcher, Watcher};
use std::collections::{HashMap, HashSet};
use std::panic::AssertUnwindSafe;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};
use wrkflw_executor::ExecutionConfig;
use wrkflw_trigger_filter::{EventContext, TriggerFilterConfig, WorkflowTriggerConfig};

/// Default cap on workflows executing concurrently in watch mode when the
/// caller does not supply an explicit limit.
pub const DEFAULT_MAX_CONCURRENT_EXECUTIONS: usize = 4;

/// Absolute upper bound on per-cycle concurrency. Each concurrent workflow
/// carries executor state (containers, tempdirs, child processes), so an
/// unbounded value can trivially OOM the host. Anything above this is
/// clamped down with a warning — users who genuinely need more should open
/// an issue so we can look at the actual workload before lifting the cap.
pub const MAX_REASONABLE_CONCURRENCY: usize = 256;

// The watcher's cached-git-state TTL now lives on
// `TriggerFilterConfig::git_state_ttl` — see the config crate for
// rationale and rationale comments. `cached_git_state` reads
// `self.cfg.trigger_filter.git_state_ttl` directly. Previously a
// file-local `GIT_STATE_CACHE_TTL` const existed as a placeholder for
// the then-missing Config struct; it has been retired to avoid the
// dead-plumbing drift the review flagged.

/// A watch event containing the changed files and trigger evaluation results.
///
/// `error` is `Some` when the cycle ran into a non-fatal failure that the
/// reporter should surface to the user (e.g. building the git event
/// context failed). The watch loop will keep running on the next event,
/// but the user needs to know that *this* cycle's "0 triggered" result is
/// degraded rather than authoritative — otherwise a missing default branch
/// or a transient git failure produces a session-long stream of silent
/// "nothing to do" reports.
///
/// `warnings` carries per-cycle non-fatal diagnostics that are NOT
/// fatal to evaluation (e.g. `git ls-files --others` failed, so
/// untracked files are missing from the change set — the rest of the
/// cycle still runs). Reporters should render these at a lower
/// severity than `error` but still above the "silent" line.
///
/// `dropped_events` is the number of filesystem events the debouncer
/// refused this cycle because its pending set was saturated. Greater
/// than zero means the reporter should tell the user something like
/// "12 change events were dropped this cycle; reduce churn or raise
/// the debouncer cap". Under normal use this is always zero.
#[derive(Debug, Clone)]
pub struct WatchEvent {
    pub changed_files: Vec<String>,
    pub triggered_workflows: Vec<String>,
    pub skipped_workflows: Vec<String>,
    pub error: Option<String>,
    pub warnings: Vec<String>,
    pub dropped_events: usize,
}

/// Configuration for [`WorkflowWatcher`]. Use the builder-style `with_*`
/// methods to set optional fields; `workflow_dir`, `repo_root`, and
/// `config` are required.
///
/// Introduced to bound the growth of `WorkflowWatcher::new`'s argument
/// list — future knobs (idle timeout, custom ignore list, event sink)
/// should be added here instead of as additional positional arguments.
#[derive(Debug, Clone)]
pub struct WatcherConfig {
    pub workflow_dir: PathBuf,
    pub repo_root: PathBuf,
    pub event_name: String,
    pub base_branch: Option<String>,
    /// Activity type to stamp on the synthesized event context — needed
    /// so workflows that filter on `pull_request: { types: [...] }` can
    /// match in watch mode without being silently rejected for "no
    /// activity type in context". `None` is fine for `push` and any
    /// event that has no `types:` filter.
    pub activity_type: Option<String>,
    pub debounce_duration: Duration,
    pub execution: ExecutionConfig,
    pub verbose: bool,
    pub max_concurrent_executions: usize,
    /// User-supplied directory names to ignore in addition to
    /// [`DEFAULT_IGNORE_DIRS`]. Extends the filter so projects using
    /// `.terraform/`, `coverage/`, `.cache/`, `.next/`, `bazel-bin/`
    /// etc. don't drown the debouncer in churn wrkflw can't anticipate
    /// out of the box.
    ///
    /// Matched by directory name (not glob, not path) to stay consistent
    /// with how `DEFAULT_IGNORE_DIRS` works: a component of the
    /// repo-relative parent path equals this name. File names
    /// (leaf components) are never matched, so a user file literally
    /// named e.g. `cache` is never silenced.
    pub extra_ignore_dirs: Vec<String>,
    /// Shared trigger-filter config. Owns the per-call git timeout,
    /// the git-state TTL, and the compiled-pattern cache size. Passed
    /// through from the CLI / TUI so a single config struct governs
    /// the entire trigger-filter pipeline instead of each layer
    /// re-declaring its own defaults.
    pub trigger_filter: TriggerFilterConfig,
    /// Upper bound on the debouncer's pending-event set. Zero means
    /// "use the debouncer's built-in default" ([`crate::debouncer::DEFAULT_MAX_PENDING_EVENTS`]).
    /// Tune upward for repos with heavy-churn workloads (large
    /// `cargo build` / `make` / `git checkout` bursts); tune downward
    /// in memory-constrained environments.
    pub max_pending_events: usize,
}

impl WatcherConfig {
    pub fn new(workflow_dir: PathBuf, repo_root: PathBuf, execution: ExecutionConfig) -> Self {
        let trigger_filter = TriggerFilterConfig::default();
        Self {
            workflow_dir,
            repo_root,
            event_name: trigger_filter.default_event.clone(),
            base_branch: None,
            activity_type: None,
            debounce_duration: Duration::from_millis(500),
            execution,
            verbose: false,
            max_concurrent_executions: DEFAULT_MAX_CONCURRENT_EXECUTIONS,
            extra_ignore_dirs: Vec::new(),
            trigger_filter,
            max_pending_events: 0, // 0 = use debouncer default
        }
    }

    pub fn with_trigger_filter_config(mut self, cfg: TriggerFilterConfig) -> Self {
        // Keep `event_name` in sync if the caller hasn't overridden it
        // yet — otherwise the new config's `default_event` would be
        // silently shadowed by the previous default.
        if self.event_name == TriggerFilterConfig::default().default_event {
            self.event_name = cfg.default_event.clone();
        }
        self.trigger_filter = cfg;
        self
    }

    pub fn with_max_pending_events(mut self, n: usize) -> Self {
        self.max_pending_events = n;
        self
    }

    /// Extend the fs-watcher's ignore list with additional directory
    /// names. See [`WatcherConfig::extra_ignore_dirs`] for the matching
    /// rules. Duplicate entries are allowed and harmless; the check is
    /// linear over a typically-tiny list.
    pub fn with_extra_ignore_dirs(mut self, dirs: Vec<String>) -> Self {
        self.extra_ignore_dirs = dirs;
        self
    }

    pub fn with_event(mut self, event: impl Into<String>) -> Self {
        self.event_name = event.into();
        self
    }

    pub fn with_base_branch(mut self, base: Option<String>) -> Self {
        self.base_branch = base;
        self
    }

    pub fn with_activity_type(mut self, activity: Option<String>) -> Self {
        self.activity_type = activity;
        self
    }

    pub fn with_debounce(mut self, d: Duration) -> Self {
        self.debounce_duration = d;
        self
    }

    pub fn with_verbose(mut self, v: bool) -> Self {
        self.verbose = v;
        self
    }

    pub fn with_max_concurrency(mut self, n: usize) -> Self {
        // 0 would deadlock buffer_unordered; clamp to at least 1.
        // Upper bound (see `MAX_REASONABLE_CONCURRENCY`) prevents OOM
        // from silly values — each concurrent workflow carries
        // executor state, so unbounded values can exhaust host
        // resources without any helpful error message.
        //
        // Both clamps must warn loudly. The previous version only
        // warned on the upper-bound clamp, so `--max-concurrency 0`
        // silently ran with concurrency=1 and the user's flag was
        // effectively ignored — the same silent-skip mode the rest of
        // this PR is built to prevent.
        if n == 0 {
            wrkflw_logging::warning(
                "max_concurrency=0 is invalid (would deadlock buffer_unordered); clamping to 1",
            );
        } else if n > MAX_REASONABLE_CONCURRENCY {
            wrkflw_logging::warning(&format!(
                "max_concurrency={} clamped to {} (higher values risk \
                 exhausting container/tempdir/process resources)",
                n, MAX_REASONABLE_CONCURRENCY
            ));
        }
        self.max_concurrent_executions = n.clamp(1, MAX_REASONABLE_CONCURRENCY);
        self
    }
}

/// Snapshot of the last-fetched git state (branch + tag) with the
/// wall-clock instant it was fetched at. Reused across cycles inside
/// [`GIT_STATE_CACHE_TTL`] so a file-save storm doesn't trigger one
/// `git rev-parse` + one `git describe` per event.
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
struct CachedGitState {
    fetched_at: Instant,
    head_mtime: Option<std::time::SystemTime>,
    branch: Option<String>,
    tag: Option<String>,
}

/// Watches for filesystem changes and triggers workflow execution.
pub struct WorkflowWatcher {
    cfg: WatcherConfig,
    /// TTL-bounded cache of `(branch, tag)`. `Mutex` rather than
    /// `RwLock` because the critical section is trivially short (one
    /// clone of an `Option<String>`), and because the write path runs
    /// at most once per TTL so contention is effectively zero. Uses
    /// `std::sync::Mutex` so the guard is cheap to acquire without
    /// involving the tokio runtime.
    git_state: Mutex<Option<CachedGitState>>,
}

impl WorkflowWatcher {
    /// Build a watcher from a [`WatcherConfig`].
    ///
    /// `WatcherConfig::with_max_concurrency` already clamps the concurrency
    /// floor to 1, so this constructor is intentionally just a struct
    /// literal — the previous re-clamp here was dead defensive code and
    /// the per-field accessor wrappers it sat alongside have been removed
    /// in favor of reading `self.cfg.x` directly.
    ///
    /// We canonicalize `workflow_dir` once at construction so every
    /// later cycle produces cache keys in the same shape. If the user
    /// passes a relative or symlinked workflow directory, the raw form
    /// drifts from whatever notify delivers, and the cache's
    /// `HashMap<PathBuf, _>` keyed on raw `read_dir` output can produce
    /// inconsistent lookups between `refresh_trigger_cache`
    /// invocations. `canonicalize_allowing_missing` falls back to the
    /// raw path if canonicalization fails (e.g. the directory doesn't
    /// exist yet) so the constructor never panics — the failure surfaces
    /// at the first `read_dir` call instead, where the error is actionable.
    pub fn from_config(mut cfg: WatcherConfig) -> Self {
        cfg.workflow_dir = canonicalize_allowing_missing(&cfg.workflow_dir);
        Self {
            cfg,
            git_state: Mutex::new(None),
        }
    }

    /// Collect workflow files from the configured directory.
    ///
    /// Runs the blocking `read_dir` syscall on a blocking thread so it
    /// doesn't stall the tokio reactor — the watcher calls this on every
    /// cycle, and a slow filesystem (e.g. a network mount or a huge
    /// workflows directory) would otherwise block incoming notify events.
    pub async fn collect_workflow_files(&self) -> Result<Vec<PathBuf>, WatchError> {
        let dir = self.cfg.workflow_dir.clone();
        tokio::task::spawn_blocking(move || collect_workflow_files_blocking(&dir))
            .await
            .map_err(|e| WatchError::Io(std::io::Error::other(e.to_string())))?
    }

    /// Start the watch loop. Calls `on_cycle_complete` after each
    /// debounced change set has been evaluated and executed.
    ///
    /// `on_cycle_complete` is dispatched fire-and-forget on a blocking
    /// thread so a slow reporter (file sink, network webhook) cannot stall
    /// the main loop. Callers MUST NOT rely on serialization between
    /// cycles in the callback.
    ///
    /// **Graceful shutdown.** Pass a [`ShutdownSignal`] so the caller
    /// can request a clean exit; the loop observes the signal at every
    /// `await` point (debounce drain, notify wait, shutdown check) and
    /// returns `Ok(())` once observed. The CLI passes
    /// [`ShutdownSignal::never`] because it relies on `process::exit`
    /// from the Ctrl+C handler; long-lived hosts (the TUI, a future
    /// daemon, tests) construct a real signal and `.trigger()` it on
    /// their own cancellation path.
    ///
    /// Note: a workflow execution already in flight when the signal
    /// fires will still run to completion for the current cycle before
    /// `run()` returns — we don't forcibly cancel the executor futures
    /// because they hold container/tempdir handles that need their
    /// normal cleanup. If the host cannot tolerate that latency, it
    /// should set a small `max_concurrent_executions` so the worst-case
    /// cycle is bounded.
    pub async fn run<F>(
        &self,
        shutdown: ShutdownSignal,
        on_cycle_complete: F,
    ) -> Result<(), WatchError>
    where
        F: Fn(WatchEvent) + Send + Sync + 'static,
    {
        let initial_workflow_files = self.collect_workflow_files().await?;

        // Canonicalize the repo root once so incoming notify paths (which the
        // OS may deliver as canonicalized — e.g. macOS `/private/var` vs
        // `/var`, or a symlinked working copy) can be made root-relative
        // without silently failing every `strip_prefix`.
        //
        // Failing this is FATAL. The previous fallback (use the raw,
        // non-canonical path) produces a totally degraded watcher on
        // macOS: every notify event arrives canonicalized as
        // `/private/var/...` and `strip_prefix("/var/...")` rejects all
        // of them, so the user sees "watching..." forever and zero
        // workflows ever fire. A watcher that *looks* alive but never
        // matches a single event is the worst possible failure mode for
        // this PR — refuse to start in that state and let the operator
        // see the underlying error directly.
        let repo_root_canonical = std::fs::canonicalize(&self.cfg.repo_root).map_err(|e| {
            WatchError::Io(std::io::Error::other(format!(
                "could not canonicalize repo root {}: {} — refusing to start the \
                 watcher in a degraded state where event paths could not be made \
                 repo-relative (this notably affects macOS /private/var and \
                 symlinked working trees). Verify the path exists and is accessible.",
                self.cfg.repo_root.display(),
                e,
            )))
        })?;

        // Honour `WatcherConfig::max_pending_events` when set;
        // otherwise fall through to the debouncer's baked-in default.
        let debouncer = Arc::new(if self.cfg.max_pending_events > 0 {
            Debouncer::with_capacity(self.cfg.debounce_duration, self.cfg.max_pending_events)
        } else {
            Debouncer::new(self.cfg.debounce_duration)
        });
        // Track dropped events across cycles so each cycle's summary
        // reports only the drops that happened *since* the last
        // drain, not the cumulative count.
        let mut last_dropped_snapshot: usize = 0;
        let callback = Arc::new(on_cycle_complete);

        // Precompute the combined ignore set (defaults + user-supplied
        // extras). Sharing via Arc means the callback closure and the
        // initial `setup_watches` walk see identical semantics without
        // allocating on every event.
        let ignore_dirs: Arc<HashSet<String>> =
            Arc::new(build_ignore_set(&self.cfg.extra_ignore_dirs));

        // Set up the notify watcher.
        //
        // The `watcher` binding is load-bearing: `RecommendedWatcher` stops
        // emitting events the moment it is dropped, so it MUST stay alive for
        // the entire duration of the watch loop below. Do not narrow this
        // scope or rebind it without preserving its lifetime.
        //
        // The callback pushes events directly into the shared debouncer set
        // (no intermediate bounded MPSC). This avoids silent drops under
        // burst load: a `HashSet::insert` on the debouncer's mutex is bounded
        // in cost, and the debouncer naturally deduplicates paths.
        //
        // We capture BOTH the raw and the canonicalized repo root and hand
        // both to `should_ignore_path`. Notify's path form is
        // backend-dependent: macOS FSEvents emits canonicalized paths
        // (`/private/var/...`), while Linux inotify emits paths rooted at
        // whatever path we passed to `.watch()` — which is the raw
        // (possibly symlinked) `self.cfg.repo_root`. Using only the
        // canonical form here silently broke the ignore filter for
        // symlinked working trees on Linux: every `target/`, `node_modules/`,
        // `.git/` event would pass through because `strip_prefix` against
        // the canonical form failed. Passing both forms lets the helper
        // try the raw form first and fall back to canonical.
        //
        // The ignore filter still runs on the callback hot path even
        // though `setup_watches` now prunes ignored subtrees at watch
        // registration time: macOS FSEvents is a process-wide stream and
        // can still deliver paths from subdirectories the walker didn't
        // register, and subtree registration doesn't cover events
        // generated by a `mv target/foo src/foo` *inside* an ignored
        // subtree on backends that do descend into them.
        let debouncer_for_callback = debouncer.clone();
        let repo_root_raw_for_callback = self.cfg.repo_root.clone();
        let repo_root_canonical_for_callback = repo_root_canonical.clone();
        let ignore_for_callback = ignore_dirs.clone();
        let mut watcher = RecommendedWatcher::new(
            move |res: Result<Event, notify::Error>| {
                // Notify can deliver `Err` for queue overflow, kernel-side
                // disconnects (Linux inotify watch budget), permission
                // failures on a newly created subdirectory, or symlink
                // resolution errors. Previously this branch dropped them
                // silently, which left the user staring at a "watching..."
                // banner while the loop was effectively dead. Surface them
                // as warnings so the failure mode matches every other
                // silent-failure hole this PR has been plugging.
                let event = match res {
                    Ok(e) => e,
                    Err(e) => {
                        wrkflw_logging::warning(&format!(
                            "filesystem watcher error: {} — some change events may be missed",
                            e
                        ));
                        return;
                    }
                };
                if !is_relevant_event_kind(&event.kind) {
                    return;
                }
                for path in event.paths {
                    if should_ignore_path(
                        &path,
                        &repo_root_raw_for_callback,
                        &repo_root_canonical_for_callback,
                        &ignore_for_callback,
                    ) {
                        continue;
                    }
                    debouncer_for_callback.add_event(path);
                }
            },
            notify::Config::default(),
        )?;

        // Register watches subtree-by-subtree, skipping ignored
        // directories (target/, node_modules/, .git/, user-supplied
        // extras). On Linux this keeps the inotify watch budget
        // bounded: a single recursive watch on the repo root used to
        // register one watch per directory for every ignored subtree,
        // and monorepos with deep `target/` or `node_modules/` trees
        // could blow past `fs.inotify.max_user_watches` (default 8192)
        // before the first file edit.
        setup_watches(&mut watcher, &self.cfg.repo_root, &ignore_dirs)?;

        wrkflw_logging::info(&format!(
            "Watching {} for changes (event={}, debounce={}ms)",
            self.cfg.repo_root.display(),
            self.cfg.event_name,
            self.cfg.debounce_duration.as_millis()
        ));

        let notify = debouncer.notifier();

        // Cache of compiled trigger configs keyed by workflow file path.
        // Invalidated only when a workflow file appears in the current
        // cycle's `changed_paths` set, so glob compilation doesn't repeat on
        // every file-save elsewhere in the repo.
        //
        // Each entry stores the workflow's canonical path alongside the
        // compiled config so the refresh loop can do change-set lookups
        // without re-running `canonicalize_allowing_missing` for every
        // workflow on every cycle. The previous implementation called
        // canonicalize once per workflow per cycle — on a network mount or
        // a deep workflows directory that adds noticeable latency between
        // debounce drain and execution.
        let mut trigger_cache: HashMap<PathBuf, TriggerCacheEntry> = HashMap::new();
        let mut workflow_files = initial_workflow_files;

        loop {
            // Observe cancellation at the top of every cycle. Fast-path
            // check avoids a `select!` alloc when the signal is a
            // `ShutdownSignal::never` handle (the CLI case).
            if shutdown.is_triggered() {
                wrkflw_logging::info("Watch loop received shutdown signal; exiting.");
                return Ok(());
            }

            // Only block on notification if no events are already pending.
            // This prevents losing events that accumulated during workflow execution.
            //
            // `tokio::select!` between the debouncer's notifier and the
            // shutdown signal so cancellation interrupts the idle wait
            // without having to wait for the next fs event. The branch
            // order is biased by the macro; we re-check `is_triggered`
            // on the next loop iteration regardless.
            if !debouncer.has_pending() {
                tokio::select! {
                    _ = notify.notified() => {}
                    _ = shutdown.wait() => {
                        wrkflw_logging::info("Watch loop received shutdown signal; exiting.");
                        return Ok(());
                    }
                }
            }

            let changed_paths = debouncer.drain().await;
            if changed_paths.is_empty() {
                continue;
            }

            // Re-collect workflow files so newly added .yml files are picked up.
            // Surface per-cycle rescan errors at debug level: the watch loop
            // *must* continue on transient failures (e.g. a temporary empty
            // directory, a fleeting I/O glitch), but silently falling back to
            // a stale `workflow_files` snapshot with no diagnostic is the
            // exact silent-skip pattern the rest of this PR has been plugging.
            // Debug keeps non-verbose runs quiet while still leaving a trail
            // for `--verbose` / debug-level operators.
            match self.collect_workflow_files().await {
                Ok(refreshed) => workflow_files = refreshed,
                Err(e) => {
                    wrkflw_logging::debug(&format!(
                        "workflow rescan failed, reusing {} cached path(s): {}",
                        workflow_files.len(),
                        e
                    ));
                }
            }

            trigger_cache = self
                .refresh_trigger_cache_async(trigger_cache, &workflow_files, &changed_paths)
                .await;

            // Build the borrowed view for evaluation.
            let configs_for_eval: Vec<&WorkflowTriggerConfig> = workflow_files
                .iter()
                .filter_map(|p| trigger_cache.get(p).map(|entry| &entry.config))
                .collect();

            let changed_files = self
                .canonicalize_changed_paths(&changed_paths, &repo_root_canonical)
                .await;

            if changed_files.is_empty() {
                if self.cfg.verbose {
                    wrkflw_logging::warning(&format!(
                        "Ignored {} change event(s): none resolved under repo root {}",
                        changed_paths.len(),
                        repo_root_canonical.display()
                    ));
                }
                continue;
            }

            // Snapshot the dropped-event counter and compute the
            // per-cycle delta before spawning the eval task. Reading
            // the counter AFTER drain catches any drops recorded
            // while the callback from the previous cycle was still
            // executing — we attribute those to the current cycle
            // so the user sees them at the first opportunity, not
            // the one after that.
            let dropped_now = debouncer.dropped_count();
            let dropped_this_cycle = dropped_now.saturating_sub(last_dropped_snapshot);
            last_dropped_snapshot = dropped_now;

            let mut event = self
                .evaluate_and_execute(&configs_for_eval, changed_files)
                .await;
            event.dropped_events = dropped_this_cycle;
            if dropped_this_cycle > 0 {
                let msg = format!(
                    "{} filesystem event(s) were dropped this cycle because the \
                     debouncer's pending set was saturated. This usually means a \
                     filesystem churn burst (cargo build, git checkout, formatter \
                     sweep) exceeded the configured cap. Raise --max-pending-events \
                     or reduce the source of churn if this keeps happening.",
                    dropped_this_cycle
                );
                wrkflw_logging::warning(&msg);
                event.warnings.push(msg);
            }

            // Fire-and-forget the callback so a slow reporter can't stall
            // the next cycle. Events that arrive during the callback still
            // accumulate in the debouncer and are processed on the next
            // round.
            //
            // Panic handling: a panicking callback used to vanish into
            // tokio's default panic reporter with no watch-loop signal —
            // exactly the silent-failure mode this PR is built to
            // prevent. We now await the blocking handle from a
            // supervisor task and surface the panic via
            // `wrkflw_logging::error` so the operator learns the
            // reporter is broken instead of wondering why "nothing
            // printed for the last N changes".
            let cb = callback.clone();
            let cb_handle = tokio::task::spawn_blocking(move || cb(event));
            tokio::spawn(async move {
                if let Err(join_err) = cb_handle.await {
                    if join_err.is_panic() {
                        wrkflw_logging::error(&format!(
                            "watch callback panicked: {} — watch loop continues, \
                             but the reporter may now be dropping events",
                            join_err
                        ));
                    }
                }
            });
        }
    }

    /// Return cached `(branch, tag)` if still fresh within
    /// [`GIT_STATE_CACHE_TTL`]; otherwise re-fetch both via concurrent
    /// git subprocess calls and update the cache.
    ///
    /// Errors out of git are propagated so the caller can build a
    /// [`WatchEvent`] with an `error` payload — the previous code
    /// path silently collapsed git failures to `branch: None`, which
    /// made every `branches:` filter deterministically reject and
    /// produced a session-long stream of "0 triggered" reports with
    /// no explanation.
    async fn cached_git_state(
        &self,
    ) -> Result<(Option<String>, Option<String>), wrkflw_trigger_filter::TriggerFilterError> {
        let ttl = self.cfg.trigger_filter.git_state_ttl;
        let cwd = Some(self.cfg.repo_root.as_path());
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
            let guard = match self.git_state.lock() {
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

        // Cache miss — fan out the two git calls concurrently, same
        // shape as `context_from_changed_files` used to do. Pass the
        // repo root explicitly so the subprocesses operate on the
        // same tree the watcher is looking at.
        let (branch_res, tag_res) = tokio::join!(
            wrkflw_trigger_filter::git::get_current_branch(cwd),
            wrkflw_trigger_filter::git::get_current_tag(cwd),
        );
        let branch = branch_res?;
        let tag = tag_res?;

        // Repopulate the cache. We accept that a racing task may
        // overwrite with its own fetch — both branches produce the
        // same value so late-writer-wins is safe, and we avoid the
        // cost of a compare-and-set dance.
        //
        // Capture HEAD mtime AFTER the git calls so the "mtime at
        // fetch time" we store is the one that was consistent with
        // the branch/tag values we just read. A checkout that
        // happens between the read and the store will show up as
        // an mtime mismatch on the very next call.
        let fetched_at = Instant::now();
        let head_mtime = wrkflw_trigger_filter::git::head_mtime(cwd);
        let mut guard = match self.git_state.lock() {
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

    /// Async wrapper around [`refresh_trigger_cache_blocking`] that moves
    /// the blocking file I/O onto a `spawn_blocking` thread. The watcher's
    /// main loop must call this rather than the sync helper directly —
    /// `load_trigger_config` reads + YAML-parses every workflow file
    /// serially, and on a network mount that latency multiplies into a
    /// reactor stall between debouncer drain and execution.
    ///
    /// Takes the cache by value and returns the updated cache so the
    /// closure owns its mutable state. The caller reassigns the result.
    async fn refresh_trigger_cache_async(
        &self,
        trigger_cache: HashMap<PathBuf, TriggerCacheEntry>,
        workflow_files: &[PathBuf],
        changed_paths: &[PathBuf],
    ) -> HashMap<PathBuf, TriggerCacheEntry> {
        // Clone the cache into the blocking task so that if the
        // closure panics, we can still hand the *previous* cache
        // back to the loop. Previously a panic returned `HashMap::new()`,
        // which forced the next cycle to re-parse every workflow —
        // an O(n) cache cliff visible on any monorepo the moment a
        // single workflow misbehaved. The cost of the extra clone is
        // bounded by the workflow count, which is small relative to
        // the parse work we're saving.
        let original = trigger_cache.clone();
        let mut working_copy = trigger_cache;
        let workflow_files = workflow_files.to_vec();
        let changed_paths = changed_paths.to_vec();
        let verbose = self.cfg.verbose;
        let tf_config = self.cfg.trigger_filter.clone();
        let result = tokio::task::spawn_blocking(move || {
            refresh_trigger_cache_blocking(
                &mut working_copy,
                &workflow_files,
                &changed_paths,
                verbose,
                &tf_config,
            );
            working_copy
        })
        .await;
        // A panic inside the blocking closure should not abort the
        // watch loop, nor should it force the next cycle to rebuild
        // from scratch. Fall back to the untouched prior cache so
        // the cycle continues against the last-known-good state.
        result.unwrap_or_else(|e| {
            wrkflw_logging::error(&format!(
                "Trigger cache refresh task panicked: {} — reusing {} previously-cached \
                 entries for the next cycle so the loop does not pay a full re-parse cliff",
                e,
                original.len()
            ));
            original
        })
    }

    /// Convert absolute change paths to repo-relative strings. Runs on a
    /// blocking thread because `canonicalize` is one `lstat` per component.
    ///
    /// **Deleted-file handling:** `canonicalize` fails for paths whose
    /// target no longer exists. Previously the fallback was the raw path,
    /// which could fail `strip_prefix` on macOS (`/private/var` vs `/var`)
    /// or symlinked trees, silently dropping deletions. We now walk back
    /// to the nearest canonicalizable ancestor and re-join the trailing
    /// components so deletions under `paths:` filters still propagate.
    ///
    /// **Path separator normalization:** on Windows, notify delivers
    /// paths with `\` separators and `PathBuf::to_string_lossy` preserves
    /// them, but `glob::Pattern` with `require_literal_separator: true`
    /// expects `/` — a `paths: ['src/**']` filter would fail to match
    /// `src\main.rs` and every Windows user would see "0 triggered". We
    /// canonicalize to `/` at the point of stringification so
    /// downstream `path_matcher` logic is platform-oblivious.
    async fn canonicalize_changed_paths(
        &self,
        changed_paths: &[PathBuf],
        repo_root_canonical: &Path,
    ) -> Vec<String> {
        let paths_for_canon = changed_paths.to_vec();
        let root_for_canon = repo_root_canonical.to_path_buf();
        let verbose = self.cfg.verbose;
        tokio::task::spawn_blocking(move || {
            let mut out = Vec::with_capacity(paths_for_canon.len());
            for p in &paths_for_canon {
                let canonical = canonicalize_allowing_missing(p);
                match canonical.strip_prefix(&root_for_canon) {
                    Ok(rel) => out.push(normalize_separators(&rel.to_string_lossy())),
                    Err(_) => {
                        // Notify is scoped to the repo root so this is
                        // unusual — symlinked target outside the tree,
                        // NFS oddity, or a notify backend bug. Surface
                        // the dropped path under verbose so the user
                        // doesn't have to guess why their `paths:`
                        // filter never fires.
                        if verbose {
                            wrkflw_logging::warning(&format!(
                                "Dropped change event {}: not under repo root {}",
                                p.display(),
                                root_for_canon.display()
                            ));
                        }
                    }
                }
            }
            out
        })
        .await
        .unwrap_or_else(|e| {
            // Mirror `refresh_trigger_cache_async`'s panic logging — a
            // silent `unwrap_or_default` here would mean a panicking
            // canonicalize task drops the entire change set for the
            // current cycle and the user sees "0 triggered" with no
            // explanation.
            wrkflw_logging::error(&format!(
                "Path canonicalization task panicked: {} — current cycle's change \
                 set is being dropped, watch loop continues",
                e
            ));
            Vec::new()
        })
    }

    /// Evaluate triggers for the given (already parsed) workflows against the
    /// current git state, then execute the matching workflows with bounded
    /// concurrency.
    ///
    /// **Degraded context handling.** If `context_from_changed_files` fails
    /// (e.g. transient git error), we no longer silently fall back to a
    /// `branch: None` context — that previously caused every `branches:`
    /// filter to deterministically reject for the rest of the session
    /// while the user just saw "0 triggered" with no explanation. We now
    /// short-circuit the cycle, attach the failure reason to the
    /// `WatchEvent`, and let the reporter callback surface it. The watch
    /// loop itself keeps running so the next event still gets a chance to
    /// build a healthy context.
    ///
    /// **Per-workflow panic isolation.** Each `wrkflw_executor::execute_workflow`
    /// future is wrapped in [`futures::FutureExt::catch_unwind`]. A panic
    /// inside one workflow's execution path used to propagate through
    /// `buffer_unordered` and abort the entire watch loop, killing the
    /// session for every workflow. Catching the unwind contains the panic
    /// to the offending workflow and lets the rest of the cycle complete.
    ///
    /// We do NOT use `tokio::spawn` for the per-workflow task: the
    /// executor's internal `dyn ContainerRuntime` is not `Sync`, so the
    /// `execute_workflow` future is not `Send`. `catch_unwind` works on
    /// the local future without requiring it to be sent across threads.
    /// `AssertUnwindSafe` is required because the captured config and
    /// path are not statically `UnwindSafe`; we accept that contract here
    /// because we discard all per-task state on panic and surface only a
    /// log line.
    async fn evaluate_and_execute(
        &self,
        configs: &[&WorkflowTriggerConfig],
        changed_files: Vec<String>,
    ) -> WatchEvent {
        // Skip the git context build entirely when there are no parseable
        // workflows. Otherwise every cycle pays for two `git` subprocess
        // calls (`get_current_branch`, `get_current_tag`) for nothing —
        // visible on a network mount, and entirely avoidable.
        if configs.is_empty() {
            return WatchEvent {
                changed_files,
                triggered_workflows: Vec::new(),
                skipped_workflows: Vec::new(),
                error: None,
                warnings: Vec::new(),
                dropped_events: 0,
            };
        }

        // Use the TTL-bounded git-state cache instead of shelling out
        // on every cycle. See [`GIT_STATE_CACHE_TTL`]'s doc for the
        // rationale — a file-save storm of 40 events was previously
        // 80 git subprocess spawns for a branch+tag pair that almost
        // never changes.
        let context = match self.cached_git_state().await {
            Ok((branch, tag)) => EventContext {
                event_name: self.cfg.event_name.clone(),
                branch,
                base_branch: self.cfg.base_branch.clone(),
                tag,
                changed_files: changed_files.clone(),
                // We observed the change set via notify events — even
                // an empty list (e.g. all events were filtered out as
                // irrelevant) is authoritative for this cycle, not a
                // "user forgot to pass --diff" situation. The
                // diagnostic layer uses this to avoid telling watch-
                // mode users to pass a CLI flag that does not exist
                // in their context.
                changed_files_explicit: true,
                // Stamp the activity type so workflows that gate on
                // `pull_request: { types: [opened, synchronize] }` can
                // actually match in watch mode. Without this, every
                // typed `pull_request` workflow is silently rejected
                // for "no activity type in context" — exactly the
                // silent-skip failure mode this PR is built to prevent.
                activity_type: self.cfg.activity_type.clone(),
                warnings: Vec::new(),
            },
            Err(e) => {
                let reason = format!("Failed to build event context: {}", e);
                wrkflw_logging::warning(&reason);
                return WatchEvent {
                    changed_files,
                    triggered_workflows: Vec::new(),
                    skipped_workflows: Vec::new(),
                    error: Some(reason),
                    warnings: Vec::new(),
                    dropped_events: 0,
                };
            }
        };

        let results = wrkflw_trigger_filter::filter_trigger_configs(configs, &context);

        let mut triggered = Vec::new();
        let mut skipped = Vec::new();
        let mut exec_futures = Vec::new();

        for result in &results {
            // Render TRIGGERED/SKIPPED labels as repo-relative paths
            // when possible. Absolute paths in the CLI output make it
            // hard to eyeball "which workflow fired" against the
            // familiar .github/workflows/ layout, and the noise scales
            // badly when a user has many workflows or a long repo root.
            let label = display_workflow_path(&result.workflow_path, &self.cfg.repo_root);

            if result.matches {
                triggered.push(label);

                let exec_config = self.cfg.execution.clone();
                let wf_path = result.workflow_path.clone();
                exec_futures.push(async move {
                    let log_path = wf_path.clone();
                    // `AssertUnwindSafe` + `catch_unwind` contains panics
                    // from inside the executor so a single rogue workflow
                    // cannot kill the watch loop. See the type-level docs
                    // above for why we use this instead of `tokio::spawn`.
                    let outcome =
                        AssertUnwindSafe(wrkflw_executor::execute_workflow(&wf_path, exec_config))
                            .catch_unwind()
                            .await;
                    match outcome {
                        Ok(Ok(exec_result)) => {
                            if exec_result.failure_details.is_some() {
                                wrkflw_logging::error(&format!(
                                    "Workflow {} failed",
                                    log_path.display()
                                ));
                            } else {
                                wrkflw_logging::info(&format!(
                                    "Workflow {} succeeded",
                                    log_path.display()
                                ));
                            }
                        }
                        Ok(Err(e)) => {
                            wrkflw_logging::error(&format!(
                                "Workflow {} error: {}",
                                log_path.display(),
                                e
                            ));
                        }
                        Err(_panic_payload) => {
                            // We deliberately discard the panic payload —
                            // recovering a `&str` from `Box<dyn Any>`
                            // works for the common case but the executor
                            // could panic with anything, and we'd rather
                            // give a consistent message than format a
                            // type name. The user can dig into executor
                            // logs for the actual panic message.
                            //
                            // Resource leak warning: a panic mid-execution
                            // bypasses the executor's normal cleanup path,
                            // so containers, tempdirs, named volumes, or
                            // child processes the workflow had spun up may
                            // still be alive. Surface that in the user log
                            // so a long-running watch session does not
                            // accumulate orphaned resources without anyone
                            // noticing.
                            wrkflw_logging::error(&format!(
                                "Workflow {} panicked during execution — \
                                 watch loop continues. Note: a panicking executor may have \
                                 left containers, tempdirs, or child processes uncleaned; \
                                 check `docker ps -a` (or your runtime's equivalent) if you \
                                 see resource buildup, and investigate the executor logs for \
                                 the actual panic.",
                                log_path.display()
                            ));
                        }
                    }
                });
            } else {
                skipped.push(label);
            }
        }

        // Execute triggered workflows with bounded concurrency. The
        // futures above are polled in place by `buffer_unordered` —
        // they are NOT wrapped in `tokio::spawn`. This is
        // load-bearing: spawning would detach them from the stream
        // and let the executor start an unbounded number of
        // workflows (each of which carries container / tempdir /
        // child-process state), defeating `max_concurrent_executions`.
        // Panics inside a workflow are contained by the
        // `AssertUnwindSafe + catch_unwind` wrapper around the
        // executor call, so a crashing workflow is logged and the
        // watch loop continues — without us ever needing `spawn` for
        // panic isolation.
        stream::iter(exec_futures)
            .buffer_unordered(self.cfg.max_concurrent_executions)
            .collect::<Vec<()>>()
            .await;

        WatchEvent {
            changed_files,
            triggered_workflows: triggered,
            skipped_workflows: skipped,
            error: None,
            warnings: Vec::new(),
            dropped_events: 0,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Build a minimal `WorkflowWatcher` over a tempdir for integration
    /// tests that need an ambient-runtime-friendly watcher (cache state,
    /// shutdown, end-to-end pipeline). Pure-function tests (ignore
    /// filter, event kind classification, path helpers, trigger cache
    /// invalidation) live in their dedicated submodules alongside
    /// the code they exercise.
    fn make_watcher_for(repo: &Path) -> WorkflowWatcher {
        let workflow_dir = repo.join(".github").join("workflows");
        std::fs::create_dir_all(&workflow_dir).expect("create workflow dir");
        let cfg = WatcherConfig::new(
            workflow_dir,
            repo.to_path_buf(),
            wrkflw_executor::ExecutionConfig {
                runtime_type: wrkflw_executor::RuntimeType::Emulation,
                verbose: false,
                preserve_containers_on_failure: false,
                secrets_config: None,
                show_action_messages: false,
                target_job: None,
            },
        );
        WorkflowWatcher::from_config(cfg)
    }

    /// Pattern test for the per-workflow panic isolation in
    /// `evaluate_and_execute`: wrapping a panicking future in
    /// `AssertUnwindSafe(...).catch_unwind()` must yield `Err(_)`
    /// rather than unwinding into the awaiter. This is the load-bearing
    /// property the watcher relies on so a single rogue workflow does
    /// not kill the entire watch loop.
    ///
    /// We use `catch_unwind` rather than `tokio::spawn` because the
    /// executor's `execute_workflow` future is not `Send` (it holds a
    /// `dyn ContainerRuntime` that is not `Sync`), so spawning is not
    /// available — see the type-level docs on `evaluate_and_execute`.
    #[tokio::test]
    async fn catch_unwind_isolates_panics_from_workflow_futures() {
        let result: Result<(), _> = AssertUnwindSafe(async {
            panic!("simulated executor panic");
        })
        .catch_unwind()
        .await;
        assert!(
            result.is_err(),
            "catch_unwind must classify a panicking future as Err so the watcher's \
             match arm can log + continue instead of unwinding the loop"
        );
    }

    #[tokio::test]
    async fn cached_git_state_reuses_within_ttl() {
        // Uses a real git tempdir so we don't have to stub the
        // subprocess layer. Two back-to-back calls must hit the
        // cache (second call shouldn't reshell out within 3s).
        use std::process::Command as StdCommand;
        let tmp = tempfile::TempDir::new().expect("tempdir");
        let repo = tmp.path();

        // Best-effort git init; if git is unavailable, just skip.
        let status = StdCommand::new("git")
            .args([
                "-C",
                repo.to_str().unwrap(),
                "init",
                "--initial-branch=main",
            ])
            .status();
        if !status.map(|s| s.success()).unwrap_or(false) {
            return;
        }
        for (k, v) in [("user.email", "t@t.t"), ("user.name", "t")] {
            StdCommand::new("git")
                .args(["-C", repo.to_str().unwrap(), "config", k, v])
                .status()
                .expect("git config");
        }
        std::fs::write(repo.join("a.txt"), "1").unwrap();
        StdCommand::new("git")
            .args(["-C", repo.to_str().unwrap(), "add", "a.txt"])
            .status()
            .expect("git add");
        StdCommand::new("git")
            .args([
                "-C",
                repo.to_str().unwrap(),
                "commit",
                "-m",
                "init",
                "--no-gpg-sign",
            ])
            .status()
            .expect("git commit");

        let watcher = make_watcher_for(repo);
        let first = watcher.cached_git_state().await.expect("first state");
        let fetched_at_first = {
            let guard = watcher.git_state.lock().unwrap();
            guard.as_ref().expect("cache populated").fetched_at
        };
        let second = watcher.cached_git_state().await.expect("second state");
        let fetched_at_second = {
            let guard = watcher.git_state.lock().unwrap();
            guard.as_ref().expect("cache still populated").fetched_at
        };
        assert_eq!(first, second);
        assert_eq!(
            fetched_at_first, fetched_at_second,
            "second call within TTL must not refresh the cache"
        );
    }

    /// Smoke test for the full notify → debounce → evaluate → callback
    /// pipeline. All other watcher tests exercise individual helpers
    /// (ignore filter, cache, debouncer). Without an end-to-end test,
    /// a `notify` backend regression (Linux inotify / macOS FSEvents /
    /// kqueue) could silently break the top-level loop while every
    /// unit test continues to pass.
    ///
    /// The test sets up a tempdir with a minimal git repo and a
    /// workflow that triggers on `paths: ['irrelevant/**']`, writes a
    /// file under `src/`, and asserts the callback is invoked with a
    /// `WatchEvent` whose `changed_files` includes the new file. The
    /// workflow is intentionally configured to NOT match the change,
    /// so no real executor work happens and the test doesn't depend
    /// on Docker / emulation wiring behaving correctly — we just care
    /// that the fs change traveled all the way from notify to the
    /// callback.
    #[tokio::test(flavor = "current_thread")]
    async fn end_to_end_pipeline_delivers_watch_event() {
        use std::process::Command as StdCommand;
        use std::sync::{Arc, Mutex as StdMutex};

        // Some CI environments don't have git; skip rather than flake.
        if StdCommand::new("git")
            .arg("--version")
            .status()
            .map(|s| !s.success())
            .unwrap_or(true)
        {
            return;
        }

        let tmp = tempfile::TempDir::new().expect("tempdir");
        let repo = tmp.path().to_path_buf();

        // Init a minimal repo so `cached_git_state` (called inside
        // `evaluate_and_execute`) has something to query. If init
        // fails (sandboxed CI, older git), skip.
        let init_status = StdCommand::new("git")
            .args([
                "-C",
                repo.to_str().unwrap(),
                "init",
                "--initial-branch=main",
            ])
            .status();
        if !init_status.map(|s| s.success()).unwrap_or(false) {
            return;
        }
        for (k, v) in [("user.email", "t@t.t"), ("user.name", "t")] {
            StdCommand::new("git")
                .args(["-C", repo.to_str().unwrap(), "config", k, v])
                .status()
                .expect("git config");
        }

        // Workflow triggers only on `irrelevant/**`, so the file we
        // touch below (`src/main.rs`) will be reported as changed but
        // no workflow will match — the executor is never invoked.
        let workflow_dir = repo.join(".github").join("workflows");
        std::fs::create_dir_all(&workflow_dir).expect("mkdir workflows");
        let wf_path = workflow_dir.join("ci.yml");
        std::fs::write(
            &wf_path,
            "name: ci\n\
             on:\n  push:\n    paths:\n      - 'irrelevant/**'\n\
             jobs:\n  build:\n    runs-on: ubuntu-latest\n    steps:\n      - run: echo hi\n",
        )
        .expect("write ci.yml");

        // `src/` MUST exist before the watcher starts so that
        // `setup_watches` registers the recursive watch over it.
        // The "known limitation" in `setup_watches` is that new
        // top-level directories created *after* startup are not
        // picked up until restart — creating `src/` here instead of
        // alongside the file write below ensures the recursive watch
        // is in place by the time we touch `src/main.rs`. Without
        // this ordering, macOS FSEvents / Linux inotify never see
        // the file-level event and the test flakes out at 5s.
        std::fs::create_dir_all(repo.join("src")).expect("mkdir src up front");

        // Commit so `get_current_branch` / `get_changed_files` have
        // something stable to query. Not strictly required (the
        // watcher tolerates git errors), but it keeps the WatchEvent's
        // `error` field `None` so the assertion is clean.
        StdCommand::new("git")
            .args(["-C", repo.to_str().unwrap(), "add", "."])
            .status()
            .expect("git add");
        StdCommand::new("git")
            .args([
                "-C",
                repo.to_str().unwrap(),
                "commit",
                "-m",
                "init",
                "--no-gpg-sign",
            ])
            .status()
            .expect("git commit");

        // Canonicalize early so assertions downstream compare against
        // the same form notify will deliver (macOS /private/var, etc.).
        let repo_canonical = std::fs::canonicalize(&repo).expect("canonicalize repo");

        let cfg = WatcherConfig::new(
            workflow_dir,
            repo_canonical.clone(),
            wrkflw_executor::ExecutionConfig {
                runtime_type: wrkflw_executor::RuntimeType::Emulation,
                verbose: false,
                preserve_containers_on_failure: false,
                secrets_config: None,
                show_action_messages: false,
                target_job: None,
            },
        )
        .with_debounce(Duration::from_millis(50));
        let watcher = WorkflowWatcher::from_config(cfg);

        // `std::sync::Mutex` (not the tokio async variant) so the
        // callback — which executes on a blocking thread — can lock
        // it without awaiting. The main test task and the callback
        // share state through the same mutex; the contention is
        // microscopic (one push per cycle) so the blocking lock is
        // effectively free.
        let events: Arc<StdMutex<Vec<WatchEvent>>> = Arc::new(StdMutex::new(Vec::new()));
        let events_for_cb = events.clone();

        let shutdown = ShutdownSignal::new();
        let shutdown_for_run = shutdown.clone();

        // `WorkflowWatcher::run` is `!Send` because the executor
        // future holds `dyn ContainerRuntime`, so plain `tokio::spawn`
        // rejects it. Drive the test inside a `LocalSet` on the
        // current-thread runtime and use `spawn_local`, which accepts
        // `!Send` futures and avoids the cross-runtime waker plumbing
        // that a separate std::thread + inner `current_thread`
        // runtime had trouble with.
        let events_outer = events.clone();
        let local = tokio::task::LocalSet::new();
        local
            .run_until(async move {
                let handle = tokio::task::spawn_local(async move {
                    watcher
                        .run(shutdown_for_run, move |ev| {
                            // `StdMutex::lock` from a blocking thread
                            // (where `spawn_blocking` inside `run()`
                            // dispatches this callback) is correct;
                            // the lock window is tiny and we're not
                            // on a reactor thread.
                            let mut guard = events_for_cb.lock().expect("events mutex");
                            guard.push(ev);
                        })
                        .await
                });

                // Give the watcher a beat to register its notify
                // watches before we start touching files. Without
                // this, macOS FSEvents occasionally loses the very
                // first event because the subscription isn't fully
                // live yet. 200ms is well below the overall 5-second
                // test budget and pragmatically enough in practice.
                tokio::time::sleep(Duration::from_millis(200)).await;

                // Trigger a change that the workflow's `paths:`
                // filter does NOT match. The watcher should still
                // report the changed file; no workflow should be
                // executed. `src/` was already created above, before
                // the watcher started, so this write lands inside an
                // actively-watched subtree.
                std::fs::write(repo_canonical.join("src").join("main.rs"), "fn main() {}\n")
                    .expect("write src/main.rs");

                // Poll until the callback records at least one event
                // or the hard timeout expires. A 5-second budget
                // gives notify room to deliver under load (CI, debug
                // builds, slow CI filesystems).
                let deadline = std::time::Instant::now() + Duration::from_secs(5);
                loop {
                    {
                        let guard = events_outer.lock().expect("events mutex");
                        if !guard.is_empty() {
                            break;
                        }
                    }
                    if std::time::Instant::now() >= deadline {
                        break;
                    }
                    tokio::time::sleep(Duration::from_millis(50)).await;
                }

                // Request clean shutdown and let the watcher task
                // finish. A 2-second budget is plenty once the
                // `tokio::select!` inside `run()` observes the
                // shutdown wait future.
                shutdown.trigger();
                let _ = tokio::time::timeout(Duration::from_secs(2), handle).await;
            })
            .await;

        let guard = events.lock().expect("events mutex");
        assert!(
            !guard.is_empty(),
            "callback must receive at least one WatchEvent end-to-end"
        );
        let first = &guard[0];
        assert!(
            first
                .changed_files
                .iter()
                .any(|f| f.ends_with("src/main.rs")),
            "changed_files must include the touched file, got {:?}",
            first.changed_files
        );
        assert!(
            first.triggered_workflows.is_empty(),
            "non-matching change must not fire any workflow, got {:?}",
            first.triggered_workflows
        );
    }

    #[tokio::test(flavor = "current_thread")]
    async fn run_returns_when_shutdown_signal_is_triggered_before_any_event() {
        // Regression: the shutdown path must interrupt the idle
        // `notify.notified().await` branch inside `run()`. Without
        // the `tokio::select!` against `shutdown.wait()`, triggering
        // shutdown on an idle watcher would park forever until the
        // next fs event arrived — which might never happen.
        //
        // `run()` is `!Send` because the executor future holds a
        // `dyn ContainerRuntime`, so plain `tokio::spawn` rejects it.
        // We drive the whole test inside a `LocalSet` on a
        // current-thread runtime instead: `spawn_local` permits
        // `!Send` futures, and everything runs on the same reactor
        // so cross-runtime waker plumbing (the previous bug that
        // hung the test when the watcher sat on its own
        // `current_thread` runtime in a `std::thread`) is avoided.
        let tmp = tempfile::TempDir::new().expect("tempdir");
        let repo = tmp.path().to_path_buf();
        std::fs::create_dir_all(repo.join(".github").join("workflows")).expect("mkdir");
        std::fs::write(
            repo.join(".github/workflows/ci.yml"),
            "name: ci\non: push\njobs:\n  b:\n    runs-on: ubuntu-latest\n    steps:\n      - run: echo hi\n",
        )
        .expect("write");

        let repo_canonical = std::fs::canonicalize(&repo).expect("canonicalize");
        let cfg = WatcherConfig::new(
            repo_canonical.join(".github").join("workflows"),
            repo_canonical,
            wrkflw_executor::ExecutionConfig {
                runtime_type: wrkflw_executor::RuntimeType::Emulation,
                verbose: false,
                preserve_containers_on_failure: false,
                secrets_config: None,
                show_action_messages: false,
                target_job: None,
            },
        );
        let watcher = WorkflowWatcher::from_config(cfg);

        let shutdown = ShutdownSignal::new();
        let shutdown_clone = shutdown.clone();

        let local = tokio::task::LocalSet::new();
        local
            .run_until(async move {
                let handle =
                    tokio::task::spawn_local(
                        async move { watcher.run(shutdown_clone, |_| {}).await },
                    );

                // Let the loop enter its idle wait.
                tokio::time::sleep(Duration::from_millis(100)).await;
                shutdown.trigger();

                let result = tokio::time::timeout(Duration::from_secs(2), handle).await;
                match result {
                    Ok(Ok(Ok(()))) => {}
                    Ok(Ok(Err(e))) => panic!("watcher errored instead of returning Ok: {}", e),
                    Ok(Err(join_err)) => panic!("watch task join failed: {}", join_err),
                    Err(_) => panic!("shutdown did not interrupt the idle watch loop within 2s"),
                }
            })
            .await;
    }
}
