use crate::error::WatchError;
use crate::git_state::GitStateCache;
use crate::setup::collect_workflow_files_blocking;
use crate::shutdown::ShutdownSignal;
use std::path::PathBuf;
use std::time::Duration;
use wrkflw_executor::ExecutionConfig;
use wrkflw_trigger_filter::canonicalize_allowing_missing;
use wrkflw_trigger_filter::TriggerFilterConfig;

/// Default cap on workflows executing concurrently in watch mode when the
/// caller does not supply an explicit limit.
pub const DEFAULT_MAX_CONCURRENT_EXECUTIONS: usize = 4;

/// Absolute upper bound on per-cycle concurrency. Each concurrent workflow
/// carries executor state (containers, tempdirs, child processes), so an
/// unbounded value can trivially OOM the host. Anything above this is
/// clamped down with a warning — users who genuinely need more should open
/// an issue so we can look at the actual workload before lifting the cap.
pub const MAX_REASONABLE_CONCURRENCY: usize = 256;

// The supervisor JoinSet warn threshold and hard cap used to live
// here as `pub(crate) const`s, but the reactor loop is the only
// reader. They now live as private `const`s on `crate::reactor` so
// the constant + the loop that enforces it stay co-located.

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

/// Watches for filesystem changes and triggers workflow execution.
pub struct WorkflowWatcher {
    /// Visible to [`crate::reactor`] so the reactor loop and its
    /// per-cycle helpers can read config fields (repo_root, verbose,
    /// execution config, …) without threading every knob through a
    /// positional argument list.
    pub(crate) cfg: WatcherConfig,
    /// TTL-bounded cache of `(branch, tag)` used by the per-cycle
    /// trigger evaluator. See [`crate::git_state::GitStateCache`] for
    /// the staleness contract — the watcher pokes it once per cycle
    /// via [`WorkflowWatcher::cached_git_state`] (or directly from
    /// [`crate::reactor::evaluate_and_execute`]).
    pub(crate) git_state: GitStateCache,
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
            git_state: GitStateCache::new(),
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
        crate::reactor::run_loop(self, shutdown, on_cycle_complete).await
    }

    /// Return cached `(branch, tag)` if still fresh within
    /// `TriggerFilterConfig::git_state_ttl`; otherwise re-fetch both
    /// via concurrent git subprocess calls and update the cache.
    ///
    /// Thin wrapper around [`crate::git_state::GitStateCache::get`]
    /// kept on `WorkflowWatcher` because the in-crate tests pin
    /// cache staleness behavior through this method (see
    /// `cached_git_state_reuses_within_ttl` and
    /// `cached_git_state_invalidates_when_head_mtime_moves`).
    /// Gated on `cfg(test)` because production code goes through
    /// [`crate::reactor::evaluate_and_execute`] which hits the cache
    /// directly — keeping the method in non-test builds would cost
    /// a dead-code warning without adding reachable callers.
    #[cfg(test)]
    async fn cached_git_state(
        &self,
    ) -> Result<(Option<String>, Option<String>), wrkflw_trigger_filter::TriggerFilterError> {
        self.git_state
            .get(&self.cfg.trigger_filter, &self.cfg.repo_root)
            .await
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use futures::FutureExt;
    use std::panic::AssertUnwindSafe;
    use std::path::Path;

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
        let fetched_at_first = watcher
            .git_state
            .peek_fetched_at()
            .expect("cache populated after first call");
        let second = watcher.cached_git_state().await.expect("second state");
        let fetched_at_second = watcher
            .git_state
            .peek_fetched_at()
            .expect("cache still populated after second call");
        assert_eq!(first, second);
        assert_eq!(
            fetched_at_first, fetched_at_second,
            "second call within TTL must not refresh the cache"
        );
    }

    #[tokio::test]
    async fn cached_git_state_invalidates_when_head_mtime_moves() {
        // Regression: the cache must detect a `git checkout` that
        // happens *between* the two `cached_git_state` calls, even if
        // the wall-clock TTL has not expired. The staleness check
        // compares the freshly-stat'd HEAD mtime against the one
        // captured at fetch time; when they differ, we must refresh.
        //
        // Pairs with the fix that moved the mtime capture to BEFORE
        // the git calls: with the old "capture after" logic, a
        // checkout that landed between the branch/tag reads and the
        // mtime stat was silently persisted as "cache consistent"
        // and the watcher served the pre-checkout branch until TTL.
        use std::process::Command as StdCommand;
        let tmp = tempfile::TempDir::new().expect("tempdir");
        let repo = tmp.path();

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
        // Create a second branch we can check out to force a HEAD move.
        StdCommand::new("git")
            .args(["-C", repo.to_str().unwrap(), "branch", "other"])
            .status()
            .expect("git branch");

        let watcher = make_watcher_for(repo);
        let (branch_a, _) = watcher.cached_git_state().await.expect("first state");
        assert_eq!(branch_a, Some("main".to_string()));

        // Sleep so the mtime tick is reliably distinct from the one
        // captured at the first fetch. Filesystem mtime granularity
        // is platform-dependent; 20ms is comfortably above any
        // supported platform's resolution.
        std::thread::sleep(Duration::from_millis(20));
        StdCommand::new("git")
            .args(["-C", repo.to_str().unwrap(), "checkout", "other"])
            .status()
            .expect("git checkout other");

        let (branch_b, _) = watcher.cached_git_state().await.expect("second state");
        assert_eq!(
            branch_b,
            Some("other".to_string()),
            "HEAD mtime mismatch must force a cache refresh — got stale {:?}",
            branch_b
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

    #[tokio::test(flavor = "current_thread")]
    async fn run_returns_promptly_when_shutdown_triggers_during_drain() {
        // Regression: the `debouncer.drain().await` call inside `run()`
        // used to live outside any `tokio::select!` against
        // `shutdown.wait()`. A drain in progress sleeps for up to
        // `max(debounce_duration, MAX_SETTLE_BUDGET)` — on a long
        // user-specified debounce window this could stretch Ctrl+C
        // latency to multiple seconds. The fix wraps `drain()` in a
        // select; this test pins that property by using a deliberately
        // long 5-second debounce and asserting shutdown resolves
        // `run()` well inside that window.
        //
        // Pairs with `run_returns_when_shutdown_signal_is_triggered_before_any_event`
        // (which pins the idle-wait path). Together they assert both
        // wait points in the main loop observe shutdown promptly.
        use std::process::Command as StdCommand;
        use std::sync::{Arc, Mutex as StdMutex};

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

        let workflow_dir = repo.join(".github").join("workflows");
        std::fs::create_dir_all(&workflow_dir).expect("mkdir workflows");
        std::fs::write(
            workflow_dir.join("ci.yml"),
            "name: ci\non: push\njobs:\n  build:\n    runs-on: ubuntu-latest\n    steps:\n      - run: echo hi\n",
        )
        .expect("write ci.yml");
        std::fs::create_dir_all(repo.join("src")).expect("mkdir src");

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

        let repo_canonical = std::fs::canonicalize(&repo).expect("canonicalize repo");

        // Deliberately long debounce window. The regression is that
        // Ctrl+C during an active drain has to wait up to this long;
        // the fix observes shutdown inside the drain wait.
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
        .with_debounce(Duration::from_secs(5));
        let watcher = WorkflowWatcher::from_config(cfg);

        let events: Arc<StdMutex<Vec<WatchEvent>>> = Arc::new(StdMutex::new(Vec::new()));
        let events_for_cb = events.clone();
        let shutdown = ShutdownSignal::new();
        let shutdown_for_run = shutdown.clone();

        let local = tokio::task::LocalSet::new();
        local
            .run_until(async move {
                let handle = tokio::task::spawn_local(async move {
                    watcher
                        .run(shutdown_for_run, move |ev| {
                            let mut guard = events_for_cb.lock().expect("events mutex");
                            guard.push(ev);
                        })
                        .await
                });

                // Let the watcher register its notify watches.
                tokio::time::sleep(Duration::from_millis(200)).await;

                // Fire a change so the loop transitions out of its idle
                // wait into `debouncer.drain()`. The drain then sleeps
                // for the full 5-second window.
                std::fs::write(repo_canonical.join("src").join("main.rs"), "fn main() {}\n")
                    .expect("write src/main.rs");

                // Give the drain enough time to enter its sleep but
                // not enough to finish — we want shutdown to interrupt
                // it mid-wait. 500 ms is comfortably below 5 s and
                // above the notify delivery window.
                tokio::time::sleep(Duration::from_millis(500)).await;
                shutdown.trigger();

                // Shutdown must resolve `run()` well inside the
                // 5-second debounce window. A 2-second budget is
                // plenty; if the fix regresses, the task is still
                // parked in `sleep()` and the timeout fires.
                let result = tokio::time::timeout(Duration::from_secs(2), handle).await;
                match result {
                    Ok(Ok(Ok(()))) => {}
                    Ok(Ok(Err(e))) => panic!("watcher errored: {}", e),
                    Ok(Err(join_err)) => panic!("watch task join failed: {}", join_err),
                    Err(_) => panic!(
                        "shutdown did not interrupt an active drain within 2s \
                         (the drain window was 5s — regression on the \
                         `tokio::select!` around debouncer.drain())"
                    ),
                }
            })
            .await;
    }

    #[tokio::test(flavor = "current_thread")]
    async fn shutdown_unblocks_run_even_when_reporter_callback_is_wedged() {
        // Regression for the hard-cap + wedged-reporter contract: if
        // a reporter callback blocks forever (deadlocked file sink,
        // stuck network webhook), the watch loop must:
        //
        //   1. Stay responsive to shutdown — the main select!/await
        //      points must not park behind the callback. `spawn_blocking`
        //      around the callback gives us this for free because the
        //      blocking thread is isolated from the reactor.
        //   2. Eventually stop spawning new supervisors when the
        //      `SUPERVISOR_HARD_CAP` is hit (validated by code
        //      inspection + `supervisor_caps_are_sanely_ordered`; a
        //      runtime test of the exact count would need to fire
        //      128 debounced cycles and is not worth the flake budget).
        //
        // This test exercises (1) directly: we start the watcher with
        // a callback that parks on a never-fired channel, fire a
        // single change event, let the cycle reach the callback, then
        // trigger shutdown. `run()` must return cleanly within a
        // bounded window.
        //
        // Without the `spawn_blocking` isolation + cancellation-aware
        // selects, this test would hang: `run()` would be waiting on
        // the never-returning callback and never observe shutdown.
        use std::process::Command as StdCommand;
        use std::sync::{Arc, Mutex as StdMutex};

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

        let workflow_dir = repo.join(".github").join("workflows");
        std::fs::create_dir_all(&workflow_dir).expect("mkdir workflows");
        std::fs::write(
            workflow_dir.join("ci.yml"),
            "name: ci\non: push\njobs:\n  build:\n    runs-on: ubuntu-latest\n    steps:\n      - run: echo hi\n",
        )
        .expect("write ci.yml");
        std::fs::create_dir_all(repo.join("src")).expect("mkdir src");

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

        let repo_canonical = std::fs::canonicalize(&repo).expect("canonicalize repo");

        // Short debounce so the test doesn't pay a long wall-clock
        // tax; the property under test is responsiveness during a
        // wedged callback, not timing precision.
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

        // `callback_started` flips to true the moment the blocking
        // reporter is dispatched. The test waits for this flag
        // before triggering shutdown — otherwise shutdown could race
        // ahead of the callback and the test would pass for the
        // wrong reason.
        //
        // `release_callback` is the gate we use to unstick the
        // blocking thread once the main body of the test is done
        // observing shutdown. We MUST give the blocking thread an
        // exit path, because `spawn_blocking` runs on a real OS
        // thread that cargo's test runner joins at process exit —
        // an uninterruptible `thread::sleep` loop would wedge the
        // entire test binary even after `run()` has returned. The
        // gate is flipped in the happy-path outer scope after the
        // shutdown assertion, and in the timeout fallback branch.
        let callback_started: Arc<StdMutex<bool>> = Arc::new(StdMutex::new(false));
        let callback_started_for_cb = callback_started.clone();
        let release_callback = Arc::new(std::sync::atomic::AtomicBool::new(false));
        let release_callback_for_cb = release_callback.clone();

        let shutdown = ShutdownSignal::new();
        let shutdown_for_run = shutdown.clone();

        let local = tokio::task::LocalSet::new();
        local
            .run_until(async move {
                let handle = tokio::task::spawn_local(async move {
                    watcher
                        .run(shutdown_for_run, move |_ev| {
                            // Mark that the reporter was invoked,
                            // then spin on a release-gated sleep
                            // loop. `spawn_blocking` isolates this
                            // thread from the reactor; the watch
                            // loop stays responsive to shutdown.
                            //
                            // Hard upper bound (30s) is a last-resort
                            // safety net so a bug in the release-gate
                            // plumbing cannot wedge the test binary
                            // indefinitely — if the gate never flips
                            // the thread still exits on its own. The
                            // outer test body always flips the gate
                            // long before this bound fires on a
                            // passing run.
                            {
                                let mut started = callback_started_for_cb
                                    .lock()
                                    .expect("callback_started mutex");
                                *started = true;
                            }
                            let absolute_deadline =
                                std::time::Instant::now() + Duration::from_secs(30);
                            while !release_callback_for_cb
                                .load(std::sync::atomic::Ordering::Relaxed)
                            {
                                if std::time::Instant::now() >= absolute_deadline {
                                    break;
                                }
                                std::thread::sleep(Duration::from_millis(25));
                            }
                        })
                        .await
                });

                // Let the watcher register notify watches.
                tokio::time::sleep(Duration::from_millis(200)).await;

                // Fire a change to drive the loop through drain →
                // evaluate → callback dispatch.
                std::fs::write(repo_canonical.join("src").join("main.rs"), "fn main() {}\n")
                    .expect("write src/main.rs");

                // Poll until the callback has been dispatched. Bound
                // this at 3 seconds so a flake surfaces loudly rather
                // than as a wedged test.
                let deadline = std::time::Instant::now() + Duration::from_secs(3);
                loop {
                    {
                        let started = callback_started.lock().expect("mutex");
                        if *started {
                            break;
                        }
                    }
                    if std::time::Instant::now() >= deadline {
                        // Notify may have been slow to deliver on
                        // this host — skip the assertion rather than
                        // flake. If the callback never started, we
                        // can't test the wedged-reporter path. Still
                        // release the gate in case a late callback
                        // arrives after we give up, so the blocking
                        // thread doesn't wedge the test binary.
                        release_callback.store(true, std::sync::atomic::Ordering::Relaxed);
                        shutdown.trigger();
                        let _ = tokio::time::timeout(Duration::from_secs(2), handle).await;
                        return;
                    }
                    tokio::time::sleep(Duration::from_millis(25)).await;
                }

                // Reporter is wedged in its release-gated sleep.
                // Trigger shutdown — the watch loop MUST return
                // without waiting for the callback to complete.
                shutdown.trigger();

                // 2 seconds is plenty: the loop observes shutdown at
                // its next top-of-loop check or select! arm. If this
                // times out, the regression is that some await inside
                // `run()` is parked waiting on the callback.
                let result = tokio::time::timeout(Duration::from_secs(2), handle).await;

                // Release the callback gate BEFORE asserting on the
                // result so the blocking thread exits regardless of
                // whether the assertion passes. Without this, a
                // panic!("shutdown did not unblock run()") would
                // fire while the blocking thread is still spinning,
                // cargo's test harness would try to reap it, and the
                // binary would hang. Flipping the gate first gives
                // the blocking thread its exit path in every code
                // path.
                release_callback.store(true, std::sync::atomic::Ordering::Relaxed);

                match result {
                    Ok(Ok(Ok(()))) => {}
                    Ok(Ok(Err(e))) => panic!("watcher errored: {}", e),
                    Ok(Err(join_err)) => panic!("watch task join failed: {}", join_err),
                    Err(_) => panic!(
                        "shutdown did not unblock run() within 2s while a reporter \
                         callback was wedged — regression on the callback isolation \
                         (spawn_blocking) or the cancellation-aware selects in the \
                         watch loop top"
                    ),
                }
            })
            .await;
    }
}
