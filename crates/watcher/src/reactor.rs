//! The watcher's per-cycle reactor loop, plus the private helpers that
//! execute inside it.
//!
//! Extracted from `watcher.rs` so the orchestration core (setup →
//! loop → drain → refresh → evaluate → dispatch → supervisor reap)
//! lives in one file, free of the `WorkflowWatcher` / `WatcherConfig`
//! struct definitions. The public entry point is
//! [`run_loop`]; [`WorkflowWatcher::run`] is a thin wrapper that
//! forwards to it.
//!
//! **Layout rationale.** The pre-refactor `watcher.rs` interleaved
//! three concerns in one 2k-line file: (a) config + struct
//! definitions, (b) this reactor loop, and (c) per-cycle workflow
//! evaluation. The extraction keeps (a) in `watcher.rs`, moves (b)
//! and (c) here, and leaves a dedicated home for future per-cycle
//! logic without bloating the top-level type file again.
//!
//! Private helpers intentionally take `watcher: &WorkflowWatcher` so
//! they can reach into `watcher.cfg.*` (verbose flag, repo_root,
//! execution config, etc.) without fan-out of positional arguments.
//! None of them escape the reactor module.

use crate::debouncer::Debouncer;
use crate::error::WatchError;
use crate::event_kind::is_relevant_event_kind;
use crate::ignore::{build_ignore_set, should_ignore_path};
use crate::paths::{display_workflow_path, normalize_separators};
use crate::setup::setup_watches;
use crate::shutdown::ShutdownSignal;
use crate::trigger_cache::{refresh_trigger_cache_blocking, TriggerCacheEntry};
use crate::watcher::{WatchEvent, WorkflowWatcher};
use futures::stream::{self, StreamExt};
use futures::FutureExt;
use notify::{Event, RecommendedWatcher, Watcher};
use std::collections::{HashMap, HashSet};
use std::panic::AssertUnwindSafe;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use wrkflw_trigger_filter::canonicalize_allowing_missing;
use wrkflw_trigger_filter::{EventContext, WorkflowTriggerConfig};

/// Soft threshold for the callback supervisor JoinSet. Crossing this
/// produces a one-shot warning so a slow reporter is surfaced without
/// spamming the log; crossing back below half clears the latch so the
/// NEXT spike warns again. Lives here rather than on `watcher.rs`
/// because the reactor loop is its only reader.
const SUPERVISOR_WARN_THRESHOLD: usize = 8;

/// Hard ceiling for the callback supervisor JoinSet. Past this we drop
/// the current cycle's `WatchEvent` rather than spawning another
/// supervisor we can't contain. Exists to bound memory under a wedged
/// reporter (deadlocked writer, stuck network webhook); the warning
/// threshold alone never reclaims anything, so a session-long hang
/// would otherwise grow the JoinSet without bound for the life of the
/// process. 128 keeps the worst-case footprint in the low MB range
/// while leaving plenty of headroom for a briefly-slow reporter.
const SUPERVISOR_HARD_CAP: usize = 128;

// Compile-time invariants: the warning threshold must stay strictly
// below the hard cap, and the hard cap must leave meaningful headroom
// (4x) above the threshold so short reporter stalls don't trip the
// drop-cycles path. A future tweak that accidentally inverts the
// ordering or sets them too close together fails the build here
// instead of drifting silently into production.
//
// `const { assert!(..) }` is the idiomatic form — clippy flags a
// runtime `assert!` on all-const operands because the check is
// trivially evaluated at compile time. Const asserts also catch the
// regression at `cargo check` time instead of waiting for `cargo test`,
// which is strictly better. Moved here alongside the constants
// themselves so a future reshuffle of the thresholds only has to touch
// this one file.
const _: () = {
    assert!(SUPERVISOR_WARN_THRESHOLD < SUPERVISOR_HARD_CAP);
    assert!(SUPERVISOR_HARD_CAP >= SUPERVISOR_WARN_THRESHOLD * 4);
};

