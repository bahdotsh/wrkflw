// Job detail view rendering
use crate::app::App;
use crate::theme::{self, COLORS};
use ratatui::{
    layout::{Constraint, Direction, Layout, Rect},
    style::{Modifier, Style},
    text::{Line, Span},
    widgets::{Paragraph, Row, Table, Wrap},
    Frame,
};

// Render the job detail view
pub fn render_job_detail_view(f: &mut Frame<'_>, app: &mut App, area: Rect) {
    let current_workflow_idx = app
        .current_execution
        .or_else(|| app.workflow_list_state.selected())
        .filter(|&idx| idx < app.workflows.len());

    if let Some(workflow_idx) = current_workflow_idx {
        if let Some(execution) = &app.workflows[workflow_idx].execution_details {
            if let Some(job_idx) = app.job_list_state.selected() {
                if job_idx < execution.jobs.len() {
                    let job = &execution.jobs[job_idx];
                    let workflow_name = &app.workflows[workflow_idx].name;

                    let chunks = Layout::default()
                        .direction(Direction::Vertical)
                        .constraints(
                            [
                                Constraint::Length(4), // Job title + breadcrumb
                                Constraint::Min(5),    // Steps table
                                Constraint::Length(8), // Step details
                            ]
                            .as_ref(),
                        )
                        .margin(1)
                        .split(area);

                    // Job title with breadcrumb
                    let (status_symbol, status_style) = theme::job_status(&job.status);
                    let status_text = match job.status {
                        wrkflw_executor::JobStatus::Success => "Success",
                        wrkflw_executor::JobStatus::Failure => "Failed",
                        wrkflw_executor::JobStatus::Skipped => "Skipped",
                    };

                    let job_title = Paragraph::new(vec![
                        // Breadcrumb
                        Line::from(vec![
                            Span::styled(workflow_name, theme::muted_style()),
                            Span::styled(
                                format!(" {} ", theme::symbols::ARROW),
                                theme::muted_style(),
                            ),
                            Span::styled(
                                &job.name,
                                Style::default()
                                    .fg(COLORS.text)
                                    .add_modifier(Modifier::BOLD),
                            ),
                        ]),
                        Line::from(vec![
                            Span::styled(status_symbol, status_style),
                            Span::raw(" "),
                            Span::styled(status_text, status_style),
                            Span::styled(
                                format!("  {} steps", job.steps.len()),
                                theme::muted_style(),
                            ),
                        ]),
                    ])
                    .block(theme::block("Job Details"));

                    f.render_widget(job_title, chunks[0]);

                    // Steps section
                    let header_cells = ["Status", "Step Name"]
                        .iter()
                        .map(|h| ratatui::widgets::Cell::from(*h).style(theme::header_style()));

                    let header = Row::new(header_cells).height(1);

                    let rows = job.steps.iter().map(|step| {
                        let (status_symbol, status_style) = theme::step_status(&step.status);

                        Row::new(vec![
                            ratatui::widgets::Cell::from(status_symbol).style(status_style),
                            ratatui::widgets::Cell::from(step.name.clone())
                                .style(Style::default().fg(COLORS.text)),
                        ])
                    });

                    let widths = [
                        Constraint::Length(4),      // Status icon column
                        Constraint::Percentage(92), // Name column
                    ];
                    let steps_table = Table::new(rows, widths)
                        .header(header)
                        .block(theme::block("Steps"))
                        .highlight_style(theme::selected_style())
                        .highlight_symbol(theme::symbols::SELECTED);

                    f.render_stateful_widget(steps_table, chunks[1], &mut app.step_table_state);

                    // Step detail section
                    if let Some(step_idx) = app.step_table_state.selected() {
                        if step_idx < job.steps.len() {
                            let step = &job.steps[step_idx];

                            let (step_symbol, step_style) = theme::step_status(&step.status);
                            let status_text = match step.status {
                                wrkflw_executor::StepStatus::Success => "Success",
                                wrkflw_executor::StepStatus::Failure => "Failed",
                                wrkflw_executor::StepStatus::Skipped => "Skipped",
                            };

                            let mut output_text = step.output.clone();
                            if output_text.len() > 5000 {
                                output_text =
                                    format!("{}\u{2026} [truncated]", &output_text[..5000]);
                            }

                            let step_detail = Paragraph::new(vec![
                                Line::from(vec![
                                    Span::styled(step_symbol, step_style),
                                    Span::raw(" "),
                                    Span::styled(
                                        step.name.clone(),
                                        Style::default()
                                            .fg(COLORS.text)
                                            .add_modifier(Modifier::BOLD),
                                    ),
                                    Span::styled(format!(" ({})", status_text), step_style),
                                ]),
                                Line::from(""),
                                Line::from(Span::styled(
                                    output_text,
                                    Style::default().fg(COLORS.text_dim),
                                )),
                            ])
                            .block(theme::block("Step Output"))
                            .wrap(Wrap { trim: false });

                            f.render_widget(step_detail, chunks[2]);
                        }
                    }
                }
            }
        }
    }
}
