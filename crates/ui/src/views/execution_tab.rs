// Live Run — three-pane layout (Jobs · Steps + Live output · Mini DAG + Timing).
//
// Mirrors `LiveRunScreen` in screens-core.jsx of the design handoff. Where the
// design uses synthetic per-job timing and per-step env capture, we render the
// fields we actually have (status, names, workflow elapsed) and clearly mark
// the rest as not yet captured.

use crate::app::App;
use crate::components::{
    dag,
    progress_dots::{self, DotState},
    timing::{self, TimingRow},
};
use crate::models::WorkflowStatus;
use crate::theme::{self, BadgeKind, COLORS};
use ratatui::{
    layout::{Alignment, Constraint, Direction, Layout, Rect},
    style::{Modifier, Style},
    text::{Line, Span},
    widgets::{Paragraph, Wrap},
    Frame,
};
use wrkflw_executor::{JobStatus, RuntimeType, StepStatus};

const RIGHT_PANE_WIDTH: u16 = 40;
const LEFT_PANE_WIDTH: u16 = 30;

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

    // ── Vertical split: summary strip + main pane ─────────────────
    let outer = Layout::default()
        .direction(Direction::Vertical)
        .constraints([Constraint::Length(3), Constraint::Min(0)])
        .split(area);

    render_summary_strip(f, app, idx, outer[0]);

    let main = Layout::default()
        .direction(Direction::Horizontal)
        .constraints([
            Constraint::Length(LEFT_PANE_WIDTH),
            Constraint::Min(0),
            Constraint::Length(RIGHT_PANE_WIDTH),
        ])
        .split(outer[1]);

    render_jobs_pane(f, app, idx, main[0]);

    let centre = Layout::default()
        .direction(Direction::Vertical)
        .constraints([Constraint::Percentage(50), Constraint::Min(0)])
        .split(main[1]);
    render_steps_pane(f, workflow, centre[0]);
    render_live_output_pane(f, app, centre[1]);

    let right = Layout::default()
        .direction(Direction::Vertical)
        .constraints([Constraint::Percentage(60), Constraint::Min(0)])
        .split(main[2]);
    render_dag_pane(f, app, idx, right[0]);
    render_timing_pane(f, workflow, right[1]);
}

// ─── Summary strip (workflow chip + progress dots + meta) ────────
fn render_summary_strip(f: &mut Frame<'_>, app: &App, idx: usize, area: Rect) {
    let workflow = &app.workflows[idx];
    let block = theme::block("Run");
    let inner_area = block.inner(area);
    f.render_widget(block, area);

    let chunks = Layout::default()
        .direction(Direction::Horizontal)
        .constraints([Constraint::Min(0), Constraint::Length(28)])
        .split(inner_area);

    // Left: workflow chip
    let (sym, sym_style) = theme::workflow_status_animated(&workflow.status, app.spinner_frame);
    let runtime_kind = match app.runtime_type {
        RuntimeType::Docker => BadgeKind::Docker,
        RuntimeType::Podman => BadgeKind::Podman,
        RuntimeType::SecureEmulation => BadgeKind::Secure,
        RuntimeType::Emulation => BadgeKind::Emulation,
    };

    let mut left_spans: Vec<Span> = vec![
        Span::styled(sym.to_string(), sym_style),
        Span::raw(" "),
        Span::styled(
            workflow.name.clone(),
            Style::default()
                .fg(COLORS.text)
                .add_modifier(Modifier::BOLD),
        ),
        Span::styled("  ·  ", Style::default().fg(COLORS.text_muted)),
    ];
    if let Some(exec) = workflow.execution_details.as_ref() {
        left_spans.push(Span::styled(
            exec.start_time.format("%H:%M:%S").to_string(),
            Style::default().fg(COLORS.text_dim),
        ));
        left_spans.push(Span::styled(
            "  ·  ",
            Style::default().fg(COLORS.text_muted),
        ));
    }
    left_spans.push(theme::badge_outline(app.runtime_type_name(), runtime_kind));

    f.render_widget(
        Paragraph::new(Line::from(left_spans)).alignment(Alignment::Left),
        chunks[0],
    );

    // Right: progress dots over the active job's steps
    if let Some(active_job) = active_job_execution(workflow) {
        let total = active_job.steps.len();
        let dots = progress_dots::synthesise(
            &active_job
                .steps
                .iter()
                .map(|s| s.status)
                .collect::<Vec<_>>(),
            total,
            &workflow.status,
        );
        let done = dots
            .iter()
            .filter(|d| matches!(d, DotState::Success | DotState::Failure | DotState::Skipped))
            .count();
        progress_dots::render(f, chunks[1], &dots, done, total);
    }
}

