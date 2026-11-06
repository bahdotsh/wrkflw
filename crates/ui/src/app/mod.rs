// App module for UI state and main TUI entry point
mod state;

use crate::handlers::workflow::start_next_workflow_execution;
use crate::models::{ExecutionResultMsg, QueuedExecution, Workflow, WorkflowStatus};
use crate::selection;
use crate::utils::load_workflows;
use crate::views::{
    render_ui, tab_of, TAB_COUNT, TAB_DAG, TAB_EXECUTION, TAB_LOGS, TAB_RUNS, TAB_SECRETS,
    TAB_TRIGGER, TAB_WORKFLOWS,
};
use crossterm::{
    event::{
        self, DisableMouseCapture, EnableMouseCapture, Event, KeyCode, KeyEvent, KeyEventKind,
        KeyModifiers,
    },
    execute,
    terminal::{disable_raw_mode, enable_raw_mode, EnterAlternateScreen, LeaveAlternateScreen},
};
use ratatui::{backend::CrosstermBackend, Terminal};
use std::collections::HashMap;
use std::io::{self, stdout};
use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{mpsc, Mutex};
use std::time::{Duration, Instant};
use wrkflw_executor::RuntimeType;

use crate::theme::ThemePreference;
use crate::views::INSPECTOR_TABS;

pub use state::{Accent, App, TriggerPlatform};

// Main entry point for the TUI interface
#[allow(clippy::ptr_arg, clippy::too_many_arguments)]
pub async fn run_wrkflw_tui(
    path: Option<&PathBuf>,
    runtime_type: RuntimeType,
    verbose: bool,
    preserve_containers_on_failure: bool,
    show_action_messages: bool,
    theme: Option<ThemePreference>,
    platform_overrides: HashMap<String, String>,
    kubernetes: wrkflw_executor::KubernetesConfig,
) -> io::Result<()> {
    // Picks the palette, which may query the terminal, so it has to
    // happen before raw mode and the event loop take over the tty.
    crate::theme::install(theme);

    // Terminal setup. Mouse capture hands drags to us so selection can
    // stay inside one pane and copy on release; see `crate::selection`.
    install_panic_hook();
    enable_raw_mode()?;
    let mut stdout = stdout();
    execute!(stdout, EnterAlternateScreen, EnableMouseCapture)?;
    let backend = CrosstermBackend::new(stdout);
    let mut terminal = Terminal::new(backend)?;
    // Some terminals (e.g. iTerm2) carry the primary screen's contents
    // into the alternate screen. ratatui assumes it starts from a blank
    // screen and only redraws cells that change, so any widget area left
    // unpainted would keep showing that stale text. Clear explicitly so
    // the first frame starts from a known-empty screen.
    terminal.clear()?;

    // Suppress logging to stdout/stderr while TUI owns the terminal
    wrkflw_logging::set_quiet_mode(true);

    // Set up channel for async communication
    let (tx, rx): (
        mpsc::Sender<ExecutionResultMsg>,
        mpsc::Receiver<ExecutionResultMsg>,
    ) = mpsc::channel();

    // Initialize app state
    let mut app = App::new(
        runtime_type.clone(),
        tx.clone(),
        preserve_containers_on_failure,
        show_action_messages,
        platform_overrides.clone(),
        kubernetes.clone(),
    );
    app.show_debug_logs = verbose;

    if app.validation_mode {
        wrkflw_logging::info("Starting in validation mode");
    }

    // Load workflows
    let dir_paths = match path {
        Some(path) if path.is_dir() => vec![path.clone()],
        Some(path) if path.is_file() => {
            // Single workflow file
            let name = path
                .file_name()
                .unwrap_or_default()
                .to_string_lossy()
                .into_owned();

            let (definition, job_names) = {
                use std::sync::Arc;
                use wrkflw_parser::workflow::parse_workflow;
                match parse_workflow(path) {
                    Ok(def) => {
                        let mut names: Vec<String> = def.jobs.keys().cloned().collect();
                        names.sort();
                        (Some(Arc::new(def)), names)
                    }
                    Err(_) => (None, crate::utils::extract_job_names(path)),
                }
            };

            app.workflows = vec![Workflow {
                name: name.clone(),
                path: path.clone(),
                selected: true,
                status: WorkflowStatus::NotStarted,
                execution_details: None,
                job_names,
                trigger_match: None,
                definition,
            }];

            // Queue the single workflow for execution
            app.execution_queue = vec![QueuedExecution {
                workflow_idx: 0,
                target_job: None,
            }];
            app.start_execution();

            // Return parent dir or current dir if no parent
            vec![path
                .parent()
                .map(|p| p.to_path_buf())
                .unwrap_or_else(|| PathBuf::from("."))]
        }
        _ => wrkflw_utils::default_workflow_dirs(),
    };

    // Only load directory if we haven't already loaded a single file
    if app.workflows.is_empty() {
        app.workflows = dir_paths
            .iter()
            .flat_map(|dir| load_workflows(dir))
            .collect();
    }

    // History feeds the Workflows preview's "last run" line.
    app.refresh_runs();

    // Run the main event loop
    let tx_clone = tx.clone();

    // Run the event loop
    let result = run_tui_event_loop(&mut terminal, &mut app, &tx_clone, &rx, verbose);

    // Clean up terminal
    TUI_ACTIVE.store(false, Ordering::SeqCst);
    wrkflw_logging::set_quiet_mode(false);
    disable_raw_mode()?;
    execute!(
        terminal.backend_mut(),
        LeaveAlternateScreen,
        DisableMouseCapture
    )?;
    terminal.show_cursor()?;

    match result {
        Ok(_) => Ok(()),
        Err(e) => {
            // If the TUI fails to initialize or crashes, fall back to CLI mode
            wrkflw_logging::error(&format!("Failed to start UI: {}", e));

            // Only for 'tui' command should we fall back to CLI mode for files
            // For other commands, return the error
            if let Some(path) = path {
                if path.is_file() {
                    wrkflw_logging::error("Falling back to CLI mode...");
                    crate::handlers::workflow::execute_workflow_cli(
                        path,
                        runtime_type,
                        verbose,
                        show_action_messages,
                        platform_overrides,
                        kubernetes,
                    )
                    .await
                } else if path.is_dir() {
                    crate::handlers::workflow::validate_workflow(path, verbose)
                } else {
                    Err(e)
                }
            } else {
                Err(e)
            }
        }
    }
}

