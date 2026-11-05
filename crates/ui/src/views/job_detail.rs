// Step inspector — a job's steps beside the selected step's output,
// with a Matrix sub-tab for matrix jobs. Output keeps the tool's own
// colours and follows the tail, since the end of a log is where a
// failure explains itself; PgUp/PgDn scroll back.

use crate::app::App;
use crate::text;
use crate::theme::{self, BadgeKind, COLORS};
use ratatui::{
    layout::{Alignment, Constraint, Direction, Layout, Rect},
    style::{Modifier, Style},
    text::{Line, Span},
    widgets::{Paragraph, Wrap},
    Frame,
};
use wrkflw_executor::JobStatus;

pub const INSPECTOR_TABS: [&str; 2] = ["Output", "Matrix"];

/// Output beyond this many lines is cut from the front, keeping the end.
const MAX_OUTPUT_LINES: usize = 5000;

pub fn render_job_detail_view(f: &mut Frame<'_>, app: &mut App, area: Rect) {
    let workflow_idx = app.shown_workflow();

    let job = workflow_idx.and_then(|w| {
        let exec = app.workflows[w].execution_details.as_ref()?;
        let j = app.job_list_state.selected()?;
        exec.jobs.get(j).map(|_| (w, j))
    });
    let Some((workflow_idx, job_idx)) = job else {
        let block = theme::block("Inspector");
        let inner = block.inner(area);
        f.render_widget(block, area);
        theme::empty_state(
            f,
            inner,
            "No finished job to inspect",
            vec![Span::styled("Esc to go back", theme::muted_style())],
        );
        return;
    };
    let workflow = &app.workflows[workflow_idx];
    let job = &workflow.execution_details.as_ref().unwrap().jobs[job_idx];

    let outer = Layout::default()
        .direction(Direction::Vertical)
        .constraints([Constraint::Length(1), Constraint::Min(0)])
        .split(area);

    render_header(f, &workflow.name, job, app.step_inspector_tab, outer[0]);

    let selected_step_idx = app
        .step_table_state
        .selected()
        .filter(|&i| i < job.steps.len());

    match app.step_inspector_tab {
        1 => render_matrix_pane(f, workflow, &job.canonical_name, outer[1]),
        _ => {
            let output = OutputSource {
                key: selected_step_idx.map(|s| (app.run_generation, workflow_idx, job_idx, s)),
                scroll_back: app.step_output_scroll,
                cache: &mut app.step_output_cache,
            };
            app.step_output_scroll =
                render_output_pane(f, job, selected_step_idx, output, outer[1]);
        }
    }
}

/// Parsed output of one step, kept across frames. Parsing ANSI and
/// measuring wrapped rows over thousands of lines on every 250 ms tick
/// is wasted work once a step has finished, which is the only time its
/// output exists.
#[derive(Default)]
pub struct StepOutputCache {
    /// (run generation, workflow, job, step) the lines belong to.
    key: (u64, usize, usize, usize),
    lines: Vec<Line<'static>>,
    /// Width `rows` was measured at.
    width: u16,
    /// Wrapped rows each line takes at `width`.
    rows: Vec<usize>,
    total: usize,
}

impl StepOutputCache {
    fn build(key: (u64, usize, usize, usize), output: &str) -> Self {
        let all: Vec<&str> = output.lines().collect();
        let skipped = all.len().saturating_sub(MAX_OUTPUT_LINES);
        let mut lines: Vec<Line<'static>> = Vec::with_capacity(all.len() - skipped + 1);
        if skipped > 0 {
            lines.push(Line::from(Span::styled(
                format!("… {skipped} earlier lines not shown"),
                theme::muted_style(),
            )));
        }
        let base = Style::default().fg(COLORS.text);
        lines.extend(all[skipped..].iter().map(|l| text::ansi_line(l, base)));
        Self {
            key,
            lines,
            ..Self::default()
        }
    }

    /// Measures every line's wrapped height at `width`, once per width.
    fn measure(&mut self, width: u16) {
        if self.width == width && self.rows.len() == self.lines.len() {
            return;
        }
        self.rows = self
            .lines
            .iter()
            .map(|l| {
                Paragraph::new(l.clone())
                    .wrap(Wrap { trim: false })
                    .line_count(width)
                    .max(1)
            })
            .collect();
        self.total = self.rows.iter().sum();
        self.width = width;
    }

