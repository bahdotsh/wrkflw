//! `wrkflw watch` command orchestration.
//!
//! Extracted from `main.rs` as the sibling to `run_workflow_cmd` so
//! the two long-lived CLI execution paths share a file layout. The
//! body still calls `std::process::exit` from each terminal failure
//! site — see the rationale on [`crate::run_workflow_cmd::run`] for
//! why further lifting to `Result<(), String>` would regress
//! user-visible output.
//!
//! The `pull_request + no base-branch` validation routes through
//! [`crate::prefilter::validate_event_requires_base_branch`] so the
//! run and watch commands emit identical error / warning text.
//! Previously each had its own inline copy that drifted on the
//! first edit.

use crate::prefilter;
use crate::RuntimeChoice;
use std::path::PathBuf;

/// Owned copy of the clap `Commands::Watch` variant fields plus the
/// global `--verbose` flag. Built by `main()` from the match arm's
/// borrowed fields.
pub(crate) struct WatchCtx {
    pub(crate) path: Option<PathBuf>,
    pub(crate) runtime: RuntimeChoice,
    pub(crate) debounce: u64,
    pub(crate) event: String,
    pub(crate) show_action_messages: bool,
    pub(crate) preserve_containers_on_failure: bool,
    pub(crate) max_concurrency: usize,
    pub(crate) base_branch: Option<String>,
    pub(crate) activity_type: Option<String>,
    pub(crate) max_pending_events: Option<usize>,
    pub(crate) ignore_dirs: Vec<String>,
    pub(crate) strict_filter: bool,
    pub(crate) no_strict_filter: bool,
    pub(crate) verbose: bool,
}