/// The watcher's main loop. See [`WorkflowWatcher::run`] for the
/// public contract — graceful shutdown, callback dispatch semantics,
/// and the in-flight cycle drain guarantee. That doc comment lives
/// on the wrapper so the public crate docs still render it.
pub(crate) async fn run_loop<F>(
    watcher: &WorkflowWatcher,
    shutdown: ShutdownSignal,
    on_cycle_complete: F,
) -> Result<(), WatchError>
where
    F: Fn(WatchEvent) + Send + Sync + 'static,
{
    let initial_workflow_files = watcher.collect_workflow_files().await?;
    // An empty workflow directory is a legitimate starting state:
    // the user may be about to create their first workflow file
    // and want the watcher to pick it up as soon as it's written.
    // Surface an info line so the banner isn't misleading, but do
    // not abort — the mid-session rescan will populate the set
    // once a `.yml` appears.
    if initial_workflow_files.is_empty() {
        wrkflw_logging::info(&format!(
            "No workflow files yet in {} — watcher will pick them up as they appear.",
            watcher.cfg.workflow_dir.display()
        ));
    }

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
    let repo_root_canonical = std::fs::canonicalize(&watcher.cfg.repo_root).map_err(|e| {
        WatchError::Io(std::io::Error::other(format!(
            "could not canonicalize repo root {}: {} — refusing to start the \
             watcher in a degraded state where event paths could not be made \
             repo-relative (this notably affects macOS /private/var and \
             symlinked working trees). Verify the path exists and is accessible.",
            watcher.cfg.repo_root.display(),
            e,
        )))
    })?;

    // Honour `WatcherConfig::max_pending_events` when set;
    // otherwise fall through to the debouncer's baked-in default.
    let debouncer = Arc::new(if watcher.cfg.max_pending_events > 0 {
        Debouncer::with_capacity(
            watcher.cfg.debounce_duration,
            watcher.cfg.max_pending_events,
        )
    } else {
        Debouncer::new(watcher.cfg.debounce_duration)
    });
    // Track dropped events across cycles so each cycle's summary
    // reports only the drops that happened *since* the last
    // drain, not the cumulative count.
    let mut last_dropped_snapshot: usize = 0;
    let callback = Arc::new(on_cycle_complete);

    // Bounded pool for callback supervisors. Previously each cycle
    // spawned a detached `tokio::spawn` that `.await`ed the
    // `spawn_blocking` handle — correct on the happy path, but a
    // stuck reporter callback would accumulate one supervisor per
    // cycle forever because the runtime's own reaper is the ONLY
    // thing polling the detached handle. Holding them in a JoinSet
    // lets us reap completed supervisors at the top of each loop
    // iteration via `try_join_next` (non-blocking, constant-time
    // when empty) and surface a threshold warning so a reporter
    // that's falling behind doesn't silently balloon memory.
    //
    // Two caps:
    //
    //   - `SUPERVISOR_WARN_THRESHOLD = 8`: small enough that a
    //     legitimately slow reporter trips it (actionable), large
    //     enough that normal cycle-to-cycle overlap doesn't (no
    //     false positives). One-shot latch per threshold crossing
    //     so a session-long backlog produces exactly one warning
    //     per climb-past-8, not a warning per iteration.
    //
    //   - `SUPERVISOR_HARD_CAP = 128`: genuine memory ceiling. A
    //     truly wedged reporter (deadlocked file sink, hung network
    //     webhook) would otherwise grow the JoinSet without bound
    //     for the life of the session — the warning alone never
    //     reclaims anything. At the hard cap we surface a loud
    //     error, drop the CURRENT cycle's event (rather than
    //     spawning a supervisor we can't contain), and keep the
    //     watch loop running. 128 is small enough that memory
    //     growth is bounded at low MB; large enough that a short
    //     reporter stall can't false-trip it.
    let mut supervisor_tasks: tokio::task::JoinSet<()> = tokio::task::JoinSet::new();
    let mut supervisor_warned_at_threshold = false;
    let mut supervisor_hard_cap_warned = false;

    // One-shot latch for the per-cycle workflow-rescan warning.
    // Mirrors `supervisor_warned_at_threshold` discipline: a
    // persistently-failing rescan (`chmod 000 .github/workflows`,
    // stuck NFS mount, missing parent) would otherwise emit one
    // warning per debounced cycle for the entire session — the
    // diagnostic-flood failure mode on the other side of the
    // silent-skip hole. Latched so the NEXT transition from
    // healthy-to-failing produces a fresh warning without spamming.
    let mut rescan_warned = false;

    // Precompute the combined ignore set (defaults + user-supplied
    // extras). Sharing via Arc means the callback closure and the
    // initial `setup_watches` walk see identical semantics without
    // allocating on every event.
    let ignore_dirs: Arc<HashSet<String>> =
        Arc::new(build_ignore_set(&watcher.cfg.extra_ignore_dirs));

    // Set up the notify watcher.
    //
    // The `notify_watcher` binding is load-bearing: `RecommendedWatcher`
    // stops emitting events the moment it is dropped, so it MUST stay
    // alive for the entire duration of the watch loop below. Do not
    // narrow this scope or rebind it without preserving its lifetime.
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
    // (possibly symlinked) `watcher.cfg.repo_root`. Using only the
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
    let repo_root_raw_for_callback = watcher.cfg.repo_root.clone();
    let repo_root_canonical_for_callback = repo_root_canonical.clone();
    let ignore_for_callback = ignore_dirs.clone();
    let mut notify_watcher = RecommendedWatcher::new(
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
    setup_watches(&mut notify_watcher, &watcher.cfg.repo_root, &ignore_dirs)?;

    wrkflw_logging::info(&format!(
        "Watching {} for changes (event={}, debounce={}ms)",
        watcher.cfg.repo_root.display(),
        watcher.cfg.event_name,
        watcher.cfg.debounce_duration.as_millis()
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

        // Reap completed callback supervisors. `try_join_next` is
        // non-blocking: returns `None` when the set is empty or no
        // task is ready, so this is a constant-time poll on the
        // happy path. We ignore the per-task `Result` — a panic
        // *inside the callback itself* is already logged by the
        // supervisor body before it returns, so the reaper's job
        // is purely memory reclamation. A panic in the supervisor
        // body (e.g. the logger is wedged) would surface here as
        // `Err(join_err)` but is strictly out of scope for
        // this fix.
        while supervisor_tasks.try_join_next().is_some() {}
        // One-shot warning on threshold crossing. Reset when the
        // backlog drains back below so a long session with an
        // intermittently-slow reporter still warns on each NEW
        // spike, not just the first one. A persistent backlog
        // produces exactly one warning per climb-past-8 event,
        // never a warning-per-cycle flood.
        let backlog = supervisor_tasks.len();
        if backlog > SUPERVISOR_WARN_THRESHOLD && !supervisor_warned_at_threshold {
            wrkflw_logging::warning(&format!(
                "{} callback supervisor task(s) are pending — your reporter \
                 callback may be slow or stuck. The watch loop will continue \
                 but memory usage will grow until the backlog drains.",
                backlog
            ));
            supervisor_warned_at_threshold = true;
        } else if backlog <= SUPERVISOR_WARN_THRESHOLD {
            supervisor_warned_at_threshold = false;
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

        // Observe shutdown during the drain wait. `drain()` sleeps
        // for up to `max(debounce_duration, MAX_SETTLE_BUDGET)`
        // before returning — without this select, Ctrl+C during an
        // active drain has to wait the whole window before the loop
        // observes cancellation. Every other await in this loop is
        // cancellation-aware; this was the last outlier.
        //
        // Losing the pending events on shutdown is strictly weaker
        // than the already-accepted "cycle in flight completes
        // before run() returns" contract documented on `run()` —
        // we're exiting; dropping queued-but-not-yet-executing
        // events is acceptable and expected.
        let changed_paths = tokio::select! {
            paths = debouncer.drain() => paths,
            _ = shutdown.wait() => {
                wrkflw_logging::info(
                    "Watch loop received shutdown signal during drain; exiting.",
                );
                return Ok(());
            }
        };
        if changed_paths.is_empty() {
            continue;
        }

        // Re-collect workflow files so newly added .yml files are picked up.
        // Surface per-cycle rescan errors at WARNING level: the watch loop
        // *must* continue on transient failures (e.g. a temporary empty
        // directory, a fleeting I/O glitch), but silently falling back to
        // a stale `workflow_files` snapshot is the exact silent-skip
        // pattern the rest of this PR has been plugging. A non-verbose
        // user will never see a `debug!` line, so a persistently-failing
        // rescan (e.g. `chmod 000` on the workflow dir, a flaking network
        // mount) would leave them staring at a "0 triggered" stream with
        // zero hint that a new workflow has been missed for the entire
        // session. Warning keeps the loop running AND tells the operator
        // something is wrong.
        //
        // One-shot latch: a chmod-000 + file-save storm would otherwise
        // emit one warning per debounced cycle for the rest of the
        // session (diagnostic flood). Warn once per failing spell and
        // reset the latch the moment a rescan succeeds so a later
        // failure still surfaces. Mirrors `supervisor_warned_at_threshold`.
        match watcher.collect_workflow_files().await {
            Ok(refreshed) => {
                workflow_files = refreshed;
                if rescan_warned {
                    wrkflw_logging::info(
                        "workflow rescan recovered — new or deleted workflow files will \
                         now be picked up again.",
                    );
                    rescan_warned = false;
                }
            }
            Err(e) => {
                if !rescan_warned {
                    wrkflw_logging::warning(&format!(
                        "workflow rescan failed, reusing {} cached path(s): {} — \
                         new or deleted workflow files will NOT be picked up until \
                         the rescan recovers. Further rescan failures will be \
                         suppressed until the next success. Investigate the workflow \
                         directory (permissions, network mount, missing parent) if \
                         this repeats.",
                        workflow_files.len(),
                        e
                    ));
                    rescan_warned = true;
                }
            }
        }

        trigger_cache =
            refresh_trigger_cache_async(watcher, trigger_cache, &workflow_files, &changed_paths)
                .await;

        // Build the borrowed view for evaluation.
        let configs_for_eval: Vec<&WorkflowTriggerConfig> = workflow_files
            .iter()
            .filter_map(|p| trigger_cache.get(p).map(|entry| &entry.config))
            .collect();

        let changed_files =
            canonicalize_changed_paths(watcher, &changed_paths, &repo_root_canonical).await;

        if changed_files.is_empty() {
            if watcher.cfg.verbose {
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

        let mut event = evaluate_and_execute(watcher, &configs_for_eval, changed_files).await;
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
        // supervisor task held in `supervisor_tasks`, which gets
        // reaped at the top of each loop iteration. A panicking
        // callback surfaces as `JoinError::is_panic()` from that
        // reaper, logged via `wrkflw_logging::error`.
        //
        // Holding the supervisor in a `JoinSet` (instead of the
        // previous detached `tokio::spawn`) is what makes the
        // accumulation bounded: a stuck reporter now trips the
        // `SUPERVISOR_WARN_THRESHOLD` log at the top of the loop
        // and, past `SUPERVISOR_HARD_CAP`, drops cycle events
        // entirely rather than leaking one supervisor per cycle
        // forever. The warning alone was not enough — a truly
        // wedged reporter (deadlocked writer, stuck webhook) never
        // drains, so the backlog grows until the process OOMs.
        // Dropping cycles at the hard cap keeps memory bounded at
        // the cost of losing events for the stuck reporter; the
        // user sees a loud error explaining why.
        if supervisor_tasks.len() >= SUPERVISOR_HARD_CAP {
            if !supervisor_hard_cap_warned {
                wrkflw_logging::error(&format!(
                    "{} callback supervisor task(s) pending — hit hard cap of {}. \
                     Dropping this cycle's WatchEvent to contain memory growth. \
                     Your reporter callback is stuck or wedged; the watch loop will \
                     continue but will keep dropping cycles until the backlog drains. \
                     Investigate the reporter (deadlocked writer? hung network sink?) \
                     and restart `wrkflw watch` to recover fully.",
                    supervisor_tasks.len(),
                    SUPERVISOR_HARD_CAP,
                ));
                supervisor_hard_cap_warned = true;
            }
            // Skip spawning a new supervisor. The WatchEvent is
            // dropped on the floor — the reaper at the top of the
            // next iteration will clear space as supervisors
            // finish, at which point new events resume flowing.
            continue;
        }
        // Clear the one-shot hard-cap latch once we're back under
        // the ceiling so a NEW spike of wedging produces a fresh
        // error log. Mirrors the `supervisor_warned_at_threshold`
        // reset discipline above.
        if supervisor_tasks.len() < SUPERVISOR_HARD_CAP / 2 {
            supervisor_hard_cap_warned = false;
        }

        let cb = callback.clone();
        let cb_handle = tokio::task::spawn_blocking(move || cb(event));
        supervisor_tasks.spawn(async move {
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
    watcher: &WorkflowWatcher,
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
    let verbose = watcher.cfg.verbose;
    let tf_config = watcher.cfg.trigger_filter.clone();
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
    watcher: &WorkflowWatcher,
    changed_paths: &[PathBuf],
    repo_root_canonical: &Path,
) -> Vec<String> {
    let paths_for_canon = changed_paths.to_vec();
    let root_for_canon = repo_root_canonical.to_path_buf();
    let verbose = watcher.cfg.verbose;
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
/// **Degraded context handling.** If `cached_git_state` fails (e.g.
/// transient git error), we no longer silently fall back to a
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
pub(crate) async fn evaluate_and_execute(
    watcher: &WorkflowWatcher,
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
    // on every cycle. See `GitStateCache` for the rationale — a
    // file-save storm of 40 events was previously 80 git subprocess
    // spawns for a branch+tag pair that almost never changes.
    let mut context = match watcher
        .git_state
        .get(&watcher.cfg.trigger_filter, &watcher.cfg.repo_root)
        .await
    {
        Ok((branch, tag)) => EventContext {
            event_name: watcher.cfg.event_name.clone(),
            branch,
            base_branch: watcher.cfg.base_branch.clone(),
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
            activity_type: watcher.cfg.activity_type.clone(),
            warnings: wrkflw_trigger_filter::MustDrainWarnings::new(),
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

    // Drain context-level warnings into the `WatchEvent` so the
    // reporter surfaces them to the user. The watcher synthesises
    // an empty warning buffer above (`GitStateCache::get` returns
    // only branch/tag), so the drain is a no-op today — but the
    // contract lives here so a future git-state cache that
    // surfaces diagnostics via `EventContext::warnings` does not
    // silently drop them, and the `MustDrainWarnings` Drop check
    // is guaranteed satisfied before `context` falls out of scope.
    let mut cycle_warnings: Vec<String> = context.warnings.take();

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
        let label = display_workflow_path(&result.workflow_path, &watcher.cfg.repo_root);

        if result.matches {
            triggered.push(label);

            let exec_config = watcher.cfg.execution.clone();
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
        .buffer_unordered(watcher.cfg.max_concurrent_executions)
        .collect::<Vec<()>>()
        .await;

    WatchEvent {
        changed_files,
        triggered_workflows: triggered,
        skipped_workflows: skipped,
        error: None,
        warnings: std::mem::take(&mut cycle_warnings),
        dropped_events: 0,
    }
}
