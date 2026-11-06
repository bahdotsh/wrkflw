// Run → Jobs view: Jobs · Steps + Output. The run's summary line lives
// in the Run header (`views::run_header`), shared with the graph and log
// views.
//
// The jobs list drops out first as the terminal narrows, so the steps
// and their output always get a readable share of the screen.

use crate::app::App;
use crate::models::{JobExecution, JobState, Workflow, WorkflowStatus};
use crate::text;
use crate::theme::{self, BadgeKind, COLORS};
use ratatui::{
    layout::{Alignment, Constraint, Direction, Layout, Rect},
    style::{Modifier, Style},
    text::{Line, Span},
    widgets::Paragraph,
    Frame,
};
use wrkflw_executor::{JobStatus, StepStatus};

pub fn render_execution_tab(f: &mut Frame<'_>, app: &mut App, area: Rect) {
    let workflow_idx = app.shown_workflow();

    let Some(idx) = workflow_idx else {
        render_empty_state(f, area);
        return;
    };
    let workflow = &app.workflows[idx];

    let jobs_w = match area.width {
        w if w >= 120 => 34,
        w if w >= 72 => 28,
        _ => 0,
    };
    let main = Layout::default()
        .direction(Direction::Horizontal)
        .constraints([Constraint::Length(jobs_w), Constraint::Min(0)])
        .split(area);

    let selected = selected_job(app, workflow);
    if jobs_w > 0 {
        render_jobs_pane(f, app, workflow, main[0]);
    }

    let centre = Layout::default()
        .direction(Direction::Vertical)
        .constraints([Constraint::Percentage(45), Constraint::Min(0)])
        .split(main[1]);
    render_steps_pane(f, workflow, selected, centre[0]);
    // The Output pane follows the cursor's row even before that job has a
    // result, so a running job's output can be pinned among parallel ones.
    let output_job = app
        .job_list_state
        .selected()
        .and_then(|row| workflow.job_row_name(row))
        .or(selected.map(|j| j.name.as_str()));
    render_output_pane(f, app, workflow, output_job, centre[1]);
}

/// The reported job the Jobs pane cursor is on. With the cursor on a job
/// not reported yet there is none; with no usable cursor, the most
/// relevant reported job.
fn selected_job<'a>(app: &App, workflow: &'a Workflow) -> Option<&'a JobExecution> {
    let exec = workflow.execution_details.as_ref()?;
    match app.job_list_state.selected() {
        Some(row) if row < exec.jobs.len() => exec.jobs.get(row),
        Some(row) if row < workflow.job_row_count() => None,
        _ => active_job_execution(workflow),
    }
}