/// True while the TUI owns the terminal; the panic hook only steps in
/// then.
static TUI_ACTIVE: AtomicBool = AtomicBool::new(false);

/// Panics from threads other than the UI thread, waiting to be shown in
/// the Logs tab.
static BACKGROUND_PANICS: Mutex<Vec<String>> = Mutex::new(Vec::new());

/// Puts the terminal back before a panic on the UI thread prints, so a
/// crash leaves a usable shell and a readable error instead of raw mode
/// on the alternate screen.
///
/// Other threads (the log processor, probes, spawned tasks such as a
/// Trigger dispatch) are left alone: the TUI survived their panics
/// before and still does. Printing their message to stderr would
/// scribble over the alternate screen, so it is queued for the event
/// loop to log instead. `try_lock` keeps the hook from deadlocking if
/// the panic hit while that lock was held.
fn install_panic_hook() {
    let ui_thread = std::thread::current().id();
    let previous = std::panic::take_hook();
    std::panic::set_hook(Box::new(move |info| {
        if !TUI_ACTIVE.load(Ordering::SeqCst) {
            previous(info);
            return;
        }
        let current = std::thread::current();
        if current.id() == ui_thread {
            let _ = disable_raw_mode();
            let _ = execute!(io::stdout(), LeaveAlternateScreen, DisableMouseCapture);
            previous(info);
            return;
        }
        if let Ok(mut queue) = BACKGROUND_PANICS.try_lock() {
            queue.push(format!(
                "thread '{}' panicked: {info}",
                current.name().unwrap_or("<unnamed>")
            ));
        }
    }));
    TUI_ACTIVE.store(true, Ordering::SeqCst);
}