    /// The line holding wrapped row `row`, and how many of that line's
    /// rows come before it.
    fn locate(&self, row: usize) -> (usize, usize) {
        let mut seen = 0;
        for (i, &r) in self.rows.iter().enumerate() {
            if seen + r > row {
                return (i, row - seen);
            }
            seen += r;
        }
        (self.rows.len(), 0)
    }
}

/// What the output pane needs from `App` besides the job itself.
struct OutputSource<'a> {
    /// Identifies the selected step's output; `None` with no step.
    key: Option<(u64, usize, usize, usize)>,
    /// Rows scrolled back from the tail.
    scroll_back: usize,
    cache: &'a mut Option<StepOutputCache>,
}

/// `workflow › job  Output · Matrix            failed` on one row.
fn render_header(
    f: &mut Frame<'_>,
    workflow_name: &str,
    job: &crate::models::JobExecution,
    active_tab: usize,
    area: Rect,
) {
    let (status, kind) = match job.status {
        JobStatus::Success => ("PASSED", BadgeKind::Success),
        JobStatus::Failure if job.failure_allowed => ("FAILED · ALLOWED", BadgeKind::Warning),
        JobStatus::Failure => ("FAILED", BadgeKind::Error),
        JobStatus::Skipped => ("SKIPPED", BadgeKind::Warning),
    };
    let badge = theme::badge_solid(status, kind);

    let mut tabs: Vec<Span> = Vec::new();
    for (i, label) in INSPECTOR_TABS.iter().enumerate() {
        tabs.push(if i == active_tab {
            Span::styled(
                format!(" {label} "),
                Style::default()
                    .bg(COLORS.bg_selected)
                    .fg(theme::current_accent())
                    .add_modifier(Modifier::BOLD),
            )
        } else {
            Span::styled(format!(" {label} "), Style::default().fg(COLORS.text_dim))
        });
    }

    let fixed = text::spans_width(&tabs) + text::width(&badge.content) + 4;
    let room = (area.width as usize).saturating_sub(fixed);
    let crumb = format!(
        "{} › {}",
        text::sanitize(workflow_name),
        text::sanitize(&job.name)
    );
    let mut left = vec![Span::styled(
        format!(" {}  ", text::truncate(&crumb, room)),
        Style::default()
            .fg(COLORS.text)
            .add_modifier(Modifier::BOLD),
    )];
    if text::spans_width(&left) + fixed <= area.width as usize + 2 {
        left.extend(tabs);
    }

    let chunks = Layout::default()
        .direction(Direction::Horizontal)
        .constraints([
            Constraint::Min(0),
            Constraint::Length(text::width(&badge.content) as u16 + 1),
        ])
        .split(area);
    f.render_widget(Paragraph::new(Line::from(left)), chunks[0]);
    f.render_widget(
        Paragraph::new(Line::from(vec![badge, Span::raw(" ")])).alignment(Alignment::Right),
        chunks[1],
    );
}

// ─── Output pane ──────────────────────────────────────────────────
/// Draws the steps list and the selected step's output, returning the
/// scroll-back clamped to what the output can actually scroll.
fn render_output_pane(
    f: &mut Frame<'_>,
    job: &crate::models::JobExecution,
    selected: Option<usize>,
    output: OutputSource<'_>,
    area: Rect,
) -> usize {
    // Side by side when there's room, stacked on narrow terminals.
    let (direction, list_size) = if area.width >= 80 {
        (
            Direction::Horizontal,
            Constraint::Length((area.width / 3).clamp(28, 48)),
        )
    } else {
        let rows = (job.steps.len() as u16 + 2).min(area.height / 3).max(3);
        (Direction::Vertical, Constraint::Length(rows))
    };
    let cols = Layout::default()
        .direction(direction)
        .constraints([list_size, Constraint::Min(0)])
        .split(area);

    render_steps_list(f, job, selected, cols[0]);
    render_step_output(f, job, selected, output, cols[1])
}

fn render_steps_list(
    f: &mut Frame<'_>,
    job: &crate::models::JobExecution,
    selected: Option<usize>,
    area: Rect,
) {
    let block = theme::with_badge(theme::block("Steps"), job.steps.len().to_string());
    let inner = block.inner(area);
    f.render_widget(block, area);

    if job.steps.is_empty() {
        theme::empty_state(f, inner, "No steps", vec![]);
        return;
    }

    let w = inner.width as usize;
    let lines: Vec<Line> = job
        .steps
        .iter()
        .enumerate()
        .map(|(i, step)| {
            let mut line = super::execution_tab::step_line(i, step, w);
            if selected == Some(i) {
                line = line.patch_style(theme::selected_style());
            }
            line
        })
        .collect();

    let viewport = inner.height as usize;
    let offset = selected
        .unwrap_or(0)
        .saturating_sub(viewport.saturating_sub(1));
    let total = lines.len();
    f.render_widget(Paragraph::new(lines).scroll((offset as u16, 0)), inner);
    theme::scrollbar(f, area, total, offset, viewport);
}

