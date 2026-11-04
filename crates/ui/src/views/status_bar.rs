// Status bar — left-aligned key chips, right-aligned runtime + meta.
use super::{
    TAB_DAG, TAB_EXECUTION, TAB_HELP, TAB_LOGS, TAB_RUNS, TAB_SECRETS, TAB_TRIGGER, TAB_WORKFLOWS,
};
use crate::app::App;
use crate::models::StatusSeverity;
use crate::text;
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
    f.render_widget(
        Paragraph::new("").style(Style::default().bg(COLORS.bg_bar)),
        area,
    );
    let width = area.width as usize;

    // Right: mode, runtime with its availability dot, workflow count.
    // Built as tiers so a narrow bar sheds the least useful part first.
    let runtime_kind = match app.runtime_type {
        RuntimeType::Auto => BadgeKind::Emulation,
        RuntimeType::Docker => BadgeKind::Docker,
        RuntimeType::Podman => BadgeKind::Podman,
        RuntimeType::Kubernetes => BadgeKind::Kubernetes,
        RuntimeType::SecureEmulation => BadgeKind::Secure,
        RuntimeType::Emulation => BadgeKind::Emulation,
    };
    // Emulation needs no daemon, so only container and cluster runtimes
    // get the reachability dot.
    let probed = matches!(
        app.runtime_type,
        RuntimeType::Docker | RuntimeType::Podman | RuntimeType::Kubernetes
    );
    let (dot, dot_color) = match (probed, app.runtime_available) {
        (false, _) => ("◆ ", runtime_kind.fg()),
        (true, true) => ("● ", COLORS.success),
        (true, false) => ("○ ", COLORS.error),
    };
    let runtime = vec![
        Span::styled(dot, Style::default().fg(dot_color)),
        Span::styled(
            app.runtime_type_name().to_string(),
            Style::default()
                .fg(runtime_kind.fg())
                .add_modifier(Modifier::BOLD),
        ),
        Span::raw(" "),
    ];
    let mode = if app.validation_mode {
        vec![
            theme::badge_solid("VALIDATE", BadgeKind::Warning),
            Span::raw(" "),
        ]
    } else {
        Vec::new()
    };
    let count = vec![
        Span::styled(
            format!("{} workflows", app.workflows.len()),
            theme::muted_style(),
        ),
        Span::styled("  ", theme::muted_style()),
    ];

    // Left: either the current toast or the mode pill and key hints.
    let mut left: Vec<Span> = Vec::new();
    if let Some(message) = &app.status_message {
        let (label, kind) = match app.status_message_severity {
            StatusSeverity::Success => (" ✔ ", BadgeKind::Success),
            StatusSeverity::Info => (" ● ", BadgeKind::Info),
            StatusSeverity::Warning => (" ⚠ ", BadgeKind::Warning),
            StatusSeverity::Error => (" ✖ ", BadgeKind::Error),
        };
        left.push(Span::styled(
            label,
            Style::default()
                .bg(kind.fg())
                .fg(COLORS.bg_dark)
                .add_modifier(Modifier::BOLD),
        ));
        left.push(Span::raw(" "));
        let room = width.saturating_sub(5 + text::spans_width(&mode) + text::spans_width(&runtime));
        left.push(Span::styled(
            text::truncate(&text::sanitize(message), room).into_owned(),
            Style::default().fg(kind.fg()).add_modifier(Modifier::BOLD),
        ));
    } else {
        left.push(Span::styled(
            format!(
                " {} ",
                super::title_bar::TAB_LABELS[app.selected_tab].to_uppercase()
            ),
            Style::default()
                .bg(theme::current_accent())
                .fg(COLORS.bg_dark)
                .add_modifier(Modifier::BOLD),
        ));
        left.push(Span::raw("  "));
    }

    // Reserve room for the first two hints; they say what the screen
    // does, which matters more than the counters on the right.
    let hints = context_hints(app);
    let reserved = if app.status_message.is_none() {
        text::spans_width(&theme::hint_spans(&hints[..hints.len().min(2)], usize::MAX))
    } else {
        0
    };
    let mut right: Vec<Span> = Vec::new();
    let left_w = text::spans_width(&left) + reserved;
    let tiers: &[&Vec<Span>] = if app.status_message.is_some() {
        &[&mode, &runtime]
    } else {
        &[&mode, &runtime, &count]
    };
    for tier in tiers {
        if left_w + text::spans_width(&right) + text::spans_width(tier) < width {
            right.extend(tier.iter().cloned());
        }
    }
    // Order on screen is mode · runtime · count regardless of what fit.
    let right_w = text::spans_width(&right);

    if app.status_message.is_none() {
        let room = width.saturating_sub(text::spans_width(&left) + right_w + 1);
        left.extend(theme::hint_spans(&hints, room));
    }

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

fn context_hints(app: &App) -> Vec<(&'static str, &'static str)> {
    match app.selected_tab {
        TAB_WORKFLOWS if app.job_selection_mode => {
            vec![("Enter", "run job"), ("a", "run all"), ("Esc", "back")]
        }
        TAB_WORKFLOWS if app.running => vec![("2", "watch run"), ("4", "logs"), ("?", "help")],
        TAB_WORKFLOWS => vec![
            ("Enter", "run"),
            ("Space", "select"),
            ("r", "run selected"),
            ("J", "pick job"),
            (
                "v",
                if app.validation_mode {
                    "execute mode"
                } else {
                    "validate mode"
                },
            ),
            ("e", "runtime"),
            (
                "d",
                if app.diff_filter_active {
                    "diff off"
                } else {
                    "diff filter"
                },
            ),
            ("t", "trigger"),
            ("?", "help"),
        ],
        TAB_EXECUTION if app.detailed_view => vec![
            ("↑↓", "step"),
            ("PgUp/PgDn", "scroll"),
            ("Tab", "output/matrix"),
            ("Esc", "back"),
        ],
        TAB_EXECUTION => vec![("↑↓", "job"), ("Enter", "inspect"), ("?", "help")],
        TAB_DAG => vec![
            ("g", if app.dag_list_view { "graph" } else { "list" }),
            ("←→", "scroll"),
            ("↑↓", "workflow"),
            ("Enter", "open run"),
        ],
        TAB_LOGS if app.log_search_active => vec![("Enter", "keep"), ("Esc", "cancel")],
        TAB_LOGS => vec![
            ("s", "search"),
            ("f", "filter"),
            ("n", "next match"),
            ("c", "clear"),
            ("End", "follow"),
        ],
        TAB_TRIGGER if app.trigger_editing() => {
            vec![("Tab", "next field"), ("Enter", "done"), ("Esc", "cancel")]
        }
        TAB_TRIGGER => vec![
            ("Enter", "dispatch"),
            ("↑↓", "workflow"),
            ("p", "platform"),
            ("b", "branch"),
            ("i", "inputs"),
            ("+", "input"),
            ("c", "copy curl"),
        ],
        TAB_SECRETS => vec![("↑↓", "provider"), ("e", "runtime"), ("?", "help")],
        TAB_RUNS => vec![("↑↓", "select"), ("?", "help")],
        TAB_HELP => vec![("↑↓", "scroll"), ("q", "quit")],
        _ => vec![],
    }
}