/// Moves queued background-thread panics into the log and status bar.
fn report_background_panics(app: &mut App) {
    let drained: Vec<String> = match BACKGROUND_PANICS.lock() {
        Ok(mut queue) if !queue.is_empty() => std::mem::take(&mut *queue),
        _ => return,
    };
    for msg in &drained {
        wrkflw_logging::error(msg);
    }
    if let Some(last) = drained.last() {
        app.set_error_message(last.clone());
    }
}

// Helper function to run the main event loop
#[allow(clippy::collapsible_match)]
fn run_tui_event_loop(
    terminal: &mut Terminal<CrosstermBackend<io::Stdout>>,
    app: &mut App,
    tx_clone: &mpsc::Sender<ExecutionResultMsg>,
    rx: &mpsc::Receiver<ExecutionResultMsg>,
    verbose: bool,
) -> io::Result<()> {
    // Max time to wait for events - keep this short to ensure UI responsiveness
    let event_poll_timeout = Duration::from_millis(50);

    // Set up a dedicated tick timer
    let tick_rate = app.tick_rate;
    let mut last_tick = Instant::now();

    loop {
        // Always redraw the UI on each loop iteration to keep it responsive
        let mut copied = None;
        terminal.draw(|f| {
            render_ui(f, app);
            copied = selection::apply(&mut app.selection, f.buffer_mut());
        })?;
        if let Some(text) = copied {
            app.report_copy(&text, selection::copy_to_clipboard(&text));
        }

        // Update the UI on every tick
        if last_tick.elapsed() >= tick_rate {
            app.tick();
            app.update_running_workflow_progress();

            // Check for log processing updates (includes system log change detection)
            app.check_log_processing_updates();

            // Request log processing if needed
            if app.logs_need_update {
                app.request_log_processing_update();
            }

            // Check for completed diff filter results
            app.check_diff_filter_results();

            last_tick = Instant::now();
        }

        // Timeline first: a run sends its last events before its result.
        app.drain_timeline();

        // Non-blocking check for execution results
        if let Ok((workflow_idx, result)) = rx.try_recv() {
            app.finish_execution(workflow_idx, result);
            app.refresh_runs();

            // Get next workflow to execute using our helper function
            start_next_workflow_execution(app, tx_clone, verbose);
        }

        // Surface any completed Trigger-tab dispatches on the status bar
        // so the user sees the outcome where they fired it, rather than
        // having to switch tabs to Logs.
        app.drain_trigger_outcomes();
        report_background_panics(app);

        // Start execution if we have a queued workflow and nothing is currently running
        if app.running && app.current_execution.is_none() && !app.execution_queue.is_empty() {
            start_next_workflow_execution(app, tx_clone, verbose);
        }

        // Handle key events with a short timeout
        if event::poll(event_poll_timeout)? {
            let event = match event::read()? {
                Event::Mouse(mouse) => match selection::handle_mouse(&mut app.selection, mouse) {
                    Some(code) => Event::Key(KeyEvent::from(code)),
                    None => continue,
                },
                // Selections are in screen cells, which a resize reflows.
                Event::Resize(..) => {
                    app.selection = None;
                    continue;
                }
                other => other,
            };
            if let Event::Key(key) = event {
                // Windows (and terminals using the kitty keyboard
                // protocol) report key releases too; acting on them would
                // fire every binding twice. Repeats are kept so a held
                // arrow key keeps scrolling.
                if key.kind == KeyEventKind::Release {
                    continue;
                }
                if key.code == KeyCode::Char('c') && key.modifiers.contains(KeyModifiers::CONTROL) {
                    break Ok(());
                }

                // Any key dismisses a selection. Esc stops there, so
                // clearing a highlight can't also quit the TUI.
                if app.selection.take().is_some() && key.code == KeyCode::Esc {
                    continue;
                }

                // Handle search input first if we're in search mode and logs tab
                if app.selected_tab == TAB_LOGS && app.log_search_active {
                    app.handle_log_search_input(key.code);
                    continue;
                }

                // When the Tweaks overlay is open it is modal: only
                // its own shortcuts are honoured, everything else is
                // swallowed so keys like `d` or a tab number can't
                // silently fire the global handler while the user is
                // in edit-mode. `q` is the one exception — quit is
                // universally modal-safe in this TUI and swallowing
                // it silently was a discoverability trap.
                if app.tweaks_open {
                    match key.code {
                        KeyCode::Char('q') => {
                            break Ok(());
                        }
                        KeyCode::Esc | KeyCode::Char(',') => {
                            app.tweaks_open = false;
                        }
                        KeyCode::Char('a') | KeyCode::Char('A') => {
                            app.tweaks_accent = app.tweaks_accent.next();
                        }
                        _ => {}
                    }
                    continue;
                }

                // Trigger tab: if a text field (branch or an input
                // row) is focused, route printable characters and
                // edit keys straight to that field before the global
                // key-map fires. Otherwise editing a value like
                // "notify=slack" would trip the `s` logs shortcut
                // below and jump tabs.
                if app.selected_tab == TAB_TRIGGER
                    && app.trigger_editing()
                    && app.trigger_handle_input_key(key.code)
                {
                    continue;
                }

                // The help overlay is modal: it scrolls, closes, or quits.
                if app.show_help {
                    match key.code {
                        KeyCode::Up | KeyCode::Char('k') => app.scroll_help_up(),
                        KeyCode::Down | KeyCode::Char('j') => app.scroll_help_down(),
                        KeyCode::PageUp => app.help_scroll = app.help_scroll.saturating_sub(10),
                        KeyCode::PageDown => app.help_scroll = app.help_scroll.saturating_add(10),
                        KeyCode::Char('q') => break Ok(()),
                        _ => app.show_help = false,
                    }
                    continue;
                }

                // Paging keys, which no tab binds to anything else.
                match (key.code, app.selected_tab) {
                    (KeyCode::PageUp, TAB_LOGS) => app.scroll_logs_by(-20),
                    (KeyCode::PageDown, TAB_LOGS) => app.scroll_logs_by(20),
                    (KeyCode::Home, TAB_LOGS) => app.scroll_logs_to_start(),
                    (KeyCode::End, TAB_LOGS) => app.scroll_logs_to_end(),
                    (KeyCode::Home | KeyCode::End, TAB_WORKFLOWS) if !app.job_selection_mode => {
                        app.jump_workflow(key.code == KeyCode::End);
                        app.dag_scroll = 0;
                    }
                    // While a run goes, the graph shows it; browsing other
                    // workflows' graphs waits until it ends.
                    (KeyCode::Home | KeyCode::End, TAB_DAG) if !app.running => {
                        app.jump_workflow(key.code == KeyCode::End);
                        app.dag_scroll = 0;
                    }
                    (KeyCode::Home | KeyCode::End, TAB_EXECUTION) if !app.inspecting_output() => {
                        if app.detailed_view {
                            app.jump_step(key.code == KeyCode::End);
                        } else {
                            app.jump_job(key.code == KeyCode::End);
                        }
                    }
                    (KeyCode::Home | KeyCode::End, TAB_RUNS) => {
                        app.jump_run(key.code == KeyCode::End)
                    }
                    // Step output only scrolls on the inspector's Output
                    // sub-tab, where rendering clamps it to the text.
                    (KeyCode::PageUp, TAB_EXECUTION) if app.inspecting_output() => {
                        app.step_output_scroll = app.step_output_scroll.saturating_add(20)
                    }
                    (KeyCode::PageDown, TAB_EXECUTION) if app.inspecting_output() => {
                        app.step_output_scroll = app.step_output_scroll.saturating_sub(20)
                    }
                    (KeyCode::Home, TAB_EXECUTION) if app.inspecting_output() => {
                        app.step_output_scroll = usize::MAX
                    }
                    (KeyCode::End, TAB_EXECUTION) if app.inspecting_output() => {
                        app.step_output_scroll = 0
                    }
                    (KeyCode::Left, TAB_DAG) => app.dag_scroll = app.dag_scroll.saturating_sub(1),
                    (KeyCode::Right, TAB_DAG) => app.dag_scroll = app.dag_scroll.saturating_add(1),
                    _ => {}
                }

                match key.code {
                    KeyCode::Char('q') => {
                        // Exit and clean up
                        break Ok(());
                    }
                    KeyCode::Esc => {
                        if app.job_selection_mode {
                            app.exit_job_selection_mode();
                        } else if app.detailed_view {
                            app.detailed_view = false;
                        } else if app.show_help {
                            app.show_help = false;
                        } else {
                            // Exit and clean up
                            break Ok(());
                        }
                    }
                    KeyCode::Tab => {
                        // Inside the Step Inspector, Tab cycles inspector
                        // sub-tabs (Output / Env / Files / Matrix / Timeline);
                        // elsewhere it cycles top-level tabs.
                        if app.selected_tab == TAB_EXECUTION && app.detailed_view {
                            app.step_inspector_tab = (app.step_inspector_tab + 1) % INSPECTOR_TABS;
                        } else if app.selected_tab == TAB_TRIGGER && app.trigger_editing() {
                            // While a Trigger field is being edited, Tab
                            // moves between fields. Outside edit mode it
                            // switches tabs like everywhere else; cycling
                            // fields there dropped the cursor into Branch,
                            // where digits and `q` became text and the
                            // only way out was Esc.
                            app.trigger_tab_next_field();
                        } else {
                            app.open_tab((tab_of(app.selected_tab) + 1) % TAB_COUNT);
                        }
                    }
                    KeyCode::BackTab => {
                        if app.selected_tab == TAB_EXECUTION && app.detailed_view {
                            app.step_inspector_tab =
                                (app.step_inspector_tab + INSPECTOR_TABS - 1) % INSPECTOR_TABS;
                        } else if app.selected_tab == TAB_TRIGGER && app.trigger_editing() {
                            app.trigger_tab_prev_field();
                        } else {
                            app.open_tab((tab_of(app.selected_tab) + TAB_COUNT - 1) % TAB_COUNT);
                        }
                    }
                    KeyCode::Char(c @ '1'..='9') if (c as usize - '1' as usize) < TAB_COUNT => {
                        app.open_tab(c as usize - '1' as usize)
                    }
                    KeyCode::Char('w') => app.switch_tab(TAB_WORKFLOWS),
                    // The Run tab's three views: jobs, graph, log.
                    KeyCode::Char('x') => app.switch_tab(TAB_EXECUTION),
                    KeyCode::Char('l') => app.switch_tab(TAB_LOGS),
                    KeyCode::Char('h') => app.show_help = true,
                    KeyCode::Char(',') => {
                        // `,` toggles the Tweaks overlay anywhere (global).
                        // Chosen because it never conflicts with our single-
                        // letter tab shortcuts or with the log-search input
                        // mode (which only consumes printable chars when
                        // `log_search_active` is true, which we already
                        // handled above with a `continue`).
                        app.tweaks_open = !app.tweaks_open;
                    }
                    KeyCode::Up | KeyCode::Char('k') => match app.selected_tab {
                        TAB_WORKFLOWS => {
                            if app.job_selection_mode {
                                app.previous_available_job();
                            } else {
                                app.previous_workflow();
                            }
                        }
                        TAB_EXECUTION => {
                            if app.detailed_view {
                                app.previous_step();
                            } else {
                                app.previous_job();
                            }
                        }
                        TAB_LOGS => {
                            if !app.log_story && !app.log_search_matches.is_empty() {
                                app.previous_search_match();
                            } else {
                                app.scroll_logs_up();
                            }
                        }
                        TAB_DAG if !app.running => {
                            app.previous_workflow();
                            app.dag_scroll = 0;
                        }
                        TAB_TRIGGER => app.trigger_tab_prev_workflow(),
                        TAB_SECRETS => app.secrets_tab_prev(),
                        TAB_RUNS => app.select_previous_run(),
                        _ => {}
                    },
                    KeyCode::Down | KeyCode::Char('j') => match app.selected_tab {
                        TAB_WORKFLOWS => {
                            if app.job_selection_mode {
                                app.next_available_job();
                            } else {
                                app.next_workflow();
                            }
                        }
                        TAB_EXECUTION => {
                            if app.detailed_view {
                                app.next_step();
                            } else {
                                app.next_job();
                            }
                        }
                        TAB_LOGS => {
                            if !app.log_story && !app.log_search_matches.is_empty() {
                                app.next_search_match();
                            } else {
                                app.scroll_logs_down();
                            }
                        }
                        TAB_DAG if !app.running => {
                            app.next_workflow();
                            app.dag_scroll = 0;
                        }
                        TAB_TRIGGER => app.trigger_tab_next_workflow(),
                        TAB_SECRETS => app.secrets_tab_next(),
                        TAB_RUNS => app.select_next_run(),
                        _ => {}
                    },
                    KeyCode::Char(' ') => {
                        if app.selected_tab == TAB_WORKFLOWS
                            && !app.running
                            && !app.job_selection_mode
                        {
                            app.toggle_selected();
                        }
                    }
                    KeyCode::Enter => {
                        match app.selected_tab {
                            TAB_WORKFLOWS => {
                                if !app.running {
                                    if app.job_selection_mode {
                                        // In job selection mode, run the selected job
                                        if app.selected_job_index < app.available_jobs.len() {
                                            let job_name =
                                                app.available_jobs[app.selected_job_index].clone();
                                            app.run_from_job_selection(Some(job_name));
                                        }
                                    } else {
                                        // Run the selected workflow directly
                                        app.queue_cursor_for_execution();
                                        app.start_execution();
                                    }
                                }
                            }
                            TAB_EXECUTION => {
                                // In execution tab, Enter shows job details
                                app.toggle_detailed_view();
                            }
                            TAB_DAG => app.switch_tab(TAB_EXECUTION),
                            TAB_TRIGGER => {
                                // Trigger tab: Enter on a non-editing row
                                // begins editing it (value first, then Tab
                                // to swap); when already editing, Enter
                                // commits the edit back to the row.
                                app.trigger_tab_enter();
                            }
                            _ => {}
                        }
                    }
                    KeyCode::Char('r') => {
                        // Some terminals deliver Shift+r as lowercase
                        // `r` plus a SHIFT modifier instead of
                        // uppercase `R` (see the `R` arm below).
                        // Route both encodings to the reset path so
                        // users don't have to reason about what their
                        // terminal emits.
                        if key.modifiers.contains(KeyModifiers::SHIFT) {
                            if !app.running {
                                app.reset_workflow_status();
                                terminal.draw(|f| {
                                    render_ui(f, app);
                                })?;
                            }
                        } else if !app.running
                            && !app.job_selection_mode
                            && app.selected_tab == TAB_WORKFLOWS
                        {
                            // Plain `r` runs every checked workflow,
                            // or the highlighted one when none are
                            // checked — a Workflows-tab action.
                            // Gate on the active tab so `r` typed on
                            // the DAG/Trigger/Secrets tabs doesn't
                            // silently mutate queue state behind the
                            // user's back.
                            app.queue_selected_for_execution();
                            app.start_execution();
                        }
                    }
                    KeyCode::Char('a') => {
                        if !app.running {
                            if app.job_selection_mode {
                                // In job selection mode, run all jobs
                                app.run_from_job_selection(None);
                            } else if app.selected_tab == TAB_WORKFLOWS {
                                // Select-all is a Workflows-tab action.
                                // Without this tab gate, pressing `a`
                                // on DAG/Trigger/Secrets (outside edit
                                // mode) would silently flip every
                                // workflow to `selected` behind the
                                // user's back.
                                for workflow in &mut app.workflows {
                                    workflow.selected = true;
                                }
                            }
                        }
                    }
                    KeyCode::Char('J') => {
                        // Enter job selection mode for selected workflow
                        if !app.running
                            && app.selected_tab == TAB_WORKFLOWS
                            && !app.job_selection_mode
                        {
                            app.enter_job_selection_mode();
                        }
                    }
                    KeyCode::Char('e') => {
                        if !app.running {
                            app.toggle_emulation_mode();
                        }
                    }
                    KeyCode::Char('v') => {
                        if !app.running {
                            app.toggle_validation_mode();
                        }
                    }
                    KeyCode::Char('d') => {
                        if !app.running && app.selected_tab == TAB_WORKFLOWS {
                            app.toggle_diff_filter();
                        }
                    }
                    KeyCode::Char('D') => {
                        // Shift+D cycles through the diff-filter event
                        // name (push → pull_request → workflow_dispatch →
                        // schedule → release → push). Previously the
                        // event name was a hardcoded "push" and the TUI
                        // silently reported "0 triggered" for any
                        // workflow gated on a non-push event — exactly
                        // the "stop lying about which workflows would
                        // run" failure mode the commit history fought.
                        if !app.running && app.selected_tab == TAB_WORKFLOWS {
                            app.cycle_diff_filter_event();
                        }
                    }
                    KeyCode::Char('n') => {
                        // Matches are in the raw log, so `n` shows it, as
                        // `s` and `f` do.
                        if app.selected_tab == TAB_LOGS && !app.log_search_query.is_empty() {
                            app.log_story = false;
                            app.next_search_match();
                        } else if app.selected_tab == TAB_WORKFLOWS && !app.running {
                            // Deselect all workflows
                            for workflow in &mut app.workflows {
                                workflow.selected = false;
                            }
                        }
                    }
                    KeyCode::Char('R') => {
                        if !app.running {
                            app.reset_workflow_status();
                            terminal.draw(|f| {
                                render_ui(f, app);
                            })?;
                        } else {
                            app.add_timestamped_log(
                                "Cannot reset workflow while another operation is running",
                            );
                        }
                    }
                    KeyCode::Char('?') => {
                        // Toggle help overlay
                        app.show_help = !app.show_help;
                    }
                    KeyCode::Char('t') => {
                        // Only trigger workflow if not already running and we're in the workflows tab
                        if !app.running && app.selected_tab == TAB_WORKFLOWS {
                            if let Some(selected_idx) = app.workflow_list_state.selected() {
                                if selected_idx < app.workflows.len() {
                                    let workflow = &app.workflows[selected_idx];
                                    if workflow.status == WorkflowStatus::NotStarted {
                                        app.trigger_selected_workflow();
                                    } else if workflow.status == WorkflowStatus::Running {
                                        let msg = format!(
                                            "Workflow '{}' is already running",
                                            workflow.name
                                        );
                                        wrkflw_logging::warning(&msg);
                                    } else {
                                        // First, get all the data we need from the workflow
                                        let workflow_name = workflow.name.clone();
                                        let status_text = match workflow.status {
                                            WorkflowStatus::Success => "Success",
                                            WorkflowStatus::Failed => "Failed",
                                            WorkflowStatus::Skipped => "Skipped",
                                            _ => "current",
                                        };
                                        let needs_reset_hint = workflow.status
                                            == WorkflowStatus::Success
                                            || workflow.status == WorkflowStatus::Failed
                                            || workflow.status == WorkflowStatus::Skipped;

                                        // Now set the status message (mutable borrow)
                                        app.set_error_message(format!(
                                            "Cannot trigger workflow '{}' in {} state. Press Shift+R to reset.",
                                            workflow_name,
                                            status_text
                                        ));

                                        // Both lines go to the global store: it
                                        // is the only sink `get_combined_logs`
                                        // renders once, and the hint has to stay
                                        // adjacent to the warning it explains.
                                        // The warning carries the workflow name
                                        // that the app-side copy used to add.
                                        wrkflw_logging::warning(&format!(
                                            "Cannot trigger workflow '{}' in {} state",
                                            workflow_name, status_text
                                        ));

                                        if needs_reset_hint {
                                            wrkflw_logging::info(
                                                "Hint: Press 'Shift+R' to reset the workflow status and allow triggering",
                                            );
                                        }
                                    }
                                }
                            } else {
                                wrkflw_logging::warning("No workflow selected to trigger");
                            }
                        } else if app.running {
                            wrkflw_logging::warning(
                                "Cannot trigger workflow while another operation is in progress",
                            );
                        } else if app.selected_tab != TAB_WORKFLOWS {
                            wrkflw_logging::warning(
                                "Switch to Workflows tab to trigger a workflow",
                            );
                            // For better UX, we could also automatically switch to the Workflows tab here
                            app.switch_tab(TAB_WORKFLOWS);
                        }
                    }
                    KeyCode::Char('s') => {
                        if app.selected_tab == TAB_LOGS {
                            // Search reads the raw log.
                            app.log_story = false;
                            app.toggle_log_search();
                        }
                    }
                    KeyCode::Char('f') => {
                        if app.selected_tab == TAB_LOGS {
                            app.log_story = false;
                            app.toggle_log_filter();
                        }
                    }
                    KeyCode::Char('o') => {
                        if app.selected_tab == TAB_LOGS {
                            app.log_story = !app.log_story;
                        }
                    }
                    KeyCode::Char('c') => {
                        if app.selected_tab == TAB_LOGS {
                            app.clear_log_search_and_filter();
                        } else if app.selected_tab == TAB_TRIGGER {
                            // Trigger tab: copy curl preview into the
                            // status bar so the user can scrape it from
                            // their scrollback without shelling out of
                            // the TUI. No clipboard integration yet —
                            // terminals vary too widely; a log line is
                            // honest.
                            app.trigger_tab_copy_curl();
                            // It lands in the raw log; show that there.
                            app.log_story = false;
                        }
                    }
                    KeyCode::Char('g') => {
                        if app.selected_tab == TAB_DAG {
                            // A second `g` flips the graph to a list.
                            app.dag_list_view = !app.dag_list_view;
                            app.dag_scroll = 0;
                        } else {
                            app.switch_tab(TAB_DAG);
                        }
                    }
                    KeyCode::Char('p') => {
                        if app.selected_tab == TAB_TRIGGER {
                            // Trigger tab: flip github ↔ gitlab. Also
                            // rebinds the branch default because each
                            // platform has its own `get_repo_info`.
                            app.trigger_tab_toggle_platform();
                        }
                    }
                    KeyCode::Char('+') | KeyCode::Char('=') => {
                        if app.selected_tab == TAB_TRIGGER {
                            app.trigger_tab_add_input();
                        }
                    }
                    KeyCode::Char('b') => {
                        // Trigger tab: focus the Branch / ref row so
                        // the user can type a non-default branch. The
                        // previous revision rendered the field as
                        // editable but provided no keystroke that
                        // reached it.
                        if app.selected_tab == TAB_TRIGGER {
                            app.trigger_tab_edit_branch();
                        }
                    }
                    KeyCode::Char('i') => {
                        // Trigger tab: jump into the input grid. Tab only
                        // cycles fields while one is being edited, so
                        // this is the direct way to reach an input row.
                        if app.selected_tab == TAB_TRIGGER {
                            app.trigger_tab_edit_inputs();
                        }
                    }
                    KeyCode::Char(c) => {
                        if app.selected_tab == TAB_LOGS && app.log_search_active {
                            app.handle_log_search_input(KeyCode::Char(c));
                        }
                    }
                    _ => {}
                }
            }
        }
    }
}
