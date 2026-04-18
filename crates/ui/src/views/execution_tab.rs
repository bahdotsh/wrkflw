// Execution tab rendering
use crate::app::App;
use crate::models::WorkflowStatus;
use crate::theme;
use ratatui::{
    layout::{Alignment, Constraint, Direction, Layout, Rect},
    style::{Modifier, Style},
    text::{Line, Span},
    widgets::{Block, Gauge, List, ListItem, Paragraph},
    Frame,
};

// Render the execution tab
pub fn render_execution_tab(f: &mut Frame<'_>, app: &mut App, area: Rect) {
    let t = &app.theme;
    let current_workflow_idx = app
        .current_execution
        .or_else(|| app.workflow_list_state.selected())
        .filter(|&idx| idx < app.workflows.len());

    if let Some(idx) = current_workflow_idx {
        let workflow = &app.workflows[idx];

        let chunks = Layout::default()
            .direction(Direction::Vertical)
            .constraints(
                [
                    Constraint::Length(5), // Workflow info with progress bar
                    Constraint::Min(5),    // Jobs list
                    Constraint::Length(7), // Execution info
                ]
                .as_ref(),
            )
            .margin(1)
            .split(area);

        // Workflow info section
        let (status_text, status_style) = match workflow.status {
            WorkflowStatus::NotStarted => ("Not Started", Style::default().fg(t.fg_dim)),
            WorkflowStatus::Running => ("Running", Style::default().fg(t.info)),
            WorkflowStatus::Success => ("Success", Style::default().fg(t.success)),
            WorkflowStatus::Failed => ("Failed", Style::default().fg(t.error)),
            WorkflowStatus::Skipped => ("Skipped", Style::default().fg(t.warning)),
        };

        let mut workflow_info = vec![
            Line::from(vec![
                Span::styled("Workflow: ", theme::label_style(t)),
                Span::styled(
                    workflow.name.clone(),
                    Style::default()
                        .fg(t.fg_bright)
                        .add_modifier(Modifier::BOLD),
                ),
            ]),
            Line::from(vec![
                Span::styled("Status: ", theme::label_style(t)),
                Span::styled(status_text, status_style),
            ]),
        ];

        if let Some(execution) = &workflow.execution_details {
            let progress = execution.progress;

            let gauge_color = match workflow.status {
                WorkflowStatus::Running => t.info,
                WorkflowStatus::Success => t.success,
                WorkflowStatus::Failed => t.error,
                _ => t.fg_dim,
            };

            let progress_text = match workflow.status {
                WorkflowStatus::Running => format!("{:.0}%", progress * 100.0),
                WorkflowStatus::Success => "Completed".to_string(),
                WorkflowStatus::Failed => "Failed".to_string(),
                _ => "Not started".to_string(),
            };

            workflow_info.push(Line::from(""));
            workflow_info.push(Line::from(vec![
                Span::styled("Progress: ", theme::label_style(t)),
                Span::styled(progress_text, Style::default().fg(t.fg_dim)),
            ]));

            let gauge = Gauge::default()
                .block(Block::default())
                .gauge_style(Style::default().fg(gauge_color).bg(t.bg_dark))
                .percent((progress * 100.0) as u16);

            let workflow_info_widget =
                Paragraph::new(workflow_info).block(theme::block(t, "Workflow Information"));

            let gauge_area = Rect {
                x: chunks[0].x + 2,
                y: chunks[0].y + 4,
                width: chunks[0].width.saturating_sub(4),
                height: 1,
            };

            f.render_widget(workflow_info_widget, chunks[0]);
            f.render_widget(gauge, gauge_area);

            // Jobs list section
            if execution.jobs.is_empty() {
                let placeholder = Paragraph::new("No jobs have started execution yet...")
                    .block(theme::block(t, "Jobs"))
                    .alignment(Alignment::Center);
                f.render_widget(placeholder, chunks[1]);
            } else {
                let job_items: Vec<ListItem> = execution
                    .jobs
                    .iter()
                    .map(|job| {
                        let (status_symbol, status_style) = theme::job_status(t, &job.status);

                        let total_steps = job.steps.len();
                        let completed_steps = job
                            .steps
                            .iter()
                            .filter(|s| {
                                s.status == wrkflw_executor::StepStatus::Success
                                    || s.status == wrkflw_executor::StepStatus::Failure
                            })
                            .count();

                        let steps_info = format!("[{}/{}]", completed_steps, total_steps);

                        ListItem::new(Line::from(vec![
                            Span::styled(status_symbol, status_style),
                            Span::raw(" "),
                            Span::styled(
                                &job.name,
                                Style::default()
                                    .fg(t.fg_bright)
                                    .add_modifier(Modifier::BOLD),
                            ),
                            Span::raw(" "),
                            Span::styled(steps_info, theme::muted_style(t)),
                        ]))
                    })
                    .collect();

                let jobs_list = List::new(job_items)
                    .block(theme::block(t, "Jobs"))
                    .highlight_style(theme::selected_style(t))
                    .highlight_symbol(theme::symbols::SELECTED);

                f.render_stateful_widget(jobs_list, chunks[1], &mut app.job_list_state);
            }

            // Execution info section
            let mut execution_info = Vec::new();

            execution_info.push(Line::from(vec![
                Span::styled("Started: ", theme::label_style(t)),
                Span::styled(
                    execution.start_time.format("%Y-%m-%d %H:%M:%S").to_string(),
                    Style::default().fg(t.fg_bright),
                ),
            ]));

            if let Some(end_time) = execution.end_time {
                execution_info.push(Line::from(vec![
                    Span::styled("Finished: ", theme::label_style(t)),
                    Span::styled(
                        end_time.format("%Y-%m-%d %H:%M:%S").to_string(),
                        Style::default().fg(t.fg_bright),
                    ),
                ]));

                let duration = end_time.signed_duration_since(execution.start_time);
                execution_info.push(Line::from(vec![
                    Span::styled("Duration: ", theme::label_style(t)),
                    Span::styled(
                        format!(
                            "{}m {}s",
                            duration.num_minutes(),
                            duration.num_seconds() % 60
                        ),
                        Style::default().fg(t.fg_bright),
                    ),
                ]));
            } else {
                let current_time = chrono::Local::now();
                let running_time = current_time.signed_duration_since(execution.start_time);
                execution_info.push(Line::from(vec![
                    Span::styled("Running for: ", theme::label_style(t)),
                    Span::styled(
                        format!(
                            "{}m {}s",
                            running_time.num_minutes(),
                            running_time.num_seconds() % 60
                        ),
                        Style::default().fg(t.fg_bright),
                    ),
                ]));
            }

            execution_info.push(Line::from(""));
            execution_info.push(Line::from(vec![
                Span::styled("Press ", theme::muted_style(t)),
                Span::styled("Enter", theme::key_style(t)),
                Span::styled(" to view job details", theme::muted_style(t)),
            ]));

            let info_widget =
                Paragraph::new(execution_info).block(theme::block(t, "Execution Information"));

            f.render_widget(info_widget, chunks[2]);
        } else {
            // No execution details
            let workflow_info_widget =
                Paragraph::new(workflow_info).block(theme::block(t, "Workflow Information"));

            f.render_widget(workflow_info_widget, chunks[0]);

            let placeholder = Paragraph::new(vec![
                Line::from(""),
                Line::from(Span::styled(
                    "No execution data available.",
                    Style::default().fg(t.warning).add_modifier(Modifier::BOLD),
                )),
                Line::from(""),
                Line::from(vec![
                    Span::styled("Press ", theme::muted_style(t)),
                    Span::styled("Enter", theme::key_style(t)),
                    Span::styled(" to run this workflow.", theme::muted_style(t)),
                ]),
                Line::from(""),
            ])
            .block(theme::block(t, "Jobs"))
            .alignment(Alignment::Center);

            f.render_widget(placeholder, chunks[1]);

            let info_widget = Paragraph::new(vec![
                Line::from(""),
                Line::from(Span::styled(
                    "No execution has been started.",
                    Style::default().fg(t.warning),
                )),
                Line::from(""),
                Line::from(vec![
                    Span::styled("Press ", theme::muted_style(t)),
                    Span::styled("Enter", theme::key_style(t)),
                    Span::styled(" in the Workflows tab to run, or ", theme::muted_style(t)),
                    Span::styled("t", theme::key_style(t)),
                    Span::styled(" to trigger on GitHub.", theme::muted_style(t)),
                ]),
            ])
            .block(theme::block(t, "Execution Information"))
            .alignment(Alignment::Center);

            f.render_widget(info_widget, chunks[2]);
        }
    } else {
        // No workflow selected
        let placeholder = Paragraph::new(vec![
            Line::from(""),
            Line::from(Span::styled(
                "No workflow execution data available.",
                Style::default().fg(t.warning).add_modifier(Modifier::BOLD),
            )),
            Line::from(""),
            Line::from(vec![
                Span::styled("Select workflows in the ", theme::muted_style(t)),
                Span::styled("Workflows", Style::default().fg(t.accent)),
                Span::styled(" tab and press ", theme::muted_style(t)),
                Span::styled("r", theme::key_style(t)),
                Span::styled(" to run them.", theme::muted_style(t)),
            ]),
            Line::from(""),
            Line::from(vec![
                Span::styled("Or press ", theme::muted_style(t)),
                Span::styled("t", theme::key_style(t)),
                Span::styled(" to trigger a workflow on GitHub.", theme::muted_style(t)),
            ]),
        ])
        .block(theme::block(t, "Execution"))
        .alignment(Alignment::Center);

        f.render_widget(placeholder, area);
    }
}
