// Job detail view rendering
use crate::app::App;
use crate::theme;
use ratatui::{
    layout::{Constraint, Direction, Layout, Rect},
    style::{Modifier, Style},
    text::{Line, Span},
    widgets::{Cell, Paragraph, Row, Table, Wrap},
    Frame,
};

pub fn render_job_detail_view(f: &mut Frame<'_>, app: &mut App, area: Rect) {
    let t = &app.theme;
    let current_workflow_idx = app
        .current_execution
        .or_else(|| app.workflow_list_state.selected())
        .filter(|&idx| idx < app.workflows.len());

    let Some(workflow_idx) = current_workflow_idx else {
        return;
    };
    let Some(execution) = &app.workflows[workflow_idx].execution_details else {
        return;
    };
    let Some(job_idx) = app.job_list_state.selected() else {
        return;
    };
    if job_idx >= execution.jobs.len() {
        return;
    }

    let job = &execution.jobs[job_idx];
    let workflow_name = &app.workflows[workflow_idx].name;

    let chunks = Layout::default()
        .direction(Direction::Vertical)
        .constraints(
            [
                Constraint::Length(2), // inline breadcrumb + status
                Constraint::Min(5),    // Steps table
                Constraint::Length(8), // Step details
            ]
            .as_ref(),
        )
        .margin(1)
        .split(area);

    // Breadcrumb: workflow › job   [status]  N steps
    let (status_symbol, status_style) = theme::job_status(t, &job.status);
    let status_text = match job.status {
        wrkflw_executor::JobStatus::Success => "Success",
        wrkflw_executor::JobStatus::Failure => "Failed",
        wrkflw_executor::JobStatus::Skipped => "Skipped",
    };
    let breadcrumb = Paragraph::new(vec![
        Line::from(vec![
            Span::styled(workflow_name, theme::muted_style(t)),
            Span::styled(
                format!(" {} ", theme::symbols::ARROW),
                theme::muted_style(t),
            ),
            Span::styled(
                &job.name,
                Style::default()
                    .fg(t.fg_bright)
                    .add_modifier(Modifier::BOLD),
            ),
            Span::raw("   "),
            Span::styled(status_symbol, status_style),
            Span::raw(" "),
            Span::styled(status_text, status_style),
            Span::styled(
                format!("   {} steps", job.steps.len()),
                theme::muted_style(t),
            ),
        ]),
        Line::from(""),
    ]);
    f.render_widget(breadcrumb, chunks[0]);

    // Steps table — keep its block, it provides real separation from Step Output.
    let header_cells = ["Status", "Step Name"]
        .iter()
        .map(|h| Cell::from(*h).style(theme::header_style(t)));
    let header = Row::new(header_cells).height(1);

    let rows: Vec<Row> = job
        .steps
        .iter()
        .map(|step| {
            let (status_symbol, status_style) = theme::step_status(t, &step.status);
            Row::new(vec![
                Cell::from(status_symbol).style(status_style),
                Cell::from(step.name.clone()).style(Style::default().fg(t.fg_bright)),
            ])
        })
        .collect();

    let widths = [Constraint::Length(4), Constraint::Percentage(92)];
    let steps_table = Table::new(rows, widths)
        .header(header)
        .block(theme::block(t, "Steps"))
        .row_highlight_style(theme::selected_style(t))
        .highlight_symbol("\u{258c} ");

    f.render_stateful_widget(steps_table, chunks[1], &mut app.step_table_state);

    // Step output
    if let Some(step_idx) = app.step_table_state.selected() {
        if step_idx < job.steps.len() {
            let step = &job.steps[step_idx];
            let (step_symbol, step_style) = theme::step_status(t, &step.status);
            let status_text = match step.status {
                wrkflw_executor::StepStatus::Success => "Success",
                wrkflw_executor::StepStatus::Failure => "Failed",
                wrkflw_executor::StepStatus::Skipped => "Skipped",
            };

            let mut output_text = step.output.clone();
            if output_text.len() > 5000 {
                output_text = format!("{}\u{2026} [truncated]", &output_text[..5000]);
            }

            let step_detail = Paragraph::new(vec![
                Line::from(vec![
                    Span::styled(step_symbol, step_style),
                    Span::raw(" "),
                    Span::styled(
                        step.name.clone(),
                        Style::default()
                            .fg(t.fg_bright)
                            .add_modifier(Modifier::BOLD),
                    ),
                    Span::styled(format!(" ({})", status_text), step_style),
                ]),
                Line::from(""),
                Line::from(Span::styled(
                    output_text,
                    Style::default().fg(t.fg_dim),
                )),
            ])
            .block(theme::block(t, "Step Output"))
            .wrap(Wrap { trim: false });

            f.render_widget(step_detail, chunks[2]);
        }
    }
}
