// Execution tab rendering
use crate::app::App;
use crate::models::WorkflowStatus;
use ratatui::{
    backend::CrosstermBackend,
    layout::{Alignment, Constraint, Direction, Layout, Rect},
    style::{Color, Modifier, Style},
    text::{Line, Span},
    widgets::{Block, BorderType, Borders, Gauge, List, ListItem, Paragraph},
    Frame,
};
use std::io;

// Render the execution tab
pub fn render_execution_tab(
    f: &mut Frame<CrosstermBackend<io::Stdout>>,
    app: &mut App,
    area: Rect,
) {
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

            // Jobs list section
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
                            executor::JobStatus::Success => "✅",
                            executor::JobStatus::Failure => "❌",
                            executor::JobStatus::Skipped => "⏭",
                        };

                        let status_style = match job.status {
                            executor::JobStatus::Success => Style::default().fg(Color::Green),
                            executor::JobStatus::Failure => Style::default().fg(Color::Red),
                            executor::JobStatus::Skipped => Style::default().fg(Color::Gray),
                        };

                        // Count completed and total steps
                        let total_steps = job.steps.len();
                        let completed_steps = job
                            .steps
                            .iter()
                            .filter(|s| {
                                s.status == executor::StepStatus::Success
                                    || s.status == executor::StepStatus::Failure
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
                let current_time = chrono::Local::now();
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
            // No workflow execution to display
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

            // No execution details to display
            let placeholder = Paragraph::new(vec![
                Line::from(""),
                Line::from(vec![Span::styled(
                    "No execution data available.",
                    Style::default()
                        .fg(Color::Yellow)
                        .add_modifier(Modifier::BOLD),
                )]),
                Line::from(""),
                Line::from("Press 'Enter' to run this workflow."),
                Line::from(""),
            ])
            .block(
                Block::default()
                    .borders(Borders::ALL)
                    .border_type(BorderType::Rounded)
                    .title(Span::styled(" Jobs ", Style::default().fg(Color::Yellow))),
            )
            .alignment(Alignment::Center);

            f.render_widget(placeholder, chunks[1]);

            // Execution information
            let info_widget = Paragraph::new(vec![
                Line::from(""),
                Line::from(vec![Span::styled(
                    "No execution has been started.",
                    Style::default().fg(Color::Yellow),
                )]),
                Line::from(""),
                Line::from("Press 'Enter' in the Workflows tab to run,"),
                Line::from("or 't' to trigger on GitHub."),
            ])
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
}