// ─── Summary strip ────────────────────────────────────────────────
/// One line: status badge, workflow name, and on the right when it
/// fits, the start time, duration and job counts.
pub(super) fn render_summary_line(f: &mut Frame<'_>, app: &App, workflow: &Workflow, area: Rect) {
    let (badge, kind) = match workflow.status {
        WorkflowStatus::NotStarted => ("IDLE", BadgeKind::Dim),
        WorkflowStatus::Running => ("RUNNING", BadgeKind::Info),
        WorkflowStatus::Success => ("PASSED", BadgeKind::Success),
        WorkflowStatus::Failed => ("FAILED", BadgeKind::Error),
        WorkflowStatus::Skipped => ("SKIPPED", BadgeKind::Warning),
    };
    let inner = area;
    let badge = if app.validation_mode && badge == "IDLE" {
        "VALIDATE"
    } else {
        badge
    };

    let mut right: Vec<Span> = Vec::new();
    if let Some(exec) = workflow.execution_details.as_ref() {
        let end = exec.end_time.unwrap_or_else(chrono::Local::now);
        let secs = end
            .signed_duration_since(exec.start_time)
            .num_seconds()
            .max(0);
        right.push(Span::styled("started ", theme::muted_style()));
        right.push(Span::styled(
            exec.start_time.format("%H:%M:%S").to_string(),
            Style::default().fg(COLORS.text_dim),
        ));
        right.push(Span::styled(
            if exec.end_time.is_some() {
                " · took "
            } else {
                " · running "
            },
            theme::muted_style(),
        ));
        right.push(Span::styled(
            format_duration(secs),
            Style::default().fg(COLORS.text_dim),
        ));
        let counts = job_counts(workflow);
        for (n, glyph, color) in [
            (counts.passed, theme::symbols::SUCCESS, COLORS.success),
            (counts.failed, theme::symbols::FAILURE, COLORS.error),
            (counts.skipped, theme::symbols::SKIPPED, COLORS.text_muted),
        ] {
            if n > 0 {
                right.push(Span::raw("  "));
                right.push(Span::styled(
                    format!("{glyph} {n}"),
                    Style::default().fg(color),
                ));
            }
        }
    }

    let (sym, sym_style) = theme::workflow_status_animated(&workflow.status, app.spinner_frame);
    let right_w = text::spans_width(&right);
    let name_room = (inner.width as usize).saturating_sub(right_w + badge.len() + 6);
    let left = vec![
        theme::badge_solid(badge, kind),
        Span::raw(" "),
        Span::styled(sym.to_string(), sym_style),
        Span::raw(" "),
        Span::styled(
            text::truncate(&text::sanitize(&workflow.name), name_room).into_owned(),
            Style::default()
                .fg(COLORS.text)
                .add_modifier(Modifier::BOLD),
        ),
    ];

    let shown_right = if text::spans_width(&left) + right_w < inner.width as usize {
        right
    } else {
        Vec::new()
    };
    let chunks = Layout::default()
        .direction(Direction::Horizontal)
        .constraints([
            Constraint::Min(0),
            Constraint::Length(text::spans_width(&shown_right) as u16),
        ])
        .split(inner);
    f.render_widget(Paragraph::new(Line::from(left)), chunks[0]);
    f.render_widget(
        Paragraph::new(Line::from(shown_right)).alignment(Alignment::Right),
        chunks[1],
    );
}

/// A step or job duration: milliseconds under a second, tenths under a
/// minute, then the coarser `format_duration`.
pub(crate) fn format_ms(ms: u64) -> String {
    match ms {
        ms if ms < 1000 => format!("{ms}ms"),
        ms if ms < 60_000 => format!("{:.1}s", ms as f64 / 1000.0),
        ms => format_duration((ms / 1000) as i64),
    }
}

pub(crate) fn format_duration(secs: i64) -> String {
    match secs {
        s if s < 60 => format!("{s}s"),
        s if s < 3600 => format!("{}m {:02}s", s / 60, s % 60),
        s => format!("{}h {:02}m", s / 3600, (s % 3600) / 60),
    }
}

struct Counts {
    passed: usize,
    failed: usize,
    skipped: usize,
}

fn job_counts(workflow: &Workflow) -> Counts {
    let jobs = workflow
        .execution_details
        .as_ref()
        .map(|e| &e.jobs[..])
        .unwrap_or(&[]);
    Counts {
        passed: jobs
            .iter()
            .filter(|j| j.status == JobStatus::Success)
            .count(),
        failed: jobs
            .iter()
            .filter(|j| j.status == JobStatus::Failure)
            .count(),
        skipped: jobs
            .iter()
            .filter(|j| j.status == JobStatus::Skipped)
            .count(),
    }
}

