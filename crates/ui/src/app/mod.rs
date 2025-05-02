// App module for UI state and main TUI entry point
mod state;

use crate::handlers::workflow::start_next_workflow_execution;
use crate::models::{ExecutionResultMsg, Workflow, WorkflowStatus};
use crate::utils::load_workflows;
use crate::views::render_ui;
use chrono::Local;
use crossterm::{
    event::{self, DisableMouseCapture, EnableMouseCapture, Event, KeyCode, KeyModifiers},
    execute,
    terminal::{disable_raw_mode, enable_raw_mode, EnterAlternateScreen, LeaveAlternateScreen},
};
use executor::RuntimeType;
use ratatui::{backend::CrosstermBackend, Terminal};
use std::io::{self, stdout};
use std::path::PathBuf;
use std::sync::mpsc;
use std::time::{Duration, Instant};

pub use state::App;

// Main entry point for the TUI interface
#[allow(clippy::ptr_arg)]
pub async fn run_wrkflw_tui(
    path: Option<&PathBuf>,
    runtime_type: RuntimeType,
    verbose: bool,
) -> io::Result<()> {
    // Terminal setup
    enable_raw_mode()?;
    let mut stdout = stdout();
    execute!(stdout, EnterAlternateScreen, EnableMouseCapture)?;
    let backend = CrosstermBackend::new(stdout);
    let mut terminal = Terminal::new(backend)?;

    // Set up channel for async communication
    let (tx, rx): (
        mpsc::Sender<ExecutionResultMsg>,
        mpsc::Receiver<ExecutionResultMsg>,
    ) = mpsc::channel();

    // Initialize app state
    let mut app = App::new(runtime_type.clone(), tx.clone());

    if app.validation_mode {
        app.logs.push("Starting in validation mode".to_string());
        logging::info("Starting in validation mode");
    }

    // Load workflows
    let dir_path = match path {
        Some(path) if path.is_dir() => path.clone(),
        Some(path) if path.is_file() => {
            // Single workflow file
            let name = path
                .file_name()
                .unwrap_or_default()
                .to_string_lossy()
                .into_owned();

            app.workflows = vec![Workflow {
                name: name.clone(),
                path: path.clone(),
                selected: true,
                status: WorkflowStatus::NotStarted,
                execution_details: None,
            }];

            // Queue the single workflow for execution
            app.execution_queue = vec![0];
            app.start_execution();

            // Return parent dir or current dir if no parent
            path.parent()
                .map(|p| p.to_path_buf())
                .unwrap_or_else(|| PathBuf::from("."))
        }
        _ => PathBuf::from(".github/workflows"),
    };

    // Only load directory if we haven't already loaded a single file
    if app.workflows.is_empty() {
        app.workflows = load_workflows(&dir_path);
    }

    // Run the main event loop
    let tx_clone = tx.clone();

    // Run the event loop
    let result = run_tui_event_loop(&mut terminal, &mut app, &tx_clone, &rx, verbose);

    // Clean up terminal
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
            logging::error(&format!("Failed to start UI: {}", e));

            // Only for 'tui' command should we fall back to CLI mode for files
            // For other commands, return the error
            if let Some(path) = path {
                if path.is_file() {
                    logging::error("Falling back to CLI mode...");
                    crate::handlers::workflow::execute_workflow_cli(path, runtime_type, verbose)
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

// Helper function to run the main event loop
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
        terminal.draw(|f| {
            render_ui(f, app);
        })?;

        // Update the UI on every tick
        if last_tick.elapsed() >= tick_rate {
            app.tick();
            app.update_running_workflow_progress();
            last_tick = Instant::now();
        }

        // Non-blocking check for execution results
        if let Ok((workflow_idx, result)) = rx.try_recv() {
            app.process_execution_result(workflow_idx, result);
            app.current_execution = None;

            // Get next workflow to execute using our helper function
            start_next_workflow_execution(app, tx_clone, verbose);
        }

        // Start execution if we have a queued workflow and nothing is currently running
        if app.running && app.current_execution.is_none() && !app.execution_queue.is_empty() {
            start_next_workflow_execution(app, tx_clone, verbose);
        }

        // Handle key events with a short timeout
        if event::poll(event_poll_timeout)? {
            if let Event::Key(key) = event::read()? {
                // Handle search input first if we're in search mode and logs tab
                if app.selected_tab == 2 && app.log_search_active {
                    app.handle_log_search_input(key.code);
                    continue;
                }

                match key.code {
                    KeyCode::Char('q') => {
                        // Exit and clean up
                        break Ok(());
                    }
                    KeyCode::Esc => {
                        if app.detailed_view {
                            app.detailed_view = false;
                        } else if app.show_help {
                            app.show_help = false;
                        } else {
                            // Exit and clean up
                            break Ok(());
                        }
                    }
                    KeyCode::Tab => {
                        // Cycle through tabs
                        app.switch_tab((app.selected_tab + 1) % 4);
                    }
                    KeyCode::BackTab => {
                        // Cycle through tabs backwards
                        app.switch_tab((app.selected_tab + 3) % 4);
                    }
                    KeyCode::Char('1') | KeyCode::Char('w') => app.switch_tab(0),
                    KeyCode::Char('2') | KeyCode::Char('x') => app.switch_tab(1),
                    KeyCode::Char('3') | KeyCode::Char('l') => app.switch_tab(2),
                    KeyCode::Char('4') | KeyCode::Char('h') => app.switch_tab(3),
                    KeyCode::Up | KeyCode::Char('k') => {
                        if app.selected_tab == 2 {
                            if !app.log_search_matches.is_empty() {
                                app.previous_search_match();
                            } else {
                                app.scroll_logs_up();
                            }
                        } else if app.selected_tab == 0 {
                            app.previous_workflow();
                        } else if app.selected_tab == 1 {
                            if app.detailed_view {
                                app.previous_step();
                            } else {
                                app.previous_job();
                            }
                        }
                    }
                    KeyCode::Down | KeyCode::Char('j') => {
                        if app.selected_tab == 2 {
                            if !app.log_search_matches.is_empty() {
                                app.next_search_match();
                            } else {
                                app.scroll_logs_down();
                            }
                        } else if app.selected_tab == 0 {
                            app.next_workflow();
                        } else if app.selected_tab == 1 {
                            if app.detailed_view {
                                app.next_step();
                            } else {
                                app.next_job();
                            }
                        }
                    }
                    KeyCode::Char(' ') => {
                        if app.selected_tab == 0 && !app.running {
                            app.toggle_selected();
                        }
                    }
                    KeyCode::Enter => {
                        match app.selected_tab {
                            0 => {
                                // In workflows tab, Enter runs the selected workflow
                                if !app.running {
                                    if let Some(idx) = app.workflow_list_state.selected() {
                                        app.workflows[idx].selected = true;
                                        app.queue_selected_for_execution();
                                        app.start_execution();
                                    }
                                }
                            }
                            1 => {
                                // In execution tab, Enter shows job details
                                app.toggle_detailed_view();
                            }
                            _ => {}
                        }
                    }
                    KeyCode::Char('r') => {
                        // Check if shift is pressed - this might be receiving the reset command
                        if key.modifiers.contains(KeyModifiers::SHIFT) {
                            let timestamp = Local::now().format("%H:%M:%S").to_string();
                            app.logs.push(format!(
                                "[{}] DEBUG: Shift+r detected - this should be uppercase R",
                                timestamp
                            ));
                            logging::info(
                                "Shift+r detected as lowercase - this should be uppercase R",
                            );

                            if !app.running {
                                // Reset workflow status with Shift+r
                                app.logs.push(format!(
                                    "[{}] Attempting to reset workflow status via Shift+r...",
                                    timestamp
                                ));
                                app.reset_workflow_status();

                                // Force redraw to update UI immediately
                                terminal.draw(|f| {
                                    render_ui(f, app);
                                })?;
                            }
                        } else if !app.running {
                            app.queue_selected_for_execution();
                            app.start_execution();
                        }
                    }
                    KeyCode::Char('a') => {
                        if !app.running {
                            // Select all workflows
                            for workflow in &mut app.workflows {
                                workflow.selected = true;
                            }
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
                    KeyCode::Char('n') => {
                        if app.selected_tab == 2 && !app.log_search_query.is_empty() {
                            app.next_search_match();
                        } else if app.selected_tab == 0 && !app.running {
                            // Deselect all workflows
                            for workflow in &mut app.workflows {
                                workflow.selected = false;
                            }
                        }
                    }
                    KeyCode::Char('R') => {
                        let timestamp = Local::now().format("%H:%M:%S").to_string();
                        app.logs.push(format!(
                            "[{}] DEBUG: Reset key 'Shift+R' pressed",
                            timestamp
                        ));
                        logging::info("Reset key 'Shift+R' pressed");

                        if !app.running {
                            // Reset workflow status
                            app.logs.push(format!(
                                "[{}] Attempting to reset workflow status...",
                                timestamp
                            ));
                            app.reset_workflow_status();

                            // Force redraw to update UI immediately
                            terminal.draw(|f| {
                                render_ui(f, app);
                            })?;
                        } else {
                            app.logs.push(format!(
                                "[{}] Cannot reset workflow while another operation is running",
                                timestamp
                            ));
                        }
                    }
                    KeyCode::Char('?') => {
                        // Toggle help overlay
                        app.show_help = !app.show_help;
                    }
                    KeyCode::Char('t') => {
                        // Only trigger workflow if not already running and we're in the workflows tab
                        if !app.running && app.selected_tab == 0 {
                            if let Some(selected_idx) = app.workflow_list_state.selected() {
                                if selected_idx < app.workflows.len() {
                                    let workflow = &app.workflows[selected_idx];
                                    if workflow.status == WorkflowStatus::NotStarted {
                                        app.trigger_selected_workflow();
                                    } else if workflow.status == WorkflowStatus::Running {
                                        app.logs.push(format!(
                                            "Workflow '{}' is already running",
                                            workflow.name
                                        ));
                                        logging::warning(&format!(
                                            "Workflow '{}' is already running",
                                            workflow.name
                                        ));
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
                                        app.set_status_message(format!(
                                            "Cannot trigger workflow '{}' in {} state. Press Shift+R to reset.",
                                            workflow_name,
                                            status_text
                                        ));

                                        // Add log entries
                                        app.logs.push(format!(
                                            "Cannot trigger workflow '{}' in {} state",
                                            workflow_name, status_text
                                        ));

                                        // Add hint about using reset
                                        if needs_reset_hint {
                                            let timestamp =
                                                Local::now().format("%H:%M:%S").to_string();
                                            app.logs.push(format!(
                                                "[{}] Hint: Press 'Shift+R' to reset the workflow status and allow triggering",
                                                timestamp
                                            ));
                                        }

                                        logging::warning(&format!(
                                            "Cannot trigger workflow in {} state",
                                            status_text
                                        ));
                                    }
                                }
                            } else {
                                app.logs.push("No workflow selected to trigger".to_string());
                                logging::warning("No workflow selected to trigger");
                            }
                        } else if app.running {
                            app.logs.push(
                                "Cannot trigger workflow while another operation is in progress"
                                    .to_string(),
                            );
                            logging::warning(
                                "Cannot trigger workflow while another operation is in progress",
                            );
                        } else if app.selected_tab != 0 {
                            app.logs
                                .push("Switch to Workflows tab to trigger a workflow".to_string());
                            logging::warning("Switch to Workflows tab to trigger a workflow");
                            // For better UX, we could also automatically switch to the Workflows tab here
                            app.switch_tab(0);
                        }
                    }
                    KeyCode::Char('s') => {
                        if app.selected_tab == 2 {
                            app.toggle_log_search();
                        }
                    }
                    KeyCode::Char('f') => {
                        if app.selected_tab == 2 {
                            app.toggle_log_filter();
                        }
                    }
                    KeyCode::Char('c') => {
                        if app.selected_tab == 2 {
                            app.clear_log_search_and_filter();
                        }
                    }
                    KeyCode::Char(c) => {
                        if app.selected_tab == 2 && app.log_search_active {
                            app.handle_log_search_input(KeyCode::Char(c));
                        }
                    }
                    _ => {}
                }
            }
        }
    }
}