fn render_step_output(
    f: &mut Frame<'_>,
    job: &crate::models::JobExecution,
    selected: Option<usize>,
    output: OutputSource<'_>,
    area: Rect,
) -> usize {
    let step = selected.and_then(|i| job.steps.get(i));
    let title = match step {
        Some(s) => format!("Output · {}", text::sanitize(&s.name)),
        None => "Output".to_string(),
    };
    let title = text::truncate(&title, (area.width as usize).saturating_sub(16)).into_owned();
    let block = theme::block_focused(&title);
    let inner = block.inner(area);

    let (Some(step), Some(key)) = (step, output.key) else {
        f.render_widget(block, area);
        theme::empty_state(f, inner, "Select a step", vec![]);
        return 0;
    };
    if step.output.trim().is_empty() {
        f.render_widget(block, area);
        theme::empty_state(f, inner, "This step printed nothing", vec![]);
        return 0;
    }
    if inner.width == 0 || inner.height == 0 {
        f.render_widget(block, area);
        return output.scroll_back;
    }

    let cache = match output.cache {
        Some(c) if c.key == key => c,
        slot => slot.insert(StepOutputCache::build(key, &step.output)),
    };
    cache.measure(inner.width);

    let total = cache.total;
    let viewport = inner.height as usize;
    let max_back = total.saturating_sub(viewport);
    let back = output.scroll_back.min(max_back);
    let offset = max_back - back;

    // Hand the paragraph only the lines that reach the screen, so the
    // scroll it gets is a few rows into the first of them. Scrolling the
    // whole text by `offset` would overflow ratatui's u16 scroll once
    // wrapped output passes 65,535 rows, and re-wrap every line.
    let (first, skip) = cache.locate(offset);
    let mut visible: Vec<Line<'static>> = Vec::new();
    let mut rows = 0;
    for (line, &r) in cache.lines[first..].iter().zip(&cache.rows[first..]) {
        visible.push(line.clone());
        rows += r;
        if rows >= skip + viewport {
            break;
        }
    }

    let badge = if back == 0 {
        "tail".to_string()
    } else {
        format!("{}/{}", offset + viewport.min(total), total)
    };
    let block = theme::with_badge(block, badge);
    f.render_widget(
        Paragraph::new(visible)
            .wrap(Wrap { trim: false })
            .block(block)
            .scroll((skip.min(u16::MAX as usize) as u16, 0)),
        area,
    );
    theme::scrollbar(f, area, total, offset, viewport);
    back
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
        theme::empty_state(f, inner_area, "Not a matrix job", vec![]);
        return;
    };

    let mut lines: Vec<Line> = Vec::new();
    lines.push(theme::section("Axes"));

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
            Span::styled(
                format!("  {}: ", name),
                Style::default().fg(theme::current_accent()),
            ),
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

    // Matrix combinations — real expansion via `wrkflw_matrix::expand_matrix`.
    //
    // We can show the combos (the *what* — design screen 5's grid) but
    // not per-combo runtime status (the *how it went* — we don't track
    // status per combo, only aggregated job status). So rows are
    // labelled `queued` by default; if the parent job finished we
    // inherit its status for every row. This is honest: a future
    // executor change to surface per-combo results will drop right
    // into this render.
    match wrkflw_matrix::expand_matrix(matrix) {
        Ok(combos) if !combos.is_empty() => {
            lines.push(Line::from(""));
            lines.push(theme::section(format!("Combinations ({})", combos.len())));
            // Key order: show axes in the order they were declared,
            // plus any extra keys an `include:` entry introduced,
            // appended after (and sorted, since `MatrixCombination.values`
            // is a HashMap whose iteration order is not stable — without
            // a sort, include-only columns could jitter between frames
            // or process runs).
            let mut key_order: Vec<String> = matrix.parameters.keys().cloned().collect();
            let mut extra: Vec<String> = Vec::new();
            for c in &combos {
                for k in c.values.keys() {
                    if !key_order.contains(k) && !extra.contains(k) {
                        extra.push(k.clone());
                    }
                }
            }
            extra.sort();
            key_order.extend(extra);
            for c in combos.iter().take(32) {
                let mut spans: Vec<Span> = vec![Span::raw("  ")];
                let status_glyph = combo_glyph(workflow, job_name, c);
                spans.push(status_glyph);
                spans.push(Span::raw(" "));
                for (i, k) in key_order.iter().enumerate() {
                    if i > 0 {
                        spans.push(Span::styled("  ", Style::default().fg(COLORS.text_muted)));
                    }
                    spans.push(Span::styled(
                        format!("{}=", k),
                        Style::default().fg(theme::current_accent()),
                    ));
                    let v = c
                        .values
                        .get(k)
                        .map(format_yaml_scalar)
                        .unwrap_or_else(|| "—".to_string());
                    spans.push(Span::styled(v, Style::default().fg(COLORS.text)));
                }
                if c.is_included {
                    spans.push(Span::raw("  "));
                    spans.push(theme::badge_outline("+include", BadgeKind::Warning));
                }
                lines.push(Line::from(spans));
            }
            if combos.len() > 32 {
                lines.push(Line::from(vec![Span::styled(
                    format!("  … +{} more", combos.len() - 32),
                    Style::default().fg(COLORS.text_muted),
                )]));
            }
        }
        Ok(_) => {
            lines.push(Line::from(""));
            lines.push(Line::from(vec![Span::styled(
                "(matrix expanded to 0 combinations — check exclude: or empty axes)",
                Style::default().fg(COLORS.text_muted),
            )]));
        }
        Err(e) => {
            lines.push(Line::from(""));
            lines.push(Line::from(vec![
                theme::badge_outline("expansion error", BadgeKind::Error),
                Span::raw(" "),
                Span::styled(e.to_string(), Style::default().fg(COLORS.text_dim)),
            ]));
        }
    }

    if let wrkflw_matrix::MatrixEntries::List(exclude) = &matrix.exclude {
        if !exclude.is_empty() {
            lines.push(Line::from(""));
            lines.push(theme::section(format!("Exclude ({})", exclude.len())));
        }
    }

    f.render_widget(Paragraph::new(lines).wrap(Wrap { trim: false }), inner_area);
}