// ─── Jobs pane ────────────────────────────────────────────────────
fn render_jobs_pane(f: &mut Frame<'_>, app: &App, workflow: &Workflow, area: Rect) {
    let jobs = workflow
        .execution_details
        .as_ref()
        .map(|e| &e.jobs[..])
        .unwrap_or(&[]);
    let pending = workflow.unreported_jobs();
    let total = jobs.len() + pending.len();
    let block = theme::with_badge(theme::block_focused("Jobs"), total.to_string());
    let inner = block.inner(area);
    f.render_widget(block, area);

    if total == 0 {
        theme::empty_state(f, inner, "No jobs", vec![]);
        return;
    }

    let w = inner.width as usize;
    let selected = app.job_list_state.selected().filter(|&i| i < total);
    let mut lines: Vec<Line> = Vec::with_capacity(total);
    for (i, job) in jobs.iter().enumerate() {
        // A matrix leg shows its own status, not the merged one.
        let state = match job.status {
            JobStatus::Success => JobState::Success,
            JobStatus::Skipped => JobState::Skipped,
            JobStatus::Failure if job.failure_allowed => JobState::FailureAllowed,
            JobStatus::Failure => JobState::Failure,
        };
        let (sym, sym_style) = theme::job_state(state, app.spinner_frame);
        let is_sel = selected == Some(i);
        let row = if is_sel {
            theme::selected_style()
        } else {
            Style::default()
        };
        lines.push(Line::from(vec![
            Span::styled(
                if is_sel { theme::SELECTED_MARK } else { "  " },
                Style::default().fg(theme::current_accent()).patch(row),
            ),
            Span::styled(sym.to_string(), sym_style.patch(row)),
            Span::styled(" ", row),
            Span::styled(
                text::fit(&text::sanitize(&job.name), w.saturating_sub(4)),
                Style::default().fg(COLORS.text).patch(row),
            ),
        ]));
    }
    for (i, name) in pending.into_iter().enumerate() {
        let (sym, sym_style) = theme::job_state(workflow.job_state(name), app.spinner_frame);
        let is_sel = selected == Some(jobs.len() + i);
        let row = if is_sel {
            theme::selected_style()
        } else {
            Style::default()
        };
        lines.push(Line::from(vec![
            Span::styled(
                if is_sel { theme::SELECTED_MARK } else { "  " },
                Style::default().fg(theme::current_accent()).patch(row),
            ),
            Span::styled(sym.to_string(), sym_style.patch(row)),
            Span::styled(" ", row),
            Span::styled(
                text::truncate(&text::sanitize(name), w.saturating_sub(4)).into_owned(),
                theme::muted_style().patch(row),
            ),
        ]));
    }

    let viewport = inner.height as usize;
    let anchor = selected.unwrap_or(0);
    let offset = anchor.saturating_sub(viewport.saturating_sub(1));
    f.render_widget(Paragraph::new(lines).scroll((offset as u16, 0)), inner);
    theme::scrollbar(f, area, total, offset, viewport);
}

// ─── Steps pane ───────────────────────────────────────────────────
fn render_steps_pane(
    f: &mut Frame<'_>,
    workflow: &Workflow,
    job: Option<&JobExecution>,
    area: Rect,
) {
    let title = match job {
        Some(j) => format!("Steps · {}", text::sanitize(&j.name)),
        None => "Steps".to_string(),
    };
    let title = text::truncate(&title, (area.width as usize).saturating_sub(10)).into_owned();
    let block = theme::block(&title);
    let inner = block.inner(area);
    f.render_widget(block, area);

    let Some(job) = job else {
        let msg = if workflow.status == WorkflowStatus::Running {
            "Steps appear here when the job finishes"
        } else {
            "Run the workflow to see its steps"
        };
        theme::empty_state(f, inner, msg, vec![]);
        return;
    };
    if job.steps.is_empty() {
        theme::empty_state(f, inner, "This job reported no steps", vec![]);
        return;
    }

    let w = inner.width as usize;
    let lines: Vec<Line> = job
        .steps
        .iter()
        .enumerate()
        .map(|(i, step)| step_line(i, step, w))
        .collect();
    let first_failure = job
        .steps
        .iter()
        .position(|s| s.status == StepStatus::Failure)
        .unwrap_or(0);
    let viewport = inner.height as usize;
    let offset = first_failure.saturating_sub(viewport.saturating_sub(1));
    let total = lines.len();
    f.render_widget(Paragraph::new(lines).scroll((offset as u16, 0)), inner);
    theme::scrollbar(f, area, total, offset, viewport);
}

/// `01 ✔ name ……… exit 1` with the exit code right-aligned.
pub(crate) fn step_line(i: usize, step: &crate::models::StepExecution, w: usize) -> Line<'static> {
    let (sym, sym_style) = theme::step_status(&step.status);
    let name_style = match step.status {
        StepStatus::Skipped => theme::muted_style(),
        StepStatus::Failure => Style::default().fg(COLORS.error),
        StepStatus::Success => Style::default().fg(COLORS.text),
    };
    let code = match step.exit_code {
        Some(c) if c != 0 => format!(" exit {c}"),
        _ => String::new(),
    };
    let name_w = w.saturating_sub(5 + code.len());
    Line::from(vec![
        Span::styled(format!("{:02} ", i + 1), theme::muted_style()),
        Span::styled(sym.to_string(), sym_style),
        Span::raw(" "),
        Span::styled(text::fit(&text::sanitize(&step.name), name_w), name_style),
        Span::styled(code, Style::default().fg(COLORS.error)),
    ])
}

