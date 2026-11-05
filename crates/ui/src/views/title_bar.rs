// Title bar — two rows: the wordmark, the numbered tabs as raised push
// buttons with their drop shadows underneath, and a right-side LIVE timer.
// Tabs collapse to numbers on narrow terminals so all of them stay reachable.
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

// Screen indices. Kept as `usize` constants rather than an enum so they
// drop into the existing `selected_tab: usize` comparisons and
// `switch_tab(usize)` calls without conversion.
pub const TAB_WORKFLOWS: usize = 0;
pub const TAB_EXECUTION: usize = 1;
pub const TAB_DAG: usize = 2;
pub const TAB_LOGS: usize = 3;
pub const TAB_TRIGGER: usize = 4;
pub const TAB_SECRETS: usize = 5;
pub const TAB_RUNS: usize = 6;
pub const SCREEN_COUNT: usize = 7;

/// The visible tabs and the screens each one holds. Jobs, graph and log
/// are three views of the same run, so they share the Run tab.
pub const TABS: [(&str, &[usize]); 5] = [
    ("Workflows", &[TAB_WORKFLOWS]),
    ("Run", &[TAB_EXECUTION, TAB_DAG, TAB_LOGS]),
    ("History", &[TAB_RUNS]),
    ("Dispatch", &[TAB_TRIGGER]),
    ("Setup", &[TAB_SECRETS]),
];
pub const TAB_COUNT: usize = TABS.len();
/// Index of the Run tab in `TABS`.
pub const TAB_RUN: usize = 1;

/// The visible tab that holds `screen`.
pub fn tab_of(screen: usize) -> usize {
    TABS.iter()
        .position(|(_, screens)| screens.contains(&screen))
        .unwrap_or(0)
}

pub fn render_title_bar(f: &mut Frame<'_>, app: &App, area: Rect) {
    let rows = Layout::default()
        .direction(Direction::Vertical)
        .constraints([Constraint::Length(1), Constraint::Length(1)])
        .split(area);
    let (area, shadow_row) = (rows[0], rows[1]);
    let width = area.width as usize;

    // The wordmark sits in a dark cell at the left end, like a system menu.
    let brand = vec![
        Span::styled(
            " ∿ wrkflw ",
            Style::default()
                .bg(COLORS.bg_dark)
                .fg(theme::current_accent())
                .add_modifier(Modifier::BOLD),
        ),
        Span::raw(" "),
    ];

    let mut right: Vec<Span> = Vec::new();
    if let Some(elapsed) = live_elapsed(app) {
        let cell = Style::default().bg(COLORS.bg_dark);
        right.push(Span::styled(
            " ● ",
            theme::pulse_style(app.spinner_frame).bg(COLORS.bg_dark),
        ));
        right.push(Span::styled(
            "LIVE ",
            cell.fg(COLORS.text).add_modifier(Modifier::BOLD),
        ));
        right.push(Span::styled(
            format!("{elapsed} "),
            cell.fg(COLORS.text_dim),
        ));
    }

    let brand_w = text::spans_width(&brand);
    let mut right_w = text::spans_width(&right);
    let selected = tab_of(app.selected_tab);
    let mut tabs = fit_tabs(selected, width.saturating_sub(brand_w + right_w));
    // The tabs matter more than the timer, which the Execution tab also shows.
    if text::spans_width(&tabs.0) > width.saturating_sub(brand_w + right_w) {
        right.clear();
        right_w = 0;
        tabs = fit_tabs(selected, width.saturating_sub(brand_w));
    }
    let (faces, shadows) = tabs;

    let (mut left, mut under) = (Vec::new(), Vec::new());
    if brand_w + text::spans_width(&faces) <= width {
        left.extend(brand);
        under.push(Span::raw(" ".repeat(brand_w)));
    }
    left.extend(faces);
    under.extend(shadows);

    let chunks = Layout::default()
        .direction(Direction::Horizontal)
        .constraints([Constraint::Min(0), Constraint::Length(right_w as u16)])
        .split(area);
    f.render_widget(Paragraph::new(Line::from(left)), chunks[0]);
    f.render_widget(
        Paragraph::new(Line::from(right)).alignment(Alignment::Right),
        chunks[1],
    );
    f.render_widget(Paragraph::new(Line::from(under)), shadow_row);
}

/// The roomiest strip of tab buttons that fits in `budget` cells, as the
/// button faces and the shadow row beneath them.
fn fit_tabs(selected: usize, budget: usize) -> (Vec<Span<'static>>, Vec<Span<'static>>) {
    let mut tabs = tab_buttons(selected, TabStyle::Full);
    for style in [TabStyle::Compact, TabStyle::ActiveOnly] {
        if text::spans_width(&tabs.0) <= budget {
            break;
        }
        tabs = tab_buttons(selected, style);
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

/// The tabs as raised push buttons, the open one lit.
fn tab_buttons(selected: usize, style: TabStyle) -> (Vec<Span<'static>>, Vec<Span<'static>>) {
    let keys: Vec<String> = (1..=TABS.len()).map(|n| n.to_string()).collect();
    let buttons: Vec<(&str, &str, bool)> = TABS
        .iter()
        .enumerate()
        .filter_map(|(i, (label, _))| {
            let lit = i == selected;
            match style {
                _ if lit => Some((keys[i].as_str(), *label, true)),
                TabStyle::Full => Some((keys[i].as_str(), *label, false)),
                TabStyle::Compact => Some((keys[i].as_str(), "", false)),
                TabStyle::ActiveOnly => None,
            }
        })
        .collect();
    theme::raised_buttons(&buttons, usize::MAX)
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
