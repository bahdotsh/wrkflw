// Step inspector — tabbed sub-view inside the Execution tab.
//
// Mirrors `StepDetailScreen` from screens-core.jsx of the design handoff:
// breadcrumb header, sub-tabs for Output / Env / Files / Matrix / Timeline.
// Env/Files don't have backing data today so we render honest placeholders;
// Output reuses each step's stdout, Matrix reads `Job.strategy`, and Timeline
// reuses the timing chart with one row per step.

use crate::app::App;
use crate::components::timing::{self, TimingRow};
use crate::theme::{self, BadgeKind, COLORS};
use ratatui::{
    layout::{Alignment, Constraint, Direction, Layout, Rect},
    style::{Modifier, Style},
    text::{Line, Span},
    widgets::{Paragraph, Wrap},
    Frame,
};
use wrkflw_executor::{JobStatus, StepStatus};

const TABS: [&str; 5] = ["Output", "Env", "Files", "Matrix", "Timeline"];

pub fn render_job_detail_view(f: &mut Frame<'_>, app: &mut App, area: Rect) {
    let workflow_idx = app
        .current_execution
        .or_else(|| app.workflow_list_state.selected())
        .filter(|&idx| idx < app.workflows.len());

    let Some(workflow_idx) = workflow_idx else {
        return;
    };
    let workflow = &app.workflows[workflow_idx];
    let Some(execution) = workflow.execution_details.as_ref() else {
        return;
    };
    let Some(job_idx) = app.job_list_state.selected() else {
        return;
    };
    if job_idx >= execution.jobs.len() {
        return;
    }
    let job = &execution.jobs[job_idx];

    let outer = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Length(2), // breadcrumb
            Constraint::Length(2), // tabs
            Constraint::Min(0),    // body
        ])
        .margin(1)
        .split(area);

    render_breadcrumb(f, &workflow.name, job, outer[0]);
    render_tab_strip(f, app.step_inspector_tab, outer[1]);

    let selected_step_idx = app
        .step_table_state
        .selected()
        .filter(|&i| i < job.steps.len());

    match app.step_inspector_tab {
        0 => render_output_pane(f, job, selected_step_idx, outer[2]),
        1 => render_env_pane(f, outer[2]),
        2 => render_files_pane(f, outer[2]),
        3 => render_matrix_pane(f, workflow, &job.name, outer[2]),
        4 => render_timeline_pane(f, job, outer[2]),
        _ => {}
    }
}

fn render_breadcrumb(
    f: &mut Frame<'_>,
    workflow_name: &str,
    job: &crate::models::JobExecution,
    area: Rect,
) {
    let (sym, sym_style) = theme::job_status(&job.status);
    let status_text = match job.status {
        JobStatus::Success => ("success", BadgeKind::Success),
        JobStatus::Failure => ("failed", BadgeKind::Error),
        JobStatus::Skipped => ("skipped", BadgeKind::Warning),
    };

    let chunks = Layout::default()
        .direction(Direction::Horizontal)
        .constraints([Constraint::Min(0), Constraint::Length(20)])
        .split(area);

    let left = Paragraph::new(Line::from(vec![
        Span::styled(
            workflow_name.to_string(),
            Style::default().fg(COLORS.text_muted),
        ),
        Span::styled(" / ", Style::default().fg(COLORS.text_muted)),
        Span::styled(sym.to_string(), sym_style),
        Span::raw(" "),
        Span::styled(
            job.name.clone(),
            Style::default()
                .fg(COLORS.text)
                .add_modifier(Modifier::BOLD),
        ),
        Span::styled(
            format!("   ({} steps)", job.steps.len()),
            Style::default().fg(COLORS.text_muted),
        ),
    ]))
    .alignment(Alignment::Left);
    f.render_widget(left, chunks[0]);

    let right = Paragraph::new(Line::from(vec![theme::badge_outline(
        status_text.0,
        status_text.1,
    )]))
    .alignment(Alignment::Right);
    f.render_widget(right, chunks[1]);
}

fn render_tab_strip(f: &mut Frame<'_>, active: usize, area: Rect) {
    let mut spans: Vec<Span> = Vec::with_capacity(TABS.len() * 3);
    for (i, label) in TABS.iter().enumerate() {
        let is_active = i == active;
        spans.push(Span::styled(
            format!(" {} ", label),
            if is_active {
                Style::default()
                    .fg(COLORS.accent)
                    .add_modifier(Modifier::BOLD | Modifier::UNDERLINED)
            } else {
                Style::default().fg(COLORS.text_dim)
            },
        ));
        if i + 1 < TABS.len() {
            spans.push(Span::styled("·", Style::default().fg(COLORS.text_muted)));
        }
    }
    spans.push(Span::raw("  "));
    spans.push(theme::key_chip("Tab"));
    spans.push(Span::styled(
        " switch  ",
        Style::default().fg(COLORS.text_muted),
    ));

    f.render_widget(
        Paragraph::new(Line::from(spans)).alignment(Alignment::Left),
        area,
    );
}

