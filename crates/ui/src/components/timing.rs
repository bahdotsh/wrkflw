// Per-job timing chart — horizontal bars sized against the longest run.
//
// We don't have per-job wall-clock timing today (executor only reports terminal
// statuses, no `started_at` per job). For now we render uniform-width bars
// coloured by status so the panel is visually present and honest. When timing
// metadata lands later, swap `weight=1.0` for `(elapsed / max).min(1.0)`.

use crate::theme::COLORS;
use ratatui::{
    layout::Rect,
    style::{Modifier, Style},
    text::{Line, Span},
    widgets::Paragraph,
    Frame,
};
use wrkflw_executor::JobStatus;

#[derive(Clone)]
pub struct TimingRow<'a> {
    pub name: &'a str,
    pub status: Option<JobStatus>, // None = pending
    pub label: &'a str,            // e.g. "1m 47s" or "—"
}

pub fn render(frame: &mut Frame<'_>, area: Rect, rows: &[TimingRow]) {
    if area.width < 12 {
        return;
    }
    // We aim for: NAME (10) | BAR (rest - 6) | LABEL (5)
    let bar_width = area.width.saturating_sub(18) as usize;
    let mut lines: Vec<Line> = Vec::with_capacity(rows.len());
    for row in rows {
        let (color, fill) = bar_props(row.status.clone());
        let filled = (fill * bar_width as f32).round() as usize;
        let empty = bar_width.saturating_sub(filled);

        lines.push(Line::from(vec![
            Span::styled(
                pad_right(row.name, 10),
                Style::default().fg(COLORS.text_dim),
            ),
            Span::styled("█".repeat(filled), Style::default().fg(color)),
            Span::styled("·".repeat(empty), Style::default().fg(COLORS.border)),
            Span::raw(" "),
            Span::styled(
                pad_left(row.label, 5),
                Style::default().fg(COLORS.text_muted),
            ),
        ]));
    }
    if rows.is_empty() {
        lines.push(Line::from(Span::styled(
            "no jobs yet",
            Style::default().fg(COLORS.text_muted),
        )));
    } else {
        lines.push(Line::from(""));
        lines.push(Line::from(vec![
            Span::styled(
                "critical path ",
                Style::default()
                    .fg(COLORS.text_muted)
                    .add_modifier(Modifier::DIM),
            ),
            Span::styled(summarise(rows), Style::default().fg(COLORS.text_dim)),
        ]));
    }
    frame.render_widget(Paragraph::new(lines), area);
}

fn bar_props(s: Option<JobStatus>) -> (ratatui::style::Color, f32) {
    match s {
        Some(JobStatus::Success) => (COLORS.success, 1.0),
        Some(JobStatus::Failure) => (COLORS.error, 1.0),
        Some(JobStatus::Skipped) => (COLORS.warning, 0.4),
        None => (COLORS.info, 0.0), // pending — empty
    }
}

fn pad_right(s: &str, n: usize) -> String {
    let mut out: String = s.chars().take(n).collect();
    while out.chars().count() < n {
        out.push(' ');
    }
    out
}

fn pad_left(s: &str, n: usize) -> String {
    let count = s.chars().count();
    if count >= n {
        return s.to_string();
    }
    let mut out = String::new();
    for _ in 0..(n - count) {
        out.push(' ');
    }
    out.push_str(s);
    out
}

fn summarise(rows: &[TimingRow]) -> String {
    let names: Vec<&str> = rows
        .iter()
        .filter(|r| matches!(r.status, Some(JobStatus::Success | JobStatus::Failure)))
        .map(|r| r.name)
        .collect();
    if names.is_empty() {
        "(awaiting first job)".to_string()
    } else {
        names.join(" → ")
    }
}
