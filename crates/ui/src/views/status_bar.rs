// Status bar — left-aligned key chips, right-aligned runtime + meta.
use crate::app::App;
use crate::models::StatusSeverity;
use crate::theme::{self, BadgeKind, COLORS};
use ratatui::{
    layout::{Alignment, Constraint, Direction, Layout, Rect},
    style::{Modifier, Style},
    text::{Line, Span},
    widgets::Paragraph,
    Frame,
};
use wrkflw_executor::RuntimeType;

pub fn render_status_bar(f: &mut Frame<'_>, app: &App, area: Rect) {
    if let Some(message) = &app.status_message {
        let bg = match app.status_message_severity {
            StatusSeverity::Success => COLORS.success,
            StatusSeverity::Info => COLORS.info,
            StatusSeverity::Warning => COLORS.warning,
            StatusSeverity::Error => COLORS.error,
        };
        let toast = Paragraph::new(Line::from(vec![Span::styled(
            format!(" {} ", message),
            Style::default()
                .bg(bg)
                .fg(COLORS.bg_dark)
                .add_modifier(Modifier::BOLD),
        )]))
        .alignment(Alignment::Center)
        .style(Style::default().bg(COLORS.bg_bar));
        f.render_widget(toast, area);
        return;
    }

    let chunks = Layout::default()
        .direction(Direction::Horizontal)
        .constraints([Constraint::Min(0), Constraint::Length(48)])
        .split(area);

    // Left: key chips
    let mut left: Vec<Span> = Vec::new();
    for (i, (key, desc)) in context_hints(app).into_iter().enumerate() {
        if i > 0 {
            left.push(Span::raw("  "));
        }
        left.push(theme::key_chip(key));
        left.push(Span::styled(format!(" {}", desc), theme::hint_style()));
    }
    let left_p = Paragraph::new(Line::from(left))
        .style(Style::default().bg(COLORS.bg_bar))
        .alignment(Alignment::Left);
    f.render_widget(left_p, chunks[0]);

    // Right: validation chip + runtime badge
    let mut right: Vec<Span> = Vec::new();

    if app.validation_mode {
        right.push(theme::badge_outline("validation", BadgeKind::Warning));
    } else {
        right.push(theme::badge_outline("execution", BadgeKind::Success));
    }
    right.push(Span::raw(" "));
    let runtime_badge_kind = match app.runtime_type {
        RuntimeType::Docker => BadgeKind::Docker,
        RuntimeType::Podman => BadgeKind::Podman,
        RuntimeType::SecureEmulation => BadgeKind::Secure,
        RuntimeType::Emulation => BadgeKind::Emulation,
    };
    right.push(theme::badge_solid(
        app.runtime_type_name(),
        runtime_badge_kind,
    ));
    right.push(Span::raw(" "));
    let avail_color = if app.runtime_available {
        COLORS.success
    } else {
        COLORS.text_muted
    };
    right.push(Span::styled(
        if app.runtime_available { "●" } else { "○" }.to_string(),
        Style::default().fg(avail_color),
    ));
    right.push(Span::raw(" "));
    right.push(Span::styled(
        format!("{} workflows", app.workflows.len()),
        Style::default().fg(COLORS.text_muted),
    ));

    let right_p = Paragraph::new(Line::from(right))
        .style(Style::default().bg(COLORS.bg_bar))
        .alignment(Alignment::Right);
    f.render_widget(right_p, chunks[1]);
}

fn context_hints(app: &App) -> Vec<(&'static str, &'static str)> {
    match app.selected_tab {
        0 => {
            if app.job_selection_mode {
                vec![
                    ("Enter", "run"),
                    ("a", "all"),
                    ("Esc", "back"),
                    ("?", "help"),
                ]
            } else if app.diff_filter_active {
                vec![
                    ("↑↓", "navigate"),
                    ("Space", "select"),
                    ("Enter", "run"),
                    ("r", "queue"),
                    ("t", "trigger"),
                    ("d", "diff:ON"),
                    ("?", "help"),
                ]
            } else {
                vec![
                    ("↑↓", "navigate"),
                    ("Space", "select"),
                    ("Enter", "run"),
                    ("r", "queue"),
                    ("t", "trigger"),
                    ("d", "diff"),
                    ("?", "help"),
                ]
            }
        }
        1 => {
            if app.detailed_view {
                vec![
                    ("Tab", "switch pane"),
                    ("↑↓", "steps"),
                    ("Esc", "back"),
                    ("?", "help"),
                ]
            } else {
                vec![
                    ("j/k", "move"),
                    ("Enter", "inspect"),
                    ("/", "search"),
                    ("p", "pause"),
                    ("?", "help"),
                ]
            }
        }
        2 => vec![
            ("↑↓", "scroll"),
            ("s", "search"),
            ("f", "filter"),
            ("?", "help"),
            ("q", "quit"),
        ],
        3 => vec![("↑↓", "scroll"), ("?", "close"), ("q", "quit")],
        _ => vec![],
    }
}
