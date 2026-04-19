// Job detail view rendering.
//
// This is the GitHub-Actions-style live view: a steps table on top where the
// running step shows an animated spinner, and a streamed Step Output pane
// below that tails `step.log_buffer` while the step is in flight. Step
// duration is shown inline once timing is known.

use crate::app::App;
use crate::models::LogLine;
use crate::theme;
use chrono::{DateTime, Local};
use ratatui::{
    layout::{Constraint, Direction, Layout, Rect},
    style::{Modifier, Style},
    text::{Line, Span},
    widgets::{Cell, Paragraph, Row, Table, Wrap},
    Frame,
};
use wrkflw_executor::events::LogStream;

pub fn render_job_detail_view(f: &mut Frame<'_>, app: &mut App, area: Rect) {
    let t = &app.theme;
    let spinner_frame = app.spinner_frame;
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
                Constraint::Min(6),    // Streamed step output
            ]
            .as_ref(),
        )
        .margin(1)
        .split(area);

    // Breadcrumb: workflow › job   [status]  N steps   (follow: on/off)
    let (status_symbol, status_style) = theme::job_status(t, &job.status);
    let status_text = match job.status {
        wrkflw_executor::JobStatus::Success => "Success",
        wrkflw_executor::JobStatus::Failure => "Failed",
        wrkflw_executor::JobStatus::Skipped => "Skipped",
    };
    let follow_indicator = if app.auto_follow_step {
        "follow: on"
    } else {
        "follow: off"
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
                format!("   {} steps   ", job.steps.len()),
                theme::muted_style(t),
            ),
            Span::styled(follow_indicator, theme::muted_style(t)),
        ]),
        Line::from(""),
    ]);
    f.render_widget(breadcrumb, chunks[0]);

    // Steps table: Status glyph / name / duration. Running step gets an
    // animated spinner glyph via `step_status_animated`.
    let header_cells = ["Status", "Step", "Duration"]
        .iter()
        .map(|h| Cell::from(*h).style(theme::header_style(t)));
    let header = Row::new(header_cells).height(1);

    let rows: Vec<Row> = job
        .steps
        .iter()
        .map(|step| {
            let (status_symbol, status_style) =
                theme::step_status_animated(t, &step.status, spinner_frame);
            let duration_text = format_step_duration(step.start_time, step.end_time);
            Row::new(vec![
                Cell::from(status_symbol).style(status_style),
                Cell::from(step.name.clone()).style(Style::default().fg(t.fg_bright)),
                Cell::from(duration_text).style(theme::muted_style(t)),
            ])
        })
        .collect();

    let widths = [
        Constraint::Length(4),
        Constraint::Percentage(75),
        Constraint::Length(10),
    ];
    let steps_table = Table::new(rows, widths)
        .header(header)
        .block(theme::block(t, "Steps"))
        .row_highlight_style(theme::selected_style(t))
        .highlight_symbol("\u{258c} ");

    f.render_stateful_widget(steps_table, chunks[1], &mut app.step_table_state);

    // Selected-step output pane: streams log_buffer; falls back to the
    // post-mortem `output` blob for legacy (non-event) paths.
    if let Some(step_idx) = app.step_table_state.selected() {
        if step_idx < job.steps.len() {
            let step = &job.steps[step_idx];
            let (step_symbol, step_style) =
                theme::step_status_animated(t, &step.status, spinner_frame);
            let status_text = match step.status {
                wrkflw_executor::StepStatus::Pending => "Pending",
                wrkflw_executor::StepStatus::Running => "Running",
                wrkflw_executor::StepStatus::Success => "Success",
                wrkflw_executor::StepStatus::Failure => "Failed",
                wrkflw_executor::StepStatus::Skipped => "Skipped",
                _ => "Unknown",
            };

            let mut body: Vec<Line> = Vec::new();
            body.push(Line::from(vec![
                Span::styled(step_symbol, step_style),
                Span::raw(" "),
                Span::styled(
                    step.name.clone(),
                    Style::default()
                        .fg(t.fg_bright)
                        .add_modifier(Modifier::BOLD),
                ),
                Span::styled(format!(" ({})", status_text), step_style),
                Span::raw("   "),
                Span::styled(
                    format_step_duration(step.start_time, step.end_time),
                    theme::muted_style(t),
                ),
            ]));
            body.push(Line::from(""));

            if !step.log_buffer.is_empty() {
                // Render the live-streamed buffer. Stderr lines are tinted
                // so the user can distinguish stderr vs stdout at a glance.
                for line in log_lines_to_render(&step.log_buffer, t) {
                    body.push(line);
                }
            } else if !step.output.is_empty() {
                // CLI-fallback path: show the post-mortem output blob.
                let mut output_text = step.output.clone();
                if output_text.len() > 5000 {
                    output_text = format!("{}\u{2026} [truncated]", &output_text[..5000]);
                }
                for line in output_text.lines() {
                    body.push(Line::from(Span::styled(
                        line.to_string(),
                        Style::default().fg(t.fg_dim),
                    )));
                }
            } else {
                body.push(Line::from(Span::styled(
                    "(no output yet)",
                    theme::muted_style(t),
                )));
            }

            // Pin the pane to the bottom so the most recently streamed lines
            // are always visible — effectively a tail -f behavior. Subtract
            // 2 rows for the block's top/bottom borders.
            let pane_inner_h = chunks[2].height.saturating_sub(2) as usize;
            let total_lines = body.len();
            let scroll_rows: u16 = total_lines
                .saturating_sub(pane_inner_h)
                .try_into()
                .unwrap_or(u16::MAX);

            let step_detail = Paragraph::new(body)
                .block(theme::block(t, "Step Output"))
                .wrap(Wrap { trim: false })
                .scroll((scroll_rows, 0));

            f.render_widget(step_detail, chunks[2]);
        }
    }
}