// ─── Output pane ──────────────────────────────────────────────────
fn render_output_pane(
    f: &mut Frame<'_>,
    app: &App,
    workflow: &Workflow,
    selected: Option<&str>,
    area: Rect,
) {
    // While steps run, show what they print: the selected job's running
    // step, else whichever step started or printed last. Once the run ends
    // the pane goes back to the run's log.
    if let Some((job, tail)) = workflow
        .execution_details
        .as_ref()
        .filter(|exec| exec.end_time.is_none())
        .and_then(|exec| exec.live_tail_for(selected))
    {
        let title = format!(
            "Output · {} › {}{}",
            text::sanitize(job),
            text::sanitize(&tail.step),
            if tail.finished { " · finished" } else { "" }
        );
        let block = theme::block(&title);
        let inner = block.inner(area);
        f.render_widget(block, area);
        let rows = inner.height as usize;
        let start = tail.lines.len().saturating_sub(rows);
        let lines: Vec<Line> = tail
            .lines
            .iter()
            .skip(start)
            .map(|line| text::ansi_line(line, Style::default().fg(COLORS.text_dim)))
            .collect();
        f.render_widget(Paragraph::new(lines), inner);
        return;
    }

    let block = theme::block("Output");
    let inner = block.inner(area);
    f.render_widget(block, area);

    if app.processed_logs.is_empty() {
        theme::empty_state(
            f,
            inner,
            "Output streams here while a workflow runs",
            vec![],
        );
        return;
    }

    // One row per entry, newest at the bottom. Wrapping would push the
    // newest lines out of view, which is the opposite of what a live
    // tail is for; the Logs tab has the full text.
    let rows = inner.height as usize;
    let w = inner.width as usize;
    let start = app.processed_logs.len().saturating_sub(rows);
    let lines: Vec<Line> = app.processed_logs[start..]
        .iter()
        .map(|entry| {
            let mut spans: Vec<Span> = vec![
                Span::styled(entry.timestamp.clone(), theme::muted_style()),
                Span::raw(" "),
            ];
            let mut used = text::width(&entry.timestamp) + 1;
            for span in &entry.content_spans {
                let room = w.saturating_sub(used);
                if room == 0 {
                    break;
                }
                let content = text::sanitize(&span.content);
                let cut = text::truncate(&content, room).into_owned();
                used += text::width(&cut);
                let style = if entry.log_type == "ERROR" {
                    span.style.fg(COLORS.error)
                } else if entry.log_type == "WARN" {
                    span.style.fg(COLORS.warning)
                } else {
                    span.style
                };
                spans.push(Span::styled(cut, style));
            }
            Line::from(spans)
        })
        .collect();
    f.render_widget(Paragraph::new(lines), inner);
}

// ─── Helpers ──────────────────────────────────────────────────────
fn active_job_execution(workflow: &Workflow) -> Option<&JobExecution> {
    let exec = workflow.execution_details.as_ref()?;
    // Prefer the failed job so the panes open where the problem is.
    exec.jobs
        .iter()
        .rfind(|j| !j.failure_allowed && matches!(j.status, JobStatus::Failure))
        .or_else(|| exec.jobs.last())
}

fn render_empty_state(f: &mut Frame<'_>, area: Rect) {
    let block = theme::block("Run");
    let inner = block.inner(area);
    f.render_widget(block, area);
    let key = |k: &'static str| {
        Span::styled(
            k,
            Style::default()
                .fg(theme::current_accent())
                .add_modifier(Modifier::BOLD),
        )
    };
    theme::empty_state(
        f,
        inner,
        "Nothing has run yet",
        vec![
            Span::styled("Pick a workflow on ", theme::muted_style()),
            key("1"),
            Span::styled(" and press ", theme::muted_style()),
            key("Enter"),
            Span::styled(" to run it", theme::muted_style()),
        ],
    );
}

#[cfg(test)]
mod tests {
    use super::format_duration;

    #[test]
    fn durations_read_like_a_human_wrote_them() {
        assert_eq!(format_duration(7), "7s");
        assert_eq!(format_duration(65), "1m 05s");
        assert_eq!(format_duration(3725), "1h 02m");
    }
}
