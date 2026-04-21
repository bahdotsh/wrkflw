// App state for the UI
use crate::log_processor::{LogProcessingRequest, LogProcessor, ProcessedLogEntry};
use crate::models::{
    ExecutionResultMsg, JobExecution, LogFilterLevel, QueuedExecution, StatusSeverity,
    StepExecution, TriggerMatchStatus, Workflow, WorkflowExecution, WorkflowStatus,
};
use chrono::Local;
use crossterm::event::KeyCode;
use ratatui::widgets::{ListState, TableState};
use std::path::{Path, PathBuf};
use std::sync::mpsc;
use std::time::{Duration, Instant};
use tokio::task::JoinHandle;
use wrkflw_executor::{JobStatus, RuntimeType, StepStatus};

/// Application state
pub struct App {
    pub workflows: Vec<Workflow>,
    pub workflow_list_state: ListState,
    pub selected_tab: usize,
    pub running: bool,
    pub show_help: bool,
    pub runtime_type: RuntimeType,
    pub validation_mode: bool,
    pub preserve_containers_on_failure: bool,
    pub show_action_messages: bool,
    pub execution_queue: Vec<QueuedExecution>, // Workflows queued for execution
    pub current_execution: Option<usize>,
    pub logs: Vec<String>,                       // Overall execution logs
    pub log_scroll: usize,                       // Scrolling position for logs
    pub job_list_state: ListState,               // For viewing job details
    pub detailed_view: bool,                     // Whether we're in detailed view mode
    pub step_list_state: ListState,              // For selecting steps in detailed view
    pub step_table_state: TableState,            // For the steps table in detailed view
    pub last_tick: Instant,                      // For UI animations and updates
    pub tick_rate: Duration,                     // How often to update the UI
    pub spinner_frame: usize,                    // Current spinner animation frame
    pub tx: mpsc::Sender<ExecutionResultMsg>,    // Channel for async communication
    pub status_message: Option<String>,          // Temporary status message to display
    pub status_message_severity: StatusSeverity, // Severity of the current status message
    pub status_message_time: Option<Instant>,    // When the message was set

    // Search and filter functionality
    pub log_search_query: String, // Current search query for logs
    pub log_search_active: bool,  // Whether search input is active
    pub log_filter_level: Option<LogFilterLevel>, // Current log level filter
    pub log_search_matches: Vec<usize>, // Indices of logs that match the search
    pub log_search_match_idx: usize, // Current match index for navigation

    // Help tab scrolling
    pub help_scroll: usize, // Scrolling position for help content

    // Background log processing
    pub log_processor: LogProcessor,
    pub processed_logs: Vec<ProcessedLogEntry>,
    pub logs_need_update: bool,        // Flag to trigger log processing
    pub last_system_logs_count: usize, // Track system log changes

    // Job selection mode
    pub job_selection_mode: bool, // Are we viewing jobs of a workflow?
    pub available_jobs: Vec<String>, // Job names from selected workflow
    pub selected_job_index: usize, // Cursor in job selection list

    // Cached container runtime availability (avoids re-checking every render frame)
    pub runtime_available: bool,
    pub last_availability_check: Instant,

    // Diff-aware trigger filtering
    pub diff_filter_active: bool,
    /// The event the TUI simulates for diff-filter evaluation. Stored on
    /// `App` rather than as a hardcoded constant so a future event selector
    /// UI is a data-flow change only.
    pub diff_filter_event: String,
    /// Activity type to stamp on the synthesized event context — same
    /// purpose as `WatcherConfig::activity_type` and the CLI's
    /// `--activity-type` flag. The TUI has no UI to set this yet, so it
    /// defaults to `None`; the field exists so a future activity-type
    /// selector is a plumbing-only change here, and so workflows that
    /// gate on `pull_request: { types: [...] }` aren't silently rejected
    /// the moment such a UI ships.
    pub diff_filter_activity_type: Option<String>,
    /// Channel receiving (workflow_path, trigger_status) pairs from the background
    /// evaluation task. We send pairs (rather than a positional Vec) so that
    /// reloading `self.workflows` between toggle and result delivery cannot
    /// mis-assign trigger statuses.
    pub diff_filter_rx: Option<DiffFilterReceiver>,
    /// Handle for the most-recently-spawned evaluation task, held so
    /// rapid toggles can cancel the previous in-flight evaluation instead
    /// of leaking wasted git + parse work.
    pub diff_filter_task: Option<JoinHandle<()>>,
    /// Set to `true` immediately before we drop the previous evaluation's
    /// receiver in [`App::toggle_diff_filter`]. The next
    /// [`App::check_diff_filter_results`] tick uses it to distinguish a
    /// self-inflicted disconnect (rapid toggle) from a genuine background
    /// task failure, so we don't tell the user "evaluation failed" for an
    /// action they took deliberately. Cleared once observed.
    pub diff_filter_aborted: bool,

    /// Active sub-tab inside the Step Inspector (job-detail) view.
    /// 0 Output, 1 Env, 2 Files, 3 Matrix, 4 Timeline.
    pub step_inspector_tab: usize,
}

/// Result rows shipped from the background diff-filter task to the UI loop.
pub type DiffFilterResults = Vec<(PathBuf, Option<TriggerMatchStatus>)>;

/// Workflow files that failed to parse during a diff-filter evaluation,
/// paired with the reason. Surfaced to the TUI log so the user is not
/// left wondering why N workflows are missing from the result table.
pub type DiffFilterParseFailures = Vec<(PathBuf, String)>;

/// Successful diff-filter evaluation payload.
///
/// Carrying `parse_failures` alongside `rows` lets the UI distinguish
/// "this workflow has triggers that did not match" from "this workflow
/// has broken YAML and was silently dropped from the result map" — the
/// previous `filter_map(... .ok())` collapsed both cases into the same
/// `trigger_match = None` rendering, leaving users with no debugging
/// signal when their `on:` block had a typo.
///
/// `warnings` carries the non-fatal diagnostics the trigger-filter
/// collected while building the event context AND while parsing each
/// workflow's `on:` block (e.g. `git ls-files --others` failed, so
/// untracked files are missing from the change set; unknown event name
/// typo in `on: pul_request`). The library routes these through struct
/// fields on purpose — hosts own the rendering policy — so every host
/// MUST drain them or reproduce the silent-skip failure mode the rest
/// of this PR is built to plug. The CLI prefilter at
/// `crates/wrkflw/src/main.rs` does this via `event_context.warnings`
/// and `trigger_config.warnings`; the TUI plumbs both through this
/// field so `check_diff_filter_results` can render them the same way
/// `parse_failures` is rendered.
#[derive(Debug, Clone)]
pub struct DiffFilterReport {
    pub rows: DiffFilterResults,
    pub parse_failures: DiffFilterParseFailures,
    pub warnings: Vec<String>,
}

/// Outcome of a background diff-filter evaluation.
///
/// Wrapping the row list in an enum lets us distinguish "we ran the
/// evaluation and some workflows matched" from "we could not even build
/// an event context" (e.g. git could not find a diff base), so the TUI
/// can show the user a real error reason instead of silently reporting
/// zero matches.
#[derive(Debug, Clone)]
pub enum DiffFilterOutcome {
    Success(DiffFilterReport),
    Failure(String),
}

pub type DiffFilterReceiver = mpsc::Receiver<DiffFilterOutcome>;

impl App {
    pub fn new(
        runtime_type: RuntimeType,
        tx: mpsc::Sender<ExecutionResultMsg>,
        preserve_containers_on_failure: bool,
        show_action_messages: bool,
    ) -> App {
        let mut workflow_list_state = ListState::default();
        workflow_list_state.select(Some(0));

        let mut job_list_state = ListState::default();
        job_list_state.select(Some(0));

        let mut step_list_state = ListState::default();
        step_list_state.select(Some(0));

        let mut step_table_state = TableState::default();
        step_table_state.select(Some(0));

        // Check container runtime availability if container runtime is selected
        let mut initial_logs = Vec::new();
        let runtime_type = match runtime_type {
            RuntimeType::Docker => {
                // Use a timeout for the Docker availability check to prevent hanging
                let is_docker_available = match std::panic::catch_unwind(|| {
                    // Use a very short timeout to prevent blocking the UI
                    let result = std::thread::scope(|s| {
                        let handle = s.spawn(|| {
                            wrkflw_utils::fd::with_stderr_to_null(
                                wrkflw_executor::docker::is_available,
                            )
                            .unwrap_or(false)
                        });

                        // Set a short timeout for the thread
                        let start = std::time::Instant::now();
                        let timeout = std::time::Duration::from_secs(1);

                        while start.elapsed() < timeout {
                            if handle.is_finished() {
                                return handle.join().unwrap_or(false);
                            }
                            std::thread::sleep(std::time::Duration::from_millis(10));
                        }

                        // If we reach here, the check took too long
                        wrkflw_logging::warning(
                            "Docker availability check timed out, falling back to emulation mode",
                        );
                        false
                    });
                    result
                }) {
                    Ok(result) => result,
                    Err(_) => {
                        wrkflw_logging::warning("Docker availability check failed with panic, falling back to emulation mode");
                        false
                    }
                };

                if !is_docker_available {
                    initial_logs.push(
                        "Docker is not available or unresponsive. Using emulation mode instead."
                            .to_string(),
                    );
                    wrkflw_logging::warning(
                        "Docker is not available or unresponsive. Using emulation mode instead.",
                    );
                    RuntimeType::Emulation
                } else {
                    wrkflw_logging::info("Docker is available, using Docker runtime");
                    RuntimeType::Docker
                }
            }
            RuntimeType::Podman => {
                // Use a timeout for the Podman availability check to prevent hanging
                let is_podman_available = match std::panic::catch_unwind(|| {
                    // Use a very short timeout to prevent blocking the UI
                    let result = std::thread::scope(|s| {
                        let handle = s.spawn(|| {
                            wrkflw_utils::fd::with_stderr_to_null(
                                wrkflw_executor::podman::is_available,
                            )
                            .unwrap_or(false)
                        });

                        // Set a short timeout for the thread
                        let start = std::time::Instant::now();
                        let timeout = std::time::Duration::from_secs(1);

                        while start.elapsed() < timeout {
                            if handle.is_finished() {
                                return handle.join().unwrap_or(false);
                            }
                            std::thread::sleep(std::time::Duration::from_millis(10));
                        }

                        // If we reach here, the check took too long
                        wrkflw_logging::warning(
                            "Podman availability check timed out, falling back to emulation mode",
                        );
                        false
                    });
                    result
                }) {
                    Ok(result) => result,
                    Err(_) => {
                        wrkflw_logging::warning("Podman availability check failed with panic, falling back to emulation mode");
                        false
                    }
                };

                if !is_podman_available {
                    initial_logs.push(
                        "Podman is not available or unresponsive. Using emulation mode instead."
                            .to_string(),
                    );
                    wrkflw_logging::warning(
                        "Podman is not available or unresponsive. Using emulation mode instead.",
                    );
                    RuntimeType::Emulation
                } else {
                    wrkflw_logging::info("Podman is available, using Podman runtime");
                    RuntimeType::Podman
                }
            }
            RuntimeType::Emulation => RuntimeType::Emulation,
            RuntimeType::SecureEmulation => RuntimeType::SecureEmulation,
        };

        // If we're still Docker/Podman after the availability check above, it was available
        let runtime_available = matches!(runtime_type, RuntimeType::Docker | RuntimeType::Podman);

        App {
            workflows: Vec::new(),
            workflow_list_state,
            selected_tab: 0,
            running: false,
            show_help: false,
            runtime_type,
            validation_mode: false,
            preserve_containers_on_failure,
            show_action_messages,
            execution_queue: Vec::new(),
            current_execution: None,
            logs: initial_logs,
            log_scroll: 0,
            job_list_state,
            detailed_view: false,
            step_list_state,
            step_table_state,
            last_tick: Instant::now(),
            tick_rate: Duration::from_millis(250), // Update 4 times per second
            spinner_frame: 0,
            tx,
            status_message: None,
            status_message_severity: StatusSeverity::default(),
            status_message_time: None,

            // Search and filter functionality
            log_search_query: String::new(),
            log_search_active: false,
            log_filter_level: Some(LogFilterLevel::All),
            log_search_matches: Vec::new(),
            log_search_match_idx: 0,
            help_scroll: 0,

            // Background log processing
            log_processor: LogProcessor::new(),
            processed_logs: Vec::new(),
            logs_need_update: true,
            last_system_logs_count: 0,

            // Job selection mode
            job_selection_mode: false,
            available_jobs: Vec::new(),
            selected_job_index: 0,

            runtime_available,
            last_availability_check: Instant::now(),

            diff_filter_active: false,
            diff_filter_event: "push".to_string(),
            diff_filter_activity_type: None,
            diff_filter_rx: None,
            diff_filter_task: None,
            diff_filter_aborted: false,
            step_inspector_tab: 0,
        }
    }

