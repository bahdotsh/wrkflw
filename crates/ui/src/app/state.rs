// App state for the UI
use crate::models::{
    ExecutionResultMsg, JobExecution, LogFilterLevel, StepExecution, Workflow, WorkflowExecution,
    WorkflowStatus,
};
use chrono::Local;
use crossterm::event::KeyCode;
use executor::{JobStatus, RuntimeType, StepStatus};
use ratatui::widgets::{ListState, TableState};
use std::sync::mpsc;
use std::time::{Duration, Instant};

/// Application state
pub struct App {
    pub workflows: Vec<Workflow>,
    pub workflow_list_state: ListState,
    pub selected_tab: usize,
    pub running: bool,
    pub show_help: bool,
    pub runtime_type: RuntimeType,
    pub validation_mode: bool,
    pub execution_queue: Vec<usize>, // Indices of workflows to execute
    pub current_execution: Option<usize>,
    pub logs: Vec<String>,                    // Overall execution logs
    pub log_scroll: usize,                    // Scrolling position for logs
    pub job_list_state: ListState,            // For viewing job details
    pub detailed_view: bool,                  // Whether we're in detailed view mode
    pub step_list_state: ListState,           // For selecting steps in detailed view
    pub step_table_state: TableState,         // For the steps table in detailed view
    pub last_tick: Instant,                   // For UI animations and updates
    pub tick_rate: Duration,                  // How often to update the UI
    pub tx: mpsc::Sender<ExecutionResultMsg>, // Channel for async communication
    pub status_message: Option<String>,       // Temporary status message to display
    pub status_message_time: Option<Instant>, // When the message was set

    // Search and filter functionality
    pub log_search_query: String, // Current search query for logs
    pub log_search_active: bool,  // Whether search input is active
    pub log_filter_level: Option<LogFilterLevel>, // Current log level filter
    pub log_search_matches: Vec<usize>, // Indices of logs that match the search
    pub log_search_match_idx: usize, // Current match index for navigation
}

impl App {
    pub fn new(runtime_type: RuntimeType, tx: mpsc::Sender<ExecutionResultMsg>) -> App {
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
    pub fn toggle_selected(&mut self) {
        if let Some(idx) = self.workflow_list_state.selected() {
            if idx < self.workflows.len() {
                self.workflows[idx].selected = !self.workflows[idx].selected;
            }
        }
    }

    pub fn toggle_emulation_mode(&mut self) {
        self.runtime_type = match self.runtime_type {
            RuntimeType::Docker => RuntimeType::Emulation,
            RuntimeType::Emulation => RuntimeType::Docker,
        };
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
        logging::info(&format!("Switched to {} mode", mode));
    }

    pub fn runtime_type_name(&self) -> &str {
        match self.runtime_type {
            RuntimeType::Docker => "Docker",
            RuntimeType::Emulation => "Emulation",
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
    pub fn next_job(&mut self) {
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
    pub fn start_execution(&mut self) {
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
    pub fn process_execution_result(
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
    pub fn get_next_workflow_to_execute(&mut self) -> Option<usize> {
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
    pub fn toggle_log_search(&mut self) {
        self.log_search_active = !self.log_search_active;
        if !self.log_search_active {
            // Don't clear the query, this allows toggling the search UI while keeping the filter
        } else {
            // When activating search, update matches
            self.update_log_search_matches();
        }
    }

    // Toggle log filter
    pub fn toggle_log_filter(&mut self) {
        self.log_filter_level = match &self.log_filter_level {
            None => Some(LogFilterLevel::Info),
            Some(level) => Some(level.next()),
        };

        // Update search matches when filter changes
        self.update_log_search_matches();
    }

    // Clear log search and filter
    pub fn clear_log_search_and_filter(&mut self) {
        self.log_search_query.clear();
        self.log_filter_level = None;
        self.log_search_matches.clear();
        self.log_search_match_idx = 0;
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
        for log in logging::get_logs() {
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
    pub fn next_search_match(&mut self) {
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
    pub fn scroll_logs_up(&mut self) {
        self.log_scroll = self.log_scroll.saturating_sub(1);
    }

    // Scroll logs down
    pub fn scroll_logs_down(&mut self) {
        // Get total log count including system logs
        let total_logs = self.logs.len() + logging::get_logs().len();
        if total_logs > 0 {
            self.log_scroll = (self.log_scroll + 1).min(total_logs - 1);
        }
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

    // Set a temporary status message to be displayed in the UI
    pub fn set_status_message(&mut self, message: String) {
        self.status_message = Some(message);
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
                        logging::error(&format!("Error sending trigger result: {}", e));
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
    pub fn reset_workflow_status(&mut self) {
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
            }
        }
    }
}
