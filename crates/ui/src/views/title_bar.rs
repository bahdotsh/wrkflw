// Title bar — brand mark, numbered tabs, right-side LIVE timer. Tabs
// collapse to numbers on narrow terminals so all of them stay reachable.
use crate::app::App;
use crate::models::WorkflowStatus;
use crate::text;
use crate::theme::{self, COLORS};
use ratatui::{
    layout::{Alignment, Constraint, Direction, Layout, Rect},
    style::{Modifier, Style},
    text::{Line, Span},
    widgets::Paragraph,
    Frame,
};

pub const TAB_LABELS: [&str; 8] = [
    "Workflows",
    "Execution",
    "DAG",
    "Logs",
    "Trigger",
    "Setup",
    "Runs",
    "Help",
];
pub const TAB_COUNT: usize = TAB_LABELS.len();

// Canonical tab indices. Kept as `usize` constants rather than an enum
// so they drop into the existing `selected_tab: usize` comparisons and
// `switch_tab(usize)` calls without conversion.
pub const TAB_WORKFLOWS: usize = 0;
pub const TAB_EXECUTION: usize = 1;
pub const TAB_DAG: usize = 2;
pub const TAB_LOGS: usize = 3;
pub const TAB_TRIGGER: usize = 4;
pub const TAB_SECRETS: usize = 5;
pub const TAB_RUNS: usize = 6;
pub const TAB_HELP: usize = 7;

pub fn render_title_bar(f: &mut Frame<'_>, app: &App, area: Rect) {
    f.render_widget(
        Paragraph::new("").style(Style::default().bg(COLORS.bg_bar)),
        area,
    );
    let accent = theme::current_accent();
    let width = area.width as usize;

    let brand = vec![
        Span::styled(
            " w∿w wrkflw ",
            Style::default()
                .bg(accent)
                .fg(COLORS.bg_dark)
                .add_modifier(Modifier::BOLD),
        ),
        Span::raw(" "),
    ];

    let mut right: Vec<Span> = Vec::new();
    if let Some(elapsed) = live_elapsed(app) {
        right.push(Span::styled("● ", theme::pulse_style(app.spinner_frame)));
        right.push(Span::styled(
            "LIVE ",
            Style::default()
                .fg(COLORS.text)
                .add_modifier(Modifier::BOLD),
        ));
        right.push(Span::styled(elapsed, Style::default().fg(COLORS.text_dim)));
        right.push(Span::raw(" "));
    }

    let brand_w = text::spans_width(&brand);
    let mut right_w = text::spans_width(&right);
    let mut tabs = fit_tabs(app.selected_tab, width.saturating_sub(brand_w + right_w));
    // The tabs matter more than the timer, which the Execution tab also shows.
    if text::spans_width(&tabs) > width.saturating_sub(brand_w + right_w) {
        right.clear();
        right_w = 0;
        tabs = fit_tabs(app.selected_tab, width.saturating_sub(brand_w));
    }

    let mut left = Vec::new();
    if brand_w + text::spans_width(&tabs) <= width {
        left.extend(brand);
    }
    left.extend(tabs);

    let chunks = Layout::default()
        .direction(Direction::Horizontal)
        .constraints([Constraint::Min(0), Constraint::Length(right_w as u16)])
        .split(area);
    f.render_widget(Paragraph::new(Line::from(left)), chunks[0]);
    f.render_widget(
        Paragraph::new(Line::from(right)).alignment(Alignment::Right),
        chunks[1],
    );
}

/// The roomiest tab strip that fits in `budget` cells.
fn fit_tabs(selected: usize, budget: usize) -> Vec<Span<'static>> {
    let mut tabs = tab_spans(selected, TabStyle::Full);
    for style in [TabStyle::Compact, TabStyle::ActiveOnly] {
        if text::spans_width(&tabs) <= budget {
            break;
        }
        tabs = tab_spans(selected, style);
    }
    tabs
}

#[derive(Copy, Clone)]
enum TabStyle {
    /// Every tab shows its number and label.
    Full,
    /// Only the active tab keeps its label; the rest shrink to numbers.
    Compact,
    /// Just the active tab, for very narrow terminals.
    ActiveOnly,
}

fn tab_spans(selected: usize, style: TabStyle) -> Vec<Span<'static>> {
    let mut spans = Vec::new();
    for (i, label) in TAB_LABELS.iter().enumerate() {
        let active = i == selected;
        if active {
            spans.push(Span::styled(
                format!(" {} {} ", i + 1, label),
                Style::default()
                    .bg(COLORS.bg_selected)
                    .fg(theme::current_accent())
                    .add_modifier(Modifier::BOLD),
            ));
            continue;
        }
        match style {
            TabStyle::Full => {
                spans.push(Span::styled(
                    format!(" {}", i + 1),
                    Style::default().fg(COLORS.text_muted),
                ));
                spans.push(Span::styled(
                    format!(" {} ", label),
                    Style::default().fg(COLORS.text_dim),
                ));
            }
            TabStyle::Compact => spans.push(Span::styled(
                format!(" {} ", i + 1),
                Style::default().fg(COLORS.text_muted),
            )),
            TabStyle::ActiveOnly => {}
        }
    }
    spans
}

/// Format elapsed time on the active execution as `mm:ss`. Returns `None`
/// when no workflow is currently running.
fn live_elapsed(app: &App) -> Option<String> {
    let idx = app.current_execution?;
    let wf = app.workflows.get(idx)?;
    if !matches!(wf.status, WorkflowStatus::Running) {
        return None;
    }
    let exec = wf.execution_details.as_ref()?;
    let now = chrono::Local::now();
    let elapsed = now.signed_duration_since(exec.start_time);
    let total = elapsed.num_seconds().max(0);
    Some(format!("{:02}:{:02}", total / 60, total % 60))
}
