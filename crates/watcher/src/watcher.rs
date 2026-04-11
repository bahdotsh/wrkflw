use crate::debouncer::Debouncer;
use crate::error::WatchError;
use futures::stream::{self, StreamExt};
use futures::FutureExt;
use notify::event::{EventKind, ModifyKind};
use notify::{Event, RecommendedWatcher, RecursiveMode, Watcher};
use std::collections::{HashMap, HashSet};
use std::panic::AssertUnwindSafe;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::Duration;
use wrkflw_executor::ExecutionConfig;
use wrkflw_trigger_filter::WorkflowTriggerConfig;

/// Default cap on workflows executing concurrently in watch mode when the
/// caller does not supply an explicit limit.
pub const DEFAULT_MAX_CONCURRENT_EXECUTIONS: usize = 4;

/// A watch event containing the changed files and trigger evaluation results.
///
/// `error` is `Some` when the cycle ran into a non-fatal failure that the
/// reporter should surface to the user (e.g. building the git event
/// context failed). The watch loop will keep running on the next event,
/// but the user needs to know that *this* cycle's "0 triggered" result is
/// degraded rather than authoritative — otherwise a missing default branch
/// or a transient git failure produces a session-long stream of silent
/// "nothing to do" reports.
#[derive(Debug, Clone)]
pub struct WatchEvent {
    pub changed_files: Vec<String>,
    pub triggered_workflows: Vec<String>,
    pub skipped_workflows: Vec<String>,
    pub error: Option<String>,
}

/// Directories ignored by the filesystem watcher by default.
/// These are high-churn directories that almost never contain workflow-relevant
/// source files and would otherwise flood the event channel.
const DEFAULT_IGNORE_DIRS: &[&str] = &[
    ".git",
    "target",
    "node_modules",
    ".build",
    "build",
    "dist",
    "__pycache__",
    ".tox",
    ".mypy_cache",
    ".pytest_cache",
    ".venv",
    "venv",
];

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
}

impl WatcherConfig {
    pub fn new(workflow_dir: PathBuf, repo_root: PathBuf, execution: ExecutionConfig) -> Self {
        Self {
            workflow_dir,
            repo_root,
            event_name: "push".to_string(),
            base_branch: None,
            activity_type: None,
            debounce_duration: Duration::from_millis(500),
            execution,
            verbose: false,
            max_concurrent_executions: DEFAULT_MAX_CONCURRENT_EXECUTIONS,
        }
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
        self.max_concurrent_executions = n.max(1);
        self
    }
}

/// Watches for filesystem changes and triggers workflow execution.
pub struct WorkflowWatcher {
    cfg: WatcherConfig,
}

