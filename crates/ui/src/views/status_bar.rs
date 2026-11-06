// Status bar — two rows: the screen's keys as raised push buttons with
// their drop shadows underneath, and the runtime and counts at the right
// end of the top row.
use super::{TAB_DAG, TAB_EXECUTION, TAB_LOGS, TAB_RUNS, TAB_SECRETS, TAB_TRIGGER, TAB_WORKFLOWS};
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
    let cell = Style::default();
    let rows = Layout::default()
        .direction(Direction::Vertical)
        .constraints([Constraint::Length(1), Constraint::Length(1)])
        .split(area);
    let (area, shadow_row) = (rows[0], rows[1]);
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
        Span::styled(" ", cell),
        Span::styled(dot, cell.fg(dot_color)),
        Span::styled(
            app.runtime_type_name().to_string(),
            cell.fg(runtime_kind.fg()).add_modifier(Modifier::BOLD),
        ),
        Span::styled(" ", cell),
    ];
    let mode = if app.validation_mode {
        vec![
            theme::badge_solid("VALIDATE", BadgeKind::Warning),
            Span::raw(" "),
        ]
    } else {
        Vec::new()
    };
    let count = vec![Span::styled(
        format!(" {} workflows ", app.workflows.len()),
        theme::muted_style(),
    )];

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
        let room = width.saturating_sub(6 + text::spans_width(&mode) + text::spans_width(&runtime));
        left.push(Span::styled(
            format!(
                " {} ",
                text::truncate(&text::sanitize(message), room).into_owned()
            ),
            cell.fg(kind.fg()).add_modifier(Modifier::BOLD),
        ));
    } else {
        // The tab strip already names the screen; the bar only says
        // what the keys do.
        left.push(Span::raw(" "));
    }

    // Reserve room for the first two hints; they say what the screen
    // does, which matters more than the counters on the right.
    let hints: Vec<(&str, &str, bool)> = context_hints(app)
        .into_iter()
        .map(|(key, label)| (key, label, false))
        .collect();
    let reserved = if app.status_message.is_none() {
        text::spans_width(&theme::raised_buttons(&hints[..hints.len().min(2)], usize::MAX).0)
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
        let (faces, shadows) = theme::raised_buttons(&hints, room);
        left.extend(faces);
        let mut under = vec![Span::raw(" ")];
        under.extend(shadows);
        f.render_widget(Paragraph::new(Line::from(under)), shadow_row);
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
        TAB_WORKFLOWS if app.running => vec![("2", "watch run"), ("l", "log"), ("?", "keys")],
        TAB_WORKFLOWS => vec![
            ("Enter", "run"),
            ("Space", "select"),
            ("J", "pick job"),
            (
                "v",
                if app.validation_mode {
                    "execute"
                } else {
                    "validate"
                },
            ),
            ("e", "runtime"),
            (
                "d",
                if app.diff_filter_active {
                    "diff off"
                } else {
                    "diff"
                },
            ),
            ("t", "dispatch"),
            ("?", "keys"),
        ],
        TAB_EXECUTION if app.detailed_view => vec![
            ("↑↓", "step"),
            ("PgUp/PgDn", "scroll"),
            ("Tab", "output/matrix"),
            ("Esc", "back"),
        ],
        TAB_EXECUTION => vec![
            ("↑↓", "job"),
            ("Enter", "inspect"),
            ("g", "graph"),
            ("l", "log"),
            ("?", "keys"),
        ],
        TAB_DAG => vec![
            ("g", if app.dag_list_view { "graph" } else { "list" }),
            ("←→", "scroll"),
            ("↑↓", "workflow"),
            ("Enter", "jobs"),
            ("l", "log"),
        ],
        TAB_LOGS if app.log_search_active => vec![("Enter", "keep"), ("Esc", "cancel")],
        TAB_LOGS if app.log_story => vec![
            ("o", "raw log"),
            ("↑↓", "scroll"),
            ("End", "follow"),
            ("s", "search raw"),
            ("x", "jobs"),
        ],
        TAB_LOGS => vec![
            ("o", "story"),
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
        TAB_SECRETS => vec![("e", "cycle runtime"), ("↑↓", "provider"), ("?", "keys")],
        TAB_RUNS => vec![("↑↓", "select"), ("?", "keys")],
        _ => vec![],
    }
}
