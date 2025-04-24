use crate::evaluator::evaluate_workflow_file;
use crate::executor::{self, JobStatus, RuntimeType, StepStatus};
use crate::logging;
use crate::utils;
use crate::utils::is_workflow_file;
use chrono::Local;
use crossterm::{
    event::{self, DisableMouseCapture, EnableMouseCapture, Event, KeyCode, KeyModifiers},
    execute,
    terminal::{disable_raw_mode, enable_raw_mode, EnterAlternateScreen, LeaveAlternateScreen},
};
use ratatui::{
    backend::CrosstermBackend,
    layout::{Alignment, Constraint, Direction, Layout, Rect},
    style::{Color, Modifier, Style},
    text::{Line, Span},
    widgets::{
        Block, BorderType, Borders, Cell, Gauge, List, ListItem, ListState, Paragraph, Row, Table,
        TableState, Tabs, Wrap,
    },
    Frame, Terminal,
};
use std::io::{self, stdout};
use std::path::{Path, PathBuf};
use std::sync::mpsc;
use std::thread;
use std::time::{Duration, Instant};

// Represents an individual workflow file
struct Workflow {
    name: String,
    path: PathBuf,
    selected: bool,
    status: WorkflowStatus,
    execution_details: Option<WorkflowExecution>,
}

// Status of a workflow
#[derive(Debug, Clone, PartialEq)]
enum WorkflowStatus {
    NotStarted,
    Running,
    Success,
    Failed,
    Skipped,
}

// Detailed execution information
#[allow(dead_code)]
struct WorkflowExecution {
    jobs: Vec<JobExecution>,
    start_time: chrono::DateTime<Local>,
    end_time: Option<chrono::DateTime<Local>>,
    logs: Vec<String>,
    progress: f64, // 0.0 - 1.0 for progress bar
}

// Job execution details
#[allow(dead_code)]
struct JobExecution {
    name: String,
    status: JobStatus,
    steps: Vec<StepExecution>,
    logs: Vec<String>,
}

// Step execution details
struct StepExecution {
    name: String,
    status: StepStatus,
    output: String,
}

// Type alias for the complex execution result type
type ExecutionResultMsg = (usize, Result<(Vec<executor::JobResult>, ()), String>);

// Application state
struct App {
    workflows: Vec<Workflow>,
    workflow_list_state: ListState,
    selected_tab: usize,
    running: bool,
    show_help: bool,
    runtime_type: RuntimeType,
    validation_mode: bool,
    execution_queue: Vec<usize>, // Indices of workflows to execute
    current_execution: Option<usize>,
    logs: Vec<String>,                    // Overall execution logs
    log_scroll: usize,                    // Scrolling position for logs
    job_list_state: ListState,            // For viewing job details
    detailed_view: bool,                  // Whether we're in detailed view mode
    step_list_state: ListState,           // For selecting steps in detailed view
    step_table_state: TableState,         // For the steps table in detailed view
    last_tick: Instant,                   // For UI animations and updates
    tick_rate: Duration,                  // How often to update the UI
    tx: mpsc::Sender<ExecutionResultMsg>, // Channel for async communication
    status_message: Option<String>,       // Temporary status message to display
    status_message_time: Option<Instant>, // When the message was set

    // Search and filter functionality
    log_search_query: String, // Current search query for logs
    log_search_active: bool,  // Whether search input is active
    log_filter_level: Option<LogFilterLevel>, // Current log level filter
    log_search_matches: Vec<usize>, // Indices of logs that match the search
    log_search_match_idx: usize, // Current match index for navigation
}

// Log filter levels
enum LogFilterLevel {
    Info,
    Warning,
    Error,
    Success,
    Trigger,
    All,
}

impl LogFilterLevel {
    fn matches(&self, log: &str) -> bool {
        match self {
            LogFilterLevel::Info => {
                log.contains("ℹ️") || (log.contains("INFO") && !log.contains("SUCCESS"))
            }
            LogFilterLevel::Warning => log.contains("⚠️") || log.contains("WARN"),
            LogFilterLevel::Error => log.contains("❌") || log.contains("ERROR"),
            LogFilterLevel::Success => log.contains("SUCCESS") || log.contains("success"),
            LogFilterLevel::Trigger => {
                log.contains("Triggering") || log.contains("triggered") || log.contains("TRIG")
            }
            LogFilterLevel::All => true,
        }
    }

    fn next(&self) -> Self {
        match self {
            LogFilterLevel::All => LogFilterLevel::Info,
            LogFilterLevel::Info => LogFilterLevel::Warning,
            LogFilterLevel::Warning => LogFilterLevel::Error,
            LogFilterLevel::Error => LogFilterLevel::Success,
            LogFilterLevel::Success => LogFilterLevel::Trigger,
            LogFilterLevel::Trigger => LogFilterLevel::All,
        }
    }

    fn to_string(&self) -> &str {
        match self {
            LogFilterLevel::All => "ALL",
            LogFilterLevel::Info => "INFO",
            LogFilterLevel::Warning => "WARNING",
            LogFilterLevel::Error => "ERROR",
            LogFilterLevel::Success => "SUCCESS",
            LogFilterLevel::Trigger => "TRIGGER",
        }
    }
}