    // Toggle workflow selection
    pub fn toggle_selected(&mut self) {
        if let Some(idx) = self.workflow_list_state.selected() {
            if idx < self.workflows.len() {
                self.workflows[idx].selected = !self.workflows[idx].selected;
            }
        }
    }

    pub fn toggle_emulation_mode(&mut self) {
        self.runtime_type = match self.runtime_type {
            RuntimeType::Docker => RuntimeType::Podman,
            RuntimeType::Podman => RuntimeType::SecureEmulation,
            RuntimeType::SecureEmulation => RuntimeType::Emulation,
            RuntimeType::Emulation => RuntimeType::Docker,
        };
        // Re-check availability for the new runtime immediately
        self.runtime_available = match self.runtime_type {
            RuntimeType::Docker => wrkflw_executor::docker::is_available(),
            RuntimeType::Podman => wrkflw_executor::podman::is_available(),
            _ => false,
        };
        self.last_availability_check = Instant::now();
        self.logs
            .push(format!("Switched to {} mode", self.runtime_type_name()));
    }

    /// Cycle through the event names the diff filter simulates.
    ///
    /// Previously `diff_filter_event` was dead-plumbing: it existed on
    /// the struct, defaulted to "push", and was never mutated. The
    /// review flagged this as a "TUI silently lies about which
    /// workflows would run" hazard, because a user debugging a
    /// `pull_request` workflow would see it reported as skipped even
    /// when the filter would have matched on the right event.
    ///
    /// The rotation covers the event names that usually appear with
    /// `branches:` / `paths:` filters. `workflow_dispatch` is included
    /// because watch-mode users frequently want to model manual runs.
    /// If an evaluation is already active we re-run it against the
    /// new event so the result table updates immediately; if it is
    /// inactive we just update the pending event name so the next
    /// toggle uses it.
    pub fn cycle_diff_filter_event(&mut self) {
        const ROTATION: &[&str] = &[
            "push",
            "pull_request",
            "pull_request_target",
            "workflow_dispatch",
            "schedule",
            "release",
        ];
        let current_idx = ROTATION
            .iter()
            .position(|name| *name == self.diff_filter_event)
            .unwrap_or(0);
        let next_idx = (current_idx + 1) % ROTATION.len();
        let next = ROTATION[next_idx].to_string();
        self.logs.push(format!(
            "Diff filter event: {} -> {}",
            self.diff_filter_event, next
        ));
        self.diff_filter_event = next;
        // If the filter is currently active, re-run evaluation so
        // the result column reflects the new event immediately.
        self.rerun_diff_filter_if_active();
    }

    /// Re-run an active diff-filter evaluation against the current
    /// event/activity fields by tearing down the in-flight task and
    /// spawning a fresh one.
    ///
    /// No-op when the filter is inactive. This is the path
    /// [`Self::cycle_diff_filter_event`] takes after mutating
    /// `diff_filter_event` so the result column reflects the new
    /// event immediately.
    ///
    /// Previously this was a double `toggle_diff_filter` call, which
    /// worked only as long as `toggle_diff_filter` stayed purely
    /// idempotent in opposing directions — any future side effect
    /// added to the toggle helper (metrics, tracing, throttle) would
    /// have silently broken the rerun behaviour. Routing directly
    /// through [`Self::spawn_evaluation`] makes the intent explicit
    /// and removes the two-call fragility.
    fn rerun_diff_filter_if_active(&mut self) {
        if !self.diff_filter_active {
            return;
        }
        self.spawn_evaluation();
    }

    /// Tear down any in-flight diff-filter evaluation.
    ///
    /// Aborts the background task handle (best-effort: `JoinHandle::abort`
    /// only signals at the next await point, so a git future already in
    /// flight may keep running until it completes) and drops the receiver
    /// so any result the task eventually produces is discarded. Arms
    /// [`App::diff_filter_aborted`] when there was actually something to
    /// abort so the next [`App::check_diff_filter_results`] tick treats
    /// the resulting `Disconnected` as self-inflicted instead of logging
    /// "evaluation failed" for an action the user took deliberately.
    ///
    /// Intentionally does NOT touch `diff_filter_active` — callers own
    /// the active-flag flip so the semantics of "toggle off" and
    /// "restart evaluation" stay separable.
    fn abort_in_flight_evaluation(&mut self) {
        // Mark the disconnect as self-inflicted BEFORE dropping the
        // receiver so the next tick's `check_diff_filter_results`
        // distinguishes "we cancelled" from "the task crashed". Only
        // arm the flag when there was actually something to abort,
        // otherwise a fresh call from a clean state would silently
        // suppress a real failure on the *next* evaluation.
        if self.diff_filter_task.is_some() || self.diff_filter_rx.is_some() {
            self.diff_filter_aborted = true;
        }
        if let Some(handle) = self.diff_filter_task.take() {
            handle.abort();
        }
        self.diff_filter_rx = None;
    }

    /// Spawn a fresh diff-filter evaluation for the current workflow
    /// list against the currently-selected `diff_filter_event` +
    /// `diff_filter_activity_type`.
    ///
    /// Aborts any in-flight evaluation first so rapid toggles or
    /// event-cycle key presses never leak wasted git + parse work.
    /// Clears the stale `diff_filter_aborted` flag before spawning so
    /// a genuine failure on the new task is surfaced to the user
    /// instead of being mistaken for a self-inflicted abort.
    ///
    /// Git + parsing work is dispatched onto the ambient tokio runtime
    /// via `tokio::task::spawn`. Results are received via
    /// [`Self::check_diff_filter_results`] on the next tick.
    fn spawn_evaluation(&mut self) {
        self.abort_in_flight_evaluation();

        // A new evaluation begins with a fresh receiver. Any
        // `diff_filter_aborted` flag still set at this point belongs
        // to a *prior* abort cycle whose receiver was dropped without
        // ever being observed (e.g. user toggled OFF, no tick ran,
        // user toggled back ON). Leaving it armed here would silently
        // swallow a genuine failure on the new task — exactly the
        // silent-skip mode this PR is built to prevent. Clear it
        // before spawning so the next `Disconnected` is treated as
        // a real failure and surfaced to the user.
        self.diff_filter_aborted = false;

        let event_name = self.diff_filter_event.clone();
        let activity_type = self.diff_filter_activity_type.clone();
        self.add_log(format!(
            "Diff filter: evaluating triggers (simulating '{}' event)...",
            event_name
        ));

        let workflow_paths: Vec<PathBuf> = self.workflows.iter().map(|w| w.path.clone()).collect();

        let (tx, rx) = mpsc::channel();
        self.diff_filter_rx = Some(rx);

        // Anchor git operations at the discovered repo root rather
        // than the process CWD. The TUI may be launched from a
        // sibling repo or a subdirectory; without this, every git
        // helper inside `auto_detect_context_default_base` would
        // run wherever the user happened to be when they started
        // `wrkflw tui`. The watcher and CLI prefilter both anchor
        // at the repo root for the same reason.
        //
        // `find_repo_root_detailed` shells out to `git rev-parse
        // --show-toplevel` synchronously. Calling it on the UI
        // thread would hitch the TUI on every toggle on a network
        // mount, so we move it onto the blocking pool inside the
        // spawned task. Using the classified `_detailed` form
        // (instead of the old `Option` wrapper) lets us surface a
        // "not in a git repository" / "git not installed" /
        // "timed out" reason to the user instead of silently
        // collapsing every failure into "0/N would trigger".
        let handle = tokio::task::spawn(async move {
            let repo_root_result =
                tokio::task::spawn_blocking(wrkflw_trigger_filter::find_repo_root_detailed).await;
            let repo_root = match repo_root_result {
                Ok(Ok(p)) => Some(p),
                // `NotInRepository` is a legitimate soft state —
                // the user may have launched `wrkflw tui` from
                // /tmp or a non-repo sandbox, and the downstream
                // git helpers will surface a clearer message
                // (e.g. "not a git repository" from `git diff`)
                // which lands in the Failure outcome below.
                Ok(Err(wrkflw_trigger_filter::FindRepoRootError::NotInRepository)) => None,
                Ok(Err(e)) => {
                    let _ = tx.send(DiffFilterOutcome::Failure(e.to_string()));
                    return;
                }
                Err(join_err) => {
                    let _ = tx.send(DiffFilterOutcome::Failure(format!(
                        "find_repo_root task panicked: {}",
                        join_err
                    )));
                    return;
                }
            };
            let results =
                evaluate_diff_filter(workflow_paths, event_name, activity_type, repo_root).await;
            let _ = tx.send(results);
        });
        self.diff_filter_task = Some(handle);
    }