/// Format a step's elapsed duration. Returns an empty string for steps that
/// haven't started yet so the column stays visually clean for Pending rows.
fn format_step_duration(start: Option<DateTime<Local>>, end: Option<DateTime<Local>>) -> String {
    match (start, end) {
        (Some(s), Some(e)) => format_secs((e - s).num_milliseconds()),
        (Some(s), None) => format_secs((Local::now() - s).num_milliseconds()),
        _ => String::new(),
    }
}

fn format_secs(ms: i64) -> String {
    if ms < 0 {
        return String::new();
    }
    let total_s = ms / 1000;
    let m = total_s / 60;
    let s = total_s % 60;
    if m > 0 {
        format!("{m}m {s:02}s")
    } else {
        format!("{s}s")
    }
}

fn log_lines_to_render<'a>(buffer: &'a [LogLine], t: &'a crate::theme::Theme) -> Vec<Line<'a>> {
    // Cap at 10k chunks so a runaway step never blows the buffer. See C8.
    let start = buffer.len().saturating_sub(10_000);
    let tail = &buffer[start..];
    let truncated = buffer.len() - tail.len();

    let mut out = Vec::with_capacity(tail.len() + 1);
    if truncated > 0 {
        out.push(Line::from(Span::styled(
            format!("\u{2026} ({truncated} earlier lines truncated)"),
            theme::muted_style(t),
        )));
    }
    for chunk in tail {
        // A chunk may include multiple embedded lines (particularly from
        // Docker's bollard output where one frame can carry a multi-line
        // payload). Split so the paragraph widget line-breaks correctly.
        let style = match chunk.stream {
            LogStream::Stdout => Style::default().fg(t.fg_normal),
            LogStream::Stderr => Style::default().fg(t.warning),
        };
        for (i, l) in chunk.text.split('\n').enumerate() {
            // Drop the trailing blank from a `"...\n"` chunk.
            if i > 0 && l.is_empty() && chunk.text.ends_with('\n') {
                continue;
            }
            out.push(Line::from(Span::styled(l.to_string(), style)));
        }
    }
    out
}