/// Execute the `wrkflw watch` command. Exits the process on every
/// terminal failure (missing dir, find_repo_root error, missing
/// `--base-branch` under strict mode, watcher startup I/O error,
/// runtime watch error). Returns normally when the watch loop
/// observes a shutdown signal cleanly.
pub(crate) async fn run(ctx: WatchCtx) {
    let strict_filter = prefilter::effective_strict_filter(ctx.strict_filter, ctx.no_strict_filter);
    let workflow_dir = ctx
        .path
        .clone()
        .unwrap_or_else(|| PathBuf::from(".github/workflows"));
    if !workflow_dir.exists() {
        eprintln!(
            "Error: workflow directory not found: {}",
            workflow_dir.display()
        );
        std::process::exit(1);
    }

    // `find_repo_root_detailed` shells out to `git rev-parse`
    // synchronously and is NOT wrapped in the trigger-filter's
    // GIT_COMMAND_TIMEOUT, so a hung git (credential prompt,
    // stuck network mount) would block the reactor if we called
    // it directly. Move it onto the blocking pool to keep the
    // tokio runtime responsive.
    //
    // We use the `_detailed` variant so each failure mode
    // (missing binary / timeout / not-in-repo / other) renders
    // its own diagnostic. The legacy `Option`-returning wrapper
    // collapsed all four into "not inside a git repository",
    // which is actively wrong for the first three and sent
    // users down the wrong fix path.
    let repo_root =
        match tokio::task::spawn_blocking(wrkflw_trigger_filter::find_repo_root_detailed).await {
            Ok(Ok(p)) => p,
            Ok(Err(e)) => {
                eprintln!("Error: {}", e);
                std::process::exit(1);
            }
            Err(join_err) => {
                eprintln!("Error: find_repo_root task panicked: {}", join_err);
                std::process::exit(1);
            }
        };

    let debounce_duration = std::time::Duration::from_millis(ctx.debounce);

    let config = wrkflw_executor::ExecutionConfig {
        runtime_type: ctx.runtime.into(),
        verbose: ctx.verbose,
        preserve_containers_on_failure: ctx.preserve_containers_on_failure,
        secrets_config: None,
        show_action_messages: ctx.show_action_messages,
        target_job: None,
    };

    use wrkflw_ui::cli_style;
    println!(
        "{}",
        cli_style::success(&format!(
            "Watching for changes (event={}, debounce={}ms)... Press Ctrl+C to stop.",
            ctx.event, ctx.debounce
        ))
    );

    // Hard-error on the load-bearing `pull_request + no base-branch`
    // combination under the default `--strict-filter`. Previously
    // this only produced a log warning that the user never saw in
    // non-interactive contexts, and the watcher then ran a
    // session-long stream of "0 triggered" results.
    //
    // Routes through [`prefilter::validate_event_requires_base_branch`]
    // so the run and watch commands produce identical error /
    // warning text — the two used to carry independent copies
    // that drifted on the first edit.
    if ctx.base_branch.is_none() {
        if let Err(msg) = prefilter::validate_event_requires_base_branch(&ctx.event, strict_filter)
        {
            eprintln!("Error: {}", msg);
            std::process::exit(1);
        }
    }

    // Resolve `--max-pending-events`. The library's
    // `WatcherConfig::max_pending_events` field keeps its
    // existing `0 = use-library-default` convention (matches
    // how `TriggerFilterConfig::pattern_cache_size == 0`
    // disables caching — library-wide sentinel style). The
    // CLI, however, exposes an honest `Option<usize>` so
    // `--help` doesn't advertise a misleading `[default: 0]`.
    //
    // `Some(0)` is almost certainly an error on the user's
    // part (cap-everything-to-zero would drop every event
    // and make the watcher useless). Mirror the
    // `max_concurrency=0 → 1` clamp pattern in the library:
    // warn loudly and fall through to the library default.
    let max_pending_for_cfg: usize = match ctx.max_pending_events {
        Some(0) => {
            wrkflw_logging::warning(
                "--max-pending-events 0 is invalid (would cap the pending \
                 set at zero and drop every event); falling back to the \
                 library default.",
            );
            0 // 0 inside the library means "use DEFAULT_MAX_PENDING_EVENTS"
        }
        Some(n) => n,
        None => 0,
    };

    let watcher_cfg = wrkflw_watcher::WatcherConfig::new(workflow_dir, repo_root, config)
        .with_event(ctx.event.clone())
        .with_base_branch(ctx.base_branch.clone())
        .with_activity_type(ctx.activity_type.clone())
        .with_debounce(debounce_duration)
        .with_verbose(ctx.verbose)
        .with_max_concurrency(ctx.max_concurrency)
        .with_max_pending_events(max_pending_for_cfg)
        .with_extra_ignore_dirs(ctx.ignore_dirs.clone());
    let watcher = wrkflw_watcher::WorkflowWatcher::from_config(watcher_cfg);

    // Pre-flight: surface any real I/O error (missing dir,
    // permission denied) before the user sees a "watching..."
    // banner. An empty directory is NOT an error — the
    // watcher's internal rescan picks up `.yml` files the
    // moment they are created, which is the whole point of
    // watch mode. `collect_workflow_files_blocking` now
    // returns `Ok(Vec::new())` for an empty dir, so only
    // genuine failures propagate past this match.
    if let Err(e) = watcher.collect_workflow_files().await {
        eprintln!("Error: {}", e);
        std::process::exit(1);
    }

    // Install a graceful Ctrl+C handler: the default
    // top-of-`main` handler uses `process::exit(0)` after a
    // timed cleanup sweep, which bypasses the watcher's
    // normal drain (workflows mid-execution would be killed
    // by the OS without running the executor's teardown).
    //
    // Instead we own a `ShutdownSignal`, trigger it on
    // Ctrl+C, and let the watcher observe the signal at its
    // existing `tokio::select!` points. The global handler
    // at the top of `main` still runs — it kicks in after
    // the watch loop returns and performs the Docker /
    // emulation cleanup sweep — so Ctrl+C produces a clean
    // two-phase teardown instead of a hard exit.
    //
    // A race exists: if Ctrl+C fires while a workflow is
    // already executing, that workflow continues to
    // completion within the current cycle before `run()`
    // returns. We accept that bounded latency because the
    // executor holds container/tempdir handles that need
    // their normal cleanup — forcibly cancelling the
    // future would defeat the very cleanup we're trying to
    // preserve. `MAX_REASONABLE_CONCURRENCY` + the
    // user-specified `--max-concurrency` bound the
    // worst-case drain time.
    let shutdown = wrkflw_watcher::ShutdownSignal::new();
    let shutdown_for_signal = shutdown.clone();
    tokio::spawn(async move {
        if tokio::signal::ctrl_c().await.is_ok() {
            wrkflw_logging::info(
                "Ctrl+C received — draining current watch cycle gracefully. \
                 Press Ctrl+C again if the drain hangs.",
            );
            shutdown_for_signal.trigger();
        }
    });

    let watch_result = watcher
        .run(shutdown, |watch_event| {
            println!(
                "\n{}",
                cli_style::section(&format!(
                    "Change detected ({} file(s) changed, {} triggered, {} skipped{})",
                    watch_event.changed_files.len(),
                    watch_event.triggered_workflows.len(),
                    watch_event.skipped_workflows.len(),
                    if watch_event.dropped_events > 0 {
                        format!(", {} dropped", watch_event.dropped_events)
                    } else {
                        String::new()
                    }
                ))
            );
            // Surface degraded cycles loudly: if the watcher
            // could not build a git event context, the trigger
            // results are not authoritative and the user needs
            // to know why before they assume "0 triggered".
            if let Some(reason) = &watch_event.error {
                eprintln!("  {} {}", cli_style::error("ERROR"), reason);
            }
            for warning in &watch_event.warnings {
                eprintln!("  {} {}", cli_style::warning("WARN"), warning);
            }
            for wf in &watch_event.triggered_workflows {
                println!("  {} {}", cli_style::success("TRIGGERED"), wf);
            }
            for wf in &watch_event.skipped_workflows {
                println!("  {} {}", cli_style::dim("SKIPPED"), wf);
            }
        })
        .await;

    if let Err(e) = watch_result {
        eprintln!("Watch error: {}", e);
        std::process::exit(1);
    }
}