    /// Toggle diff-aware trigger filtering and evaluate all workflows.
    ///
    /// On ON→OFF: aborts any in-flight task, drops the receiver,
    /// clears the per-workflow trigger match state, and logs
    /// "Diff filter OFF".
    ///
    /// On OFF→ON: delegates to [`Self::spawn_evaluation`], which
    /// dispatches git + parsing onto the ambient tokio runtime.
    /// Results are received via [`Self::check_diff_filter_results`]
    /// on the next tick.
    ///
    /// If an evaluation is already in flight (rapid toggle), the
    /// previous task's [`JoinHandle`] is aborted and its `mpsc::Sender`
    /// is dropped. `JoinHandle::abort` only signals at the next await
    /// point — git futures already in flight may keep running until
    /// they complete — but the receiver is gone, so any results they
    /// produce are discarded. From the user's perspective the
    /// previous evaluation is dead; in reality it's "no longer
    /// observed."
    pub fn toggle_diff_filter(&mut self) {
        self.diff_filter_active = !self.diff_filter_active;

        if self.diff_filter_active {
            self.spawn_evaluation();
        } else {
            self.abort_in_flight_evaluation();
            for workflow in &mut self.workflows {
                workflow.trigger_match = None;
            }
            self.logs.push("Diff filter OFF".to_string());
        }
    }

    /// Check for completed diff filter results from the background task.
    /// Called on each TUI tick to apply results without blocking.
    ///
    /// Results are looked up by workflow path so that reloading
    /// `self.workflows` between toggle and result delivery (e.g. a new file
    /// shows up on disk) does not cause statuses to be assigned to the wrong
    /// workflow.
    ///
    /// A channel payload of [`DiffFilterOutcome::Failure`] surfaces the
    /// underlying error reason to the TUI log instead of silently leaving
    /// every workflow as `None` — previously the user would see
    /// "0/N workflows would trigger" with no explanation.
    pub fn check_diff_filter_results(&mut self) {
        let results = match self.diff_filter_rx.as_ref() {
            Some(rx) => match rx.try_recv() {
                Ok(results) => results,
                Err(mpsc::TryRecvError::Empty) => return,
                Err(mpsc::TryRecvError::Disconnected) => {
                    self.diff_filter_rx = None;
                    self.diff_filter_task = None;
                    // A `Disconnected` here can mean two things:
                    //   1. The background task panicked or returned without
                    //      sending — a real failure the user should see.
                    //   2. We deliberately aborted the previous evaluation
                    //      because the user toggled rapidly. The receiver
                    //      we are draining is the *abandoned* one and the
                    //      next tick will see a fresh channel.
                    // Use the `diff_filter_aborted` flag (set in
                    // `toggle_diff_filter`) to distinguish them. Self-
                    // inflicted aborts are silent; everything else is a
                    // real failure and stays loud.
                    if self.diff_filter_aborted {
                        self.diff_filter_aborted = false;
                    } else {
                        self.logs.push("Diff filter: evaluation failed".to_string());
                    }
                    return;
                }
            },
            None => return,
        };
        self.diff_filter_rx = None;
        self.diff_filter_task = None;
        // We received a real payload, so any pending "we aborted" flag
        // belonged to a previous cycle whose disconnect we never observed
        // (because the new task beat it to the channel). Clear it so the
        // next genuine failure isn't silently swallowed.
        self.diff_filter_aborted = false;

        match results {
            DiffFilterOutcome::Success(DiffFilterReport {
                rows,
                parse_failures,
                warnings,
            }) => {
                let by_path: std::collections::HashMap<PathBuf, Option<TriggerMatchStatus>> =
                    rows.into_iter().collect();
                for workflow in self.workflows.iter_mut() {
                    workflow.trigger_match = by_path.get(&workflow.path).cloned().flatten();
                }

                let matched = self
                    .workflows
                    .iter()
                    .filter(|w| matches!(&w.trigger_match, Some(TriggerMatchStatus::Matched(_))))
                    .count();
                self.logs.push(format!(
                    "Diff filter ON: {}/{} workflows would trigger",
                    matched,
                    self.workflows.len()
                ));

                // Surface non-fatal warnings from the library BEFORE
                // the parse-failure block so the most actionable
                // diagnostics land closest to the "N/M triggered"
                // summary line. `warnings` carries both context-level
                // (e.g. `git ls-files --others` failed — untracked
                // files missing from the change set) and parser-level
                // (e.g. unknown event name typo) diagnostics. Previously
                // the TUI dropped all of these on the floor even
                // though the library deliberately routed them through
                // `EventContext::warnings` / `WorkflowTriggerConfig::warnings`
                // for hosts to render — the CLI prefilter logs them
                // at `crates/wrkflw/src/main.rs`; parity is
                // load-bearing to avoid the silent-skip mode this PR
                // is built to plug.
                if !warnings.is_empty() {
                    self.logs
                        .push(format!("Diff filter: {} warning(s)", warnings.len()));
                    for w in &warnings {
                        self.logs.push(format!("  warning: {}", w));
                    }
                }

                // Parse failures used to be silently dropped via
                // `filter_map(... .ok())`, leaving the user with N
                // workflows showing as `-` (untriggered) and no clue why.
                // Surface each failure individually so the YAML/glob
                // typo is the first thing they see in the log pane.
                if !parse_failures.is_empty() {
                    self.logs.push(format!(
                        "Diff filter: {} workflow file(s) failed to parse and were skipped",
                        parse_failures.len()
                    ));
                    for (path, reason) in &parse_failures {
                        self.logs
                            .push(format!("  parse error: {}: {}", path.display(), reason));
                    }
                }

                // Ad-hoc `self.logs.push(...)` skips the cap that
                // `add_log` / `mark_logs_for_update` normally enforce.
                // A single large evaluation could push dozens of
                // rows + warnings + parse failures all at once, and
                // without this trim the buffer can temporarily exceed
                // `LOG_BUFFER_CAP` until the next render pass happens
                // to route through `mark_logs_for_update`. Mirror the
                // `add_log` discipline here so the cap invariant is
                // reasserted immediately.
                self.trim_logs_to_cap();
            }
            DiffFilterOutcome::Failure(reason) => {
                for workflow in &mut self.workflows {
                    workflow.trigger_match = None;
                }
                self.logs
                    .push(format!("Diff filter: evaluation failed — {}", reason));
                self.trim_logs_to_cap();
            }
        }
    }

    pub fn toggle_validation_mode(&mut self) {
        self.validation_mode = !self.validation_mode;
        let mode = if self.validation_mode {
            "validation"
        } else {
            "normal"
        };
        let timestamp = Local::now().format("%H:%M:%S").to_string();
        self.logs
            .push(format!("[{}] Switched to {} mode", timestamp, mode));
        wrkflw_logging::info(&format!("Switched to {} mode", mode));
    }

    pub fn runtime_type_name(&self) -> &str {
        match self.runtime_type {
            RuntimeType::Docker => "Docker",
            RuntimeType::Podman => "Podman",
            RuntimeType::SecureEmulation => "Secure Emulation",
            RuntimeType::Emulation => "Emulation (Unsafe)",
        }
    }

    // Move cursor up in the workflow list
    pub fn previous_workflow(&mut self) {
        if self.workflows.is_empty() {
            return;
        }

        let i = match self.workflow_list_state.selected() {
            Some(i) => {
                if i == 0 {
                    self.workflows.len() - 1
                } else {
                    i - 1
                }
            }
            None => 0,
        };
        self.workflow_list_state.select(Some(i));
    }

    // Move cursor down in the workflow list
    pub fn next_workflow(&mut self) {
        if self.workflows.is_empty() {
            return;
        }

        let i = match self.workflow_list_state.selected() {
            Some(i) => {
                if i >= self.workflows.len() - 1 {
                    0
                } else {
                    i + 1
                }
            }
            None => 0,
        };
        self.workflow_list_state.select(Some(i));
    }

    // Move cursor up in the job list
    pub fn previous_job(&mut self) {
        let current_workflow_idx = self
            .current_execution
            .or_else(|| self.workflow_list_state.selected())
            .filter(|&idx| idx < self.workflows.len());

        if let Some(workflow_idx) = current_workflow_idx {
            if let Some(execution) = &self.workflows[workflow_idx].execution_details {
                if execution.jobs.is_empty() {
                    return;
                }

                let i = match self.job_list_state.selected() {
                    Some(i) => {
                        if i == 0 {
                            execution.jobs.len() - 1
                        } else {
                            i - 1
                        }
                    }
                    None => 0,
                };
                self.job_list_state.select(Some(i));

                // Reset step selection when changing jobs
                self.step_list_state.select(Some(0));
            }
        }
    }

    // Move cursor down in the job list
    pub fn next_job(&mut self) {
        let current_workflow_idx = self
            .current_execution
            .or_else(|| self.workflow_list_state.selected())
            .filter(|&idx| idx < self.workflows.len());

        if let Some(workflow_idx) = current_workflow_idx {
            if let Some(execution) = &self.workflows[workflow_idx].execution_details {
                if execution.jobs.is_empty() {
                    return;
                }

                let i = match self.job_list_state.selected() {
                    Some(i) => {
                        if i >= execution.jobs.len() - 1 {
                            0
                        } else {
                            i + 1
                        }
                    }
                    None => 0,
                };
                self.job_list_state.select(Some(i));

                // Reset step selection when changing jobs
                self.step_list_state.select(Some(0));
            }
        }
    }

    // Move cursor up in step list
    pub fn previous_step(&mut self) {
        let current_workflow_idx = self
            .current_execution
            .or_else(|| self.workflow_list_state.selected())
            .filter(|&idx| idx < self.workflows.len());

        if let Some(workflow_idx) = current_workflow_idx {
            if let Some(execution) = &self.workflows[workflow_idx].execution_details {
                if let Some(job_idx) = self.job_list_state.selected() {
                    if job_idx < execution.jobs.len() {
                        let steps = &execution.jobs[job_idx].steps;
                        if steps.is_empty() {
                            return;
                        }

                        let i = match self.step_list_state.selected() {
                            Some(i) => {
                                if i == 0 {
                                    steps.len() - 1
                                } else {
                                    i - 1
                                }
                            }
                            None => 0,
                        };
                        self.step_list_state.select(Some(i));
                        // Update the table state to match
                        self.step_table_state.select(Some(i));
                    }
                }
            }
        }
    }

    // Move cursor down in step list
    pub fn next_step(&mut self) {
        let current_workflow_idx = self
            .current_execution
            .or_else(|| self.workflow_list_state.selected())
            .filter(|&idx| idx < self.workflows.len());

        if let Some(workflow_idx) = current_workflow_idx {
            if let Some(execution) = &self.workflows[workflow_idx].execution_details {
                if let Some(job_idx) = self.job_list_state.selected() {
                    if job_idx < execution.jobs.len() {
                        let steps = &execution.jobs[job_idx].steps;
                        if steps.is_empty() {
                            return;
                        }

                        let i = match self.step_list_state.selected() {
                            Some(i) => {
                                if i >= steps.len() - 1 {
                                    0
                                } else {
                                    i + 1
                                }
                            }
                            None => 0,
                        };
                        self.step_list_state.select(Some(i));
                        // Update the table state to match
                        self.step_table_state.select(Some(i));
                    }
                }
            }
        }
    }

