use crate::evaluator::evaluate_workflow_file;
use crate::executor::{self, ExecutionResult, JobStatus, RuntimeType, StepStatus};
use crate::logging;
use crate::utils::is_workflow_file;
use chrono::Local;
use crossterm::{
    event::{self, DisableMouseCapture, EnableMouseCapture, Event, KeyCode},
    execute,
    terminal::{disable_raw_mode, enable_raw_mode, EnterAlternateScreen, LeaveAlternateScreen},
};
use ratatui::widgets::TableState;
use ratatui::{
    backend::CrosstermBackend,
    layout::{Alignment, Constraint, Direction, Layout, Rect},
    style::{Color, Modifier, Style},
    text::{Line, Span},
    widgets::{
        Block, BorderType, Borders, Cell, Gauge, List, ListItem, ListState, Paragraph, Row, Table,
        Tabs, Wrap,
    },
    Frame, Terminal,
};
use std::io::{self, stdout};
use std::path::{Path, PathBuf};
use std::process;
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

// Application state
struct App {
    workflows: Vec<Workflow>,
    workflow_list_state: ListState,
    selected_tab: usize,
    running: bool,
    show_help: bool,
    runtime_type: RuntimeType,
    execution_queue: Vec<usize>, // Indices of workflows to execute
    current_execution: Option<usize>,
    logs: Vec<String>,            // Overall execution logs
    log_scroll: usize,            // Scrolling position for logs
    job_list_state: ListState,    // For viewing job details
    detailed_view: bool,          // Whether we're in detailed view mode
    step_list_state: ListState,   // For selecting steps in detailed view
    step_table_state: TableState, // For the steps table in detailed view
    last_tick: Instant,           // For UI animations and updates
    tick_rate: Duration,          // How often to update the UI
}