impl App {
    fn new(runtime_type: RuntimeType, tx: mpsc::Sender<ExecutionResultMsg>) -> App {
        let mut workflow_list_state = ListState::default();
        workflow_list_state.select(Some(0));

        let mut job_list_state = ListState::default();
        job_list_state.select(Some(0));

        let mut step_list_state = ListState::default();
        step_list_state.select(Some(0));

        let mut step_table_state = TableState::default();
        step_table_state.select(Some(0));

        // Check Docker availability if Docker runtime is selected
        let mut initial_logs = Vec::new();
        let runtime_type = match runtime_type {
            RuntimeType::Docker => {
                // Use a timeout for the Docker availability check to prevent hanging
                let is_docker_available = match std::panic::catch_unwind(|| {
                    // Use a very short timeout to prevent blocking the UI
                    let result = std::thread::scope(|s| {
                        let handle = s.spawn(|| {
                            utils::fd::with_stderr_to_null(executor::docker::is_available)
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
                        logging::warning(
                            "Docker availability check timed out, falling back to emulation mode",
                        );
                        false
                    });
                    result
                }) {
                    Ok(result) => result,
                    Err(_) => {
                        logging::warning("Docker availability check failed with panic, falling back to emulation mode");
                        false
                    }
                };

                if !is_docker_available {
                    initial_logs.push(
                        "Docker is not available or unresponsive. Using emulation mode instead."
                            .to_string(),
                    );
                    logging::warning(
                        "Docker is not available or unresponsive. Using emulation mode instead.",
                    );
                    RuntimeType::Emulation
                } else {
                    logging::info("Docker is available, using Docker runtime");
                    RuntimeType::Docker
                }
            }
            RuntimeType::Emulation => RuntimeType::Emulation,
        };

        App {
            workflows: Vec::new(),
            workflow_list_state,
            selected_tab: 0,
            running: false,
            show_help: false,
            runtime_type,
            validation_mode: false,
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
            tx,
            status_message: None,
            status_message_time: None,

            // Search and filter functionality
            log_search_query: String::new(),
            log_search_active: false,
            log_filter_level: Some(LogFilterLevel::All),
            log_search_matches: Vec::new(),
            log_search_match_idx: 0,
        }
    }

    // Toggle workflow selection
    fn toggle_selected(&mut self) {
        if let Some(idx) = self.workflow_list_state.selected() {
            if idx < self.workflows.len() {
                self.workflows[idx].selected = !self.workflows[idx].selected;
            }
        }
    }

    fn toggle_emulation_mode(&mut self) {
        self.runtime_type = match self.runtime_type {
            RuntimeType::Docker => RuntimeType::Emulation,
            RuntimeType::Emulation => RuntimeType::Docker,
        };
        self.logs
            .push(format!("Switched to {} mode", self.runtime_type_name()));
    }

    fn toggle_validation_mode(&mut self) {
        self.validation_mode = !self.validation_mode;
        let mode = if self.validation_mode {
            "validation"
        } else {
            "normal"
        };
        let timestamp = Local::now().format("%H:%M:%S").to_string();
        self.logs
            .push(format!("[{}] Switched to {} mode", timestamp, mode));
        logging::info(&format!("Switched to {} mode", mode));
    }

    fn runtime_type_name(&self) -> &str {
        match self.runtime_type {
            RuntimeType::Docker => "Docker",
            RuntimeType::Emulation => "Emulation",
        }
    }

    // Move cursor up in the workflow list
    fn previous_workflow(&mut self) {
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
    fn next_workflow(&mut self) {
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
    fn previous_job(&mut self) {
        let current_workflow_idx = self
            .current_execution
            .or_else(|| self.workflow_list_state.selected());

        if let Some(workflow_idx) = current_workflow_idx {
            if workflow_idx >= self.workflows.len() {
                return;
            }

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
    fn next_job(&mut self) {
        let current_workflow_idx = self
            .current_execution
            .or_else(|| self.workflow_list_state.selected())
            .filter(|&idx| idx < self.workflows.len());

        if let Some(workflow_idx) = current_workflow_idx {
            if workflow_idx >= self.workflows.len() {
                return;
            }

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
    fn previous_step(&mut self) {
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
    fn next_step(&mut self) {
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
    fn switch_tab(&mut self, tab: usize) {
        self.selected_tab = tab;
    }

    // Queue selected workflows for execution
    fn queue_selected_for_execution(&mut self) {
        if let Some(idx) = self.workflow_list_state.selected() {
            if idx < self.workflows.len() && !self.execution_queue.contains(&idx) {
                self.execution_queue.push(idx);
                let timestamp = Local::now().format("%H:%M:%S").to_string();
                self.logs.push(format!(
                    "[{}] Added '{}' to execution queue. Press 'Enter' to start.",
                    timestamp, self.workflows[idx].name
                ));
            }
        }
    }

    // Start workflow execution process
    fn start_execution(&mut self) {
        // Only start if we have workflows in queue and nothing is currently running
        if !self.execution_queue.is_empty() && self.current_execution.is_none() {
            self.running = true;

            // Log only once at the beginning - don't initialize execution details here
            // since that will happen in start_next_workflow_execution
            let timestamp = Local::now().format("%H:%M:%S").to_string();
            self.logs
                .push(format!("[{}] Starting workflow execution...", timestamp));
            logging::info("Starting workflow execution...");
        }
    }

    // Process execution results and update UI
    fn process_execution_result(
        &mut self,
        workflow_idx: usize,
        result: Result<(Vec<executor::JobResult>, ()), String>,
    ) {
        if workflow_idx >= self.workflows.len() {
            let timestamp = Local::now().format("%H:%M:%S").to_string();
            self.logs.push(format!(
                "[{}] Error: Invalid workflow index received",
                timestamp
            ));
            logging::error("Invalid workflow index received in process_execution_result");
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

                    // Convert executor::JobResult to our JobExecution struct
                    execution_details.jobs = jobs
                        .iter()
                        .map(|job_result| JobExecution {
                            name: job_result.name.clone(),
                            status: match job_result.status {
                                executor::JobStatus::Success => JobStatus::Success,
                                executor::JobStatus::Failure => JobStatus::Failure,
                                executor::JobStatus::Skipped => JobStatus::Skipped,
                            },
                            steps: job_result
                                .steps
                                .iter()
                                .map(|step_result| StepExecution {
                                    name: step_result.name.clone(),
                                    status: match step_result.status {
                                        executor::StepStatus::Success => StepStatus::Success,
                                        executor::StepStatus::Failure => StepStatus::Failure,
                                        executor::StepStatus::Skipped => StepStatus::Skipped,
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
                logging::info(&format!(
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
                logging::error(&format!(
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
    fn get_next_workflow_to_execute(&mut self) -> Option<usize> {
        if self.execution_queue.is_empty() {
            return None;
        }

        let next = self.execution_queue.remove(0);
        self.workflows[next].status = WorkflowStatus::Running;
        self.current_execution = Some(next);
        self.logs
            .push(format!("Executing workflow: {}", self.workflows[next].name));
        logging::info(&format!(
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

        Some(next)
    }

    // Toggle detailed view mode
    fn toggle_detailed_view(&mut self) {
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
    fn handle_log_search_input(&mut self, key: KeyCode) {
        match key {
            KeyCode::Esc => {
                self.log_search_active = false;
                self.log_search_query.clear();
                self.log_search_matches.clear();
            }
            KeyCode::Backspace => {
                self.log_search_query.pop();
                self.update_log_search_matches();
            }
            KeyCode::Enter => {
                self.log_search_active = false;
                // Keep the search query and matches
            }
            KeyCode::Char(c) => {
                self.log_search_query.push(c);
                self.update_log_search_matches();
            }
            _ => {}
        }
    }

    // Toggle log search mode
    fn toggle_log_search(&mut self) {
        self.log_search_active = !self.log_search_active;
        if !self.log_search_active {
            // Don't clear the query, this allows toggling the search UI while keeping the filter
        } else {
            // When activating search, update matches
            self.update_log_search_matches();
        }
    }

    // Toggle log filter
    fn toggle_log_filter(&mut self) {
        self.log_filter_level = match &self.log_filter_level {
            None => Some(LogFilterLevel::Info),
            Some(level) => Some(level.next()),
        };

        // Update search matches when filter changes
        self.update_log_search_matches();
    }

    // Clear log search and filter
    fn clear_log_search_and_filter(&mut self) {
        self.log_search_query.clear();
        self.log_filter_level = None;
        self.log_search_matches.clear();
        self.log_search_match_idx = 0;
    }

    // Update matches based on current search and filter
    fn update_log_search_matches(&mut self) {
        self.log_search_matches.clear();
        self.log_search_match_idx = 0;

        // Get all logs (app logs + system logs)
        let mut all_logs = Vec::new();
        for log in &self.logs {
            all_logs.push(log.clone());
        }
        for log in crate::logging::get_logs() {
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
                    self.set_status_message(format!(
                        "Found {} matches for '{}'",
                        self.log_search_matches.len(),
                        self.log_search_query
                    ));
                }
            }
        } else if !self.log_search_query.is_empty() {
            // No matches found
            self.set_status_message(format!("No matches found for '{}'", self.log_search_query));
        }
    }

    // Navigate to next search match
    fn next_search_match(&mut self) {
        if !self.log_search_matches.is_empty() {
            self.log_search_match_idx =
                (self.log_search_match_idx + 1) % self.log_search_matches.len();
            if let Some(&idx) = self.log_search_matches.get(self.log_search_match_idx) {
                self.log_scroll = idx;

                // Set status message showing which match we're on
                self.set_status_message(format!(
                    "Search match {}/{} for '{}'",
                    self.log_search_match_idx + 1,
                    self.log_search_matches.len(),
                    self.log_search_query
                ));
            }
        }
    }

    // Navigate to previous search match
    fn previous_search_match(&mut self) {
        if !self.log_search_matches.is_empty() {
            self.log_search_match_idx = if self.log_search_match_idx == 0 {
                self.log_search_matches.len() - 1
            } else {
                self.log_search_match_idx - 1
            };
            if let Some(&idx) = self.log_search_matches.get(self.log_search_match_idx) {
                self.log_scroll = idx;

                // Set status message showing which match we're on
                self.set_status_message(format!(
                    "Search match {}/{} for '{}'",
                    self.log_search_match_idx + 1,
                    self.log_search_matches.len(),
                    self.log_search_query
                ));
            }
        }
    }

    // Scroll logs up
    fn scroll_logs_up(&mut self) {
        self.log_scroll = self.log_scroll.saturating_sub(1);
    }

    // Scroll logs down
    fn scroll_logs_down(&mut self) {
        // Get total log count including system logs
        let total_logs = self.logs.len() + crate::logging::get_logs().len();
        if total_logs > 0 {
            self.log_scroll = (self.log_scroll + 1).min(total_logs - 1);
        }
    }

    // Update progress for running workflows
    fn update_running_workflow_progress(&mut self) {
        if let Some(idx) = self.current_execution {
            if let Some(execution) = &mut self.workflows[idx].execution_details {
                if execution.end_time.is_none() {
                    // Gradually increase progress for visual feedback
                    execution.progress = (execution.progress + 0.01).min(0.95);
                }
            }
        }
    }

    // Set a temporary status message to be displayed in the UI
    fn set_status_message(&mut self, message: String) {
        self.status_message = Some(message);
        self.status_message_time = Some(Instant::now());
    }

    // Check if tick should happen
    fn tick(&mut self) -> bool {
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
            true
        } else {
            false
        }
    }

    // Trigger the selected workflow
    fn trigger_selected_workflow(&mut self) {
        if let Some(selected_idx) = self.workflow_list_state.selected() {
            if selected_idx < self.workflows.len() {
                let workflow = &self.workflows[selected_idx];

                if workflow.name.is_empty() {
                    let timestamp = Local::now().format("%H:%M:%S").to_string();
                    self.logs
                        .push(format!("[{}] Error: Invalid workflow selection", timestamp));
                    logging::error("Invalid workflow selection in trigger_selected_workflow");
                    return;
                }

                // Set up background task to execute the workflow via GitHub Actions REST API
                let timestamp = Local::now().format("%H:%M:%S").to_string();
                self.logs.push(format!(
                    "[{}] Triggering workflow: {}",
                    timestamp, workflow.name
                ));
                logging::info(&format!("Triggering workflow: {}", workflow.name));

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
                            eprintln!("Failed to create Tokio runtime: {}", e);
                            // Return early from the current function with appropriate error handling
                            let _ = tx_clone.send((
                                selected_idx,
                                Err("Failed to create runtime for execution".to_string()),
                            ));
                            return;
                        }
                    };

                    // Execute the GitHub Actions trigger API call
                    let result =
                        rt.block_on(async { execute_curl_trigger(&workflow_name, None).await });

                    // Send the result back to the main thread
                    if let Err(e) = tx_clone.send((selected_idx, result)) {
                        eprintln!("Error sending trigger result: {}", e);
                    }
                });
            } else {
                let timestamp = Local::now().format("%H:%M:%S").to_string();
                self.logs
                    .push(format!("[{}] No workflow selected to trigger", timestamp));
                logging::warning("No workflow selected to trigger");
            }
        } else {
            self.logs
                .push("No workflow selected to trigger".to_string());
            logging::warning("No workflow selected to trigger");
        }
    }

    // Reset a workflow's status to NotStarted
    fn reset_workflow_status(&mut self) {
        // Log whether a selection exists
        if self.workflow_list_state.selected().is_none() {
            let timestamp = Local::now().format("%H:%M:%S").to_string();
            self.logs.push(format!(
                "[{}] Debug: No workflow selected for reset",
                timestamp
            ));
            logging::warning("No workflow selected for reset");
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
                logging::info(&format!(
                    "Reset workflow '{}' from {} state to NotStarted - status is now {:?}",
                    workflow.name, old_status, workflow.status
                ));

                // Set a success status message
                self.set_status_message(format!("✅ Workflow '{}' has been reset!", workflow_name));

                // } else {
                //     let timestamp = Local::now().format("%H:%M:%S").to_string();
                //     self.logs.push(format!(
                //         "[{}] Cannot reset workflow '{}' while it is running",
                //         timestamp,
                //         workflow.name
                //     ));
                //     logging::warning(&format!(
                //         "Cannot reset workflow '{}' while it is running",
                //         workflow.name
                //     ));
                // }
            }
        }
    }
}

// Find and load all workflow files in a directory
fn load_workflows(dir_path: &Path) -> Vec<Workflow> {
    let mut workflows = Vec::new();

    // Default path is .github/workflows
    let default_workflows_dir = Path::new(".github").join("workflows");
    let is_default_dir = dir_path == default_workflows_dir || dir_path.ends_with("workflows");

    if let Ok(entries) = std::fs::read_dir(dir_path) {
        for entry in entries.flatten() {
            let path = entry.path();
            if path.is_file() && (is_workflow_file(&path) || !is_default_dir) {
                let name = path.file_name().map_or_else(
                    || "[unknown]".to_string(),
                    |fname| fname.to_string_lossy().into_owned(),
                );

                workflows.push(Workflow {
                    name,
                    path,
                    selected: false,
                    status: WorkflowStatus::NotStarted,
                    execution_details: None,
                });
            }
        }
    }

    // Sort workflows by name
    workflows.sort_by(|a, b| a.name.cmp(&b.name));
    workflows
}

// Main render function for the UI
fn render_ui(f: &mut Frame<CrosstermBackend<io::Stdout>>, app: &mut App) {
    // Check if help should be shown as an overlay
    if app.show_help {
        render_help_overlay(f);
        return;
    }

    let size = f.size();

    // Create main layout
    let main_chunks = Layout::default()
        .direction(Direction::Vertical)
        .constraints(
            [
                Constraint::Length(3), // Title bar and tabs
                Constraint::Min(5),    // Main content
                Constraint::Length(2), // Status bar
            ]
            .as_ref(),
        )
        .split(size);

    // Render title bar with tabs
    render_title_bar(f, app, main_chunks[0]);

    // Render main content based on selected tab
    match app.selected_tab {
        0 => render_workflows_tab(f, app, main_chunks[1]),
        1 => render_execution_tab(f, app, main_chunks[1]),
        2 => render_logs_tab(f, app, main_chunks[1]),
        3 => render_help_tab(f, main_chunks[1]),
        _ => {}
    }

    // Render status bar
    render_status_bar(f, app, main_chunks[2]);
}

// Render the title bar with tabs
fn render_title_bar(f: &mut Frame<CrosstermBackend<io::Stdout>>, app: &App, area: Rect) {
    let titles = ["Workflows", "Execution", "Logs", "Help"];
    let tabs = Tabs::new(
        titles
            .iter()
            .enumerate()
            .map(|(i, t)| {
                if i == 1 {
                    // Special case for "Execution"
                    let e_part = &t[0..1]; // "E"
                    let x_part = &t[1..2]; // "x"
                    let rest = &t[2..]; // "ecution"
                    Line::from(vec![
                        Span::styled(e_part, Style::default().fg(Color::White)),
                        Span::styled(
                            x_part,
                            Style::default()
                                .fg(Color::Yellow)
                                .add_modifier(Modifier::UNDERLINED),
                        ),
                        Span::styled(rest, Style::default().fg(Color::White)),
                    ])
                } else {
                    // Original styling for other tabs
                    let (first, rest) = t.split_at(1);
                    Line::from(vec![
                        Span::styled(
                            first,
                            Style::default()
                                .fg(Color::Yellow)
                                .add_modifier(Modifier::UNDERLINED),
                        ),
                        Span::styled(rest, Style::default().fg(Color::White)),
                    ])
                }
            })
            .collect(),
    )
    .block(
        Block::default()
            .borders(Borders::ALL)
            .border_type(BorderType::Rounded)
            .title(Span::styled(
                " wrkflw ",
                Style::default()
                    .fg(Color::Cyan)
                    .add_modifier(Modifier::BOLD),
            ))
            .title_alignment(Alignment::Center),
    )
    .highlight_style(
        Style::default()
            .bg(Color::DarkGray)
            .fg(Color::Yellow)
            .add_modifier(Modifier::BOLD),
    )
    .select(app.selected_tab)
    .divider(Span::raw("|"));

    f.render_widget(tabs, area);
}

// Render the workflow list tab
fn render_workflows_tab(f: &mut Frame<CrosstermBackend<io::Stdout>>, app: &mut App, area: Rect) {
    // Create a more structured layout for the workflow tab
    let chunks = Layout::default()
        .direction(Direction::Vertical)
        .constraints(
            [
                Constraint::Length(3), // Header with instructions
                Constraint::Min(5),    // Workflow list
            ]
            .as_ref(),
        )
        .margin(1)
        .split(area);

    // Render header with instructions
    let header_text = vec![
        Line::from(vec![Span::styled(
            "Available Workflows",
            Style::default()
                .fg(Color::Yellow)
                .add_modifier(Modifier::BOLD),
        )]),
        Line::from(vec![
            Span::styled("Space", Style::default().fg(Color::Cyan)),
            Span::raw(": Toggle selection   "),
            Span::styled("Enter", Style::default().fg(Color::Cyan)),
            Span::raw(": Run   "),
            Span::styled("t", Style::default().fg(Color::Cyan)),
            Span::raw(": Trigger remotely"),
        ]),
    ];

    let header = Paragraph::new(header_text)
        .block(
            Block::default()
                .borders(Borders::ALL)
                .border_type(BorderType::Rounded),
        )
        .alignment(Alignment::Center);

    f.render_widget(header, chunks[0]);

    // Create a table for workflows instead of a list for better organization
    let selected_style = Style::default()
        .bg(Color::DarkGray)
        .add_modifier(Modifier::BOLD);

    // Normal style definition removed as it was unused

    let header_cells = ["", "Status", "Workflow Name", "Path"]
        .iter()
        .map(|h| Cell::from(*h).style(Style::default().fg(Color::Yellow)));

    let header = Row::new(header_cells)
        .style(Style::default().add_modifier(Modifier::BOLD))
        .height(1);

    let rows = app.workflows.iter().map(|workflow| {
        // Create cells for each column
        let checkbox = if workflow.selected { "✓" } else { " " };

        let (status_symbol, status_style) = match workflow.status {
            WorkflowStatus::NotStarted => ("○", Style::default().fg(Color::Gray)),
            WorkflowStatus::Running => ("⟳", Style::default().fg(Color::Cyan)),
            WorkflowStatus::Success => ("✅", Style::default().fg(Color::Green)),
            WorkflowStatus::Failed => ("❌", Style::default().fg(Color::Red)),
            WorkflowStatus::Skipped => ("⏭", Style::default().fg(Color::Yellow)),
        };

        let path_display = workflow.path.to_string_lossy();
        let path_shortened = if path_display.len() > 30 {
            format!("...{}", &path_display[path_display.len() - 30..])
        } else {
            path_display.to_string()
        };

        Row::new(vec![
            Cell::from(checkbox).style(Style::default().fg(Color::Green)),
            Cell::from(status_symbol).style(status_style),
            Cell::from(workflow.name.clone()),
            Cell::from(path_shortened).style(Style::default().fg(Color::DarkGray)),
        ])
    });

    let workflows_table = Table::new(rows)
        .header(header)
        .block(
            Block::default()
                .borders(Borders::ALL)
                .border_type(BorderType::Rounded)
                .title(Span::styled(
                    " Workflows ",
                    Style::default().fg(Color::Yellow),
                )),
        )
        .highlight_style(selected_style)
        .highlight_symbol("» ")
        .widths(&[
            Constraint::Length(3),      // Checkbox column
            Constraint::Length(4),      // Status icon column
            Constraint::Percentage(45), // Name column
            Constraint::Percentage(45), // Path column
        ]);

    // We need to convert ListState to TableState
    let mut table_state = TableState::default();
    table_state.select(app.workflow_list_state.selected());

    f.render_stateful_widget(workflows_table, chunks[1], &mut table_state);

    // Update the app list state to match the table state
    app.workflow_list_state.select(table_state.selected());
}

fn render_execution_tab(f: &mut Frame<CrosstermBackend<io::Stdout>>, app: &mut App, area: Rect) {
    if app.detailed_view {
        render_job_detail_view(f, app, area);
        return;
    }

    // Get the workflow index either from current_execution or selected workflow
    let current_workflow_idx = app
        .current_execution
        .or_else(|| app.workflow_list_state.selected())
        .filter(|&idx| idx < app.workflows.len());

    if let Some(idx) = current_workflow_idx {
        let workflow = &app.workflows[idx];

        // Split the area into sections
        let chunks = Layout::default()
            .direction(Direction::Vertical)
            .constraints(
                [
                    Constraint::Length(5), // Workflow info with progress bar
                    Constraint::Min(5),    // Jobs list or Remote execution info
                    Constraint::Length(7), // Execution info
                ]
                .as_ref(),
            )
            .margin(1)
            .split(area);

        // Workflow info section
        let status_text = match workflow.status {
            WorkflowStatus::NotStarted => "Not Started",
            WorkflowStatus::Running => "Running",
            WorkflowStatus::Success => "Success",
            WorkflowStatus::Failed => "Failed",
            WorkflowStatus::Skipped => "Skipped",
        };

        let status_style = match workflow.status {
            WorkflowStatus::NotStarted => Style::default().fg(Color::Gray),
            WorkflowStatus::Running => Style::default().fg(Color::Cyan),
            WorkflowStatus::Success => Style::default().fg(Color::Green),
            WorkflowStatus::Failed => Style::default().fg(Color::Red),
            WorkflowStatus::Skipped => Style::default().fg(Color::Yellow),
        };

        let mut workflow_info = vec![
            Line::from(vec![
                Span::styled("Workflow: ", Style::default().fg(Color::Blue)),
                Span::styled(
                    workflow.name.clone(),
                    Style::default()
                        .fg(Color::White)
                        .add_modifier(Modifier::BOLD),
                ),
            ]),
            Line::from(vec![
                Span::styled("Status: ", Style::default().fg(Color::Blue)),
                Span::styled(status_text, status_style),
            ]),
        ];

        // Add progress bar for running workflows or workflows with execution details
        if let Some(execution) = &workflow.execution_details {
            // Calculate progress
            let progress = execution.progress;

            // Add progress bar
            let gauge_color = match workflow.status {
                WorkflowStatus::Running => Color::Cyan,
                WorkflowStatus::Success => Color::Green,
                WorkflowStatus::Failed => Color::Red,
                _ => Color::Gray,
            };

            let progress_text = match workflow.status {
                WorkflowStatus::Running => format!("{:.0}%", progress * 100.0),
                WorkflowStatus::Success => "Completed".to_string(),
                WorkflowStatus::Failed => "Failed".to_string(),
                _ => "Not started".to_string(),
            };

            // Add empty line before progress bar
            workflow_info.push(Line::from(""));

            // Add the gauge widget to the paragraph data
            workflow_info.push(Line::from(vec![Span::styled(
                format!("Progress: {}", progress_text),
                Style::default().fg(Color::Blue),
            )]));

            let gauge = Gauge::default()
                .block(Block::default())
                .gauge_style(Style::default().fg(gauge_color).bg(Color::Black))
                .percent((progress * 100.0) as u16);

            // Render gauge separately after the paragraph
            let workflow_info_widget = Paragraph::new(workflow_info).block(
                Block::default()
                    .borders(Borders::ALL)
                    .border_type(BorderType::Rounded)
                    .title(Span::styled(
                        " Workflow Information ",
                        Style::default().fg(Color::Yellow),
                    )),
            );

            let gauge_area = Rect {
                x: chunks[0].x + 2,
                y: chunks[0].y + 4,
                width: chunks[0].width - 4,
                height: 1,
            };

            f.render_widget(workflow_info_widget, chunks[0]);
            f.render_widget(gauge, gauge_area);

            // Jobs list section - show appropriate information based on operation type
            let is_remote_trigger = execution
                .logs
                .iter()
                .any(|log| log.contains("Triggering workflow remotely"));

            if is_remote_trigger {
                // Handle remotely triggered workflows differently
                let (message, url_opt) = if workflow.status == WorkflowStatus::Success {
                    // Try to extract the URL from execution logs
                    let url = execution
                        .logs
                        .iter()
                        .filter_map(|log| {
                            if log.contains("https://github.com") {
                                log.lines()
                                    .find(|line| line.contains("https://github.com"))
                                    .map(|line| {
                                        // Extract URL with regex
                                        let re = regex::Regex::new(
                                            r"https://github\.com/[^/]+/[^/]+/actions[^\s]+",
                                        )
                                        .unwrap();
                                        re.find(line).map_or_else(|| line.trim(), |m| m.as_str())
                                    })
                            } else {
                                None
                            }
                        })
                        .next();

                    (
                        "Workflow triggered successfully on GitHub.\nCheck the Actions tab on GitHub to view progress.",
                        url
                    )
                } else if workflow.status == WorkflowStatus::Failed {
                    // Extract error message from logs
                    let error_msg = execution
                        .logs
                        .iter()
                        .find(|log| log.contains("Error:") || log.contains("Failed"));

                    (
                        error_msg.map_or("Failed to trigger workflow on GitHub.", |s| s.as_str()),
                        None,
                    )
                } else {
                    ("Triggering workflow on GitHub...", None)
                };

                // Create a more structured remote execution display
                let mut remote_trigger_info = vec![
                    Line::from(vec![Span::styled(
                        "GitHub Actions Remote Trigger",
                        Style::default()
                            .fg(Color::Yellow)
                            .add_modifier(Modifier::BOLD),
                    )]),
                    Line::from(""),
                    Line::from(message),
                    Line::from(""),
                ];

                // Add URL if available
                if let Some(url) = url_opt {
                    remote_trigger_info.push(Line::from(vec![
                        Span::styled("View on GitHub: ", Style::default().fg(Color::Blue)),
                        Span::styled(
                            url,
                            Style::default()
                                .fg(Color::Cyan)
                                .add_modifier(Modifier::UNDERLINED),
                        ),
                    ]));
                }

                let remote_display = Paragraph::new(remote_trigger_info)
                    .block(
                        Block::default()
                            .borders(Borders::ALL)
                            .border_type(BorderType::Rounded)
                            .title(Span::styled(
                                " Remote Execution ",
                                Style::default().fg(Color::Yellow),
                            )),
                    )
                    .alignment(Alignment::Left)
                    .wrap(Wrap { trim: true });

                f.render_widget(remote_display, chunks[1]);

                // Show a simpler execution info for remote triggers
                let mut execution_info = Vec::new();

                execution_info.push(Line::from(vec![
                    Span::styled("Started: ", Style::default().fg(Color::Blue)),
                    Span::styled(
                        execution.start_time.format("%Y-%m-%d %H:%M:%S").to_string(),
                        Style::default().fg(Color::White),
                    ),
                ]));

                if let Some(end_time) = execution.end_time {
                    execution_info.push(Line::from(vec![
                        Span::styled("Finished: ", Style::default().fg(Color::Blue)),
                        Span::styled(
                            end_time.format("%Y-%m-%d %H:%M:%S").to_string(),
                            Style::default().fg(Color::White),
                        ),
                    ]));

                    // Calculate duration
                    let duration = end_time.signed_duration_since(execution.start_time);
                    execution_info.push(Line::from(vec![
                        Span::styled("Duration: ", Style::default().fg(Color::Blue)),
                        Span::styled(
                            format!(
                                "{}m {}s",
                                duration.num_minutes(),
                                duration.num_seconds() % 60
                            ),
                            Style::default().fg(Color::White),
                        ),
                    ]));
                } else {
                    // Show running time for active workflows
                    let current_time = Local::now();
                    let running_time = current_time.signed_duration_since(execution.start_time);
                    execution_info.push(Line::from(vec![
                        Span::styled("Running for: ", Style::default().fg(Color::Blue)),
                        Span::styled(
                            format!(
                                "{}m {}s",
                                running_time.num_minutes(),
                                running_time.num_seconds() % 60
                            ),
                            Style::default().fg(Color::White),
                        ),
                    ]));
                }

                // Add hint for viewing execution on GitHub
                execution_info.push(Line::from(""));
                execution_info.push(Line::from(vec![
                    Span::styled("Status: ", Style::default().fg(Color::Blue)),
                    Span::styled(
                        match workflow.status {
                            WorkflowStatus::Success => "✅ Workflow triggered successfully",
                            WorkflowStatus::Failed => "❌ Failed to trigger workflow",
                            WorkflowStatus::Running => "⟳ Triggering workflow...",
                            _ => "Waiting...",
                        },
                        match workflow.status {
                            WorkflowStatus::Success => Style::default().fg(Color::Green),
                            WorkflowStatus::Failed => Style::default().fg(Color::Red),
                            WorkflowStatus::Running => Style::default().fg(Color::Cyan),
                            _ => Style::default().fg(Color::Gray),
                        },
                    ),
                ]));

                let info_widget = Paragraph::new(execution_info).block(
                    Block::default()
                        .borders(Borders::ALL)
                        .border_type(BorderType::Rounded)
                        .title(Span::styled(
                            " Trigger Information ",
                            Style::default().fg(Color::Yellow),
                        )),
                );

                f.render_widget(info_widget, chunks[2]);
            } else {
                // Standard local execution display
                if execution.jobs.is_empty() {
                    let placeholder = Paragraph::new("No jobs have started execution yet...")
                        .block(
                            Block::default()
                                .borders(Borders::ALL)
                                .border_type(BorderType::Rounded)
                                .title(Span::styled(" Jobs ", Style::default().fg(Color::Yellow))),
                        )
                        .alignment(Alignment::Center);
                    f.render_widget(placeholder, chunks[1]);
                } else {
                    let job_items: Vec<ListItem> = execution
                        .jobs
                        .iter()
                        .map(|job| {
                            let status_symbol = match job.status {
                                JobStatus::Success => "✅",
                                JobStatus::Failure => "❌",
                                JobStatus::Skipped => "⏭",
                            };

                            let status_style = match job.status {
                                JobStatus::Success => Style::default().fg(Color::Green),
                                JobStatus::Failure => Style::default().fg(Color::Red),
                                JobStatus::Skipped => Style::default().fg(Color::Gray),
                            };

                            // Count completed and total steps
                            let total_steps = job.steps.len();
                            let completed_steps = job
                                .steps
                                .iter()
                                .filter(|s| {
                                    s.status == StepStatus::Success
                                        || s.status == StepStatus::Failure
                                })
                                .count();

                            let steps_info = format!("[{}/{}]", completed_steps, total_steps);

                            ListItem::new(Line::from(vec![
                                Span::styled(status_symbol, status_style),
                                Span::raw(" "),
                                Span::styled(&job.name, Style::default().fg(Color::White)),
                                Span::raw(" "),
                                Span::styled(steps_info, Style::default().fg(Color::DarkGray)),
                            ]))
                        })
                        .collect();

                    let jobs_list = List::new(job_items)
                        .block(
                            Block::default()
                                .borders(Borders::ALL)
                                .border_type(BorderType::Rounded)
                                .title(Span::styled(" Jobs ", Style::default().fg(Color::Yellow))),
                        )
                        .highlight_style(
                            Style::default()
                                .bg(Color::DarkGray)
                                .add_modifier(Modifier::BOLD),
                        )
                        .highlight_symbol("» ");

                    f.render_stateful_widget(jobs_list, chunks[1], &mut app.job_list_state);
                }

                // Execution info section
                let mut execution_info = Vec::new();

                execution_info.push(Line::from(vec![
                    Span::styled("Started: ", Style::default().fg(Color::Blue)),
                    Span::styled(
                        execution.start_time.format("%Y-%m-%d %H:%M:%S").to_string(),
                        Style::default().fg(Color::White),
                    ),
                ]));

                if let Some(end_time) = execution.end_time {
                    execution_info.push(Line::from(vec![
                        Span::styled("Finished: ", Style::default().fg(Color::Blue)),
                        Span::styled(
                            end_time.format("%Y-%m-%d %H:%M:%S").to_string(),
                            Style::default().fg(Color::White),
                        ),
                    ]));

                    // Calculate duration
                    let duration = end_time.signed_duration_since(execution.start_time);
                    execution_info.push(Line::from(vec![
                        Span::styled("Duration: ", Style::default().fg(Color::Blue)),
                        Span::styled(
                            format!(
                                "{}m {}s",
                                duration.num_minutes(),
                                duration.num_seconds() % 60
                            ),
                            Style::default().fg(Color::White),
                        ),
                    ]));
                } else {
                    // Show running time for active workflows
                    let current_time = Local::now();
                    let running_time = current_time.signed_duration_since(execution.start_time);
                    execution_info.push(Line::from(vec![
                        Span::styled("Running for: ", Style::default().fg(Color::Blue)),
                        Span::styled(
                            format!(
                                "{}m {}s",
                                running_time.num_minutes(),
                                running_time.num_seconds() % 60
                            ),
                            Style::default().fg(Color::White),
                        ),
                    ]));
                }

                // Add hint for Enter key to see details
                execution_info.push(Line::from(""));
                execution_info.push(Line::from(vec![
                    Span::styled("Press ", Style::default().fg(Color::DarkGray)),
                    Span::styled("Enter", Style::default().fg(Color::Yellow)),
                    Span::styled(" to view job details", Style::default().fg(Color::DarkGray)),
                ]));

                let info_widget = Paragraph::new(execution_info).block(
                    Block::default()
                        .borders(Borders::ALL)
                        .border_type(BorderType::Rounded)
                        .title(Span::styled(
                            " Execution Information ",
                            Style::default().fg(Color::Yellow),
                        )),
                );

                f.render_widget(info_widget, chunks[2]);
            }
        } else {
            // No workflow execution to display
            let placeholder = Paragraph::new(vec![
                Line::from(""),
                Line::from(vec![Span::styled(
                    "No workflow execution data available.",
                    Style::default()
                        .fg(Color::Yellow)
                        .add_modifier(Modifier::BOLD),
                )]),
                Line::from(""),
                Line::from("Select workflows in the Workflows tab and press 'r' to run them."),
                Line::from(""),
                Line::from("Or press Enter on a selected workflow to run it directly."),
                Line::from(""),
                Line::from("You can also press 't' to trigger a workflow on GitHub remotely."),
            ])
            .block(
                Block::default()
                    .borders(Borders::ALL)
                    .border_type(BorderType::Rounded)
                    .title(Span::styled(
                        " Execution ",
                        Style::default().fg(Color::Yellow),
                    )),
            )
            .alignment(Alignment::Center);

            f.render_widget(placeholder, area);
        }
    } else {
        // No workflow execution to display
        let placeholder = Paragraph::new(vec![
            Line::from(""),
            Line::from(vec![Span::styled(
                "No workflow execution data available.",
                Style::default()
                    .fg(Color::Yellow)
                    .add_modifier(Modifier::BOLD),
            )]),
            Line::from(""),
            Line::from("Select workflows in the Workflows tab and press 'r' to run them."),
            Line::from(""),
            Line::from("Or press Enter on a selected workflow to run it directly."),
            Line::from(""),
            Line::from("You can also press 't' to trigger a workflow on GitHub remotely."),
        ])
        .block(
            Block::default()
                .borders(Borders::ALL)
                .border_type(BorderType::Rounded)
                .title(Span::styled(
                    " Execution ",
                    Style::default().fg(Color::Yellow),
                )),
        )
        .alignment(Alignment::Center);

        f.render_widget(placeholder, area);
    }
}

// Render detailed job view
fn render_job_detail_view(f: &mut Frame<CrosstermBackend<io::Stdout>>, app: &mut App, area: Rect) {
    // Get the current workflow and job
    let current_workflow_idx = app
        .current_execution
        .or_else(|| app.workflow_list_state.selected())
        .filter(|&idx| idx < app.workflows.len());

    if let Some(workflow_idx) = current_workflow_idx {
        let workflow = &app.workflows[workflow_idx];

        if let Some(execution) = &workflow.execution_details {
            if execution.jobs.is_empty() {
                let placeholder = Paragraph::new("This job has no steps or execution data.")
                    .block(
                        Block::default()
                            .borders(Borders::ALL)
                            .border_type(BorderType::Rounded)
                            .title(Span::styled(
                                " Job Details ",
                                Style::default().fg(Color::Yellow),
                            )),
                    )
                    .alignment(Alignment::Center);

                f.render_widget(placeholder, area);
                return;
            }

            // Ensure job index is valid
            let job_idx = app
                .job_list_state
                .selected()
                .unwrap_or(0)
                .min(execution.jobs.len() - 1);

            // Update the job_list_state in case we adjusted the selection
            app.job_list_state.select(Some(job_idx));

            let job = &execution.jobs[job_idx];

            // Split area for job details
            let chunks = Layout::default()
                .direction(Direction::Vertical)
                .constraints(
                    [
                        Constraint::Length(4), // Job info
                        Constraint::Length(7), // Steps list with border
                        Constraint::Min(3),    // Step output
                    ]
                    .as_ref(),
                )
                .margin(1)
                .split(area);

            // Job info
            let status_style = match job.status {
                JobStatus::Success => Style::default().fg(Color::Green),
                JobStatus::Failure => Style::default().fg(Color::Red),
                JobStatus::Skipped => Style::default().fg(Color::Gray),
            };

            let status_text = match job.status {
                JobStatus::Success => "Success",
                JobStatus::Failure => "Failed",
                JobStatus::Skipped => "Skipped",
            };

            let job_info = Paragraph::new(vec![
                Line::from(vec![
                    Span::styled("Job: ", Style::default().fg(Color::Blue)),
                    Span::styled(
                        job.name.clone(),
                        Style::default()
                            .fg(Color::White)
                            .add_modifier(Modifier::BOLD),
                    ),
                ]),
                Line::from(vec![
                    Span::styled("Status: ", Style::default().fg(Color::Blue)),
                    Span::styled(status_text, status_style),
                ]),
                Line::from(vec![
                    Span::styled("Steps: ", Style::default().fg(Color::Blue)),
                    Span::styled(
                        format!("{}", job.steps.len()),
                        Style::default().fg(Color::White),
                    ),
                ]),
            ])
            .block(
                Block::default()
                    .borders(Borders::ALL)
                    .border_type(BorderType::Rounded)
                    .title(Span::styled(
                        format!(" Job Details ({}/{}) ", job_idx + 1, execution.jobs.len()),
                        Style::default().fg(Color::Yellow),
                    )),
            );

            f.render_widget(job_info, chunks[0]);

            // Steps list with more details
            // Create a table for better aligned step information
            let header_cells = ["Status", "Step Name", "Duration"]
                .iter()
                .map(|h| Cell::from(*h).style(Style::default().fg(Color::Yellow)));

            let header = Row::new(header_cells)
                .style(Style::default().add_modifier(Modifier::BOLD))
                .height(1);

            let rows = job.steps.iter().map(|step| {
                let status_symbol = match step.status {
                    StepStatus::Success => "✅",
                    StepStatus::Failure => "❌",
                    StepStatus::Skipped => "⏭",
                };

                let status_style = match step.status {
                    StepStatus::Success => Style::default().fg(Color::Green),
                    StepStatus::Failure => Style::default().fg(Color::Red),
                    StepStatus::Skipped => Style::default().fg(Color::Gray),
                };

                // Calculate fake duration (would be real in a complete app)
                let duration = "1s";

                Row::new(vec![
                    Cell::from(status_symbol).style(status_style),
                    Cell::from(step.name.clone()),
                    Cell::from(duration).style(Style::default().fg(Color::DarkGray)),
                ])
                .height(1)
            });

            let steps_table = Table::new(rows)
                .header(header)
                .block(
                    Block::default()
                        .borders(Borders::ALL)
                        .border_type(BorderType::Rounded)
                        .title(Span::styled(" Steps ", Style::default().fg(Color::Yellow))),
                )
                .highlight_style(Style::default().bg(Color::DarkGray))
                .highlight_symbol("» ")
                .widths(&[
                    Constraint::Length(8),      // Status column
                    Constraint::Percentage(70), // Name column (takes most space)
                    Constraint::Length(10),     // Duration column
                ]);

            // Update step_list_state to match table state if needed
            if let Some(table_selected) = app.step_table_state.selected() {
                if app.step_list_state.selected() != Some(table_selected) {
                    app.step_list_state.select(Some(table_selected));
                }
            }

            f.render_stateful_widget(steps_table, chunks[1], &mut app.step_table_state);

            // Step output - show output from the selected step
            let output_text = if !job.steps.is_empty() {
                // Use the table_state for step selection to keep them in sync
                let step_idx = app
                    .step_table_state
                    .selected()
                    .unwrap_or(0)
                    .min(job.steps.len() - 1);

                // Also update step_list_state to stay in sync
                app.step_list_state.select(Some(step_idx));

                let step = &job.steps[step_idx];

                let mut output = step.output.clone();
                if output.is_empty() {
                    output = "No output for this step.".to_string();
                }

                // Limit output to prevent performance issues with very long strings
                if output.len() > 2000 {
                    let truncated = &output[..2000];
                    format!("{}\n... (output truncated) ...", truncated)
                } else {
                    output
                }
            } else {
                "No steps to display output for.".to_string()
            };

            let output_widget = Paragraph::new(output_text)
                .block(
                    Block::default()
                        .borders(Borders::ALL)
                        .border_type(BorderType::Rounded)
                        .title(Span::styled(" Output ", Style::default().fg(Color::Yellow))),
                )
                .wrap(Wrap { trim: true });

            f.render_widget(output_widget, chunks[2]);
        } else {
            // No execution details
            let placeholder = Paragraph::new("No job execution details available.")
                .block(
                    Block::default()
                        .borders(Borders::ALL)
                        .border_type(BorderType::Rounded)
                        .title(Span::styled(
                            " Job Details ",
                            Style::default().fg(Color::Yellow),
                        )),
                )
                .alignment(Alignment::Center);

            f.render_widget(placeholder, area);
        }
    } else {
        // No workflow selected
        let placeholder = Paragraph::new("No workflow execution available to display details.")
            .block(
                Block::default()
                    .borders(Borders::ALL)
                    .border_type(BorderType::Rounded)
                    .title(Span::styled(
                        " Job Details ",
                        Style::default().fg(Color::Yellow),
                    )),
            )
            .alignment(Alignment::Center);

        f.render_widget(placeholder, area);
    }
}

// Render the logs tab
fn render_logs_tab(f: &mut Frame<CrosstermBackend<io::Stdout>>, app: &App, area: Rect) {
    // Split the area into header, search bar (optionally shown), and log content
    let chunks = Layout::default()
        .direction(Direction::Vertical)
        .constraints(
            [
                Constraint::Length(3), // Header with instructions
                Constraint::Length(
                    if app.log_search_active
                        || !app.log_search_query.is_empty()
                        || app.log_filter_level.is_some()
                    {
                        3
                    } else {
                        0
                    },
                ), // Search bar (optional)
                Constraint::Min(3),    // Logs content
            ]
            .as_ref(),
        )
        .margin(1)
        .split(area);

    // Determine if search/filter bar should be shown
    let show_search_bar =
        app.log_search_active || !app.log_search_query.is_empty() || app.log_filter_level.is_some();

    // Render header with instructions
    let mut header_text = vec![
        Line::from(vec![Span::styled(
            "Execution and System Logs",
            Style::default()
                .fg(Color::Yellow)
                .add_modifier(Modifier::BOLD),
        )]),
        Line::from(vec![
            Span::styled("↑/↓", Style::default().fg(Color::Cyan)),
            Span::raw(" or "),
            Span::styled("j/k", Style::default().fg(Color::Cyan)),
            Span::raw(": Navigate logs/matches   "),
            Span::styled("s", Style::default().fg(Color::Cyan)),
            Span::raw(": Search   "),
            Span::styled("f", Style::default().fg(Color::Cyan)),
            Span::raw(": Filter   "),
            Span::styled("Tab", Style::default().fg(Color::Cyan)),
            Span::raw(": Switch tabs"),
        ]),
    ];

    if show_search_bar {
        header_text.push(Line::from(vec![
            Span::styled("Enter", Style::default().fg(Color::Cyan)),
            Span::raw(": Apply search   "),
            Span::styled("Esc", Style::default().fg(Color::Cyan)),
            Span::raw(": Clear search   "),
            Span::styled("c", Style::default().fg(Color::Cyan)),
            Span::raw(": Clear all filters"),
        ]));
    }

    let header = Paragraph::new(header_text)
        .block(
            Block::default()
                .borders(Borders::ALL)
                .border_type(BorderType::Rounded),
        )
        .alignment(Alignment::Center);

    f.render_widget(header, chunks[0]);

    // Render search bar if active or has content
    if show_search_bar {
        let search_text = if app.log_search_active {
            format!("Search: {}█", app.log_search_query)
        } else {
            format!("Search: {}", app.log_search_query)
        };

        let filter_text = match &app.log_filter_level {
            Some(level) => format!("Filter: {}", level.to_string()),
            None => "No filter".to_string(),
        };

        let match_info = if !app.log_search_matches.is_empty() {
            format!(
                "Matches: {}/{}",
                app.log_search_match_idx + 1,
                app.log_search_matches.len()
            )
        } else if !app.log_search_query.is_empty() {
            "No matches".to_string()
        } else {
            "".to_string()
        };

        let search_info = Line::from(vec![
            Span::raw(search_text),
            Span::raw("   "),
            Span::styled(
                filter_text,
                Style::default().fg(match &app.log_filter_level {
                    Some(LogFilterLevel::Error) => Color::Red,
                    Some(LogFilterLevel::Warning) => Color::Yellow,
                    Some(LogFilterLevel::Info) => Color::Cyan,
                    Some(LogFilterLevel::Success) => Color::Green,
                    Some(LogFilterLevel::Trigger) => Color::Magenta,
                    Some(LogFilterLevel::All) | None => Color::Gray,
                }),
            ),
            Span::raw("   "),
            Span::styled(match_info, Style::default().fg(Color::Magenta)),
        ]);

        let search_block = Paragraph::new(search_info)
            .block(
                Block::default()
                    .borders(Borders::ALL)
                    .border_type(BorderType::Rounded)
                    .title(Span::styled(
                        " Search & Filter ",
                        Style::default().fg(Color::Yellow),
                    )),
            )
            .alignment(Alignment::Left);

        f.render_widget(search_block, chunks[1]);
    }

    // Combine application logs with system logs
    let mut all_logs = Vec::new();

    // Now all logs should have timestamps in the format [HH:MM:SS]

    // Process app logs
    for log in &app.logs {
        all_logs.push(log.clone());
    }

    // Process system logs
    for log in crate::logging::get_logs() {
        all_logs.push(log.clone());
    }

    // Sort logs by timestamp if needed
    // all_logs.sort_by(|a, b| {
    //     // Extract timestamps and compare
    //     // For now we're keeping the order as they came in
    // });

    // Filter logs based on search query and filter level
    let filtered_logs = if !app.log_search_query.is_empty() || app.log_filter_level.is_some() {
        all_logs
            .iter()
            .filter(|log| {
                let passes_filter = match &app.log_filter_level {
                    None => true,
                    Some(level) => level.matches(log),
                };

                let matches_search = if app.log_search_query.is_empty() {
                    true
                } else {
                    log.to_lowercase()
                        .contains(&app.log_search_query.to_lowercase())
                };

                passes_filter && matches_search
            })
            .cloned()
            .collect::<Vec<String>>()
    } else {
        all_logs.clone() // Clone to avoid moving all_logs
    };

    // Create a table for logs for better organization
    let header_cells = ["Time", "Type", "Message"]
        .iter()
        .map(|h| Cell::from(*h).style(Style::default().fg(Color::Yellow)));

    let header = Row::new(header_cells)
        .style(Style::default().add_modifier(Modifier::BOLD))
        .height(1);

    let rows = filtered_logs.iter().map(|log_line| {
        // Parse log line to extract timestamp, type and message

        // Extract timestamp from log format [HH:MM:SS]
        let timestamp = if log_line.starts_with('[') && log_line.contains(']') {
            let end = log_line.find(']').unwrap_or(0);
            if end > 1 {
                log_line[1..end].to_string()
            } else {
                "??:??:??".to_string() // Show placeholder for malformed logs
            }
        } else {
            "??:??:??".to_string() // Show placeholder for malformed logs
        };

        let (log_type, log_style, _) =
            if log_line.contains("Error") || log_line.contains("error") || log_line.contains("❌")
            {
                ("ERROR", Style::default().fg(Color::Red), log_line.as_str())
            } else if log_line.contains("Warning")
                || log_line.contains("warning")
                || log_line.contains("⚠️")
            {
                (
                    "WARN",
                    Style::default().fg(Color::Yellow),
                    log_line.as_str(),
                )
            } else if log_line.contains("Success")
                || log_line.contains("success")
                || log_line.contains("✅")
            {
                (
                    "SUCCESS",
                    Style::default().fg(Color::Green),
                    log_line.as_str(),
                )
            } else if log_line.contains("Running")
                || log_line.contains("running")
                || log_line.contains("⟳")
            {
                ("INFO", Style::default().fg(Color::Cyan), log_line.as_str())
            } else if log_line.contains("Triggering") || log_line.contains("triggered") {
                (
                    "TRIG",
                    Style::default().fg(Color::Magenta),
                    log_line.as_str(),
                )
            } else {
                ("INFO", Style::default().fg(Color::Gray), log_line.as_str())
            };

        // Extract content after timestamp
        let content = if log_line.starts_with('[') && log_line.contains(']') {
            let start = log_line.find(']').unwrap_or(0) + 1;
            log_line[start..].trim()
        } else {
            log_line.as_str()
        };

        // Highlight search matches in content if search is active
        let mut content_spans = Vec::new();
        if !app.log_search_query.is_empty() {
            let lowercase_content = content.to_lowercase();
            let lowercase_query = app.log_search_query.to_lowercase();

            if lowercase_content.contains(&lowercase_query) {
                let mut last_idx = 0;
                while let Some(idx) = lowercase_content[last_idx..].find(&lowercase_query) {
                    let real_idx = last_idx + idx;

                    // Add text before match
                    if real_idx > last_idx {
                        content_spans.push(Span::raw(content[last_idx..real_idx].to_string()));
                    }

                    // Add matched text with highlight
                    let match_end = real_idx + app.log_search_query.len();
                    content_spans.push(Span::styled(
                        content[real_idx..match_end].to_string(),
                        Style::default().bg(Color::Yellow).fg(Color::Black),
                    ));

                    last_idx = match_end;
                }

                // Add remaining text after last match
                if last_idx < content.len() {
                    content_spans.push(Span::raw(content[last_idx..].to_string()));
                }
            } else {
                content_spans.push(Span::raw(content));
            }
        } else {
            content_spans.push(Span::raw(content));
        }

        Row::new(vec![
            Cell::from(timestamp),
            Cell::from(log_type).style(log_style),
            Cell::from(Line::from(content_spans)),
        ])
    });

    let content_idx = if show_search_bar { 2 } else { 1 };

    let log_table = Table::new(rows)
        .header(header)
        .block(
            Block::default()
                .borders(Borders::ALL)
                .border_type(BorderType::Rounded)
                .title(Span::styled(
                    format!(
                        " Logs ({}/{}) ",
                        if filtered_logs.is_empty() {
                            0
                        } else {
                            app.log_scroll + 1
                        },
                        filtered_logs.len()
                    ),
                    Style::default().fg(Color::Yellow),
                )),
        )
        .highlight_style(Style::default().bg(Color::DarkGray))
        .widths(&[
            Constraint::Length(10),     // Timestamp column
            Constraint::Length(7),      // Log type column
            Constraint::Percentage(80), // Message column
        ]);

    // We need to convert log_scroll index to a TableState
    let mut log_table_state = TableState::default();

    if !filtered_logs.is_empty() {
        // If we have search matches, use the match index as the selected row
        if !app.log_search_matches.is_empty() {
            // Make sure we're within bounds
            let match_index = app
                .log_search_match_idx
                .min(app.log_search_matches.len() - 1);

            // Get the filtered log index corresponding to the current search match
            if let Some(&original_idx) = app.log_search_matches.get(match_index) {
                // We need to map from the original all_logs index to filtered_logs index
                let filtered_idx = filtered_logs.iter().position(|log| {
                    // Find this log in all_logs
                    let all_logs_idx = if original_idx < app.logs.len() {
                        // This is an app log
                        app.logs
                            .get(original_idx)
                            .map(|l| all_logs.iter().position(|al| al == l))
                    } else {
                        // This is a system log
                        let system_idx = original_idx - app.logs.len();
                        crate::logging::get_logs()
                            .get(system_idx)
                            .map(|l| all_logs.iter().position(|al| al == l))
                    };

                    // If we found the log and it matches the current filtered log, select it
                    if let Some(Some(idx)) = all_logs_idx {
                        if idx < all_logs.len() && &all_logs[idx] == log {
                            return true;
                        }
                    }
                    false
                });

                // If we found the correct index in filtered logs, select it
                if let Some(idx) = filtered_idx {
                    log_table_state.select(Some(idx));
                } else {
                    // Fall back to regular scroll if we can't find the match
                    log_table_state.select(Some(app.log_scroll.min(filtered_logs.len() - 1)));
                }
            } else {
                // Fall back to regular scroll if match index is out of bounds
                log_table_state.select(Some(app.log_scroll.min(filtered_logs.len() - 1)));
            }
        } else {
            // No search matches, use regular scroll position
            log_table_state.select(Some(app.log_scroll.min(filtered_logs.len() - 1)));
        }
    }

    f.render_stateful_widget(log_table, chunks[content_idx], &mut log_table_state);
}

// Render the help tab
fn render_help_tab(f: &mut Frame<CrosstermBackend<io::Stdout>>, area: Rect) {
    let help_text = vec![
        Line::from(Span::styled(
            "Keyboard Controls",
            Style::default()
                .fg(Color::Cyan)
                .add_modifier(Modifier::BOLD),
        )),
        Line::from(""),
        Line::from(vec![
            Span::styled(
                "Tab",
                Style::default()
                    .fg(Color::Yellow)
                    .add_modifier(Modifier::BOLD),
            ),
            Span::raw(" - Switch between tabs"),
        ]),
        Line::from(vec![
            Span::styled(
                "1-4",
                Style::default()
                    .fg(Color::Yellow)
                    .add_modifier(Modifier::BOLD),
            ),
            Span::raw(" - Switch directly to tab"),
        ]),
        Line::from(vec![
            Span::styled(
                "Up/Down or j/k",
                Style::default()
                    .fg(Color::Yellow)
                    .add_modifier(Modifier::BOLD),
            ),
            Span::raw(" - Navigate logs or search matches"),
        ]),
        Line::from(vec![
            Span::styled(
                "Selected line",
                Style::default().fg(Color::White).bg(Color::DarkGray),
            ),
            Span::raw(" - Current scroll position or search match"),
        ]),
        Line::from(vec![
            Span::styled(
                "Space",
                Style::default()
                    .fg(Color::Yellow)
                    .add_modifier(Modifier::BOLD),
            ),
            Span::raw(" - Toggle workflow selection"),
        ]),
        Line::from(vec![
            Span::styled(
                "Enter",
                Style::default()
                    .fg(Color::Yellow)
                    .add_modifier(Modifier::BOLD),
            ),
            Span::raw(" - Run selected workflow / View job details"),
        ]),
        Line::from(vec![
            Span::styled(
                "r",
                Style::default()
                    .fg(Color::Yellow)
                    .add_modifier(Modifier::BOLD),
            ),
            Span::raw(" - Run all selected workflows"),
        ]),
        Line::from(vec![
            Span::styled(
                "a",
                Style::default()
                    .fg(Color::Yellow)
                    .add_modifier(Modifier::BOLD),
            ),
            Span::raw(" - Select all workflows"),
        ]),
        Line::from(vec![
            Span::styled(
                "e",
                Style::default()
                    .fg(Color::Yellow)
                    .add_modifier(Modifier::BOLD),
            ),
            Span::raw(" - Toggle between Docker and Emulation mode"),
        ]),
        Line::from(vec![
            Span::styled(
                "v",
                Style::default()
                    .fg(Color::Yellow)
                    .add_modifier(Modifier::BOLD),
            ),
            Span::raw(" - Toggle between Execution and Validation mode"),
        ]),
        Line::from(vec![
            Span::styled(
                "n",
                Style::default()
                    .fg(Color::Yellow)
                    .add_modifier(Modifier::BOLD),
            ),
            Span::raw(" - Deselect all workflows"),
        ]),
        Line::from(vec![
            Span::styled(
                "Shift+R",
                Style::default()
                    .fg(Color::Yellow)
                    .add_modifier(Modifier::BOLD),
            ),
            Span::raw(" - Reset workflow status to allow re-triggering"),
        ]),
        Line::from(vec![
            Span::styled(
                "Esc",
                Style::default()
                    .fg(Color::Yellow)
                    .add_modifier(Modifier::BOLD),
            ),
            Span::raw(" - Back / Exit detailed view"),
        ]),
        Line::from(vec![
            Span::styled(
                "?",
                Style::default()
                    .fg(Color::Yellow)
                    .add_modifier(Modifier::BOLD),
            ),
            Span::raw(" - Toggle help overlay"),
        ]),
        Line::from(vec![
            Span::styled(
                "q",
                Style::default()
                    .fg(Color::Yellow)
                    .add_modifier(Modifier::BOLD),
            ),
            Span::raw(" - Quit application"),
        ]),
        Line::from(vec![
            Span::styled(
                "t",
                Style::default()
                    .fg(Color::Yellow)
                    .add_modifier(Modifier::BOLD),
            ),
            Span::raw(" - Trigger selected workflow"),
        ]),
        Line::from(""),
        Line::from(Span::styled(
            "Runtime Modes",
            Style::default()
                .fg(Color::Cyan)
                .add_modifier(Modifier::BOLD),
        )),
        Line::from(""),
        Line::from(vec![
            Span::styled(
                "Docker",
                Style::default()
                    .fg(Color::Blue)
                    .add_modifier(Modifier::BOLD),
            ),
            Span::raw(" - Uses Docker to run workflows (default)"),
        ]),
        Line::from(vec![
            Span::styled(
                "Emulation",
                Style::default()
                    .fg(Color::Magenta)
                    .add_modifier(Modifier::BOLD),
            ),
            Span::raw(" - Emulates GitHub Actions environment locally (no Docker required)"),
        ]),
        Line::from(""),
        Line::from(Span::styled(
            "Logs Tab",
            Style::default()
                .fg(Color::Cyan)
                .add_modifier(Modifier::BOLD),
        )),
        Line::from(""),
        Line::from(vec![
            Span::styled(
                "Up/Down or j/k",
                Style::default()
                    .fg(Color::Yellow)
                    .add_modifier(Modifier::BOLD),
            ),
            Span::raw(" - Scroll through logs"),
        ]),
        Line::from(vec![
            Span::styled(
                "Selected line",
                Style::default().fg(Color::White).bg(Color::DarkGray),
            ),
            Span::raw(" - Current scroll position"),
        ]),
        Line::from(vec![
            Span::styled("Tab", Style::default().fg(Color::Yellow)),
            Span::raw(" - Navigate between tabs"),
        ]),
        Line::from(vec![
            Span::styled("s", Style::default().fg(Color::Yellow)),
            Span::raw(" - Search logs (in Logs tab)"),
        ]),
        Line::from(vec![
            Span::styled("f", Style::default().fg(Color::Yellow)),
            Span::raw(" - Toggle log filter (in Logs tab)"),
        ]),
        Line::from(vec![
            Span::styled("c", Style::default().fg(Color::Yellow)),
            Span::raw(" - Clear log search/filter (in Logs tab)"),
        ]),
        Line::from(vec![
            Span::styled("↑/↓", Style::default().fg(Color::Yellow)),
            Span::raw(" - Navigate between logs or search matches (in Logs tab)"),
        ]),
    ];

    let help_widget = Paragraph::new(help_text)
        .block(
            Block::default()
                .borders(Borders::ALL)
                .border_type(BorderType::Rounded)
                .title(Span::styled(" Help ", Style::default().fg(Color::Yellow))),
        )
        .wrap(Wrap { trim: true });

    f.render_widget(help_widget, area);
}

// Render a help overlay
fn render_help_overlay(f: &mut Frame<CrosstermBackend<io::Stdout>>) {
    let size = f.size();

    // Create a slightly smaller centered modal
    let width = size.width.min(60);
    let height = size.height.min(20);
    let x = (size.width - width) / 2;
    let y = (size.height - height) / 2;

    let help_area = Rect {
        x,
        y,
        width,
        height,
    };

    // Create a clear background
    let clear = Block::default().style(Style::default().bg(Color::Black));
    f.render_widget(clear, size);

    // Render the help content
    render_help_tab(f, help_area);
}

// Render the status bar
fn render_status_bar(f: &mut Frame<CrosstermBackend<io::Stdout>>, app: &App, area: Rect) {
    // If we have a status message, show it instead of the normal status bar
    if let Some(message) = &app.status_message {
        // Determine if this is a success message (starts with ✅)
        let is_success = message.starts_with("✅");

        let status_message = Paragraph::new(Line::from(vec![Span::styled(
            format!(" {} ", message),
            Style::default()
                .bg(if is_success { Color::Green } else { Color::Red })
                .fg(Color::White)
                .add_modifier(Modifier::BOLD),
        )]))
        .alignment(Alignment::Center);

        f.render_widget(status_message, area);
        return;
    }

    // Normal status bar
    let mut status_items = vec![];

    // Add mode info
    status_items.push(Span::styled(
        format!(" {} ", app.runtime_type_name()),
        Style::default()
            .bg(match app.runtime_type {
                RuntimeType::Docker => Color::Blue,
                RuntimeType::Emulation => Color::Magenta,
            })
            .fg(Color::White),
    ));

    // Add Docker status if relevant
    if app.runtime_type == RuntimeType::Docker {
        // Check Docker silently using safe FD redirection
        let is_docker_available =
            match utils::fd::with_stderr_to_null(executor::docker::is_available) {
                Ok(result) => result,
                Err(_) => {
                    logging::debug("Failed to redirect stderr when checking Docker availability.");
                    false
                }
            };

        status_items.push(Span::raw(" "));
        status_items.push(Span::styled(
            if is_docker_available {
                " Docker: Connected "
            } else {
                " Docker: Not Available "
            },
            Style::default()
                .bg(if is_docker_available {
                    Color::Green
                } else {
                    Color::Red
                })
                .fg(Color::White),
        ));
    }

    // Add validation/execution mode
    status_items.push(Span::raw(" "));
    status_items.push(Span::styled(
        format!(
            " {} ",
            if app.validation_mode {
                "Validation"
            } else {
                "Execution"
            }
        ),
        Style::default()
            .bg(if app.validation_mode {
                Color::Yellow
            } else {
                Color::Green
            })
            .fg(Color::Black),
    ));

    // Add context-specific help based on current tab
    status_items.push(Span::raw(" "));
    let help_text = match app.selected_tab {
        0 => {
            if let Some(idx) = app.workflow_list_state.selected() {
                if idx < app.workflows.len() {
                    let workflow = &app.workflows[idx];
                    match workflow.status {
                        WorkflowStatus::NotStarted => "[Space] Toggle selection   [Enter] Run selected   [r] Run all selected   [t] Trigger Workflow  [Shift+R] Reset workflow",
                        WorkflowStatus::Running => "[Space] Toggle selection   [Enter] Run selected   [r] Run all selected   (Workflow running...)",
                        WorkflowStatus::Success | WorkflowStatus::Failed | WorkflowStatus::Skipped => "[Space] Toggle selection   [Enter] Run selected   [r] Run all selected   [Shift+R] Reset workflow",
                    }
                } else {
                    "[Space] Toggle selection   [Enter] Run selected   [r] Run all selected"
                }
            } else {
                "[Space] Toggle selection   [Enter] Run selected   [r] Run all selected"
            }
        }
        1 => {
            if app.detailed_view {
                "[Esc] Back to jobs   [↑/↓] Navigate steps"
            } else {
                "[Enter] View details   [↑/↓] Navigate jobs"
            }
        }
        2 => {
            // For logs tab, show scrolling instructions
            let log_count = app.logs.len() + crate::logging::get_logs().len();
            if log_count > 0 {
                // Convert to a static string for consistent return type
                let scroll_text = format!(
                    "[↑/↓] Scroll logs ({}/{}) [s] Search [f] Filter",
                    app.log_scroll + 1,
                    log_count
                );
                Box::leak(scroll_text.into_boxed_str())
            } else {
                "[No logs to display]"
            }
        }
        3 => "[?] Toggle help overlay",
        _ => "",
    };
    status_items.push(Span::styled(
        format!(" {} ", help_text),
        Style::default().fg(Color::White),
    ));

    // Show keybindings for common actions
    status_items.push(Span::raw(" "));
    status_items.push(Span::styled(
        " [Tab] Switch tabs ",
        Style::default().fg(Color::White),
    ));
    status_items.push(Span::styled(
        " [?] Help ",
        Style::default().fg(Color::White),
    ));
    status_items.push(Span::styled(
        " [q] Quit ",
        Style::default().fg(Color::White),
    ));

    let status_bar = Paragraph::new(Line::from(status_items))
        .style(Style::default().bg(Color::DarkGray))
        .alignment(Alignment::Left);

    f.render_widget(status_bar, area);
}

// Validate a workflow or directory containing workflows
#[allow(clippy::ptr_arg)]
pub fn validate_workflow(path: &PathBuf, verbose: bool) -> io::Result<()> {
    let mut workflows = Vec::new();

    if path.is_dir() {
        let entries = std::fs::read_dir(path)?;

        for entry in entries {
            let entry = entry?;
            let entry_path = entry.path();

            if entry_path.is_file() && is_workflow_file(&entry_path) {
                workflows.push(entry_path);
            }
        }
    } else if path.is_file() {
        workflows.push(path.clone());
    } else {
        return Err(io::Error::new(
            io::ErrorKind::NotFound,
            format!("Path does not exist: {}", path.display()),
        ));
    }

    let mut valid_count = 0;
    let mut invalid_count = 0;

    println!("Validating {} workflow file(s)...", workflows.len());

    for workflow_path in workflows {
        match evaluate_workflow_file(&workflow_path, verbose) {
            Ok(result) => {
                if result.is_valid {
                    println!("✅ Valid: {}", workflow_path.display());
                    valid_count += 1;
                } else {
                    println!("❌ Invalid: {}", workflow_path.display());
                    for (i, issue) in result.issues.iter().enumerate() {
                        println!("   {}. {}", i + 1, issue);
                    }
                    invalid_count += 1;
                }
            }
            Err(e) => {
                println!("❌ Error processing {}: {}", workflow_path.display(), e);
                invalid_count += 1;
            }
        }
    }

    println!(
        "\nSummary: {} valid, {} invalid",
        valid_count, invalid_count
    );

    Ok(())
}

// Main entry point for the TUI interface
#[allow(clippy::ptr_arg)]
pub async fn run_wrkflw_tui(
    path: Option<&PathBuf>,
    runtime_type: RuntimeType,
    verbose: bool,
) -> io::Result<()> {
    // Call the TUI function directly - this was previously incorrectly referring to run_tui
    let result = {
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

        // Run the main event loop (simplified for this fix)
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

        result
    };

    match result {
        Ok(_) => Ok(()),
        Err(e) => {
            // If the TUI fails to initialize or crashes, fall back to CLI mode
            eprintln!("Failed to start UI: {}", e);

            // Only for 'tui' command should we fall back to CLI mode for files
            // For other commands, return the error
            if let Some(path) = path {
                if path.is_file() {
                    eprintln!("Falling back to CLI mode...");
                    execute_workflow_cli(path, runtime_type, verbose).await
                } else if path.is_dir() {
                    validate_workflow(path, verbose)
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

// Helper function to execute workflow trigger using curl
async fn execute_curl_trigger(
    workflow_name: &str,
    branch: Option<&str>,
) -> Result<(Vec<executor::JobResult>, ()), String> {
    // Get GitHub token
    let token = std::env::var("GITHUB_TOKEN").map_err(|_| {
        "GitHub token not found. Please set GITHUB_TOKEN environment variable".to_string()
    })?;

    // Debug log to check if GITHUB_TOKEN is set
    match std::env::var("GITHUB_TOKEN") {
        Ok(token) => logging::info(&format!("GITHUB_TOKEN is set: {}", &token[..5])), // Log first 5 characters for security
        Err(_) => logging::error("GITHUB_TOKEN is not set"),
    }

    // Get repository information
    let repo_info = crate::github::get_repo_info()
        .map_err(|e| format!("Failed to get repository info: {}", e))?;

    // Determine branch to use
    let branch_ref = branch.unwrap_or(&repo_info.default_branch);

    // Construct JSON payload
    let payload = format!("{{\"ref\":\"{}\"}}", branch_ref);

    // Construct API URL
    let url = format!(
        "https://api.github.com/repos/{}/{}/actions/workflows/{}/dispatches",
        repo_info.owner, repo_info.repo, workflow_name
    );

    // Log the constructed API URL and payload for debugging
    logging::info(&format!(
        "Triggering workflow with URL: {} and payload: {}",
        url, payload
    ));

    // Use a Command to run curl - disable verbose flags for better performance
    let output = tokio::process::Command::new("curl")
        .arg("-s") // Silent mode
        .arg("-X")
        .arg("POST")
        .arg("-H")
        .arg(format!("Authorization: Bearer {}", token.trim()))
        .arg("-H")
        .arg("Accept: application/vnd.github.v3+json")
        .arg("-H")
        .arg("Content-Type: application/json")
        .arg("-H")
        .arg("User-Agent: wrkflw-cli")
        .arg("-d")
        .arg(payload)
        .arg(url)
        .output()
        .await
        .map_err(|e| format!("Failed to execute curl: {}", e))?;

    // Log the output of the curl command for debugging
    if !output.status.success() {
        let error = String::from_utf8_lossy(&output.stderr);
        logging::error(&format!("Curl command failed: {}", error));
        return Err(format!("Curl command failed: {}", error));
    } else {
        let success_output = String::from_utf8_lossy(&output.stdout);
        logging::info(&format!("Curl command succeeded: {}", success_output));
    }

    // Success message with URL to view the workflow
    let success_msg = format!(
        "Workflow triggered successfully. View it at: https://github.com/{}/{}/actions/workflows/{}.yml",
        repo_info.owner, repo_info.repo, workflow_name
    );

    // Create a job result structure
    let job_result = executor::JobResult {
        name: "GitHub Trigger".to_string(),
        status: executor::JobStatus::Success,
        steps: vec![executor::StepResult {
            name: "Remote Trigger".to_string(),
            status: executor::StepStatus::Success,
            output: success_msg,
        }],
        logs: "Workflow triggered remotely on GitHub".to_string(),
    };

    Ok((vec![job_result], ()))
}

// Add a new function before the run_tui_event_loop function
// Extract common workflow execution logic to avoid duplication
fn start_next_workflow_execution(
    app: &mut App,
    tx_clone: &mpsc::Sender<ExecutionResultMsg>,
    verbose: bool,
) {
    if let Some(next_idx) = app.get_next_workflow_to_execute() {
        app.current_execution = Some(next_idx);
        let tx_clone_inner = tx_clone.clone();
        let workflow_path = app.workflows[next_idx].path.clone();

        // Log whether verbose mode is enabled
        if verbose {
            app.logs
                .push("Verbose mode: Step outputs will be displayed in full".to_string());
            logging::info("Verbose mode: Step outputs will be displayed in full");
        } else {
            app.logs.push(
                "Standard mode: Only step status will be shown (use --verbose for full output)"
                    .to_string(),
            );
            logging::info(
                "Standard mode: Only step status will be shown (use --verbose for full output)",
            );
        }

        // Check Docker availability again if Docker runtime is selected
        let runtime_type = match app.runtime_type {
            RuntimeType::Docker => {
                // Use safe FD redirection to check Docker availability
                let is_docker_available =
                    match utils::fd::with_stderr_to_null(executor::docker::is_available) {
                        Ok(result) => result,
                        Err(_) => {
                            logging::debug(
                                "Failed to redirect stderr when checking Docker availability.",
                            );
                            false
                        }
                    };

                if !is_docker_available {
                    app.logs
                        .push("Docker is not available. Using emulation mode instead.".to_string());
                    logging::warning("Docker is not available. Using emulation mode instead.");
                    RuntimeType::Emulation
                } else {
                    RuntimeType::Docker
                }
            }
            RuntimeType::Emulation => RuntimeType::Emulation,
        };

        let validation_mode = app.validation_mode;

        // Update workflow status and add execution details
        app.workflows[next_idx].status = WorkflowStatus::Running;

        // Initialize execution details if not already done
        if app.workflows[next_idx].execution_details.is_none() {
            app.workflows[next_idx].execution_details = Some(WorkflowExecution {
                jobs: Vec::new(),
                start_time: Local::now(),
                end_time: None,
                logs: Vec::new(),
                progress: 0.0,
            });
        }

        thread::spawn(move || {
            let rt = match tokio::runtime::Runtime::new() {
                Ok(runtime) => runtime,
                Err(e) => {
                    eprintln!("Failed to create Tokio runtime: {}", e);
                    // Return early from the current function with appropriate error handling
                    let _ = tx_clone_inner.send((
                        next_idx,
                        Err("Failed to create runtime for execution".to_string()),
                    ));
                    return;
                }
            };

            let result = rt.block_on(async {
                if validation_mode {
                    // Perform validation instead of execution
                    match evaluate_workflow_file(&workflow_path, verbose) {
                        Ok(validation_result) => {
                            // Create execution result based on validation
                            let status = if validation_result.is_valid {
                                executor::JobStatus::Success
                            } else {
                                executor::JobStatus::Failure
                            };

                            // Create a synthetic job result for validation
                            let jobs = vec![executor::JobResult {
                                name: "Validation".to_string(),
                                status,
                                steps: vec![executor::StepResult {
                                    name: "Validator".to_string(),
                                    status: if validation_result.is_valid {
                                        executor::StepStatus::Success
                                    } else {
                                        executor::StepStatus::Failure
                                    },
                                    output: validation_result.issues.join("\n"),
                                }],
                                logs: format!(
                                    "Validation result: {}",
                                    if validation_result.is_valid {
                                        "PASSED"
                                    } else {
                                        "FAILED"
                                    }
                                ),
                            }];

                            Ok((jobs, ()))
                        }
                        Err(e) => Err(e.to_string()),
                    }
                } else {
                    // Use safe FD redirection for execution
                    let execution_result = utils::fd::with_stderr_to_null(|| {
                        futures::executor::block_on(async {
                            executor::execute_workflow(&workflow_path, runtime_type, verbose).await
                        })
                    })
                    .map_err(|e| format!("Failed to redirect stderr during execution: {}", e))?;

                    match execution_result {
                        Ok(execution_result) => {
                            // Send back the job results in a wrapped result
                            Ok((execution_result.jobs, ()))
                        }
                        Err(e) => Err(e.to_string()),
                    }
                }
            });

            // Only send if we get a valid result
            if let Err(e) = tx_clone_inner.send((next_idx, result)) {
                eprintln!("Error sending execution result: {}", e);
            }
        });
    } else {
        app.running = false;
        let timestamp = Local::now().format("%H:%M:%S").to_string();
        app.logs
            .push(format!("[{}] All workflows completed execution", timestamp));
        logging::info("All workflows completed execution");
    }
}

#[allow(clippy::ptr_arg)]
pub async fn execute_workflow_cli(
    path: &PathBuf,
    runtime_type: RuntimeType,
    verbose: bool,
) -> io::Result<()> {
    if !path.exists() {
        return Err(io::Error::new(
            io::ErrorKind::NotFound,
            format!("Workflow file does not exist: {}", path.display()),
        ));
    }

    println!("Validating workflow...");
    match evaluate_workflow_file(path, false) {
        Ok(result) => {
            if !result.is_valid {
                println!("❌ Cannot execute invalid workflow: {}", path.display());
                for (i, issue) in result.issues.iter().enumerate() {
                    println!("   {}. {}", i + 1, issue);
                }
                return Err(io::Error::new(
                    io::ErrorKind::InvalidData,
                    "Workflow validation failed",
                ));
            }
        }
        Err(e) => {
            return Err(io::Error::new(
                io::ErrorKind::Other,
                format!("Error validating workflow: {}", e),
            ));
        }
    }

    // Check Docker availability if Docker runtime is selected
    let runtime_type = match runtime_type {
        RuntimeType::Docker => {
            if !executor::docker::is_available() {
                println!("⚠️ Docker is not available. Using emulation mode instead.");
                logging::warning("Docker is not available. Using emulation mode instead.");
                RuntimeType::Emulation
            } else {
                RuntimeType::Docker
            }
        }
        RuntimeType::Emulation => RuntimeType::Emulation,
    };

    println!("Executing workflow: {}", path.display());
    println!("Runtime mode: {:?}", runtime_type);

    // Log the start of the execution in debug mode with more details
    logging::debug(&format!(
        "Starting workflow execution: path={}, runtime={:?}, verbose={}",
        path.display(),
        runtime_type,
        verbose
    ));

    match executor::execute_workflow(path, runtime_type, verbose).await {
        Ok(result) => {
            println!("\nWorkflow execution results:");

            // Track if the workflow had any failures
            let mut any_job_failed = false;

            for job in &result.jobs {
                match job.status {
                    JobStatus::Success => {
                        println!("\n✅ Job succeeded: {}", job.name);
                    }
                    JobStatus::Failure => {
                        println!("\n❌ Job failed: {}", job.name);
                        any_job_failed = true;
                    }
                    JobStatus::Skipped => {
                        println!("\n⏭️ Job skipped: {}", job.name);
                    }
                }

                println!("-------------------------");

                // Log the job details for debug purposes
                logging::debug(&format!("Job: {}, Status: {:?}", job.name, job.status));

                for step in job.steps.iter() {
                    match step.status {
                        StepStatus::Success => {
                            println!("  ✅ {}", step.name);

                            // Check if this is a GitHub action output that should be hidden
                            let should_hide = std::env::var("WRKFLW_HIDE_ACTION_MESSAGES")
                                .map(|val| val == "true")
                                .unwrap_or(false)
                                && step.output.contains("Would execute GitHub action:");

                            // Only show output if not hidden and it's short
                            if !should_hide
                                && !step.output.trim().is_empty()
                                && step.output.lines().count() <= 3
                            {
                                // For short outputs, show directly
                                println!("    {}", step.output.trim());
                            }
                        }
                        StepStatus::Failure => {
                            println!("  ❌ {}", step.name);

                            // Ensure we capture and show exit code
                            if let Some(exit_code) = step
                                .output
                                .lines()
                                .find(|line| line.trim().starts_with("Exit code:"))
                                .map(|line| line.trim().to_string())
                            {
                                println!("    {}", exit_code);
                            }

                            // Show command/run details in debug mode
                            if logging::get_log_level() <= logging::LogLevel::Debug {
                                if let Some(cmd_output) = step
                                    .output
                                    .lines()
                                    .skip_while(|l| !l.trim().starts_with("$"))
                                    .take(1)
                                    .next()
                                {
                                    println!("    Command: {}", cmd_output.trim());
                                }
                            }

                            // Always show error output from failed steps, but keep it to a reasonable length
                            let output_lines: Vec<&str> = step
                                .output
                                .lines()
                                .filter(|line| !line.trim().starts_with("Exit code:"))
                                .collect();

                            if !output_lines.is_empty() {
                                println!("    Error output:");
                                for line in output_lines.iter().take(10) {
                                    println!("    {}", line.trim().replace('\n', "\n    "));
                                }

                                if output_lines.len() > 10 {
                                    println!(
                                        "    ... (and {} more lines)",
                                        output_lines.len() - 10
                                    );
                                    println!("    Use --debug to see full output");
                                }
                            }
                        }
                        StepStatus::Skipped => {
                            println!("  ⏭️ {} (skipped)", step.name);
                        }
                    }

                    // Always log the step details for debug purposes
                    logging::debug(&format!(
                        "Step: {}, Status: {:?}, Output length: {} lines",
                        step.name,
                        step.status,
                        step.output.lines().count()
                    ));

                    // In debug mode, log all step output
                    if logging::get_log_level() == logging::LogLevel::Debug
                        && !step.output.trim().is_empty()
                    {
                        logging::debug(&format!(
                            "Step output for '{}': \n{}",
                            step.name, step.output
                        ));
                    }
                }
            }

            if any_job_failed {
                println!("\n❌ Workflow completed with failures");
                // In the case of failure, we'll also inform the user about the debug option
                // if they're not already using it
                if logging::get_log_level() > logging::LogLevel::Debug {
                    println!("    Run with --debug for more detailed output");
                }
            } else {
                println!("\n✅ Workflow completed successfully!");
            }

            Ok(())
        }
        Err(e) => {
            println!("❌ Failed to execute workflow: {}", e);
            logging::error(&format!("Failed to execute workflow: {}", e));
            Err(io::Error::new(io::ErrorKind::Other, e))
        }
    }
}
