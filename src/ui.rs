use crate::evaluator::evaluate_workflow_file;
use crate::executor::{self, ExecutionResult, JobStatus, RuntimeType, StepStatus};
use crate::utils::is_workflow_file;
use chrono::Local;
use crossterm::{
    event::{self, DisableMouseCapture, EnableMouseCapture, Event, KeyCode},
    execute,
    terminal::{disable_raw_mode, enable_raw_mode, EnterAlternateScreen, LeaveAlternateScreen},
};
use std::io::{self, stdout};
use std::path::{Path, PathBuf};
use std::process;
use std::sync::mpsc;
use std::thread;
use std::time::Duration;
use tui::{
    backend::CrosstermBackend,
    layout::{Constraint, Direction, Layout, Rect},
    style::{Color, Modifier, Style},
    text::{Span, Spans},
    widgets::{Block, Borders, List, ListItem, Paragraph, Tabs, Wrap},
    Terminal,
};

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
struct WorkflowExecution {
    jobs: Vec<JobExecution>,
    start_time: chrono::DateTime<Local>,
    end_time: Option<chrono::DateTime<Local>>,
    logs: Vec<String>,
}

// Job execution details
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
    selected_workflow_index: usize,
    selected_tab: usize,
    running: bool,
    show_help: bool,
    runtime_type: RuntimeType,
    execution_queue: Vec<usize>, // Indices of workflows to execute
    current_execution: Option<usize>,
    logs: Vec<String>,         // Overall execution logs
    selected_job_index: usize, // For viewing job details
    detailed_view: bool,       // Whether we're in detailed view mode
}

impl App {
    fn new(runtime_type: RuntimeType) -> App {
        App {
            workflows: Vec::new(),
            selected_workflow_index: 0,
            selected_tab: 0,
            running: false,
            show_help: false,
            runtime_type,
            execution_queue: Vec::new(),
            current_execution: None,
            logs: Vec::new(),
            selected_job_index: 0,
            detailed_view: false,
        }
    }

    // Toggle workflow selection
    fn toggle_selected(&mut self) {
        if !self.workflows.is_empty() {
            let idx = self.selected_workflow_index;
            self.workflows[idx].selected = !self.workflows[idx].selected;
        }
    }

    // Move cursor up in the list
    fn previous(&mut self) {
        if !self.workflows.is_empty() {
            self.selected_workflow_index = self.selected_workflow_index.saturating_sub(1);
        }
    }