    // Change the tab
    pub fn switch_tab(&mut self, tab: usize) {
        self.selected_tab = tab;
    }

    // Queue selected workflows for execution
    pub fn queue_selected_for_execution(&mut self) {
        if let Some(idx) = self.workflow_list_state.selected() {
            if idx < self.workflows.len()
                && !self.execution_queue.iter().any(|e| e.workflow_idx == idx)
            {
                self.execution_queue.push(QueuedExecution {
                    workflow_idx: idx,
                    target_job: None,
                });
                self.add_timestamped_log(&format!(
                    "Added '{}' to execution queue. Press 'Enter' to start.",
                    self.workflows[idx].name
                ));
            }
        }
    }

    // Start workflow execution process
    pub fn start_execution(&mut self) {
        // Only start if we have workflows in queue and nothing is currently running
        if !self.execution_queue.is_empty() && self.current_execution.is_none() {
            self.running = true;

            // Log only once at the beginning - don't initialize execution details here
            // since that will happen in start_next_workflow_execution
            let timestamp = Local::now().format("%H:%M:%S").to_string();
            self.logs
                .push(format!("[{}] Starting workflow execution...", timestamp));
            wrkflw_logging::info("Starting workflow execution...");
        }
    }

    // Process execution results and update UI
    pub fn process_execution_result(
        &mut self,
        workflow_idx: usize,
        result: Result<(Vec<wrkflw_executor::JobResult>, ()), String>,
    ) {
        if workflow_idx >= self.workflows.len() {
            let timestamp = Local::now().format("%H:%M:%S").to_string();
            self.logs.push(format!(
                "[{}] Error: Invalid workflow index received",
                timestamp
            ));
            wrkflw_logging::error("Invalid workflow index received in process_execution_result");
            return;
        }

        let workflow = &mut self.workflows[workflow_idx];

        // Ensure execution details exist
        if workflow.execution_details.is_none() {
            workflow.execution_details = Some(WorkflowExecution {
                jobs: Vec::new(),
                start_time: Local::now(),
                end_time: Some(Local::now()),
                logs: Vec::new(),
                progress: 1.0,
            });
        }

        // Update execution details with end time
        if let Some(execution_details) = &mut workflow.execution_details {
            execution_details.end_time = Some(Local::now());

            match &result {
                Ok((jobs, _)) => {
                    let timestamp = Local::now().format("%H:%M:%S").to_string();
                    execution_details
                        .logs
                        .push(format!("[{}] Operation completed successfully.", timestamp));
                    execution_details.progress = 1.0;

                    // Convert wrkflw_executor::JobResult to our JobExecution struct
                    execution_details.jobs = jobs
                        .iter()
                        .map(|job_result| JobExecution {
                            name: job_result.name.clone(),
                            status: match job_result.status {
                                wrkflw_executor::JobStatus::Success => JobStatus::Success,
                                wrkflw_executor::JobStatus::Failure => JobStatus::Failure,
                                wrkflw_executor::JobStatus::Skipped => JobStatus::Skipped,
                            },
                            steps: job_result
                                .steps
                                .iter()
                                .map(|step_result| StepExecution {
                                    name: step_result.name.clone(),
                                    status: match step_result.status {
                                        wrkflw_executor::StepStatus::Success => StepStatus::Success,
                                        wrkflw_executor::StepStatus::Failure => StepStatus::Failure,
                                        wrkflw_executor::StepStatus::Skipped => StepStatus::Skipped,
                                    },
                                    output: step_result.output.clone(),
                                })
                                .collect::<Vec<StepExecution>>(),
                            logs: vec![job_result.logs.clone()],
                        })
                        .collect::<Vec<JobExecution>>();
                }
                Err(e) => {
                    let timestamp = Local::now().format("%H:%M:%S").to_string();
                    execution_details
                        .logs
                        .push(format!("[{}] Error: {}", timestamp, e));
                    execution_details.progress = 1.0;

                    // Create a dummy job with the error information so users can see details
                    execution_details.jobs = vec![JobExecution {
                        name: "Workflow Execution".to_string(),
                        status: JobStatus::Failure,
                        steps: vec![StepExecution {
                            name: "Execution Error".to_string(),
                            status: StepStatus::Failure,
                            output: format!("Error: {}\n\nThis error prevented the workflow from executing properly.", e),
                        }],
                        logs: vec![format!("Workflow execution error: {}", e)],
                    }];
                }
            }
        }

        match result {
            Ok(_) => {
                workflow.status = WorkflowStatus::Success;
                let timestamp = Local::now().format("%H:%M:%S").to_string();
                self.logs.push(format!(
                    "[{}] Workflow '{}' completed successfully!",
                    timestamp, workflow.name
                ));
                wrkflw_logging::info(&format!(
                    "[{}] Workflow '{}' completed successfully!",
                    timestamp, workflow.name
                ));
            }
            Err(e) => {
                workflow.status = WorkflowStatus::Failed;
                let timestamp = Local::now().format("%H:%M:%S").to_string();
                self.logs.push(format!(
                    "[{}] Workflow '{}' failed: {}",
                    timestamp, workflow.name, e
                ));
                wrkflw_logging::error(&format!(
                    "[{}] Workflow '{}' failed: {}",
                    timestamp, workflow.name, e
                ));
            }
        }

        // Only clear current_execution if it matches the processed workflow
        if let Some(current_idx) = self.current_execution {
            if current_idx == workflow_idx {
                self.current_execution = None;
            }
        }
    }

    // Get next workflow for execution
    pub fn get_next_workflow_to_execute(&mut self) -> Option<(usize, Option<String>)> {
        if self.execution_queue.is_empty() {
            return None;
        }

        let entry = self.execution_queue.remove(0);
        let next = entry.workflow_idx;
        let target_job = entry.target_job;
        self.workflows[next].status = WorkflowStatus::Running;
        self.current_execution = Some(next);
        self.logs
            .push(format!("Executing workflow: {}", self.workflows[next].name));
        wrkflw_logging::info(&format!(
            "Executing workflow: {}",
            self.workflows[next].name
        ));

        // Initialize execution details
        self.workflows[next].execution_details = Some(WorkflowExecution {
            jobs: Vec::new(),
            start_time: Local::now(),
            end_time: None,
            logs: vec!["Execution started".to_string()],
            progress: 0.0, // Just started
        });

        Some((next, target_job))
    }

    // Enter job selection mode for the currently selected workflow
    pub fn enter_job_selection_mode(&mut self) {
        if let Some(idx) = self.workflow_list_state.selected() {
            if idx < self.workflows.len() {
                let job_names = &self.workflows[idx].job_names;

                if job_names.is_empty() {
                    self.add_timestamped_log(&format!(
                        "No jobs found in workflow '{}'",
                        self.workflows[idx].name
                    ));
                    return;
                }

                self.available_jobs = job_names.clone();
                self.selected_job_index = 0;
                self.job_selection_mode = true;
            }
        }
    }

    // Exit job selection mode back to workflow list
    pub fn exit_job_selection_mode(&mut self) {
        self.job_selection_mode = false;
        self.available_jobs.clear();
        self.selected_job_index = 0;
    }

    // Navigate to next job in selection list
    pub fn next_available_job(&mut self) {
        if !self.available_jobs.is_empty() {
            self.selected_job_index = (self.selected_job_index + 1) % self.available_jobs.len();
        }
    }

    // Navigate to previous job in selection list
    pub fn previous_available_job(&mut self) {
        if !self.available_jobs.is_empty() {
            if self.selected_job_index == 0 {
                self.selected_job_index = self.available_jobs.len() - 1;
            } else {
                self.selected_job_index -= 1;
            }
        }
    }

    // Run from job selection mode with an optional target job.
    // Callers must ensure `!self.running` before calling.
    pub fn run_from_job_selection(&mut self, target_job: Option<String>) {
        if let Some(ref name) = target_job {
            self.add_timestamped_log(&format!("Running job '{}'", name));
        } else {
            self.add_timestamped_log("Running all jobs");
        }

        if let Some(idx) = self.workflow_list_state.selected() {
            if idx < self.workflows.len() {
                self.workflows[idx].selected = true;
                self.execution_queue.push(QueuedExecution {
                    workflow_idx: idx,
                    target_job,
                });
            }
        }

        self.exit_job_selection_mode();
        self.start_execution();
    }

    // Toggle detailed view mode
    pub fn toggle_detailed_view(&mut self) {
        self.detailed_view = !self.detailed_view;

        // When entering detailed view, make sure step selection is initialized
        if self.detailed_view {
            // Ensure the step_table_state matches the step_list_state
            if let Some(step_idx) = self.step_list_state.selected() {
                self.step_table_state.select(Some(step_idx));
            } else {
                // Initialize both to the first item if nothing is selected
                self.step_list_state.select(Some(0));
                self.step_table_state.select(Some(0));
            }

            // Also ensure job_list_state has a selection
            if self.job_list_state.selected().is_none() {
                self.job_list_state.select(Some(0));
            }
        }
    }

    // Function to handle keyboard input for log search
    pub fn handle_log_search_input(&mut self, key: KeyCode) {
        match key {
            KeyCode::Esc => {
                self.log_search_active = false;
                self.log_search_query.clear();
                self.log_search_matches.clear();
                self.mark_logs_for_update();
            }
            KeyCode::Backspace => {
                self.log_search_query.pop();
                self.mark_logs_for_update();
            }
            KeyCode::Enter => {
                self.log_search_active = false;
                // Keep the search query and matches
            }
            KeyCode::Char(c) => {
                self.log_search_query.push(c);
                self.mark_logs_for_update();
            }
            _ => {}
        }
    }

    // Toggle log search mode
    pub fn toggle_log_search(&mut self) {
        self.log_search_active = !self.log_search_active;
        if !self.log_search_active {
            // Don't clear the query, this allows toggling the search UI while keeping the filter
        } else {
            // When activating search, trigger update
            self.mark_logs_for_update();
        }
    }

    // Toggle log filter
    pub fn toggle_log_filter(&mut self) {
        self.log_filter_level = match &self.log_filter_level {
            None => Some(LogFilterLevel::Info),
            Some(level) => Some(level.next()),
        };

        // Trigger log processing update when filter changes
        self.mark_logs_for_update();
    }

    // Clear log search and filter
    pub fn clear_log_search_and_filter(&mut self) {
        self.log_search_query.clear();
        self.log_filter_level = None;
        self.log_search_matches.clear();
        self.log_search_match_idx = 0;
        self.mark_logs_for_update();
    }

