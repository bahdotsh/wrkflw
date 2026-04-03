// App state for the UI
use crate::log_processor::{LogProcessingRequest, LogProcessor, ProcessedLogEntry};
use crate::models::{
    ExecutionResultMsg, JobExecution, LogFilterLevel, QueuedExecution, StatusSeverity,
    StepExecution, Workflow, WorkflowExecution, WorkflowStatus,
};
use chrono::Local;
use crossterm::event::KeyCode;
use ratatui::widgets::{ListState, TableState};
use std::sync::mpsc;
use std::time::{Duration, Instant};
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
}

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

    /// Trigger log processing when search/filter changes
    pub fn mark_logs_for_update(&mut self) {
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
        self.mark_logs_for_update();
    }

    /// Add a formatted log entry with timestamp and trigger log processing update
    pub fn add_timestamped_log(&mut self, message: &str) {
        let timestamp = Local::now().format("%H:%M:%S").to_string();
        let formatted_message = format!("[{}] {}", timestamp, message);
        self.add_log(formatted_message);
    }
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
            },
            Workflow {
                name: "deploy".to_string(),
                path: PathBuf::from("deploy.yml"),
                selected: false,
                status: WorkflowStatus::NotStarted,
                execution_details: None,
                job_names: vec![],
            },
        ];
        app.workflow_list_state.select(Some(0));
        app
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