// ─── Jobs pane (left) ─────────────────────────────────────────────
fn render_jobs_pane(f: &mut Frame<'_>, app: &App, idx: usize, area: Rect) {
    let block = theme::block("Jobs");
    let inner_area = block.inner(area);
    f.render_widget(block, area);

    let workflow = &app.workflows[idx];
    let exec = workflow.execution_details.as_ref();

    // Synthesise full job list from workflow.job_names; jobs in `exec.jobs`
    // get their real status, the rest are pending (or running for the next slot).
    let mut lines: Vec<Line> = Vec::new();
    let active_name = active_job_name(workflow);
    for (i, name) in workflow.job_names.iter().enumerate() {
        let job_exec = exec.and_then(|e| e.jobs.iter().find(|j| j.name == *name));
        let (sym, sym_style) = match (job_exec, active_name.as_deref() == Some(name)) {
            (Some(j), _) => theme::job_status(&j.status),
            (None, true) => (
                theme::spinner(app.spinner_frame),
                Style::default().fg(COLORS.info),
            ),
            (None, false) => (
                theme::symbols::NOT_STARTED,
                Style::default().fg(COLORS.text_muted),
            ),
        };
        let is_selected = i == app.job_list_state.selected().unwrap_or(0);
        let row_style = if is_selected {
            theme::selected_style()
        } else {
            Style::default()
        };
        let name_style = if job_exec.is_some() || active_name.as_deref() == Some(name) {
            Style::default().fg(COLORS.text)
        } else {
            Style::default().fg(COLORS.text_muted)
        };
        lines.push(Line::from(vec![
            Span::styled(" ", row_style),
            Span::styled(sym.to_string(), sym_style),
            Span::raw(" "),
            Span::styled(name.clone(), name_style.patch(row_style)),
        ]));
    }
    if lines.is_empty() {
        lines.push(Line::from(Span::styled(
            "no jobs in this workflow",
            Style::default().fg(COLORS.text_muted),
        )));
    }

    // Footer summary
    let total = workflow.job_names.len();
    let done = exec
        .map(|e| {
            e.jobs
                .iter()
                .filter(|j| matches!(j.status, JobStatus::Success | JobStatus::Skipped))
                .count()
        })
        .unwrap_or(0);
    let failed = exec
        .map(|e| {
            e.jobs
                .iter()
                .filter(|j| matches!(j.status, JobStatus::Failure))
                .count()
        })
        .unwrap_or(0);
    let pending = total.saturating_sub(done + failed);

    lines.push(Line::from(""));
    lines.push(Line::from(vec![
        Span::styled(
            format!("{}/{}", done, total),
            Style::default().fg(COLORS.success),
        ),
        Span::styled(" done · ", Style::default().fg(COLORS.text_muted)),
        Span::styled(format!("{}", failed), Style::default().fg(COLORS.error)),
        Span::styled(" failed · ", Style::default().fg(COLORS.text_muted)),
        Span::styled(
            format!("{}", pending),
            Style::default().fg(COLORS.text_muted),
        ),
        Span::styled(" pending", Style::default().fg(COLORS.text_muted)),
    ]));

    f.render_widget(Paragraph::new(lines), inner_area);
}