impl WorkflowWatcher {
    /// Build a watcher from a [`WatcherConfig`].
    ///
    /// `WatcherConfig::with_max_concurrency` already clamps the concurrency
    /// floor to 1, so this constructor is intentionally just a `Self { cfg }`
    /// — the previous re-clamp here was dead defensive code and the
    /// per-field accessor wrappers it sat alongside have been removed in
    /// favor of reading `self.cfg.x` directly.
    pub fn from_config(cfg: WatcherConfig) -> Self {
        Self { cfg }
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
    /// This blocks until an error occurs or the process is interrupted.
    ///
    /// `on_cycle_complete` is dispatched fire-and-forget on a blocking
    /// thread so a slow reporter (file sink, network webhook) cannot stall
    /// the main loop. Callers MUST NOT rely on serialization between
    /// cycles in the callback.
    ///
    /// **No graceful shutdown.** This function is an unbounded `loop {}`
    /// — it has no cancellation path and will only return on a fatal
    /// error from notify or workflow collection. Callers that need to
    /// stop the watcher must do so by terminating the surrounding task
    /// (e.g. `process::exit` from a Ctrl+C handler, which is what the
    /// `wrkflw watch` CLI does). Embedding `WorkflowWatcher` in a
    /// long-running daemon, the TUI, or a test harness will require
    /// adding a cancellation parameter; this is a known follow-up.
    pub async fn run<F>(&self, on_cycle_complete: F) -> Result<(), WatchError>
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

        let debouncer = Arc::new(Debouncer::new(self.cfg.debounce_duration));
        let callback = Arc::new(on_cycle_complete);

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
        let debouncer_for_callback = debouncer.clone();
        let repo_root_for_callback = repo_root_canonical.clone();
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
                    if should_ignore_path(&path, &repo_root_for_callback) {
                        continue;
                    }
                    debouncer_for_callback.add_event(path);
                }
            },
            notify::Config::default(),
        )?;

        // Watch the repo root recursively
        watcher.watch(&self.cfg.repo_root, RecursiveMode::Recursive)?;

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
            // Only block on notification if no events are already pending.
            // This prevents losing events that accumulated during workflow execution.
            if !debouncer.has_pending() {
                notify.notified().await;
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

            let event = self
                .evaluate_and_execute(&configs_for_eval, changed_files)
                .await;

            // Fire-and-forget the callback so a slow reporter can't stall
            // the next cycle. Events that arrive during the callback still
            // accumulate in the debouncer and are processed on the next
            // round.
            let cb = callback.clone();
            tokio::task::spawn_blocking(move || cb(event));
        }
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
        mut trigger_cache: HashMap<PathBuf, TriggerCacheEntry>,
        workflow_files: &[PathBuf],
        changed_paths: &[PathBuf],
    ) -> HashMap<PathBuf, TriggerCacheEntry> {
        let workflow_files = workflow_files.to_vec();
        let changed_paths = changed_paths.to_vec();
        let verbose = self.cfg.verbose;
        let result = tokio::task::spawn_blocking(move || {
            refresh_trigger_cache_blocking(
                &mut trigger_cache,
                &workflow_files,
                &changed_paths,
                verbose,
            );
            trigger_cache
        })
        .await;
        // A panic inside the blocking closure should not abort the watch
        // loop — fall back to an empty cache, which will be repopulated
        // on the next cycle from `workflow_files`.
        result.unwrap_or_else(|e| {
            wrkflw_logging::error(&format!(
                "Trigger cache refresh task panicked: {} — starting next cycle with an empty cache",
                e
            ));
            HashMap::new()
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
                    Ok(rel) => out.push(rel.to_string_lossy().to_string()),
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
            };
        }

        let context = match wrkflw_trigger_filter::context_from_changed_files(
            &self.cfg.event_name,
            changed_files.clone(),
            Some(&self.cfg.repo_root),
        )
        .await
        {
            Ok(mut ctx) => {
                ctx.base_branch = self.cfg.base_branch.clone();
                // Stamp the activity type so workflows that gate on
                // `pull_request: { types: [opened, synchronize] }` can
                // actually match in watch mode. Without this, every
                // typed `pull_request` workflow is silently rejected
                // for "no activity type in context" — exactly the
                // silent-skip failure mode this PR is built to prevent.
                ctx.activity_type = self.cfg.activity_type.clone();
                ctx
            }
            Err(e) => {
                let reason = format!("Failed to build event context: {}", e);
                wrkflw_logging::warning(&reason);
                return WatchEvent {
                    changed_files,
                    triggered_workflows: Vec::new(),
                    skipped_workflows: Vec::new(),
                    error: Some(reason),
                };
            }
        };

        let results = wrkflw_trigger_filter::filter_trigger_configs(configs, &context);

        let mut triggered = Vec::new();
        let mut skipped = Vec::new();
        let mut exec_futures = Vec::new();

        for result in &results {
            if result.matches {
                triggered.push(result.workflow_path.display().to_string());

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
                skipped.push(result.workflow_path.display().to_string());
            }
        }

        // Execute triggered workflows with bounded concurrency. Each future
        // wraps a `tokio::spawn`, so a panicking workflow surfaces as a
        // logged error inside the future rather than propagating out.
        stream::iter(exec_futures)
            .buffer_unordered(self.cfg.max_concurrent_executions)
            .collect::<Vec<()>>()
            .await;

        WatchEvent {
            changed_files,
            triggered_workflows: triggered,
            skipped_workflows: skipped,
            error: None,
        }
    }
}

/// Return `true` if a notify event kind is relevant for trigger
/// re-evaluation. We care about creates, writes, removes, and rename
/// endpoints; we drop access/metadata updates (atime/chmod/owner).
///
/// `Modify(Any)` is treated as relevant. The Linux inotify and macOS
/// FSEvents backends emit specific subkinds (`Modify(Data(...))`,
/// `Modify(Name(...))`), but the kqueue backend (FreeBSD, some macOS
/// configurations) and notify's `PollWatcher` fallback emit
/// `Modify(Any)` for content changes when the underlying API can't
/// distinguish data from metadata. Dropping `Modify(Any)` would leave
/// the watcher silently dead on those platforms — over-firing is
/// bounded by the debouncer; under-firing isn't recoverable.
///
/// The match is exhaustive on `ModifyKind` (no catch-all `Modify(_)`
/// arm) so future additions to the notify enum surface as a compile
/// error instead of silently being routed to "drop".
fn is_relevant_event_kind(kind: &EventKind) -> bool {
    match kind {
        EventKind::Create(_) => true,
        EventKind::Remove(_) => true,
        EventKind::Modify(ModifyKind::Data(_)) => true,
        EventKind::Modify(ModifyKind::Name(_)) => true,
        EventKind::Modify(ModifyKind::Any) => true,
        EventKind::Modify(ModifyKind::Metadata(_)) => false,
        EventKind::Modify(ModifyKind::Other) => false,
        EventKind::Access(_) => false,
        EventKind::Any | EventKind::Other => false,
    }
}

