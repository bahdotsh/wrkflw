// Title bar — brand mark, numbered tabs, right-side LIVE + runtime indicator.
use crate::app::App;
use crate::models::WorkflowStatus;
use crate::theme::{self, BadgeKind, COLORS};
use ratatui::{
    layout::{Alignment, Constraint, Direction, Layout, Rect},
    style::{Modifier, Style},
    text::{Line, Span},
    widgets::Paragraph,
    Frame,
};
use wrkflw_executor::RuntimeType;

const TAB_LABELS: [&str; 4] = ["Workflows", "Execution", "Logs", "Help"];

pub fn render_title_bar(f: &mut Frame<'_>, app: &App, area: Rect) {
    let chunks = Layout::default()
        .direction(Direction::Horizontal)
        .constraints([
            Constraint::Length(14), // brand
            Constraint::Min(0),     // tabs
            Constraint::Length(34), // right indicators
        ])
        .split(area);

    // ─── Brand ────────────────────────────────────────────────
    let brand = Paragraph::new(Line::from(vec![
        Span::styled(" w∿w ", Style::default().fg(COLORS.accent)),
        Span::styled(
            "wrkflw",
            Style::default()
                .fg(COLORS.accent)
                .add_modifier(Modifier::BOLD),
        ),
    ]))
    .style(Style::default().bg(COLORS.bg_dark))
    .alignment(Alignment::Left);
    f.render_widget(brand, chunks[0]);

    // ─── Tabs ─────────────────────────────────────────────────
    let mut tab_spans: Vec<Span> = Vec::with_capacity(TAB_LABELS.len() * 4);
    tab_spans.push(Span::styled(" │ ", Style::default().fg(COLORS.border)));
    for (i, label) in TAB_LABELS.iter().enumerate() {
        let active = i == app.selected_tab;
        tab_spans.push(Span::styled(
            format!("{}", i + 1),
            Style::default().fg(COLORS.text_muted),
        ));
        tab_spans.push(Span::raw(" "));
        tab_spans.push(Span::styled(
            label.to_string(),
            if active {
                Style::default()
                    .fg(COLORS.text)
                    .add_modifier(Modifier::BOLD | Modifier::UNDERLINED)
            } else {
                Style::default().fg(COLORS.text_dim)
            },
        ));
        if i + 1 < TAB_LABELS.len() {
            tab_spans.push(Span::raw("   "));
        }
    }
    let tabs = Paragraph::new(Line::from(tab_spans))
        .style(Style::default().bg(COLORS.bg_dark))
        .alignment(Alignment::Left);
    f.render_widget(tabs, chunks[1]);

    // ─── Right: LIVE + runtime ────────────────────────────────
    let mut right: Vec<Span> = Vec::new();
    if let Some(elapsed) = live_elapsed(app) {
        right.push(Span::styled("●", theme::pulse_style(app.spinner_frame)));
        right.push(Span::raw(" "));
        right.push(Span::styled(
            "LIVE",
            Style::default()
                .fg(COLORS.text)
                .add_modifier(Modifier::BOLD),
        ));
        right.push(Span::raw(" "));
        right.push(Span::styled(elapsed, Style::default().fg(COLORS.text_dim)));
        right.push(Span::raw("  "));
    }
    let runtime_kind = match app.runtime_type {
        RuntimeType::Docker => BadgeKind::Docker,
        RuntimeType::Podman => BadgeKind::Podman,
        RuntimeType::SecureEmulation => BadgeKind::Secure,
        RuntimeType::Emulation => BadgeKind::Emulation,
    };
    right.push(theme::badge_outline(app.runtime_type_name(), runtime_kind));
    right.push(Span::raw(" "));

    let right_p = Paragraph::new(Line::from(right))
        .style(Style::default().bg(COLORS.bg_dark))
        .alignment(Alignment::Right);
    f.render_widget(right_p, chunks[2]);
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