    // Update matches based on current search and filter
    pub fn update_log_search_matches(&mut self) {
        self.log_search_matches.clear();
        self.log_search_match_idx = 0;

        // Get all logs (app logs + system logs)
        let mut all_logs = Vec::new();
        for log in &self.logs {
            all_logs.push(log.clone());
        }
        for log in wrkflw_logging::get_logs() {
            all_logs.push(log.clone());
        }

        // Apply filter and search
        for (idx, log) in all_logs.iter().enumerate() {
            let passes_filter = match &self.log_filter_level {
                None => true,
                Some(level) => level.matches(log),
            };

            let matches_search = if self.log_search_query.is_empty() {
                true
            } else {
                log.to_lowercase()
                    .contains(&self.log_search_query.to_lowercase())
            };

            if passes_filter && matches_search {
                self.log_search_matches.push(idx);
            }
        }

        // Jump to first match and provide feedback
        if !self.log_search_matches.is_empty() {
            // Jump to the first match
            if let Some(&idx) = self.log_search_matches.first() {
                self.log_scroll = idx;

                if !self.log_search_query.is_empty() {
                    self.set_success_message(format!(
                        "Found {} matches for '{}'",
                        self.log_search_matches.len(),
                        self.log_search_query
                    ));
                }
            }
        } else if !self.log_search_query.is_empty() {
            // No matches found
            self.set_warning_message(format!("No matches found for '{}'", self.log_search_query));
        }
    }

    // Navigate to next search match
    pub fn next_search_match(&mut self) {
        if !self.log_search_matches.is_empty() {
            self.log_search_match_idx =
                (self.log_search_match_idx + 1) % self.log_search_matches.len();
            if let Some(&idx) = self.log_search_matches.get(self.log_search_match_idx) {
                self.log_scroll = idx;

                // Set status message showing which match we're on
                self.set_success_message(format!(
                    "Search match {}/{} for '{}'",
                    self.log_search_match_idx + 1,
                    self.log_search_matches.len(),
                    self.log_search_query
                ));
            }
        }
    }

    // Navigate to previous search match
    pub fn previous_search_match(&mut self) {
        if !self.log_search_matches.is_empty() {
            self.log_search_match_idx = if self.log_search_match_idx == 0 {
                self.log_search_matches.len() - 1
            } else {
                self.log_search_match_idx - 1
            };
            if let Some(&idx) = self.log_search_matches.get(self.log_search_match_idx) {
                self.log_scroll = idx;

                // Set status message showing which match we're on
                self.set_success_message(format!(
                    "Search match {}/{} for '{}'",
                    self.log_search_match_idx + 1,
                    self.log_search_matches.len(),
                    self.log_search_query
                ));
            }
        }
    }

    // Scroll logs up
    pub fn scroll_logs_up(&mut self) {
        self.log_scroll = self.log_scroll.saturating_sub(1);
    }

    // Scroll logs down
    pub fn scroll_logs_down(&mut self) {
        // Get total log count including system logs
        let total_logs = self.logs.len() + wrkflw_logging::get_logs().len();
        if total_logs > 0 {
            self.log_scroll = (self.log_scroll + 1).min(total_logs - 1);
        }
    }

    // Scroll help content up
    pub fn scroll_help_up(&mut self) {
        self.help_scroll = self.help_scroll.saturating_sub(1);
    }

    // Scroll help content down
    pub fn scroll_help_down(&mut self) {
        // The help content has a fixed number of lines, so we set a reasonable max
        const MAX_HELP_SCROLL: usize = 30; // Adjust based on help content length
        self.help_scroll = (self.help_scroll + 1).min(MAX_HELP_SCROLL);
    }

    // Update progress for running workflows
    pub fn update_running_workflow_progress(&mut self) {
        if let Some(idx) = self.current_execution {
            if let Some(execution) = &mut self.workflows[idx].execution_details {
                if execution.end_time.is_none() {
                    // Gradually increase progress for visual feedback
                    execution.progress = (execution.progress + 0.01).min(0.95);
                }
            }
        }
    }

    // Set a temporary error status message to be displayed in the UI
    pub fn set_error_message(&mut self, message: String) {
        self.status_message = Some(message);
        self.status_message_severity = StatusSeverity::Error;
        self.status_message_time = Some(Instant::now());
    }

    // Set a temporary warning status message
    pub fn set_warning_message(&mut self, message: String) {
        self.status_message = Some(message);
        self.status_message_severity = StatusSeverity::Warning;
        self.status_message_time = Some(Instant::now());
    }

    // Set a temporary info status message
    pub fn set_info_message(&mut self, message: String) {
        self.status_message = Some(message);
        self.status_message_severity = StatusSeverity::Info;
        self.status_message_time = Some(Instant::now());
    }

    // Set a temporary success status message
    pub fn set_success_message(&mut self, message: String) {
        self.status_message = Some(message);
        self.status_message_severity = StatusSeverity::Success;
        self.status_message_time = Some(Instant::now());
    }

    // Check if tick should happen
    pub fn tick(&mut self) -> bool {
        let now = Instant::now();

        // Check if we should clear a status message (after 3 seconds)
        if let Some(message_time) = self.status_message_time {
            if now.duration_since(message_time).as_secs() >= 3 {
                self.status_message = None;
                self.status_message_time = None;
            }
        }

        if now.duration_since(self.last_tick) >= self.tick_rate {
            self.last_tick = now;
            self.spinner_frame = (self.spinner_frame + 1) % crate::theme::symbols::SPINNER.len();

            // Refresh container runtime availability every 30 seconds
            if now.duration_since(self.last_availability_check) >= Duration::from_secs(30) {
                self.last_availability_check = now;
                self.runtime_available = match self.runtime_type {
                    RuntimeType::Docker => wrkflw_executor::docker::is_available(),
                    RuntimeType::Podman => wrkflw_executor::podman::is_available(),
                    _ => false,
                };
            }

            true
        } else {
            false
        }
    }

    // Trigger the selected workflow
    pub fn trigger_selected_workflow(&mut self) {
        if let Some(selected_idx) = self.workflow_list_state.selected() {
            if selected_idx < self.workflows.len() {
                let workflow = &self.workflows[selected_idx];

                if workflow.name.is_empty() {
                    let timestamp = Local::now().format("%H:%M:%S").to_string();
                    self.logs
                        .push(format!("[{}] Error: Invalid workflow selection", timestamp));
                    wrkflw_logging::error(
                        "Invalid workflow selection in trigger_selected_workflow",
                    );
                    return;
                }

                // Set up background task to execute the workflow via GitHub Actions REST API
                let timestamp = Local::now().format("%H:%M:%S").to_string();
                self.logs.push(format!(
                    "[{}] Triggering workflow: {}",
                    timestamp, workflow.name
                ));
                wrkflw_logging::info(&format!("Triggering workflow: {}", workflow.name));

                // Clone necessary values for the async task
                let workflow_name = workflow.name.clone();
                let tx_clone = self.tx.clone();

                // Set this tab as the current execution to ensure it shows in the Execution tab
                self.current_execution = Some(selected_idx);

                // Switch to execution tab for better user feedback
                self.selected_tab = 1; // Switch to Execution tab manually to avoid the borrowing issue

                // Create a thread instead of using tokio runtime directly since send() is not async
                std::thread::spawn(move || {
                    // Create a runtime for the thread
                    let rt = match tokio::runtime::Runtime::new() {
                        Ok(runtime) => runtime,
                        Err(e) => {
                            let _ = tx_clone.send((
                                selected_idx,
                                Err(format!("Failed to create Tokio runtime: {}", e)),
                            ));
                            return;
                        }
                    };

                    // Execute the GitHub Actions trigger API call
                    let result = rt.block_on(async {
                        crate::handlers::workflow::execute_curl_trigger(&workflow_name, None).await
                    });

                    // Send the result back to the main thread
                    if let Err(e) = tx_clone.send((selected_idx, result)) {
                        wrkflw_logging::error(&format!("Error sending trigger result: {}", e));
                    }
                });
            } else {
                let timestamp = Local::now().format("%H:%M:%S").to_string();
                self.logs
                    .push(format!("[{}] No workflow selected to trigger", timestamp));
                wrkflw_logging::warning("No workflow selected to trigger");
            }
        } else {
            self.logs
                .push("No workflow selected to trigger".to_string());
            wrkflw_logging::warning("No workflow selected to trigger");
        }
    }

    // Reset a workflow's status to NotStarted
    pub fn reset_workflow_status(&mut self) {
        // Log whether a selection exists
        if self.workflow_list_state.selected().is_none() {
            let timestamp = Local::now().format("%H:%M:%S").to_string();
            self.logs.push(format!(
                "[{}] Debug: No workflow selected for reset",
                timestamp
            ));
            wrkflw_logging::warning("No workflow selected for reset");
            return;
        }

        if let Some(idx) = self.workflow_list_state.selected() {
            if idx < self.workflows.len() {
                let workflow = &mut self.workflows[idx];
                // Log before status
                let timestamp = Local::now().format("%H:%M:%S").to_string();
                self.logs.push(format!(
                    "[{}] Debug: Attempting to reset workflow '{}' from {:?} state",
                    timestamp, workflow.name, workflow.status
                ));

                // Debug: Reset unconditionally for testing
                // if workflow.status != WorkflowStatus::Running {
                let old_status = match workflow.status {
                    WorkflowStatus::Success => "Success",
                    WorkflowStatus::Failed => "Failed",
                    WorkflowStatus::Skipped => "Skipped",
                    WorkflowStatus::NotStarted => "NotStarted",
                    WorkflowStatus::Running => "Running",
                };

                // Store workflow name for the success message
                let workflow_name = workflow.name.clone();

                // Reset regardless of current status (for debugging)
                workflow.status = WorkflowStatus::NotStarted;
                // Clear execution details to reset all state
                workflow.execution_details = None;

                let timestamp = Local::now().format("%H:%M:%S").to_string();
                self.logs.push(format!(
                    "[{}] Reset workflow '{}' from {} state to NotStarted - status is now {:?}",
                    timestamp, workflow.name, old_status, workflow.status
                ));
                wrkflw_logging::info(&format!(
                    "Reset workflow '{}' from {} state to NotStarted - status is now {:?}",
                    workflow.name, old_status, workflow.status
                ));

                // Set a success status message
                self.set_success_message(format!("Workflow '{}' has been reset!", workflow_name));
            }
        }
    }

    /// Request log processing update from background thread
    pub fn request_log_processing_update(&mut self) {
        let request = LogProcessingRequest {
            search_query: self.log_search_query.clone(),
            filter_level: self.log_filter_level.clone(),
            app_logs: self.logs.clone(),
            app_logs_count: self.logs.len(),
            system_logs_count: wrkflw_logging::get_logs().len(),
        };

        if self.log_processor.request_update(request).is_err() {
            // Log processor channel disconnected, recreate it
            self.log_processor = LogProcessor::new();
            self.logs_need_update = true;
        }
    }