impl App {
    fn new(runtime_type: RuntimeType) -> App {
        let mut workflow_list_state = ListState::default();
        workflow_list_state.select(Some(0));

        let mut job_list_state = ListState::default();
        job_list_state.select(Some(0));

        let mut step_list_state = ListState::default();
        step_list_state.select(Some(0));

        let mut step_table_state = TableState::default();
        step_table_state.select(Some(0));

        App {
            workflows: Vec::new(),
            workflow_list_state,
            selected_tab: 0,
            running: false,
            show_help: false,
            runtime_type,
            execution_queue: Vec::new(),
            current_execution: None,
            logs: Vec::new(),
            log_scroll: 0,
            job_list_state,
            detailed_view: false,
            step_list_state,
            step_table_state,
            last_tick: Instant::now(),
            tick_rate: Duration::from_millis(250), // Update 4 times per second
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
            .or_else(|| self.workflow_list_state.selected());

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
                    }
                }
            }
        }
        if let Some(i) = self.step_list_state.selected() {
            self.step_table_state.select(Some(i));
        }
    }

    // Move cursor down in step list
    fn next_step(&mut self) {
        let current_workflow_idx = self
            .current_execution
            .or_else(|| self.workflow_list_state.selected());

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
                    }
                }
            }
        }
        if let Some(i) = self.step_list_state.selected() {
            self.step_table_state.select(Some(i));
        }
    }

    // Change the tab
    fn switch_tab(&mut self, tab: usize) {
        self.selected_tab = tab;
    }

    // Queue selected workflows for execution
    fn queue_selected_for_execution(&mut self) {
        self.execution_queue.clear();
        for (i, workflow) in self.workflows.iter().enumerate() {
            if workflow.selected {
                self.execution_queue.push(i);
            }
        }
        self.logs.push(format!(
            "Queued {} workflow(s) for execution",
            self.execution_queue.len()
        ));
        logging::info(&format!(
            "Queued {} workflow(s) for execution",
            self.execution_queue.len()
        ));
    }

    // Start workflow execution process
    fn start_execution(&mut self) {
        if self.execution_queue.is_empty() {
            self.logs
                .push("No workflows selected for execution".to_string());
            logging::warning("No workflows selected for execution");
            return;
        }

        self.running = true;
        self.logs.push("Starting workflow execution...".to_string());
        logging::info("Starting workflow execution...");

        // Update all queued workflows to "Queued" state
        for &idx in &self.execution_queue {
            self.workflows[idx].status = WorkflowStatus::Skipped;
        }
    }

    // Process execution results and update UI
    fn process_execution_result(&mut self, workflow_idx: usize, result: ExecutionResult) {
        let workflow = &mut self.workflows[workflow_idx];

        // Convert execution results to our internal format
        let mut job_executions = Vec::new();
        for job in result.jobs {
            let mut step_executions = Vec::new();
            for step in job.steps {
                step_executions.push(StepExecution {
                    name: step.name,
                    status: step.status,
                    output: step.output,
                });
            }

            job_executions.push(JobExecution {
                name: job.name,
                status: job.status,
                steps: step_executions,
                logs: vec![job.logs],
            });
        }

        let end_time = Local::now();

        // Determine overall workflow status
        if job_executions
            .iter()
            .any(|j| j.status == JobStatus::Failure)
        {
            workflow.status = WorkflowStatus::Failed;
            logging::error(&format!("Workflow '{}' failed", workflow.name));
        } else {
            workflow.status = WorkflowStatus::Success;
            logging::success(&format!(
                "Workflow '{}' completed successfully",
                workflow.name
            ));
        }

        // Update workflow execution details
        if let Some(exec) = &mut workflow.execution_details {
            exec.jobs = job_executions;
            exec.end_time = Some(end_time);
            exec.progress = 1.0; // Completed
        } else {
            workflow.execution_details = Some(WorkflowExecution {
                jobs: job_executions,
                start_time: end_time - chrono::Duration::seconds(1), // Approximate
                end_time: Some(end_time),
                logs: vec!["Execution completed".to_string()],
                progress: 1.0, // Completed
            });
        }

        // Add workflow completion to logs
        self.logs.push(format!(
            "Workflow '{}' execution completed with status: {:?}",
            workflow.name, workflow.status
        ));
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
    }

    // Scroll logs up
    fn scroll_logs_up(&mut self) {
        self.log_scroll = self.log_scroll.saturating_sub(1);
    }

    // Scroll logs down
    fn scroll_logs_down(&mut self) {
        if !self.logs.is_empty() {
            self.log_scroll = (self.log_scroll + 1).min(self.logs.len() - 1);
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

    // Check if tick should happen
    fn tick(&mut self) -> bool {
        let now = Instant::now();
        if now.duration_since(self.last_tick) >= self.tick_rate {
            self.last_tick = now;
            true
        } else {
            false
        }
    }
}

// Find and load all workflow files in a directory
fn load_workflows(dir_path: &Path) -> Vec<Workflow> {
    let mut workflows = Vec::new();

    // Default path is .github/workflows
    let default_workflows_dir = Path::new(".github").join("workflows");
    let is_default_dir = dir_path == &default_workflows_dir || dir_path.ends_with("workflows");

    if let Ok(entries) = std::fs::read_dir(dir_path) {
        for entry in entries {
            if let Ok(entry) = entry {
                let path = entry.path();
                if path.is_file() && (is_workflow_file(&path) || !is_default_dir) {
                    let name = path
                        .file_name()
                        .unwrap_or_default()
                        .to_string_lossy()
                        .into_owned();

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
    }

    // Sort workflows by name
    workflows.sort_by(|a, b| a.name.cmp(&b.name));
    workflows
}

// Main UI function to run the TUI application
pub fn run_tui(
    workflow_path: Option<&PathBuf>,
    runtime_type: &RuntimeType,
    verbose: bool,
) -> io::Result<()> {
    // Terminal setup
    enable_raw_mode()?;
    let mut stdout = stdout();
    execute!(stdout, EnterAlternateScreen, EnableMouseCapture)?;
    let backend = CrosstermBackend::new(stdout);
    let mut terminal = Terminal::new(backend)?;

    // Set up channel for async communication
    let (tx, rx) = mpsc::channel();

    // Initialize app state
    let mut app = App::new(runtime_type.clone());

    // Load workflows
    let dir_path = match workflow_path {
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

    // Main event loop
    loop {
        // Update UI on tick
        if app.tick() {
            app.update_running_workflow_progress();
        }

        // Draw UI
        terminal.draw(|f| {
            render_ui(f, &mut app);
        })?;

        // Handle incoming execution results
        if let Ok((workflow_idx, result)) = rx.try_recv() {
            app.process_execution_result(workflow_idx, result);
            app.current_execution = None;

            // Get next workflow to execute
            if let Some(next_idx) = app.get_next_workflow_to_execute() {
                let tx_clone = tx.clone();
                let workflow_path = app.workflows[next_idx].path.clone();
                let runtime_type = app.runtime_type.clone();

                thread::spawn(move || {
                    let runtime = tokio::runtime::Runtime::new().unwrap();
                    let result = runtime.block_on(async {
                        match executor::execute_workflow(&workflow_path, runtime_type, verbose)
                            .await
                        {
                            Ok(result) => result,
                            Err(e) => {
                                // Create a failed execution result with error message
                                let failed_job = executor::JobResult {
                                    name: "Error".to_string(),
                                    status: JobStatus::Failure,
                                    steps: vec![executor::StepResult {
                                        name: "Execution Error".to_string(),
                                        status: StepStatus::Failure,
                                        output: format!("Error: {}", e),
                                    }],
                                    logs: format!("Error executing workflow: {}", e),
                                };

                                ExecutionResult {
                                    jobs: vec![failed_job],
                                }
                            }
                        }
                    });

                    tx_clone.send((next_idx, result)).unwrap();
                });
            } else {
                app.running = false;
                app.logs
                    .push("All workflows completed execution".to_string());
                logging::info("All workflows completed execution");
            }
        }

        // Start execution if we have a queued workflow and nothing is currently running
        if app.running && app.current_execution.is_none() && !app.execution_queue.is_empty() {
            if let Some(next_idx) = app.get_next_workflow_to_execute() {
                let tx_clone = tx.clone();
                let workflow_path = app.workflows[next_idx].path.clone();
                let runtime_type = app.runtime_type.clone();

                thread::spawn(move || {
                    let runtime = tokio::runtime::Runtime::new().unwrap();
                    let result = runtime.block_on(async {
                        match executor::execute_workflow(&workflow_path, runtime_type, verbose)
                            .await
                        {
                            Ok(result) => result,
                            Err(e) => {
                                // Create a failed execution result with error message
                                let failed_job = executor::JobResult {
                                    name: "Error".to_string(),
                                    status: JobStatus::Failure,
                                    steps: vec![executor::StepResult {
                                        name: "Execution Error".to_string(),
                                        status: StepStatus::Failure,
                                        output: format!("Error: {}", e),
                                    }],
                                    logs: format!("Error executing workflow: {}", e),
                                };

                                ExecutionResult {
                                    jobs: vec![failed_job],
                                }
                            }
                        }
                    });

                    tx_clone.send((next_idx, result)).unwrap();
                });
            }
        }

        // Handle key events
        if event::poll(Duration::from_millis(100))? {
            if let Event::Key(key) = event::read()? {
                match key.code {
                    KeyCode::Char('q') => break,
                    KeyCode::Esc => {
                        if app.detailed_view {
                            app.detailed_view = false;
                        } else if app.show_help {
                            app.show_help = false;
                        } else {
                            break;
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
                    KeyCode::Char('1') => app.switch_tab(0),
                    KeyCode::Char('2') => app.switch_tab(1),
                    KeyCode::Char('3') => app.switch_tab(2),
                    KeyCode::Char('4') | KeyCode::Char('?') | KeyCode::Char('h') => {
                        app.switch_tab(3)
                    }
                    KeyCode::Up | KeyCode::Char('k') => match app.selected_tab {
                        0 => app.previous_workflow(),
                        1 => {
                            if app.detailed_view {
                                app.previous_step();
                            } else {
                                app.previous_job();
                            }
                        }
                        2 => app.scroll_logs_up(),
                        _ => {}
                    },
                    KeyCode::Down | KeyCode::Char('j') => match app.selected_tab {
                        0 => app.next_workflow(),
                        1 => {
                            if app.detailed_view {
                                app.next_step();
                            } else {
                                app.next_job();
                            }
                        }
                        2 => app.scroll_logs_down(),
                        _ => {}
                    },
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
                        if !app.running {
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
                    KeyCode::Char('n') => {
                        if !app.running {
                            // Deselect all workflows
                            for workflow in &mut app.workflows {
                                workflow.selected = false;
                            }
                        }
                    }
                    _ => {}
                }
            }
        }
    }

    // Clean up terminal
    disable_raw_mode()?;
    execute!(
        terminal.backend_mut(),
        LeaveAlternateScreen,
        DisableMouseCapture
    )?;
    terminal.show_cursor()?;

    Ok(())
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
    // Create tabs
    let titles = vec!["Workflows", "Execution", "Logs", "Help"];
    let tabs = Tabs::new(
        titles
            .iter()
            .map(|t| {
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
    let items: Vec<ListItem> = app
        .workflows
        .iter()
        .map(|w| {
            let checked = if w.selected { "✓ " } else { "  " };
            let status_indicator = match w.status {
                WorkflowStatus::NotStarted => "  ",
                WorkflowStatus::Running => "⟳ ",
                WorkflowStatus::Success => "✅ ",
                WorkflowStatus::Failed => "❌ ",
                WorkflowStatus::Skipped => "⏭  ",
            };

            let status_style = match w.status {
                WorkflowStatus::NotStarted => Style::default(),
                WorkflowStatus::Running => Style::default().fg(Color::Cyan),
                WorkflowStatus::Success => Style::default().fg(Color::Green),
                WorkflowStatus::Failed => Style::default().fg(Color::Red),
                WorkflowStatus::Skipped => Style::default().fg(Color::Gray),
            };

            ListItem::new(Line::from(vec![
                Span::styled(checked, Style::default().fg(Color::Green)),
                Span::styled(status_indicator, status_style),
                Span::raw(&w.name),
            ]))
        })
        .collect();

    let workflows_list = List::new(items)
        .block(
            Block::default()
                .borders(Borders::ALL)
                .border_type(BorderType::Rounded)
                .title(Span::styled(
                    " Available Workflows ",
                    Style::default().fg(Color::Yellow),
                )),
        )
        .highlight_style(
            Style::default()
                .bg(Color::DarkGray)
                .add_modifier(Modifier::BOLD),
        )
        .highlight_symbol("» ");

    f.render_stateful_widget(workflows_list, area, &mut app.workflow_list_state);
}

fn render_execution_tab(f: &mut Frame<CrosstermBackend<io::Stdout>>, app: &mut App, area: Rect) {
    if app.detailed_view {
        render_job_detail_view(f, app, area);
        return;
    }
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
                    Constraint::Min(5),    // Jobs list
                    Constraint::Length(6), // Execution info
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

        // Add progress bar for running workflows
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
        } else {
            // No execution details
            let workflow_info_widget = Paragraph::new(workflow_info).block(
                Block::default()
                    .borders(Borders::ALL)
                    .border_type(BorderType::Rounded)
                    .title(Span::styled(
                        " Workflow Information ",
                        Style::default().fg(Color::Yellow),
                    )),
            );
            f.render_widget(workflow_info_widget, chunks[0]);
        }

        // Jobs list section
        if let Some(execution) = &workflow.execution_details {
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
                                s.status == StepStatus::Success || s.status == StepStatus::Failure
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
        } else {
            // No execution details yet
            let placeholder = Paragraph::new("Execution has not started yet...")
                .block(
                    Block::default()
                        .borders(Borders::ALL)
                        .border_type(BorderType::Rounded)
                        .title(Span::styled(" Jobs ", Style::default().fg(Color::Yellow))),
                )
                .alignment(Alignment::Center);

            f.render_widget(placeholder, chunks[1]);

            let info_placeholder = Paragraph::new("Waiting for execution to start...")
                .block(
                    Block::default()
                        .borders(Borders::ALL)
                        .border_type(BorderType::Rounded)
                        .title(Span::styled(
                            " Execution Information ",
                            Style::default().fg(Color::Yellow),
                        )),
                )
                .alignment(Alignment::Center);

            f.render_widget(info_placeholder, chunks[2]);
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

            f.render_stateful_widget(steps_table, chunks[1], &mut app.step_table_state);

            // Step output - show output from the selected step
            let output_text = if !job.steps.is_empty() {
                let step_idx = app
                    .step_list_state
                    .selected()
                    .unwrap_or(0)
                    .min(job.steps.len() - 1);
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
    // Combine application logs with system logs
    let mut all_logs = app.logs.clone();
    all_logs.extend(crate::logging::get_logs());

    // Create visible log lines, with timestamps and color coding
    let log_items: Vec<ListItem> = all_logs
        .iter()
        .map(|log_line| {
            // Try to parse log line to extract type (info, error, etc)
            let style = if log_line.contains("Error")
                || log_line.contains("error")
                || log_line.contains("❌")
            {
                Style::default().fg(Color::Red)
            } else if log_line.contains("Warning")
                || log_line.contains("warning")
                || log_line.contains("⚠️")
            {
                Style::default().fg(Color::Yellow)
            } else if log_line.contains("Success")
                || log_line.contains("success")
                || log_line.contains("✅")
            {
                Style::default().fg(Color::Green)
            } else if log_line.contains("Running")
                || log_line.contains("running")
                || log_line.contains("⟳")
            {
                Style::default().fg(Color::Cyan)
            } else {
                Style::default().fg(Color::Gray)
            };

            ListItem::new(Line::from(Span::styled(log_line, style)))
        })
        .collect();

    let logs_list = List::new(log_items)
        .block(
            Block::default()
                .borders(Borders::ALL)
                .border_type(BorderType::Rounded)
                .title(Span::styled(
                    " Execution Logs ",
                    Style::default().fg(Color::Yellow),
                )),
        )
        .start_corner(ratatui::layout::Corner::BottomLeft); // Show most recent logs at the bottom

    f.render_widget(logs_list, area);
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
            Span::raw(" - Navigate lists"),
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
                "n",
                Style::default()
                    .fg(Color::Yellow)
                    .add_modifier(Modifier::BOLD),
            ),
            Span::raw(" - Deselect all workflows"),
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
                "q",
                Style::default()
                    .fg(Color::Yellow)
                    .add_modifier(Modifier::BOLD),
            ),
            Span::raw(" - Quit application"),
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
    let runtime_mode = match app.runtime_type {
        RuntimeType::Docker => "Docker",
        RuntimeType::Emulation => "Emulation",
    };

    let runtime_style = match app.runtime_type {
        RuntimeType::Docker => Style::default()
            .fg(Color::Blue)
            .add_modifier(Modifier::BOLD),
        RuntimeType::Emulation => Style::default()
            .fg(Color::Magenta)
            .add_modifier(Modifier::BOLD),
    };

    // Left side of status bar
    let left_text = Line::from(vec![
        Span::raw("Runtime: "),
        Span::styled(runtime_mode, runtime_style),
        Span::raw(" | "),
        Span::styled(
            format!("{} workflow(s) loaded", app.workflows.len()),
            Style::default().fg(Color::White),
        ),
    ]);

    // Right side of status bar
    let right_text = Line::from(vec![
        Span::styled(
            "q",
            Style::default()
                .fg(Color::Yellow)
                .add_modifier(Modifier::BOLD),
        ),
        Span::raw(": Quit | "),
        Span::styled(
            "?",
            Style::default()
                .fg(Color::Yellow)
                .add_modifier(Modifier::BOLD),
        ),
        Span::raw(": Help"),
    ]);

    // Create a layout with two parts for left and right aligned text
    let status_chunks = Layout::default()
        .direction(Direction::Horizontal)
        .constraints([Constraint::Percentage(50), Constraint::Percentage(50)])
        .split(area);

    let left_status = Paragraph::new(left_text).alignment(Alignment::Left);

    let right_status = Paragraph::new(right_text).alignment(Alignment::Right);

    f.render_widget(left_status, status_chunks[0]);
    f.render_widget(right_status, status_chunks[1]);
}

// Validate a workflow or directory containing workflows
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

    println!("Executing workflow: {}", path.display());
    println!("Runtime mode: {:?}", runtime_type);

    match executor::execute_workflow(path, runtime_type, verbose).await {
        Ok(result) => {
            println!("\nWorkflow execution results:");

            for job in &result.jobs {
                match job.status {
                    JobStatus::Success => {
                        println!("\n✅ Job succeeded: {}", job.name);
                    }
                    JobStatus::Failure => {
                        println!("\n❌ Job failed: {}", job.name);
                    }
                    JobStatus::Skipped => {
                        println!("\n⏭️ Job skipped: {}", job.name);
                    }
                }

                println!("-------------------------");

                for step in job.steps.iter() {
                    match step.status {
                        StepStatus::Success => {
                            println!("  ✅ {}", step.name);

                            if !step.output.trim().is_empty() && step.output.lines().count() <= 3 {
                                // For short outputs, show directly
                                println!("    {}", step.output.trim());
                            }
                        }
                        StepStatus::Failure => {
                            println!("  ❌ {}", step.name);

                            // For failures, always show output (truncated)
                            let output = if step.output.len() > 500 {
                                format!("{}... (truncated)", &step.output[..500])
                            } else {
                                step.output.clone()
                            };

                            println!("    {}", output.trim().replace('\n', "\n    "));
                        }
                        StepStatus::Skipped => {
                            println!("  ⏭️ {} (skipped)", step.name);
                        }
                    }
                }
            }

            // Determine overall success
            let failures = result
                .jobs
                .iter()
                .filter(|job| job.status == JobStatus::Failure)
                .count();

            if failures > 0 {
                println!("\n❌ Workflow completed with failures");
                return Err(io::Error::new(
                    io::ErrorKind::Other,
                    "Workflow execution failed",
                ));
            } else {
                println!("\n✅ Workflow completed successfully!");
                Ok(())
            }
        }
        Err(e) => {
            println!("❌ Failed to execute workflow: {}", e);
            Err(io::Error::new(
                io::ErrorKind::Other,
                format!("Workflow execution error: {}", e),
            ))
        }
    }
}

// Main entry point for the TUI interface
pub async fn run_wrkflw_tui(path: Option<&PathBuf>, runtime_type: RuntimeType, verbose: bool) {
    match run_tui(path, &runtime_type, verbose) {
        Ok(_) => {}
        Err(e) => {
            // If the TUI fails to initialize or crashes, fall back to CLI mode
            eprintln!("Failed to start UI: {}", e);

            if let Some(path) = path {
                if path.is_file() {
                    println!("Falling back to CLI mode...");
                    if let Err(e) = execute_workflow_cli(path, runtime_type, verbose).await {
                        eprintln!("Error: {}", e);
                        process::exit(1);
                    }
                } else {
                    validate_workflow(path, verbose).unwrap_or_else(|e| {
                        eprintln!("Error: {}", e);
                        process::exit(1);
                    });
                }
            }
        }
    }
}