// ─── Steps pane (centre top) ──────────────────────────────────────
fn render_steps_pane(f: &mut Frame<'_>, workflow: &crate::models::Workflow, area: Rect) {
    let title = match active_job_execution(workflow) {
        Some(j) => format!("Steps — {}", j.name),
        None => "Steps".to_string(),
    };
    let block = theme::block(&title);
    let inner_area = block.inner(area);
    f.render_widget(block, area);

    let job = match active_job_execution(workflow) {
        Some(j) => j,
        None => {
            f.render_widget(
                Paragraph::new(Line::from(Span::styled(
                    "no active job",
                    Style::default().fg(COLORS.text_muted),
                ))),
                inner_area,
            );
            return;
        }
    };

    let mut lines: Vec<Line> = Vec::new();
    for (i, step) in job.steps.iter().enumerate() {
        let (sym, sym_style) = theme::step_status(&step.status);
        let name_style = match step.status {
            StepStatus::Skipped => Style::default().fg(COLORS.text_muted),
            _ => Style::default().fg(COLORS.text),
        };
        lines.push(Line::from(vec![
            Span::styled(
                format!("{:02} ", i + 1),
                Style::default().fg(COLORS.text_muted),
            ),
            Span::styled(sym.to_string(), sym_style),
            Span::raw("  "),
            Span::styled(step.name.clone(), name_style),
        ]));
    }
    if lines.is_empty() {
        lines.push(Line::from(Span::styled(
            "no steps reported yet",
            Style::default().fg(COLORS.text_muted),
        )));
    }
    f.render_widget(Paragraph::new(lines), inner_area);
}

// ─── Live output pane (centre bottom) ─────────────────────────────
fn render_live_output_pane(f: &mut Frame<'_>, app: &App, area: Rect) {
    let block = theme::block_focused("Live output");
    let inner_area = block.inner(area);
    f.render_widget(block, area);

    if app.processed_logs.is_empty() {
        f.render_widget(
            Paragraph::new(Line::from(Span::styled(
                "(no log lines yet — output will stream here)",
                Style::default().fg(COLORS.text_muted),
            ))),
            inner_area,
        );
        return;
    }

    // Take the last N lines that fit in the pane.
    let max_rows = inner_area.height as usize;
    let start = app.processed_logs.len().saturating_sub(max_rows);
    let lines: Vec<Line> = app.processed_logs[start..]
        .iter()
        .map(|entry| {
            let mut spans: Vec<Span> = Vec::with_capacity(entry.content_spans.len() + 4);
            spans.push(Span::styled(
                entry.timestamp.clone(),
                Style::default().fg(COLORS.text_muted),
            ));
            spans.push(Span::raw(" "));
            spans.push(Span::styled(entry.log_type.clone(), entry.log_style));
            spans.push(Span::raw(" "));
            spans.extend(entry.content_spans.iter().cloned());
            Line::from(spans)
        })
        .collect();

    f.render_widget(Paragraph::new(lines).wrap(Wrap { trim: false }), inner_area);
}

// ─── Mini DAG pane (right top) ────────────────────────────────────
fn render_dag_pane(f: &mut Frame<'_>, app: &App, idx: usize, area: Rect) {
    let block = theme::block("DAG");
    let inner_area = block.inner(area);
    f.render_widget(block, area);

    let workflow = &app.workflows[idx];
    let def = workflow.definition.as_deref();
    let exec = workflow.execution_details.as_ref();
    let active = active_job_name(workflow);

    let state_of = |name: &str| -> dag::NodeState {
        if let Some(e) = exec {
            if let Some(j) = e.jobs.iter().find(|j| j.name == name) {
                return match j.status {
                    JobStatus::Success => dag::NodeState::Success,
                    JobStatus::Failure => dag::NodeState::Failure,
                    JobStatus::Skipped => dag::NodeState::Skipped,
                };
            }
        }
        if active.as_deref() == Some(name) {
            dag::NodeState::Running
        } else {
            dag::NodeState::Pending
        }
    };

    dag::render(f, inner_area, def, state_of, app.spinner_frame);
}