// ─── Output pane (default) ────────────────────────────────────────
fn render_output_pane(
    f: &mut Frame<'_>,
    job: &crate::models::JobExecution,
    selected_step: Option<usize>,
    area: Rect,
) {
    let cols = Layout::default()
        .direction(Direction::Horizontal)
        .constraints([Constraint::Length(34), Constraint::Min(0)])
        .split(area);

    render_steps_list(f, job, selected_step, cols[0]);
    render_step_stdout(f, job, selected_step, cols[1]);
}

fn render_steps_list(
    f: &mut Frame<'_>,
    job: &crate::models::JobExecution,
    selected: Option<usize>,
    area: Rect,
) {
    let block = theme::block("Steps");
    let inner_area = block.inner(area);
    f.render_widget(block, area);

    let mut lines: Vec<Line> = Vec::new();
    for (i, step) in job.steps.iter().enumerate() {
        let (sym, sym_style) = theme::step_status(&step.status);
        let highlighted = selected == Some(i);
        let row_style = if highlighted {
            theme::selected_style()
        } else {
            Style::default()
        };
        let name_style = match step.status {
            StepStatus::Skipped => Style::default().fg(COLORS.text_muted),
            _ => Style::default().fg(COLORS.text),
        };
        lines.push(Line::from(vec![
            Span::styled(
                format!("{:02} ", i + 1),
                Style::default().fg(COLORS.text_muted).patch(row_style),
            ),
            Span::styled(sym.to_string(), sym_style),
            Span::raw(" "),
            Span::styled(step.name.clone(), name_style.patch(row_style)),
        ]));
    }
    if lines.is_empty() {
        lines.push(Line::from(Span::styled(
            "no steps",
            Style::default().fg(COLORS.text_muted),
        )));
    }
    f.render_widget(Paragraph::new(lines), inner_area);
}

fn render_step_stdout(
    f: &mut Frame<'_>,
    job: &crate::models::JobExecution,
    selected: Option<usize>,
    area: Rect,
) {
    let title = match selected.and_then(|i| job.steps.get(i)) {
        Some(s) => format!("stdout — {}", s.name),
        None => "stdout".to_string(),
    };
    let block = theme::block_focused(&title);
    let inner_area = block.inner(area);
    f.render_widget(block, area);

    let Some(step) = selected.and_then(|i| job.steps.get(i)) else {
        f.render_widget(
            Paragraph::new(Line::from(Span::styled(
                "(select a step on the left)",
                Style::default().fg(COLORS.text_muted),
            ))),
            inner_area,
        );
        return;
    };

    if step.output.is_empty() {
        f.render_widget(
            Paragraph::new(Line::from(Span::styled(
                "(no output captured for this step)",
                Style::default().fg(COLORS.text_muted),
            ))),
            inner_area,
        );
        return;
    }

    let mut output = step.output.clone();
    if output.len() > 8000 {
        output = format!("{}…[truncated]", &output[..8000]);
    }
    let lines: Vec<Line> = output
        .split('\n')
        .map(|l| {
            Line::from(Span::styled(
                l.to_string(),
                Style::default().fg(COLORS.text_dim),
            ))
        })
        .collect();

    f.render_widget(Paragraph::new(lines).wrap(Wrap { trim: false }), inner_area);
}

// ─── Env pane (placeholder) ───────────────────────────────────────
fn render_env_pane(f: &mut Frame<'_>, area: Rect) {
    let block = theme::block("Process environment");
    let inner_area = block.inner(area);
    f.render_widget(block, area);
    let lines = vec![
        Line::from(""),
        Line::from(Span::styled(
            "Per-step environment capture is not yet plumbed.",
            Style::default()
                .fg(COLORS.warning)
                .add_modifier(Modifier::BOLD),
        )),
        Line::from(""),
        Line::from(Span::styled(
            "When the executor starts snapshotting `cmd.env` per step, the",
            Style::default().fg(COLORS.text_dim),
        )),
        Line::from(Span::styled(
            "env table will appear here with secret-masking on by default.",
            Style::default().fg(COLORS.text_dim),
        )),
    ];
    f.render_widget(
        Paragraph::new(lines).alignment(Alignment::Center),
        inner_area,
    );
}

// ─── Files pane (placeholder) ─────────────────────────────────────
fn render_files_pane(f: &mut Frame<'_>, area: Rect) {
    let block = theme::block("Workspace changes");
    let inner_area = block.inner(area);
    f.render_widget(block, area);
    let lines = vec![
        Line::from(""),
        Line::from(Span::styled(
            "Workspace diff per step is not yet captured.",
            Style::default()
                .fg(COLORS.warning)
                .add_modifier(Modifier::BOLD),
        )),
        Line::from(""),
        Line::from(Span::styled(
            "Will list new/modified artifacts under the job workspace once the",
            Style::default().fg(COLORS.text_dim),
        )),
        Line::from(Span::styled(
            "runtime exposes a watched-fs handle.",
            Style::default().fg(COLORS.text_dim),
        )),
    ];
    f.render_widget(
        Paragraph::new(lines).alignment(Alignment::Center),
        inner_area,
    );
}