    /// Check for and apply log processing updates
    pub fn check_log_processing_updates(&mut self) {
        // Check if system logs have changed
        let current_system_logs_count = wrkflw_logging::get_logs().len();
        if current_system_logs_count != self.last_system_logs_count {
            self.last_system_logs_count = current_system_logs_count;
            self.mark_logs_for_update();
        }

        if let Some(response) = self.log_processor.try_get_update() {
            self.processed_logs = response.processed_logs;
            self.log_search_matches = response.search_matches;

            // Update scroll position to first match if we have search results
            if !self.log_search_matches.is_empty() && !self.log_search_query.is_empty() {
                self.log_search_match_idx = 0;
                if let Some(&idx) = self.log_search_matches.first() {
                    self.log_scroll = idx;
                }
            }

            self.logs_need_update = false;
        }
    }

    /// Trigger log processing when search/filter changes.
    ///
    /// Also enforces the log buffer cap so ad-hoc `self.logs.push(...)`
    /// sites — which are sprinkled throughout the codebase for
    /// historical reasons — don't need to each remember to call
    /// [`trim_logs_to_cap`]. Every log mutation eventually routes
    /// through `mark_logs_for_update` (that's what makes the logs
    /// actually render), so trimming here is the single
    /// unavoidable choke point.
    pub fn mark_logs_for_update(&mut self) {
        self.trim_logs_to_cap();
        self.logs_need_update = true;
        self.request_log_processing_update();
    }

    /// Get combined app and system logs for background processing
    pub fn get_combined_logs(&self) -> Vec<String> {
        let mut all_logs = Vec::new();

        // Add app logs
        for log in &self.logs {
            all_logs.push(log.clone());
        }

        // Add system logs
        for log in wrkflw_logging::get_logs() {
            all_logs.push(log.clone());
        }

        all_logs
    }

    /// Add a log entry and trigger log processing update
    pub fn add_log(&mut self, message: String) {
        self.logs.push(message);
        self.trim_logs_to_cap();
        self.mark_logs_for_update();
    }

    /// Add a formatted log entry with timestamp and trigger log processing update
    pub fn add_timestamped_log(&mut self, message: &str) {
        let timestamp = Local::now().format("%H:%M:%S").to_string();
        let formatted_message = format!("[{}] {}", timestamp, message);
        self.add_log(formatted_message);
    }

    /// Upper bound on the TUI's in-memory log buffer. A long-lived TUI
    /// session (especially with rapid diff-filter toggles, each of
    /// which appends 2+ entries) previously grew unbounded — the
    /// review flagged this as a slow-leak hazard.
    ///
    /// **Why 5000?** Sized against the two dominant log sources:
    ///
    ///   - The executor emits roughly 1-3 lines per workflow step
    ///     (status + stdout/stderr header). A typical session running
    ///     a handful of workflows with ~10 steps each stays well below
    ///     1000 lines per run.
    ///   - Diff-filter toggles and watch-style re-evaluations emit
    ///     2-5 lines per cycle (summary + matched/skipped breakdown +
    ///     optional warnings). Even at a frenzied toggle-per-second
    ///     pace this produces ~300 lines/minute.
    ///
    /// 5000 lines therefore holds ~15 minutes of aggressive toggling
    /// or several full multi-workflow runs of scrollback — enough for
    /// a user to debug the most recent failure without bloating RSS
    /// by the multi-megabyte String heap that an unbounded buffer
    /// eventually produces in a day-long session. Below ~1000 the
    /// cap starts losing context mid-run; above ~20000 the heap
    /// footprint becomes visible on slow machines. If a future TUI
    /// gains a "save full transcript" feature, route it to a file
    /// sink rather than holding the transcript in this in-memory
    /// buffer — the cap should stay in the 5000 neighbourhood
    /// regardless of scrollback-export needs.
    const LOG_BUFFER_CAP: usize = 5000;

    /// Enforce [`LOG_BUFFER_CAP`] by dropping the oldest entries
    /// until the buffer is within bounds. Called from every
    /// [`add_log`] path AND from [`trim_logs_to_cap`] so ad-hoc
    /// `self.logs.push(...)` sites can opt into the cap by calling
    /// this once after pushing.
    ///
    /// Uses `drain(0..N)` instead of rebuilding the vec so the tail
    /// entries don't get re-cloned; the operation is O(n) in the
    /// number of *dropped* entries, which is zero on the fast path.
    pub fn trim_logs_to_cap(&mut self) {
        if self.logs.len() > Self::LOG_BUFFER_CAP {
            let excess = self.logs.len() - Self::LOG_BUFFER_CAP;
            self.logs.drain(0..excess);
        }
    }
}