// ─── Timing pane (right bottom) ───────────────────────────────────
fn render_timing_pane(f: &mut Frame<'_>, workflow: &crate::models::Workflow, area: Rect) {
    let block = theme::block("Timing");
    let inner_area = block.inner(area);
    f.render_widget(block, area);

    let exec = match workflow.execution_details.as_ref() {
        Some(e) => e,
        None => {
            f.render_widget(
                Paragraph::new(Line::from(Span::styled(
                    "no run yet",
                    Style::default().fg(COLORS.text_muted),
                ))),
                inner_area,
            );
            return;
        }
    };

    let labels: Vec<String> = exec
        .jobs
        .iter()
        .map(|j| match j.status {
            JobStatus::Success => "ok".to_string(),
            JobStatus::Failure => "fail".to_string(),
            JobStatus::Skipped => "skip".to_string(),
        })
        .collect();

    let rows: Vec<TimingRow> = exec
        .jobs
        .iter()
        .zip(labels.iter())
        .map(|(j, label)| TimingRow {
            name: j.name.as_str(),
            status: Some(j.status.clone()),
            label: label.as_str(),
        })
        .collect();

    timing::render(f, inner_area, &rows);
}

// ─── Helpers ──────────────────────────────────────────────────────
fn active_job_execution(
    workflow: &crate::models::Workflow,
) -> Option<&crate::models::JobExecution> {
    let exec = workflow.execution_details.as_ref()?;
    if matches!(workflow.status, WorkflowStatus::Running) {
        // Most-recent job is the one currently driving the run.
        exec.jobs.last()
    } else {
        // Otherwise prefer the failed job so the live output points at the
        // interesting place; fall back to the last job.
        exec.jobs
            .iter()
            .rfind(|j| matches!(j.status, JobStatus::Failure))
            .or_else(|| exec.jobs.last())
    }
}

fn active_job_name(workflow: &crate::models::Workflow) -> Option<String> {
    if !matches!(workflow.status, WorkflowStatus::Running) {
        return None;
    }
    let exec = workflow.execution_details.as_ref()?;
    // The first job in `job_names` not yet present in `exec.jobs` is the one
    // about to run; the last job in `exec.jobs` may still be in flight.
    let executed: std::collections::HashSet<&str> =
        exec.jobs.iter().map(|j| j.name.as_str()).collect();
    workflow
        .job_names
        .iter()
        .find(|n| !executed.contains(n.as_str()))
        .cloned()
        .or_else(|| exec.jobs.last().map(|j| j.name.clone()))
}

fn render_empty_state(f: &mut Frame<'_>, area: Rect) {
    let block = theme::block("Run");
    let inner_area = block.inner(area);
    f.render_widget(block, area);
    f.render_widget(
        Paragraph::new(vec![
            Line::from(""),
            Line::from(Span::styled(
                "No workflow execution yet.",
                Style::default()
                    .fg(COLORS.warning)
                    .add_modifier(Modifier::BOLD),
            )),
            Line::from(""),
            Line::from(vec![
                Span::styled("Switch to ", Style::default().fg(COLORS.text_muted)),
                Span::styled("Workflows", Style::default().fg(COLORS.accent)),
                Span::styled(" and press ", Style::default().fg(COLORS.text_muted)),
                theme::key_chip("r"),
                Span::styled(" to run, or ", Style::default().fg(COLORS.text_muted)),
                theme::key_chip("t"),
                Span::styled(
                    " to trigger remotely.",
                    Style::default().fg(COLORS.text_muted),
                ),
            ]),
        ])
        .alignment(Alignment::Center),
        inner_area,
    );
}