// ─── Matrix pane ──────────────────────────────────────────────────
fn render_matrix_pane(
    f: &mut Frame<'_>,
    workflow: &crate::models::Workflow,
    job_name: &str,
    area: Rect,
) {
    let block = theme::block("Matrix");
    let inner_area = block.inner(area);
    f.render_widget(block, area);

    let strategy = workflow
        .definition
        .as_ref()
        .and_then(|d| d.jobs.get(job_name))
        .and_then(|j| j.strategy.as_ref());

    let Some(strategy) = strategy else {
        let lines = vec![
            Line::from(""),
            Line::from(Span::styled(
                format!("`{}` is not a matrix job.", job_name),
                Style::default().fg(COLORS.text_dim),
            )),
        ];
        f.render_widget(
            Paragraph::new(lines).alignment(Alignment::Center),
            inner_area,
        );
        return;
    };

    let mut lines: Vec<Line> = Vec::new();
    lines.push(Line::from(vec![Span::styled(
        "AXES",
        Style::default()
            .fg(COLORS.highlight)
            .add_modifier(Modifier::BOLD),
    )]));

    let Some(matrix) = strategy.matrix.as_ref() else {
        lines.push(Line::from(Span::styled(
            "(matrix strategy with no axes)",
            Style::default().fg(COLORS.text_muted),
        )));
        f.render_widget(Paragraph::new(lines), inner_area);
        return;
    };

    for (name, value) in &matrix.parameters {
        let values: Vec<String> = match value.as_sequence() {
            Some(seq) => seq
                .iter()
                .filter_map(|n| {
                    n.as_str()
                        .map(|s| s.to_string())
                        .or_else(|| n.as_i64().map(|i| i.to_string()))
                        .or_else(|| n.as_f64().map(|f| f.to_string()))
                })
                .collect(),
            None => vec![format!("{:?}", value)],
        };
        lines.push(Line::from(vec![
            Span::styled(format!("  {}: ", name), Style::default().fg(COLORS.accent)),
            Span::styled(values.join(", "), Style::default().fg(COLORS.text)),
        ]));
    }

    let mut chips: Vec<Span> = Vec::new();
    if let Some(max) = strategy.max_parallel.or(matrix.max_parallel) {
        chips.push(theme::badge_outline(
            format!("max-parallel: {}", max),
            BadgeKind::Dim,
        ));
        chips.push(Span::raw(" "));
    }
    let fail_fast = strategy.fail_fast.or(matrix.fail_fast).unwrap_or(true);
    if !fail_fast {
        chips.push(theme::badge_outline("fail-fast: false", BadgeKind::Warning));
    }
    if !chips.is_empty() {
        lines.push(Line::from(""));
        lines.push(Line::from(chips));
    }
    if !matrix.include.is_empty() {
        lines.push(Line::from(""));
        lines.push(Line::from(vec![Span::styled(
            format!("INCLUDE ({})", matrix.include.len()),
            Style::default()
                .fg(COLORS.highlight)
                .add_modifier(Modifier::BOLD),
        )]));
    }
    if !matrix.exclude.is_empty() {
        lines.push(Line::from(vec![Span::styled(
            format!("EXCLUDE ({})", matrix.exclude.len()),
            Style::default()
                .fg(COLORS.highlight)
                .add_modifier(Modifier::BOLD),
        )]));
    }

    f.render_widget(Paragraph::new(lines), inner_area);
}

// ─── Timeline pane (uses timing component) ────────────────────────
fn render_timeline_pane(f: &mut Frame<'_>, job: &crate::models::JobExecution, area: Rect) {
    let block = theme::block("Step timeline");
    let inner_area = block.inner(area);
    f.render_widget(block, area);

    let labels: Vec<String> = job
        .steps
        .iter()
        .map(|s| match s.status {
            StepStatus::Success => "ok".to_string(),
            StepStatus::Failure => "fail".to_string(),
            StepStatus::Skipped => "skip".to_string(),
        })
        .collect();

    let rows: Vec<TimingRow> = job
        .steps
        .iter()
        .zip(labels.iter())
        .map(|(s, label)| TimingRow {
            name: s.name.as_str(),
            status: match s.status {
                StepStatus::Success => Some(JobStatus::Success),
                StepStatus::Failure => Some(JobStatus::Failure),
                StepStatus::Skipped => Some(JobStatus::Skipped),
            },
            label: label.as_str(),
        })
        .collect();

    timing::render(f, inner_area, &rows);
}