/// Run git + trigger evaluation as an async task on the ambient runtime.
///
/// Returns a [`DiffFilterOutcome`] so the UI can distinguish "evaluation
/// ran and produced rows" from "no event context could be built" — the
/// latter is typically a missing-default-branch or missing-commits error
/// that the user needs a real explanation for.
///
/// The event name is passed through from the caller rather than hardcoded,
/// so a future TUI change that adds event selection is a plumbing-only
/// change here.
async fn evaluate_diff_filter(
    workflow_paths: Vec<PathBuf>,
    event_name: String,
    activity_type: Option<String>,
    repo_root: Option<PathBuf>,
) -> DiffFilterOutcome {
    // Nothing to evaluate — bail out before paying for the git subprocess
    // calls. Without this, toggling the diff filter on an empty workflow
    // list would still shell out to `git rev-parse`/`git diff`/`git
    // describe` and just throw the result away. Mirrors the watcher's
    // `configs.is_empty()` short-circuit in `evaluate_and_execute`.
    if workflow_paths.is_empty() {
        return DiffFilterOutcome::Success(DiffFilterReport {
            rows: Vec::new(),
            parse_failures: Vec::new(),
            warnings: Vec::new(),
        });
    }

    // Pass the discovered repo root through to every git helper so the
    // diff/branch/tag queries run against the user's actual repo, not
    // whatever the process CWD happens to be. `None` is still tolerated
    // (e.g. user launched the TUI outside any repo) — the helpers will
    // surface a `GitError` and the TUI will log it.
    let cwd: Option<&Path> = repo_root.as_deref();
    // The TUI is a hot-path host: the user may toggle the diff filter
    // many times during a session, and the dirty-tree info message
    // that `get_default_diff_base` emits would flood the log pane
    // every toggle. Pass `verbose = false` so the CLI's loud message
    // stays quiet here; users who need the explanation run `wrkflw
    // run --diff --verbose` on the command line.
    let mut context = match wrkflw_trigger_filter::auto_detect_context_default_base(
        &event_name,
        cwd,
        false,
    )
    .await
    {
        Ok(ctx) => ctx,
        Err(e) => {
            return DiffFilterOutcome::Failure(format!("{}", e));
        }
    };
    // Stamp the activity type so workflows that filter on
    // `pull_request: { types: [...] }` can match. Mirrors the
    // CLI prefilter and the watcher; without it the TUI silently
    // rejects every typed pull_request workflow.
    context.activity_type = activity_type;

    // Drain the context-level warnings into our own accumulator
    // BEFORE we hand the context to `filter_trigger_configs`. The
    // library routes these through `EventContext::warnings` instead
    // of calling `wrkflw_logging::warning` directly on purpose —
    // hosts own the rendering policy — so every host MUST consume
    // them or reproduce the silent-skip failure mode the rest of
    // this PR is built to plug. The CLI prefilter does this at
    // `crates/wrkflw/src/main.rs`; parity is load-bearing.
    //
    // `MustDrainWarnings::take()` leaves an empty buffer in its
    // place so the downstream `filter_trigger_configs` call still
    // sees a well-formed context without any of the cost of a
    // clone. `take` (not read-only iteration) is also what
    // satisfies the `MustDrainWarnings` Drop-check contract — if we
    // borrowed instead, the context's Drop would fire the
    // "dropped without being drained" eprintln in debug builds.
    let mut warnings: Vec<String> = context.warnings.take();

    // Trigger config parsing is synchronous file I/O; run it on a
    // blocking thread so we don't hold the reactor while reading every
    // .yml in the repo. `load_trigger_configs` consolidates read + parse
    // and partitions the result into successes + per-file failures, so
    // the TUI and the watcher fail identically on the same broken file
    // and the failure branch is never silently dropped.
    let paths_for_parse = workflow_paths.clone();
    // Route through the shared LRU so toggling the TUI diff filter
    // repeatedly on the same workflows pays the parse cost exactly
    // once per (path, mtime). The CLI prefilter and the watcher hit
    // the same cache — unifying the three entry points was a review
    // ask specifically to prevent future drift.
    let tf_config = wrkflw_trigger_filter::TriggerFilterConfig::default();
    let parse_outcome: Result<
        (
            Vec<wrkflw_trigger_filter::WorkflowTriggerConfig>,
            DiffFilterParseFailures,
        ),
        _,
    > = tokio::task::spawn_blocking(move || {
        wrkflw_trigger_filter::load_trigger_configs_cached(&paths_for_parse, &tf_config)
    })
    .await;

    let (mut configs, parse_failures) = match parse_outcome {
        Ok(pair) => pair,
        Err(e) => {
            return DiffFilterOutcome::Failure(format!("background task failed: {}", e));
        }
    };

    // Harvest per-workflow parser warnings (unknown event names, etc.)
    // the same way the CLI prefilter at `crates/wrkflw/src/main.rs`
    // does. `parse_trigger_config` stores typo-detection diagnostics
    // on `WorkflowTriggerConfig::warnings` instead of logging them,
    // so each successfully-parsed config may still carry a warning.
    // Prefixing with the workflow path lets the log pane point the
    // user at exactly which file has the problem.
    //
    // `.take()` (not read-only iteration) is load-bearing: it is
    // what satisfies the `MustDrainWarnings` Drop-check contract.
    // A borrow-only `for w in &cfg.warnings` would leave the
    // buffer non-empty on `cfg` drop, and the debug-build Drop
    // impl would fire the "dropped without being drained"
    // eprintln. Keeping this draining form also prevents the
    // silent-skip regression the Drop check was designed to catch.
    for cfg in configs.iter_mut() {
        let path_display = cfg.workflow_path.display().to_string();
        for w in cfg.warnings.take() {
            warnings.push(format!("{}: {}", path_display, w));
        }
    }

    let borrowed: Vec<&wrkflw_trigger_filter::WorkflowTriggerConfig> = configs.iter().collect();
    let results = wrkflw_trigger_filter::filter_trigger_configs(&borrowed, &context);
    let results_by_path: std::collections::HashMap<
        PathBuf,
        &wrkflw_trigger_filter::TriggerMatchResult,
    > = results
        .iter()
        .map(|r| (r.workflow_path.clone(), r))
        .collect();

    let rows = workflow_paths
        .into_iter()
        .map(|path| {
            let status = results_by_path.get(&path).map(|result| {
                if result.matches {
                    TriggerMatchStatus::Matched(result.reason.clone())
                } else {
                    TriggerMatchStatus::Skipped(result.reason.clone())
                }
            });
            (path, status)
        })
        .collect();

    DiffFilterOutcome::Success(DiffFilterReport {
        rows,
        parse_failures,
        warnings,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::PathBuf;

    fn make_app() -> App {
        let (tx, _rx) = mpsc::channel();
        let mut app = App::new(RuntimeType::Emulation, tx, false, false);
        app.workflows = vec![
            Workflow {
                name: "ci".to_string(),
                path: PathBuf::from("ci.yml"),
                selected: false,
                status: WorkflowStatus::NotStarted,
                execution_details: None,
                job_names: vec!["build".to_string(), "lint".to_string(), "test".to_string()],
                trigger_match: None,
                definition: None,
            },
            Workflow {
                name: "deploy".to_string(),
                path: PathBuf::from("deploy.yml"),
                selected: false,
                status: WorkflowStatus::NotStarted,
                execution_details: None,
                job_names: vec![],
                trigger_match: None,
                definition: None,
            },
        ];
        app.workflow_list_state.select(Some(0));
        app
    }

    #[test]
    fn log_buffer_caps_at_configured_size() {
        // Long-running TUI sessions (especially with rapid diff-filter
        // toggles) previously grew `logs` unbounded. The cap has to
        // hold even when callers push directly to `self.logs` without
        // going through `add_log`, because every render path eventually
        // routes through `mark_logs_for_update`. Verify the cap kicks
        // in on both shapes.
        let mut app = make_app();
        // Start from a clean slate so the setup-time log lines do not
        // pollute the size assertion.
        app.logs.clear();
        for i in 0..(App::LOG_BUFFER_CAP + 200) {
            app.logs.push(format!("line {}", i));
        }
        app.mark_logs_for_update();
        assert_eq!(
            app.logs.len(),
            App::LOG_BUFFER_CAP,
            "log buffer must be capped even for direct pushes routed through mark_logs_for_update"
        );
        // The tail must be the most recent entries, not the oldest.
        assert!(
            app.logs
                .last()
                .unwrap()
                .contains(&format!("{}", App::LOG_BUFFER_CAP + 199)),
            "newest entry must survive the drain, got {:?}",
            app.logs.last()
        );
    }

    #[test]
    fn cycle_diff_filter_event_rotates_through_known_events() {
        let mut app = make_app();
        assert_eq!(app.diff_filter_event, "push");
        app.cycle_diff_filter_event();
        assert_eq!(app.diff_filter_event, "pull_request");
        app.cycle_diff_filter_event();
        assert_eq!(app.diff_filter_event, "pull_request_target");
        // Six-step rotation wraps back to push.
        for _ in 0..4 {
            app.cycle_diff_filter_event();
        }
        assert_eq!(
            app.diff_filter_event, "push",
            "rotation must wrap after exhausting the known-event list"
        );
    }

    #[test]
    fn enter_job_selection_mode_populates_jobs() {
        let mut app = make_app();
        app.enter_job_selection_mode();

        assert!(app.job_selection_mode);
        assert_eq!(app.available_jobs, vec!["build", "lint", "test"]);
        assert_eq!(app.selected_job_index, 0);
    }

    #[test]
    fn enter_job_selection_mode_no_jobs_stays_in_normal_mode() {
        let mut app = make_app();
        app.workflow_list_state.select(Some(1)); // deploy has no jobs
        app.enter_job_selection_mode();

        assert!(!app.job_selection_mode);
        assert!(app.available_jobs.is_empty());
    }

    #[test]
    fn exit_job_selection_mode_clears_state() {
        let mut app = make_app();
        app.enter_job_selection_mode();
        app.selected_job_index = 2;
        app.exit_job_selection_mode();

        assert!(!app.job_selection_mode);
        assert!(app.available_jobs.is_empty());
        assert_eq!(app.selected_job_index, 0);
    }

    #[test]
    fn next_available_job_wraps_around() {
        let mut app = make_app();
        app.enter_job_selection_mode();

        app.next_available_job(); // 0 -> 1
        assert_eq!(app.selected_job_index, 1);
        app.next_available_job(); // 1 -> 2
        assert_eq!(app.selected_job_index, 2);
        app.next_available_job(); // 2 -> 0 (wrap)
        assert_eq!(app.selected_job_index, 0);
    }

    #[test]
    fn previous_available_job_wraps_around() {
        let mut app = make_app();
        app.enter_job_selection_mode();

        app.previous_available_job(); // 0 -> 2 (wrap)
        assert_eq!(app.selected_job_index, 2);
        app.previous_available_job(); // 2 -> 1
        assert_eq!(app.selected_job_index, 1);
        app.previous_available_job(); // 1 -> 0
        assert_eq!(app.selected_job_index, 0);
    }

    #[test]
    fn navigate_jobs_noop_when_empty() {
        let mut app = make_app();
        // Don't enter job selection mode — available_jobs is empty
        app.next_available_job();
        assert_eq!(app.selected_job_index, 0);
        app.previous_available_job();
        assert_eq!(app.selected_job_index, 0);
    }

    #[test]
    fn check_diff_filter_results_keys_by_path_not_position() {
        // Regression: previously the result was zipped positionally with
        // self.workflows. If the workflow list reloaded between toggle and
        // result delivery, statuses would land on the wrong workflow.
        let mut app = make_app();
        // Simulate a reload that reorders workflows AFTER we captured paths
        app.workflows = vec![
            Workflow {
                name: "deploy".into(),
                path: PathBuf::from("deploy.yml"),
                selected: false,
                status: WorkflowStatus::NotStarted,
                execution_details: None,
                job_names: vec![],
                trigger_match: None,
                definition: None,
            },
            Workflow {
                name: "ci".into(),
                path: PathBuf::from("ci.yml"),
                selected: false,
                status: WorkflowStatus::NotStarted,
                execution_details: None,
                job_names: vec![],
                trigger_match: None,
                definition: None,
            },
        ];

        // Background thread reports results keyed to the OLD order
        let (tx, rx) = mpsc::channel();
        app.diff_filter_rx = Some(rx);
        app.diff_filter_active = true;
        tx.send(DiffFilterOutcome::Success(DiffFilterReport {
            rows: vec![
                (
                    PathBuf::from("ci.yml"),
                    Some(TriggerMatchStatus::Matched("matched ci".into())),
                ),
                (
                    PathBuf::from("deploy.yml"),
                    Some(TriggerMatchStatus::Skipped("skipped deploy".into())),
                ),
            ],
            parse_failures: Vec::new(),
            warnings: Vec::new(),
        }))
        .unwrap();

        app.check_diff_filter_results();

        // After applying, ci.yml must be Matched and deploy.yml Skipped
        // regardless of their position in self.workflows.
        let by_name: std::collections::HashMap<&str, &Option<TriggerMatchStatus>> = app
            .workflows
            .iter()
            .map(|w| (w.name.as_str(), &w.trigger_match))
            .collect();
        assert!(matches!(
            by_name.get("ci").unwrap(),
            Some(TriggerMatchStatus::Matched(_))
        ));
        assert!(matches!(
            by_name.get("deploy").unwrap(),
            Some(TriggerMatchStatus::Skipped(_))
        ));
    }

    #[test]
    fn check_diff_filter_results_surfaces_parse_failures_to_logs() {
        // Regression: previously, workflows whose YAML failed to parse
        // were silently dropped via `.filter_map(... .ok())`. The user
        // saw `0/N would trigger`, the workflow row stayed at `-`, and
        // there was no signal that the YAML was broken. After the fix,
        // each parse failure must appear in the log pane with its
        // path + error reason.
        let mut app = make_app();
        let log_count_before = app.logs.len();

        let (tx, rx) = mpsc::channel();
        app.diff_filter_rx = Some(rx);
        app.diff_filter_active = true;
        tx.send(DiffFilterOutcome::Success(DiffFilterReport {
            rows: vec![(
                PathBuf::from("ci.yml"),
                Some(TriggerMatchStatus::Matched("matched ci".into())),
            )],
            parse_failures: vec![(
                PathBuf::from("broken.yml"),
                "Invalid glob pattern '[unclosed' under 'push.paths'".to_string(),
            )],
            warnings: Vec::new(),
        }))
        .unwrap();

        app.check_diff_filter_results();

        let new_logs: Vec<&String> = app.logs.iter().skip(log_count_before).collect();
        assert!(
            new_logs.iter().any(|l| l.contains("failed to parse")),
            "expected parse-failure summary line in logs, got {:?}",
            new_logs
        );
        assert!(
            new_logs
                .iter()
                .any(|l| l.contains("broken.yml") && l.contains("[unclosed")),
            "expected per-file parse error line in logs, got {:?}",
            new_logs
        );
    }

    #[test]
    fn check_diff_filter_results_surfaces_context_and_parser_warnings_to_logs() {
        // Regression: previously the TUI dropped every
        // `EventContext::warnings` and every `WorkflowTriggerConfig::warnings`
        // on the floor, even though the library deliberately routes
        // those through struct fields so hosts own the rendering
        // policy. The CLI prefilter at `crates/wrkflw/src/main.rs`
        // logs both sources via `wrkflw_logging::warning`; the TUI must
        // produce matching output in its log pane or reproduce the
        // silent-skip failure mode the rest of this PR was built to
        // plug (e.g. a `git ls-files --others` failure silently drops
        // untracked files from the change set; an `on: pul_request`
        // typo never surfaces; every workflow shows `-` with no clue).
        //
        // This test covers TWO warning sources in one payload:
        //   1. A context-level warning (think: `git ls-files --others`
        //      safe-directory rejection).
        //   2. A parser-level warning already prefixed with the
        //      workflow path, the same shape `evaluate_diff_filter`
        //      produces when it harvests `WorkflowTriggerConfig::warnings`.
        // Both must show up in the log burst, and the summary count
        // line must match the number of warnings delivered.
        let mut app = make_app();
        let log_count_before = app.logs.len();

        let (tx, rx) = mpsc::channel();
        app.diff_filter_rx = Some(rx);
        app.diff_filter_active = true;
        tx.send(DiffFilterOutcome::Success(DiffFilterReport {
            rows: vec![(
                PathBuf::from("ci.yml"),
                Some(TriggerMatchStatus::Matched("matched ci".into())),
            )],
            parse_failures: Vec::new(),
            warnings: vec![
                "git ls-files --others failed (exit 128): fatal: unsafe repository".to_string(),
                ".github/workflows/ci.yml: workflow test.yml uses unknown event 'pul_request'"
                    .to_string(),
            ],
        }))
        .unwrap();

        app.check_diff_filter_results();

        let new_logs: Vec<&String> = app.logs.iter().skip(log_count_before).collect();
        assert!(
            new_logs
                .iter()
                .any(|l| l.contains("Diff filter: 2 warning(s)")),
            "expected warning-count summary line in logs, got {:?}",
            new_logs
        );
        assert!(
            new_logs
                .iter()
                .any(|l| l.contains("git ls-files --others failed")),
            "context warning must surface in logs, got {:?}",
            new_logs
        );
        assert!(
            new_logs.iter().any(|l| l.contains("pul_request")),
            "parser warning (unknown event typo) must surface in logs, got {:?}",
            new_logs
        );
    }

    #[test]
    fn check_diff_filter_results_surfaces_failure_reason_to_logs() {
        // Regression: previously, if auto_detect_context_default_base
        // errored (e.g. fresh repo with no remote default branch), the
        // TUI silently showed every workflow as None and the summary
        // line said "0/N workflows would trigger" with no explanation.
        let mut app = make_app();
        let log_count_before = app.logs.len();

        let (tx, rx) = mpsc::channel();
        app.diff_filter_rx = Some(rx);
        app.diff_filter_active = true;
        tx.send(DiffFilterOutcome::Failure(
            "could not detect a diff base".into(),
        ))
        .unwrap();

        app.check_diff_filter_results();

        // The failure reason should be visible in the log, not silently dropped.
        let new_logs: Vec<&String> = app.logs.iter().skip(log_count_before).collect();
        assert!(
            new_logs.iter().any(|l| l.contains("could not detect")),
            "expected failure reason in logs, got {:?}",
            new_logs
        );
        // All workflows must have trigger_match cleared on failure.
        for wf in &app.workflows {
            assert!(wf.trigger_match.is_none());
        }
    }

    #[test]
    fn check_diff_filter_results_silences_self_inflicted_disconnect() {
        // Regression: previously, dropping the receiver (e.g. via a rapid
        // toggle that aborted the in-flight task) reached the
        // `Disconnected` arm and logged "evaluation failed" — misleading,
        // because the user took the action themselves. After the fix, a
        // disconnect that follows an `aborted` flag must be silent, and
        // the flag must be cleared so the *next* genuine failure is still
        // surfaced loudly.
        let mut app = make_app();
        let log_count_before = app.logs.len();

        // Build a channel and immediately drop the sender to simulate an
        // aborted background task: the next try_recv will see Disconnected.
        let (tx, rx) = mpsc::channel::<DiffFilterOutcome>();
        drop(tx);
        app.diff_filter_rx = Some(rx);
        app.diff_filter_aborted = true;

        app.check_diff_filter_results();

        // No "evaluation failed" line should have been added.
        let new_logs: Vec<&String> = app.logs.iter().skip(log_count_before).collect();
        assert!(
            !new_logs.iter().any(|l| l.contains("evaluation failed")),
            "self-inflicted abort must not log a failure, got {:?}",
            new_logs
        );
        // Flag must be consumed so the next disconnect is loud again.
        assert!(
            !app.diff_filter_aborted,
            "abort flag must be cleared after being observed"
        );

        // Now simulate a real failure (no abort flag set) on a fresh
        // disconnect — it should produce a log line.
        let log_count_after_silent = app.logs.len();
        let (tx2, rx2) = mpsc::channel::<DiffFilterOutcome>();
        drop(tx2);
        app.diff_filter_rx = Some(rx2);
        // Note: aborted flag is intentionally NOT set this time.

        app.check_diff_filter_results();

        let final_logs: Vec<&String> = app.logs.iter().skip(log_count_after_silent).collect();
        assert!(
            final_logs.iter().any(|l| l.contains("evaluation failed")),
            "genuine disconnect must still be reported, got {:?}",
            final_logs
        );
    }

    #[tokio::test]
    async fn toggle_diff_filter_arms_abort_flag_only_when_task_in_flight() {
        // Toggling from a clean state (no in-flight task) must NOT arm
        // the abort flag — otherwise the *next* evaluation's genuine
        // failure would be silently swallowed.
        //
        // This test runs under `#[tokio::test]` because the active branch
        // of `toggle_diff_filter` calls `tokio::task::spawn`, which panics
        // without an ambient runtime. We don't await the spawned task
        // (the git/parse work would actually try to shell out); we only
        // assert the synchronous flag-arming behavior that runs before
        // the spawn returns.
        let mut app = make_app();
        assert!(app.diff_filter_task.is_none());
        assert!(app.diff_filter_rx.is_none());

        app.toggle_diff_filter();
        // After a fresh toggle ON, the abort flag must remain false:
        // there was nothing to abort, so the next disconnect is real.
        assert!(
            !app.diff_filter_aborted,
            "fresh toggle must not arm the abort flag"
        );

        // Toggling again — this time there IS an in-flight task and
        // receiver — must arm the flag so the resulting Disconnected
        // tick is treated as self-inflicted.
        app.toggle_diff_filter();
        assert!(
            app.diff_filter_aborted,
            "toggle with task in flight must arm the abort flag"
        );

        // Cancel the spawned task so the test doesn't leak it.
        if let Some(handle) = app.diff_filter_task.take() {
            handle.abort();
        }
    }

    #[tokio::test]
    async fn aborted_flag_does_not_silence_failures_across_toggle_cycles() {
        // Regression for the stale-abort-flag bug: when the user toggles
        // OFF (which arms the flag and drops the receiver) and the next
        // tick observes `rx is None` and returns early, the flag is left
        // armed. The *next* toggle ON spawns a fresh task with a fresh
        // receiver, and a real failure on that fresh task used to be
        // silenced because the leftover flag was treated as a self-
        // inflicted abort. After the fix, starting a new evaluation
        // must clear any stale flag so a genuine failure is loud.
        let mut app = make_app();

        // Step 1: Toggle ON — spawns task A.
        app.toggle_diff_filter();
        assert!(app.diff_filter_task.is_some(), "task A should be in flight");
        assert!(!app.diff_filter_aborted);

        // Step 2: Toggle OFF — aborts A, arms flag, drops rx.
        app.toggle_diff_filter();
        assert!(app.diff_filter_aborted, "OFF with in-flight task arms flag");
        assert!(app.diff_filter_rx.is_none());

        // Step 3: A tick fires while OFF. With rx=None it returns early
        // and never observes/clears the flag — this is the gap that the
        // old code left behind.
        app.check_diff_filter_results();
        assert!(
            app.diff_filter_aborted,
            "early-return tick must not touch the flag"
        );

        // Step 4: Toggle ON again. The fix is here: starting a new
        // evaluation must clear the stale flag so the next failure is
        // not mistaken for a self-inflicted abort.
        app.toggle_diff_filter();
        assert!(
            !app.diff_filter_aborted,
            "stale abort flag from a prior cycle must be cleared on new evaluation"
        );

        // Step 5: Cancel the real spawned task and inject a fresh
        // already-disconnected channel to simulate task B failing
        // (panic, send-side dropped). The flag is currently false, so
        // the disconnect must be reported as a genuine failure.
        if let Some(handle) = app.diff_filter_task.take() {
            handle.abort();
        }
        let (tx, rx) = mpsc::channel::<DiffFilterOutcome>();
        drop(tx);
        app.diff_filter_rx = Some(rx);

        let log_count_before = app.logs.len();
        app.check_diff_filter_results();
        let new_logs: Vec<&String> = app.logs.iter().skip(log_count_before).collect();
        assert!(
            new_logs.iter().any(|l| l.contains("evaluation failed")),
            "real failure on a fresh evaluation must surface, got {:?}",
            new_logs
        );
    }

    #[test]
    fn run_from_job_selection_queues_with_target_job() {
        let mut app = make_app();
        app.enter_job_selection_mode();
        app.run_from_job_selection(Some("build".to_string()));

        assert!(!app.job_selection_mode);
        assert!(app.available_jobs.is_empty());
        assert_eq!(app.execution_queue.len(), 1);
        assert_eq!(app.execution_queue[0].workflow_idx, 0);
        assert_eq!(app.execution_queue[0].target_job, Some("build".to_string()));
    }

    #[test]
    fn run_from_job_selection_none_queues_all_jobs() {
        let mut app = make_app();
        app.enter_job_selection_mode();
        app.run_from_job_selection(None);

        assert_eq!(app.execution_queue.len(), 1);
        assert_eq!(app.execution_queue[0].target_job, None);
    }

    #[test]
    fn run_from_job_selection_allows_same_workflow_different_jobs() {
        let mut app = make_app();

        app.enter_job_selection_mode();
        app.run_from_job_selection(Some("build".to_string()));

        // Drain the queue to simulate the executor consuming it
        app.execution_queue.clear();
        app.current_execution = None;
        app.running = false;

        app.enter_job_selection_mode();
        app.run_from_job_selection(Some("test".to_string()));

        assert_eq!(app.execution_queue.len(), 1);
        assert_eq!(app.execution_queue[0].target_job, Some("test".to_string()));
    }

    #[test]
    fn get_next_workflow_to_execute_threads_target_job() {
        let mut app = make_app();
        app.execution_queue.push(QueuedExecution {
            workflow_idx: 0,
            target_job: Some("lint".to_string()),
        });

        let result = app.get_next_workflow_to_execute();
        assert!(result.is_some());
        let (idx, target) = result.unwrap();
        assert_eq!(idx, 0);
        assert_eq!(target, Some("lint".to_string()));
        assert!(app.execution_queue.is_empty());
    }

    #[test]
    fn get_next_workflow_to_execute_returns_none_when_empty() {
        let mut app = make_app();
        assert!(app.get_next_workflow_to_execute().is_none());
    }

    #[test]
    fn single_job_navigation_wraps_correctly() {
        let mut app = make_app();
        app.available_jobs = vec!["only-job".to_string()];
        app.selected_job_index = 0;

        app.next_available_job(); // 0 -> 0 (only one item)
        assert_eq!(app.selected_job_index, 0);
        app.previous_available_job(); // 0 -> 0
        assert_eq!(app.selected_job_index, 0);
    }

    #[test]
    fn run_from_job_selection_noop_when_no_workflow_selected() {
        let mut app = make_app();
        app.workflow_list_state.select(None);
        app.job_selection_mode = true;
        app.available_jobs = vec!["build".to_string()];

        app.run_from_job_selection(Some("build".to_string()));

        assert!(app.execution_queue.is_empty());
        assert!(!app.job_selection_mode);
        assert!(app.available_jobs.is_empty());
    }

    #[test]
    fn enter_job_selection_mode_noop_when_index_out_of_bounds() {
        let mut app = make_app();
        app.workflow_list_state.select(Some(99)); // out of bounds

        app.enter_job_selection_mode();

        assert!(!app.job_selection_mode);
        assert!(app.available_jobs.is_empty());
    }
}