    // Move cursor down in the list
    fn next(&mut self) {
        if !self.workflows.is_empty() {
            let len = self.workflows.len();
            self.selected_workflow_index = (self.selected_workflow_index + 1) % len;
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
    }

    // Start workflow execution process
    fn start_execution(&mut self) {
        if self.execution_queue.is_empty() {
            self.logs
                .push("No workflows selected for execution".to_string());
            return;
        }

        self.running = true;
        self.logs.push("Starting workflow execution...".to_string());

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
        } else {
            workflow.status = WorkflowStatus::Success;
        }

        // Update workflow execution details
        if let Some(exec) = &mut workflow.execution_details {
            exec.jobs = job_executions;
            exec.end_time = Some(end_time);
        } else {
            workflow.execution_details = Some(WorkflowExecution {
                jobs: job_executions,
                start_time: end_time - chrono::Duration::seconds(1), // Approximate
                end_time: Some(end_time),
                logs: vec!["Execution completed".to_string()],
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

        // Initialize execution details
        self.workflows[next].execution_details = Some(WorkflowExecution {
            jobs: Vec::new(),
            start_time: Local::now(),
            end_time: None,
            logs: vec!["Execution started".to_string()],
        });

        Some(next)
    }

    // Toggle detailed view mode
    fn toggle_detailed_view(&mut self) {
        self.detailed_view = !self.detailed_view;
    }

    // Select previous job in detailed view
    fn previous_job(&mut self) {
        if self.detailed_view {
            if let Some(idx) = self.current_execution {
                if let Some(exec) = &self.workflows[idx].execution_details {
                    if !exec.jobs.is_empty() {
                        self.selected_job_index = self.selected_job_index.saturating_sub(1);
                    }
                }
            }
        }
    }

    // Select next job in detailed view
    fn next_job(&mut self) {
        if self.detailed_view {
            if let Some(idx) = self.current_execution {
                if let Some(exec) = &self.workflows[idx].execution_details {
                    let len = exec.jobs.len();
                    if len > 0 {
                        self.selected_job_index = (self.selected_job_index + 1) % len;
                    }
                }
            }
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
        // Draw UI
        terminal.draw(|f| {
            let size = f.size();
            let chunks = Layout::default()
                .direction(Direction::Vertical)
                .margin(1)
                .constraints(
                    [
                        Constraint::Length(3), // Tabs
                        Constraint::Min(10),   // Main content
                        Constraint::Length(1), // Status line
                    ]
                    .as_ref(),
                )
                .split(size);

            // Tab bar
            let titles = vec!["Workflows", "Execution", "Logs", "Help"];
            let tabs = Tabs::new(
                titles
                    .iter()
                    .map(|t| Spans::from(Span::styled(*t, Style::default().fg(Color::White))))
                    .collect(),
            )
            .block(Block::default().borders(Borders::ALL).title("wrkflw"))
            .highlight_style(
                Style::default()
                    .fg(Color::Yellow)
                    .add_modifier(Modifier::BOLD),
            )
            .select(app.selected_tab);

            f.render_widget(tabs, chunks[0]);

            // Main content based on selected tab
            match app.selected_tab {
                0 => render_workflows_tab(f, &app, chunks[1]),
                1 => render_execution_tab(f, &app, chunks[1]),
                2 => render_logs_tab(f, &app, chunks[1]),
                3 => render_help_tab(f, chunks[1]),
                _ => {}
            }

            // Status line
            let runtime_mode = match app.runtime_type {
                RuntimeType::Docker => "Docker",
                RuntimeType::Emulation => "Emulation",
            };

            let status = format!(
                "Runtime: {} | Press q to quit | {} workflow(s) loaded",
                runtime_mode,
                app.workflows.len()
            );

            let status_widget = Paragraph::new(status).style(Style::default().fg(Color::White));

            f.render_widget(status_widget, chunks[2]);
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
                    KeyCode::Char('4') => app.switch_tab(3),
                    KeyCode::Up | KeyCode::Char('k') => {
                        if app.detailed_view {
                            app.previous_job();
                        } else {
                            app.previous();
                        }
                    }
                    KeyCode::Down | KeyCode::Char('j') => {
                        if app.detailed_view {
                            app.next_job();
                        } else {
                            app.next();
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
                                    app.toggle_selected();
                                    app.queue_selected_for_execution();
                                    app.start_execution();
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

// Render the workflow list tab
fn render_workflows_tab(f: &mut tui::Frame<CrosstermBackend<io::Stdout>>, app: &App, area: Rect) {
    let items: Vec<ListItem> = app
        .workflows
        .iter()
        .enumerate()
        .map(|(i, w)| {
            let checked = if w.selected { "[✓] " } else { "[ ] " };
            let status_indicator = match w.status {
                WorkflowStatus::NotStarted => "  ",
                WorkflowStatus::Running => "⟳ ",
                WorkflowStatus::Success => "✓ ",
                WorkflowStatus::Failed => "✗ ",
                WorkflowStatus::Skipped => "- ",
            };

            let mut style = Style::default();
            if i == app.selected_workflow_index {
                style = style.fg(Color::Yellow).add_modifier(Modifier::BOLD);
            }

            let status_style = match w.status {
                WorkflowStatus::NotStarted => Style::default(),
                WorkflowStatus::Running => Style::default().fg(Color::Cyan),
                WorkflowStatus::Success => Style::default().fg(Color::Green),
                WorkflowStatus::Failed => Style::default().fg(Color::Red),
                WorkflowStatus::Skipped => Style::default().fg(Color::Gray),
            };

            ListItem::new(Spans::from(vec![
                Span::styled(checked, style),
                Span::styled(status_indicator, status_style),
                Span::styled(w.name.clone(), style),
            ]))
        })
        .collect();

    let workflows_list = List::new(items)
        .block(
            Block::default()
                .borders(Borders::ALL)
                .title("Available Workflows"),
        )
        .highlight_style(
            Style::default()
                .fg(Color::Yellow)
                .add_modifier(Modifier::BOLD),
        );

    f.render_widget(workflows_list, area);
}
// Render the execution tab
fn render_execution_tab(f: &mut tui::Frame<CrosstermBackend<io::Stdout>>, app: &App, area: Rect) {
    if app.detailed_view {
        render_job_detail_view(f, app, area);
        return;
    }

    // Determine which workflow to display - either the one currently running or the last one executed
    let current_workflow_idx = app.current_execution.or_else(|| {
        app.workflows
            .iter()
            .position(|w| matches!(w.status, WorkflowStatus::Success | WorkflowStatus::Failed))
    });

    if let Some(idx) = current_workflow_idx {
        let workflow = &app.workflows[idx];

        // Split the area into sections
        let chunks = Layout::default()
            .direction(Direction::Vertical)
            .constraints(
                [
                    Constraint::Length(3), // Workflow info
                    Constraint::Min(5),    // Jobs list
                    Constraint::Length(5), // Execution info
                ]
                .as_ref(),
            )
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
            WorkflowStatus::NotStarted => Style::default(),
            WorkflowStatus::Running => Style::default().fg(Color::Cyan),
            WorkflowStatus::Success => Style::default().fg(Color::Green),
            WorkflowStatus::Failed => Style::default().fg(Color::Red),
            WorkflowStatus::Skipped => Style::default().fg(Color::Gray),
        };

        let workflow_info = Paragraph::new(vec![
            Spans::from(vec![
                Span::raw("Workflow: "),
                Span::styled(
                    workflow.name.clone(),
                    Style::default().add_modifier(Modifier::BOLD),
                ),
            ]),
            Spans::from(vec![
                Span::raw("Status: "),
                Span::styled(status_text, status_style),
            ]),
        ])
        .block(
            Block::default()
                .borders(Borders::ALL)
                .title("Workflow Information"),
        );

        f.render_widget(workflow_info, chunks[0]);

        // Jobs list section
        if let Some(execution) = &workflow.execution_details {
            let job_items: Vec<ListItem> = execution
                .jobs
                .iter()
                .map(|job| {
                    let status_indicator = match job.status {
                        JobStatus::Success => "✓ ",
                        JobStatus::Failure => "✗ ",
                        JobStatus::Skipped => "- ",
                    };

                    let status_style = match job.status {
                        JobStatus::Success => Style::default().fg(Color::Green),
                        JobStatus::Failure => Style::default().fg(Color::Red),
                        JobStatus::Skipped => Style::default().fg(Color::Gray),
                    };

                    ListItem::new(Spans::from(vec![
                        Span::styled(status_indicator, status_style),
                        Span::raw(job.name.clone()),
                    ]))
                })
                .collect();

            let jobs_list =
                List::new(job_items).block(Block::default().borders(Borders::ALL).title("Jobs"));

            f.render_widget(jobs_list, chunks[1]);

            // Execution info section
            let mut execution_info = Vec::new();

            execution_info.push(Spans::from(vec![
                Span::raw("Started: "),
                Span::styled(
                    execution.start_time.format("%Y-%m-%d %H:%M:%S").to_string(),
                    Style::default(),
                ),
            ]));

            if let Some(end_time) = execution.end_time {
                execution_info.push(Spans::from(vec![
                    Span::raw("Finished: "),
                    Span::styled(
                        end_time.format("%Y-%m-%d %H:%M:%S").to_string(),
                        Style::default(),
                    ),
                ]));

                // Calculate duration
                let duration = end_time.signed_duration_since(execution.start_time);
                execution_info.push(Spans::from(vec![
                    Span::raw("Duration: "),
                    Span::styled(
                        format!(
                            "{}m {}s",
                            duration.num_minutes(),
                            duration.num_seconds() % 60
                        ),
                        Style::default(),
                    ),
                ]));
            }

            let info_widget = Paragraph::new(execution_info).block(
                Block::default()
                    .borders(Borders::ALL)
                    .title("Execution Information"),
            );

            f.render_widget(info_widget, chunks[2]);
        } else {
            // No execution details yet
            let placeholder = Paragraph::new("Execution has not started yet...")
                .block(Block::default().borders(Borders::ALL).title("Jobs"));

            f.render_widget(placeholder, chunks[1]);

            let info_placeholder = Paragraph::new("Waiting for execution to start...").block(
                Block::default()
                    .borders(Borders::ALL)
                    .title("Execution Information"),
            );

            f.render_widget(info_placeholder, chunks[2]);
        }
    } else {
        // No workflow execution to display
        let placeholder = Paragraph::new("No workflow execution data available.\n\nSelect workflows in the Workflows tab and press 'r' to run them.")
            .block(Block::default().borders(Borders::ALL).title("Execution"))
            .wrap(Wrap { trim: true });

        f.render_widget(placeholder, area);
    }
}

// Render detailed job view
fn render_job_detail_view(f: &mut tui::Frame<CrosstermBackend<io::Stdout>>, app: &App, area: Rect) {
    // Get the current workflow and job
    if let Some(workflow_idx) = app.current_execution.or_else(|| {
        app.workflows
            .iter()
            .position(|w| matches!(w.status, WorkflowStatus::Success | WorkflowStatus::Failed))
    }) {
        let workflow = &app.workflows[workflow_idx];

        if let Some(execution) = &workflow.execution_details {
            if execution.jobs.is_empty() {
                let placeholder = Paragraph::new("This job has no steps or execution data.")
                    .block(Block::default().borders(Borders::ALL).title("Job Details"))
                    .wrap(Wrap { trim: true });

                f.render_widget(placeholder, area);
                return;
            }

            // Ensure job index is valid
            let job_idx = app.selected_job_index.min(execution.jobs.len() - 1);
            let job = &execution.jobs[job_idx];

            // Split area
            let chunks = Layout::default()
                .direction(Direction::Vertical)
                .constraints(
                    [
                        Constraint::Length(3), // Job info
                        Constraint::Min(10),   // Steps list
                        Constraint::Length(8), // Step output
                    ]
                    .as_ref(),
                )
                .split(area);

            // Job info
            let status_style = match job.status {
                JobStatus::Success => Style::default().fg(Color::Green),
                JobStatus::Failure => Style::default().fg(Color::Red),
                JobStatus::Skipped => Style::default().fg(Color::Gray),
            };

            let job_info = Paragraph::new(vec![
                Spans::from(vec![
                    Span::raw("Job: "),
                    Span::styled(
                        job.name.clone(),
                        Style::default().add_modifier(Modifier::BOLD),
                    ),
                ]),
                Spans::from(vec![
                    Span::raw("Status: "),
                    Span::styled(format!("{:?}", job.status), status_style),
                ]),
            ])
            .block(Block::default().borders(Borders::ALL).title(format!(
                "Job Details ({}/{})",
                job_idx + 1,
                execution.jobs.len()
            )));

            f.render_widget(job_info, chunks[0]);

            // Steps list
            let step_items: Vec<ListItem> = job
                .steps
                .iter()
                .map(|step| {
                    let status_indicator = match step.status {
                        StepStatus::Success => "✓ ",
                        StepStatus::Failure => "✗ ",
                        StepStatus::Skipped => "- ",
                    };

                    let status_style = match step.status {
                        StepStatus::Success => Style::default().fg(Color::Green),
                        StepStatus::Failure => Style::default().fg(Color::Red),
                        StepStatus::Skipped => Style::default().fg(Color::Gray),
                    };

                    ListItem::new(Spans::from(vec![
                        Span::styled(status_indicator, status_style),
                        Span::raw(step.name.clone()),
                    ]))
                })
                .collect();

            let steps_list =
                List::new(step_items).block(Block::default().borders(Borders::ALL).title("Steps"));

            f.render_widget(steps_list, chunks[1]);

            // Step output - show output from the selected step
            let output_text = if !job.steps.is_empty() {
                // For simplicity, we'll show output from the first failed step if any, or the first step otherwise
                let step = job
                    .steps
                    .iter()
                    .find(|s| s.status == StepStatus::Failure)
                    .unwrap_or(&job.steps[0]);

                let mut output = step.output.clone();
                if output.is_empty() {
                    output = "No output for this step.".to_string();
                }

                // Limit output to prevent performance issues
                if output.len() > 2000 {
                    output.truncate(2000);
                    output.push_str("\n... (output truncated) ...");
                }

                format!("Step output ({}): \n{}", step.name, output)
            } else {
                "No steps to display output for.".to_string()
            };

            let output_widget = Paragraph::new(output_text)
                .block(Block::default().borders(Borders::ALL).title("Output"))
                .wrap(Wrap { trim: true });

            f.render_widget(output_widget, chunks[2]);
        } else {
            // No execution details
            let placeholder = Paragraph::new("No job execution details available.")
                .block(Block::default().borders(Borders::ALL).title("Job Details"))
                .wrap(Wrap { trim: true });

            f.render_widget(placeholder, area);
        }
    } else {
        // No workflow selected
        let placeholder = Paragraph::new("No workflow execution available to display details.")
            .block(Block::default().borders(Borders::ALL).title("Job Details"))
            .wrap(Wrap { trim: true });

        f.render_widget(placeholder, area);
    }
}

// Render the logs tab
fn render_logs_tab(f: &mut tui::Frame<CrosstermBackend<io::Stdout>>, app: &App, area: Rect) {
    // Combine application logs with system logs
    let mut all_logs = app.logs.clone();
    all_logs.extend(crate::logging::get_logs());

    // Sort logs by timestamp (if we added timestamps to them)
    // This might require additional parsing

    let logs = all_logs.join("\n");
    let logs_widget = Paragraph::new(logs)
        .block(
            Block::default()
                .borders(Borders::ALL)
                .title("Execution Logs"),
        )
        .wrap(Wrap { trim: true });

    f.render_widget(logs_widget, area);
}

// Render the help tab
fn render_help_tab(f: &mut tui::Frame<CrosstermBackend<io::Stdout>>, area: Rect) {
    let help_text = vec![
        Spans::from(Span::styled(
            "Keyboard Controls",
            Style::default().add_modifier(Modifier::BOLD),
        )),
        Spans::from(""),
        Spans::from(vec![
            Span::styled("Tab", Style::default().add_modifier(Modifier::BOLD)),
            Span::raw(" - Switch between tabs"),
        ]),
        Spans::from(vec![
            Span::styled("1-4", Style::default().add_modifier(Modifier::BOLD)),
            Span::raw(" - Switch directly to tab"),
        ]),
        Spans::from(vec![
            Span::styled(
                "Up/Down or j/k",
                Style::default().add_modifier(Modifier::BOLD),
            ),
            Span::raw(" - Navigate lists"),
        ]),
        Spans::from(vec![
            Span::styled("Space", Style::default().add_modifier(Modifier::BOLD)),
            Span::raw(" - Toggle workflow selection"),
        ]),
        Spans::from(vec![
            Span::styled("Enter", Style::default().add_modifier(Modifier::BOLD)),
            Span::raw(" - Run selected workflow / View job details"),
        ]),
        Spans::from(vec![
            Span::styled("r", Style::default().add_modifier(Modifier::BOLD)),
            Span::raw(" - Run all selected workflows"),
        ]),
        Spans::from(vec![
            Span::styled("a", Style::default().add_modifier(Modifier::BOLD)),
            Span::raw(" - Select all workflows"),
        ]),
        Spans::from(vec![
            Span::styled("n", Style::default().add_modifier(Modifier::BOLD)),
            Span::raw(" - Deselect all workflows"),
        ]),
        Spans::from(vec![
            Span::styled("Esc", Style::default().add_modifier(Modifier::BOLD)),
            Span::raw(" - Back / Exit detailed view"),
        ]),
        Spans::from(vec![
            Span::styled("q", Style::default().add_modifier(Modifier::BOLD)),
            Span::raw(" - Quit application"),
        ]),
        Spans::from(""),
        Spans::from(Span::styled(
            "Runtime Modes",
            Style::default().add_modifier(Modifier::BOLD),
        )),
        Spans::from(""),
        Spans::from(vec![
            Span::styled("Docker", Style::default().fg(Color::Cyan)),
            Span::raw(" - Uses Docker to run workflows (default)"),
        ]),
        Spans::from(vec![
            Span::styled("Emulation", Style::default().fg(Color::Yellow)),
            Span::raw(" - Emulates GitHub Actions environment locally (no Docker required)"),
        ]),
    ];

    let help_widget = Paragraph::new(help_text)
        .block(Block::default().borders(Borders::ALL).title("Help"))
        .wrap(Wrap { trim: true });

    f.render_widget(help_widget, area);
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

// Execute a workflow in CLI mode (not TUI)
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
                        println!("\n⚠️ Job skipped: {}", job.name);
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
                            println!("  ⚠️ {} (skipped)", step.name);
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