fn format_yaml_scalar(v: &serde_yaml::Value) -> String {
    match v {
        serde_yaml::Value::String(s) => collapse_newlines(s),
        serde_yaml::Value::Bool(b) => b.to_string(),
        serde_yaml::Value::Number(n) => n.to_string(),
        serde_yaml::Value::Null => "~".to_string(),
        other => {
            // Sequences and maps round-trip to multi-line YAML; each
            // combo renders into a single ratatui Span, so embedded
            // newlines would silently garble the layout. Collapse
            // them into a visible ` · ` separator.
            let raw = serde_yaml::to_string(other).unwrap_or_default();
            collapse_newlines(raw.trim())
        }
    }
}

/// Replace any `\n` / `\r` with a visible separator so multi-line
/// payloads don't break single-Line rendering.
fn collapse_newlines(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    let mut last_was_sep = false;
    for ch in s.chars() {
        if ch == '\n' || ch == '\r' {
            if !last_was_sep {
                out.push_str(" · ");
                last_was_sep = true;
            }
        } else {
            out.push(ch);
            last_was_sep = false;
        }
    }
    out
}

/// Status glyph for one combination. Each matrix leg reports under its
/// combination's display name, so the lookup is exact per row.
fn combo_glyph<'a>(
    workflow: &'a crate::models::Workflow,
    job_name: &str,
    combo: &wrkflw_matrix::MatrixCombination,
) -> Span<'a> {
    let leg = wrkflw_matrix::format_combination_name(job_name, combo);
    let job = workflow
        .execution_details
        .as_ref()
        .and_then(|e| e.jobs.iter().find(|j| j.name == leg));
    let (glyph, color) = match job.map(|j| (&j.status, j.failure_allowed)) {
        Some((JobStatus::Success, _)) => (theme::symbols::SUCCESS, COLORS.success),
        Some((JobStatus::Failure, true)) => (theme::symbols::FAILURE, COLORS.warning),
        Some((JobStatus::Failure, false)) => (theme::symbols::FAILURE, COLORS.error),
        Some((JobStatus::Skipped, _)) => (theme::symbols::SKIPPED, COLORS.text_muted),
        None => (theme::symbols::NOT_STARTED, COLORS.text_muted),
    };
    Span::styled(glyph.to_string(), Style::default().fg(color))
}
