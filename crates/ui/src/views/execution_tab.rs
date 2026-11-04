// Live Run — summary strip over Jobs · Steps + Output · DAG + Results.
//
// Panes drop out from the right as the terminal narrows: the DAG and
// results summary first, then the jobs list, so the steps and their
// output always get a readable share of the screen.

use crate::app::App;
use crate::components::dag;
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
    let workflow_idx = app
        .current_execution
        .or_else(|| app.workflow_list_state.selected())
        .filter(|&idx| idx < app.workflows.len());

    let Some(idx) = workflow_idx else {
        render_empty_state(f, area);
        return;
    };
    let workflow = &app.workflows[idx];

    let outer = Layout::default()
        .direction(Direction::Vertical)
        .constraints([Constraint::Length(3), Constraint::Min(0)])
        .split(area);
    render_summary_strip(f, app, workflow, outer[0]);

    let (jobs_w, right_w) = match area.width {
        w if w >= 120 => (30, 38),
        w if w >= 72 => (28, 0),
        _ => (0, 0),
    };
    let main = Layout::default()
        .direction(Direction::Horizontal)
        .constraints([
            Constraint::Length(jobs_w),
            Constraint::Min(0),
            Constraint::Length(right_w),
        ])
        .split(outer[1]);

    let selected = selected_job(app, workflow);
    if jobs_w > 0 {
        render_jobs_pane(f, app, workflow, main[0]);
    }

    let centre = Layout::default()
        .direction(Direction::Vertical)
        .constraints([Constraint::Percentage(45), Constraint::Min(0)])
        .split(main[1]);
    render_steps_pane(f, workflow, selected, centre[0]);
    render_output_pane(f, app, centre[1]);

    if right_w > 0 {
        let right = Layout::default()
            .direction(Direction::Vertical)
            .constraints([Constraint::Min(0), Constraint::Length(8)])
            .split(main[2]);
        render_dag_pane(f, app, workflow, right[0]);
        render_results_pane(f, workflow, right[1]);
    }
}

/// The job the Jobs pane cursor is on, or the most relevant one when
/// the cursor hasn't moved onto a reported job.
fn selected_job<'a>(app: &App, workflow: &'a Workflow) -> Option<&'a JobExecution> {
    let exec = workflow.execution_details.as_ref()?;
    app.job_list_state
        .selected()
        .and_then(|i| exec.jobs.get(i))
        .or_else(|| active_job_execution(workflow))
}

// ─── Summary strip ────────────────────────────────────────────────
fn render_summary_strip(f: &mut Frame<'_>, app: &App, workflow: &Workflow, area: Rect) {
    let (badge, kind) = match workflow.status {
        WorkflowStatus::NotStarted => ("IDLE", BadgeKind::Dim),
        WorkflowStatus::Running => ("RUNNING", BadgeKind::Info),
        WorkflowStatus::Success => ("PASSED", BadgeKind::Success),
        WorkflowStatus::Failed => ("FAILED", BadgeKind::Error),
        WorkflowStatus::Skipped => ("SKIPPED", BadgeKind::Warning),
    };
    let block = theme::block(if app.validation_mode {
        "Validation"
    } else {
        "Run"
    });
    let inner = block.inner(area);
    f.render_widget(block, area);

    let mut right: Vec<Span> = Vec::new();
    if let Some(exec) = workflow.execution_details.as_ref() {
        let end = exec.end_time.unwrap_or_else(chrono::Local::now);
        let secs = end
            .signed_duration_since(exec.start_time)
            .num_seconds()
            .max(0);
        right.push(Span::styled(
            exec.start_time.format("%H:%M:%S").to_string(),
            theme::muted_style(),
        ));
        right.push(Span::styled("  ", theme::muted_style()));
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
    pending: usize,
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
        pending: workflow.unreported_jobs().len(),
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
    let selected = app.job_list_state.selected().filter(|&i| i < jobs.len());
    let mut lines: Vec<Line> = Vec::with_capacity(total);
    for (i, job) in jobs.iter().enumerate() {
        let state = workflow.job_state(&job.canonical_name);
        let state = match (job.status.clone(), state) {
            // A matrix leg reports its own status, not the merged one.
            (JobStatus::Success, _) => JobState::Success,
            (JobStatus::Skipped, _) => JobState::Skipped,
            (JobStatus::Failure, _) if job.failure_allowed => JobState::FailureAllowed,
            (JobStatus::Failure, _) => JobState::Failure,
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
    for name in pending {
        let (sym, sym_style) = theme::job_state(workflow.job_state(name), app.spinner_frame);
        lines.push(Line::from(vec![
            Span::raw("  "),
            Span::styled(sym.to_string(), sym_style),
            Span::raw(" "),
            Span::styled(
                text::truncate(&text::sanitize(name), w.saturating_sub(4)).into_owned(),
                theme::muted_style(),
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
fn render_output_pane(f: &mut Frame<'_>, app: &App, area: Rect) {
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

// ─── Mini DAG pane ────────────────────────────────────────────────
fn render_dag_pane(f: &mut Frame<'_>, app: &App, workflow: &Workflow, area: Rect) {
    let block = theme::block("Graph");
    let inner = block.inner(area);
    f.render_widget(block, area);
    dag::render(f, inner, workflow, app.spinner_frame);
}

// ─── Results pane ─────────────────────────────────────────────────
fn render_results_pane(f: &mut Frame<'_>, workflow: &Workflow, area: Rect) {
    let block = theme::block("Results");
    let inner = block.inner(area);
    f.render_widget(block, area);

    let c = job_counts(workflow);
    let total = c.passed + c.failed + c.skipped + c.pending;
    if total == 0 {
        theme::empty_state(f, inner, "No jobs", vec![]);
        return;
    }

    // One proportional bar of the run's jobs, coloured by outcome.
    let w = inner.width as usize;
    let mut bar: Vec<Span> = Vec::new();
    let mut used = 0;
    let parts = [
        (c.passed, COLORS.success),
        (c.failed, COLORS.error),
        (c.skipped, COLORS.warning),
        (c.pending, COLORS.border),
    ];
    for (i, (n, color)) in parts.iter().enumerate() {
        let cells = if i == parts.len() - 1 {
            w - used
        } else {
            (n * w).div_ceil(total).min(w - used)
        };
        if *n > 0 && cells > 0 {
            bar.push(Span::styled("━".repeat(cells), Style::default().fg(*color)));
            used += cells;
        }
    }

    let row = |label: &'static str, n: usize, color| {
        Line::from(vec![
            Span::styled(format!("{label:<10}"), theme::muted_style()),
            Span::styled(
                n.to_string(),
                Style::default().fg(color).add_modifier(Modifier::BOLD),
            ),
        ])
    };
    let pending_label = if workflow.status == WorkflowStatus::Running {
        "running"
    } else {
        "not run"
    };
    let lines = vec![
        Line::from(bar),
        row("passed", c.passed, COLORS.success),
        row("failed", c.failed, COLORS.error),
        row("skipped", c.skipped, COLORS.warning),
        row(pending_label, c.pending, COLORS.text_dim),
    ];
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
