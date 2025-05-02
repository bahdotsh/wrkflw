// Job detail view rendering
use crate::app::App;
use ratatui::{
    backend::CrosstermBackend,
    layout::{Constraint, Direction, Layout, Rect},
    style::{Color, Modifier, Style},
    text::{Line, Span},
    widgets::{Block, BorderType, Borders, Paragraph, Row, Table},
    Frame,
};
use std::io;

// Render the job detail view
pub fn render_job_detail_view(
    f: &mut Frame<CrosstermBackend<io::Stdout>>,
    app: &mut App,
    area: Rect,
) {
    // Get the workflow index either from current_execution or selected workflow
    let current_workflow_idx = app
        .current_execution
        .or_else(|| app.workflow_list_state.selected())
        .filter(|&idx| idx < app.workflows.len());

    if let Some(workflow_idx) = current_workflow_idx {
        // Only proceed if we have execution details
        if let Some(execution) = &app.workflows[workflow_idx].execution_details {
            // Only proceed if we have a valid job selection
            if let Some(job_idx) = app.job_list_state.selected() {
                if job_idx < execution.jobs.len() {
                    let job = &execution.jobs[job_idx];

                    // Split the area into sections
                    let chunks = Layout::default()
                        .direction(Direction::Vertical)
                        .constraints(
                            [
                                Constraint::Length(3), // Job title
                                Constraint::Min(5),    // Steps table
                                Constraint::Length(8), // Step details
                            ]
                            .as_ref(),
                        )
                        .margin(1)
                        .split(area);

                    // Job title section
                    let status_text = match job.status {
                        executor::JobStatus::Success => "Success",
                        executor::JobStatus::Failure => "Failed",
                        executor::JobStatus::Skipped => "Skipped",
                    };

                    let status_style = match job.status {
                        executor::JobStatus::Success => Style::default().fg(Color::Green),
                        executor::JobStatus::Failure => Style::default().fg(Color::Red),
                        executor::JobStatus::Skipped => Style::default().fg(Color::Yellow),
                    };

                    let job_title = Paragraph::new(vec![
                        Line::from(vec![
                            Span::styled("Job: ", Style::default().fg(Color::Blue)),
                            Span::styled(
                                job.name.clone(),
                                Style::default()
                                    .fg(Color::White)
                                    .add_modifier(Modifier::BOLD),
                            ),
                            Span::raw(" ("),
                            Span::styled(status_text, status_style),
                            Span::raw(")"),
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
                                " Job Details ",
                                Style::default().fg(Color::Yellow),
                            )),
                    );

                    f.render_widget(job_title, chunks[0]);

                    // Steps section
                    let header_cells = ["Status", "Step Name"].iter().map(|h| {
                        ratatui::widgets::Cell::from(*h).style(Style::default().fg(Color::Yellow))
                    });

                    let header = Row::new(header_cells)
                        .style(Style::default().add_modifier(Modifier::BOLD))
                        .height(1);

                    let rows = job.steps.iter().map(|step| {
                        let status_symbol = match step.status {
                            executor::StepStatus::Success => "✅",
                            executor::StepStatus::Failure => "❌",
                            executor::StepStatus::Skipped => "⏭",
                        };

                        let status_style = match step.status {
                            executor::StepStatus::Success => Style::default().fg(Color::Green),
                            executor::StepStatus::Failure => Style::default().fg(Color::Red),
                            executor::StepStatus::Skipped => Style::default().fg(Color::Gray),
                        };

                        Row::new(vec![
                            ratatui::widgets::Cell::from(status_symbol).style(status_style),
                            ratatui::widgets::Cell::from(step.name.clone()),
                        ])
                    });

                    let steps_table = Table::new(rows)
                        .header(header)
                        .block(
                            Block::default()
                                .borders(Borders::ALL)
                                .border_type(BorderType::Rounded)
                                .title(Span::styled(" Steps ", Style::default().fg(Color::Yellow))),
                        )
                        .highlight_style(
                            Style::default()
                                .bg(Color::DarkGray)
                                .add_modifier(Modifier::BOLD),
                        )
                        .highlight_symbol("» ")
                        .widths(&[
                            Constraint::Length(8),      // Status icon column
                            Constraint::Percentage(92), // Name column
                        ]);

                    // We need to use the table state from the app
                    f.render_stateful_widget(steps_table, chunks[1], &mut app.step_table_state);

                    // Step detail section
                    if let Some(step_idx) = app.step_table_state.selected() {
                        if step_idx < job.steps.len() {
                            let step = &job.steps[step_idx];

                            // Show step output with proper styling
                            let status_text = match step.status {
                                executor::StepStatus::Success => "Success",
                                executor::StepStatus::Failure => "Failed",
                                executor::StepStatus::Skipped => "Skipped",
                            };

                            let status_style = match step.status {
                                executor::StepStatus::Success => Style::default().fg(Color::Green),
                                executor::StepStatus::Failure => Style::default().fg(Color::Red),
                                executor::StepStatus::Skipped => Style::default().fg(Color::Yellow),
                            };

                            let mut output_text = step.output.clone();
                            // Truncate if too long
                            if output_text.len() > 1000 {
                                output_text = format!("{}... [truncated]", &output_text[..1000]);
                            }

                            let step_detail = Paragraph::new(vec![
                                Line::from(vec![
                                    Span::styled("Step: ", Style::default().fg(Color::Blue)),
                                    Span::styled(
                                        step.name.clone(),
                                        Style::default()
                                            .fg(Color::White)
                                            .add_modifier(Modifier::BOLD),
                                    ),
                                    Span::raw(" ("),
                                    Span::styled(status_text, status_style),
                                    Span::raw(")"),
                                ]),
                                Line::from(""),
                                Line::from(output_text),
                            ])
                            .block(
                                Block::default()
                                    .borders(Borders::ALL)
                                    .border_type(BorderType::Rounded)
                                    .title(Span::styled(
                                        " Step Output ",
                                        Style::default().fg(Color::Yellow),
                                    )),
                            )
                            .wrap(ratatui::widgets::Wrap { trim: false });

                            f.render_widget(step_detail, chunks[2]);
                        }
                    }
                }
            }
        }
    }
}