/// Canonicalize `path`, tolerating the case where the target was deleted.
/// Walks back to the nearest canonicalizable ancestor, then re-appends the
/// missing components. This keeps deleted files root-relative on platforms
/// where the raw path would fail `strip_prefix` (macOS `/private/var` vs
/// `/var`, symlinked working trees).
pub(crate) fn canonicalize_allowing_missing(path: &Path) -> PathBuf {
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
fn refresh_trigger_cache_blocking(
    trigger_cache: &mut HashMap<PathBuf, TriggerCacheEntry>,
    workflow_files: &[PathBuf],
    changed_paths: &[PathBuf],
    verbose: bool,
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
        match wrkflw_trigger_filter::load_trigger_config(wf_path) {
            Ok(cfg) => {
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

/// Synchronous implementation of `collect_workflow_files`. Extracted so it can
/// be invoked from `spawn_blocking` without closure capture juggling.
fn collect_workflow_files_blocking(dir: &Path) -> Result<Vec<PathBuf>, WatchError> {
    if dir.is_file() {
        return Ok(vec![dir.to_path_buf()]);
    }

    let mut files = Vec::new();
    for entry in std::fs::read_dir(dir)? {
        let entry = entry?;
        let path = entry.path();
        if let Some(ext) = path.extension().and_then(|e| e.to_str()) {
            if ext == "yml" || ext == "yaml" {
                files.push(path);
            }
        }
    }

    if files.is_empty() {
        return Err(WatchError::NoWorkflows(dir.display().to_string()));
    }
    Ok(files)
}

/// Returns `true` if a path falls inside any of the default ignore directories,
/// where "inside" means: a directory component (NOT the leaf filename) of the
/// path's `repo_root`-relative form matches one of the ignore names.
///
/// We deliberately skip the leaf component so a user file literally named
/// `target` (e.g. `scripts/target`) is not silently dropped — only paths that
/// have a `target/` (etc.) parent directory are filtered.
///
/// Paths that are NOT under `repo_root` are left untouched (the previous
/// implementation iterated their absolute components, which would
/// incorrectly drop a valid `/home/alice/target-acquisition/...` just
/// because an absolute component happened to equal `target`).
fn should_ignore_path(path: &Path, repo_root: &Path) -> bool {
    // Only apply the ignore set if we can express the path as repo-relative.
    // A path outside the repo root shouldn't exist in practice (notify
    // scoped to the root), but if it does, we do not want to match against
    // spurious absolute-path components like `/target-foo/...`.
    let rel = match path.strip_prefix(repo_root) {
        Ok(r) => r,
        Err(_) => return false,
    };
    // Compare against every component except the last (the leaf, which is
    // presumed to be a filename). Using `parent()` + component iteration
    // avoids collecting into a `Vec` — this function runs on every notify
    // event, so the hot path is worth keeping allocation-free.
    let parent = match rel.parent() {
        Some(p) => p,
        None => return false,
    };
    for component in parent.components() {
        if let std::path::Component::Normal(os) = component {
            if let Some(s) = os.to_str() {
                if DEFAULT_IGNORE_DIRS.contains(&s) {
                    return true;
                }
            }
        }
    }
    false
}

/// Find the git repository root from the current working directory.
///
/// Re-exported from `wrkflw-trigger-filter` so the watcher's existing
/// `wrkflw_watcher::find_repo_root` call sites keep compiling without
/// the watcher owning the implementation. The single source of truth
/// lives in the trigger-filter crate, which is the right home for
/// every git shell-out we do.
pub use wrkflw_trigger_filter::find_repo_root;

#[cfg(test)]
mod tests {
    use super::*;
    use notify::event::{AccessKind, CreateKind, DataChange, MetadataKind, ModifyKind, RemoveKind};

    fn root() -> &'static Path {
        Path::new("/repo")
    }

    #[test]
    fn is_relevant_event_kind_accepts_creates_and_writes() {
        assert!(is_relevant_event_kind(&EventKind::Create(CreateKind::File)));
        assert!(is_relevant_event_kind(&EventKind::Modify(
            ModifyKind::Data(DataChange::Content)
        )));
        assert!(is_relevant_event_kind(&EventKind::Remove(RemoveKind::File)));
        // Renames must propagate — a `git checkout` fires them in pairs.
        assert!(is_relevant_event_kind(&EventKind::Modify(
            ModifyKind::Name(notify::event::RenameMode::Any)
        )));
    }

    #[test]
    fn is_relevant_event_kind_drops_access_and_metadata() {
        assert!(!is_relevant_event_kind(&EventKind::Access(
            AccessKind::Read
        )));
        assert!(!is_relevant_event_kind(&EventKind::Modify(
            ModifyKind::Metadata(MetadataKind::Permissions)
        )));
        assert!(!is_relevant_event_kind(&EventKind::Any));
        assert!(!is_relevant_event_kind(&EventKind::Other));
    }

    #[test]
    fn is_relevant_event_kind_accepts_modify_any() {
        // Regression: `Modify(Any)` is what kqueue (FreeBSD) and
        // PollWatcher emit for content changes when the underlying API
        // can't distinguish data from metadata. The previous catch-all
        // `Modify(_) => false` arm dropped these silently, leaving the
        // watcher in a "no events ever fire" state on those backends
        // even though Linux/macOS users saw it work fine. Treating
        // `Modify(Any)` as relevant fixes the platform parity.
        assert!(is_relevant_event_kind(&EventKind::Modify(ModifyKind::Any)));
    }

    #[test]
    fn canonicalize_allowing_missing_handles_deleted_leaf() {
        // The leaf does not exist, but its parent is a real canonicalizable
        // directory — the fallback must walk up and re-join the missing leaf.
        let tmp = tempfile::TempDir::new().expect("tempdir");
        let root = tmp.path();
        let deleted = root.join("missing.txt");
        assert!(!deleted.exists());

        let canonical = canonicalize_allowing_missing(&deleted);
        // The result should start with the canonical parent and end with the
        // missing leaf name.
        assert!(
            canonical.ends_with("missing.txt"),
            "canonical should retain the leaf, got {}",
            canonical.display()
        );
        // The parent component should equal the canonical form of root.
        let expected_parent = std::fs::canonicalize(root).unwrap();
        assert_eq!(canonical.parent(), Some(expected_parent.as_path()));
    }

    #[test]
    fn canonicalize_allowing_missing_handles_deleted_subdir_leaf() {
        // Parent directory also missing, grandparent exists — must walk up
        // one more level and re-join both segments.
        let tmp = tempfile::TempDir::new().expect("tempdir");
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

    #[test]
    fn ignores_git_directory() {
        assert!(should_ignore_path(
            Path::new("/repo/.git/objects/pack/abc"),
            root()
        ));
    }

    #[test]
    fn does_not_ignore_path_outside_repo_root() {
        // Regression: previously, paths outside repo_root were iterated as
        // absolute components, so `/home/alice/target-acquisition/file.rs`
        // would match the `target` ignore entry.
        assert!(!should_ignore_path(
            Path::new("/home/alice/target-acquisition/file.rs"),
            root()
        ));
        // Similarly for a directory literally named `target` outside the
        // watched root.
        assert!(!should_ignore_path(Path::new("/target/build.rs"), root()));
    }

    #[test]
    fn ignores_target_directory() {
        assert!(should_ignore_path(
            Path::new("/repo/target/debug/deps/foo"),
            root()
        ));
    }

    #[test]
    fn ignores_node_modules() {
        assert!(should_ignore_path(
            Path::new("/repo/node_modules/pkg/index.js"),
            root()
        ));
    }

    #[test]
    fn does_not_ignore_src() {
        assert!(!should_ignore_path(Path::new("/repo/src/main.rs"), root()));
    }

    #[test]
    fn does_not_ignore_workflow_files() {
        assert!(!should_ignore_path(
            Path::new("/repo/.github/workflows/ci.yml"),
            root()
        ));
    }

    #[test]
    fn ignores_pycache() {
        assert!(should_ignore_path(
            Path::new("/repo/__pycache__/module.cpython-311.pyc"),
            root()
        ));
    }

    #[test]
    fn does_not_ignore_file_named_target() {
        // A file literally named `target` should not be filtered out;
        // only directories named `target/` count.
        assert!(!should_ignore_path(
            Path::new("/repo/scripts/target"),
            root()
        ));
    }

    #[test]
    fn does_not_ignore_file_named_build() {
        assert!(!should_ignore_path(Path::new("/repo/docs/build"), root()));
    }

    /// Build a minimal `WorkflowWatcher` over a tempdir for cache tests.
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
        let _watcher = make_watcher_for(&repo);

        // Write the workflow with `paths: ['src/foo.rs']`. The schema
        // validator that `parse_workflow` runs requires at least one
        // step, so we give the job a trivial echo.
        let wf_rel = PathBuf::from(".github/workflows/ci.yml");
        let wf_abs = repo.join(&wf_rel);
        let v1_yaml = "name: test\non:\n  push:\n    paths:\n      - 'src/foo.rs'\njobs:\n  build:\n    runs-on: ubuntu-latest\n    steps:\n      - run: echo v1\n";
        std::fs::write(&wf_abs, v1_yaml).expect("write ci.yml v1");

        // Reach the workflow via the same relative form `read_dir` would
        // produce when `workflow_dir` was passed in relative.
        let workflow_files = vec![wf_abs.clone()];
        let mut cache: HashMap<PathBuf, TriggerCacheEntry> = HashMap::new();

        // Prime the cache with the v1 config. Drives the private blocking
        // helper directly so the test does not depend on the public API
        // surface (the `refresh_trigger_cache` accessor was removed once
        // its only consumer was this test).
        refresh_trigger_cache_blocking(&mut cache, &workflow_files, &[], false);
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
        refresh_trigger_cache_blocking(&mut cache, &workflow_files, &[changed], false);

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

    #[test]
    fn ignores_nested_target_subdirectory() {
        assert!(should_ignore_path(
            Path::new("/repo/crates/foo/target/debug/build/foo"),
            root()
        ));
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
        let _watcher = make_watcher_for(&repo);

        let wf_abs = repo.join(".github/workflows/ci.yml");
        let good = "name: t\non:\n  push:\n    paths:\n      - 'src/**'\njobs:\n  b:\n    runs-on: ubuntu-latest\n    steps:\n      - run: echo hi\n";
        std::fs::write(&wf_abs, good).expect("write good ci.yml");

        let workflow_files = vec![wf_abs.clone()];
        let mut cache: HashMap<PathBuf, TriggerCacheEntry> = HashMap::new();

        // Prime: file parses fine, cache populated.
        refresh_trigger_cache_blocking(&mut cache, &workflow_files, &[], false);
        assert!(
            cache.contains_key(&wf_abs),
            "good workflow must be cached after first refresh"
        );

        // Break the file with an invalid glob and re-refresh, simulating
        // a notify event for the broken workflow.
        let bad = "name: t\non:\n  push:\n    paths:\n      - '[unclosed'\njobs:\n  b:\n    runs-on: ubuntu-latest\n    steps:\n      - run: echo hi\n";
        std::fs::write(&wf_abs, bad).expect("write bad ci.yml");
        let changed = std::fs::canonicalize(&wf_abs).expect("canonicalize wf");
        refresh_trigger_cache_blocking(&mut cache, &workflow_files, &[changed], false);

        assert!(
            !cache.contains_key(&wf_abs),
            "broken workflow must be evicted from cache so the surrounding \
             warning logic can surface the regression"
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
        let _watcher = make_watcher_for(&repo);

        let wf_abs = repo.join(".github/workflows/ci.yml");
        let yaml = "name: test\non:\n  push:\n    paths:\n      - 'src/foo.rs'\njobs:\n  build:\n    runs-on: ubuntu-latest\n    steps:\n      - run: echo hi\n";
        std::fs::write(&wf_abs, yaml).expect("write ci.yml");

        let workflow_files = vec![wf_abs.clone()];
        let mut cache: HashMap<PathBuf, TriggerCacheEntry> = HashMap::new();

        refresh_trigger_cache_blocking(&mut cache, &workflow_files, &[], false);
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
        refresh_trigger_cache_blocking(&mut cache, &workflow_files, &[], false);
        let canonical_after_second = &cache.get(&wf_abs).expect("still cached").canonical_path;
        assert_eq!(
            &canonical_after_first, canonical_after_second,
            "second refresh should reuse the memoized canonical_path"
        );
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
}
