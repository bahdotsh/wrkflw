// Bottom-chrome helpers.
//
// The bottom border of the outer frame carries context-sensitive key hints
// on the left and a position indicator on the right. When `App` has a
// status_message set, `toast_title` replaces the normal bottom chrome with
// a single colored banner line.

use crate::app::App;
use crate::models::StatusSeverity;
use ratatui::{
    style::{Modifier, Style},
    text::{Line, Span},
};

/// When a toast is set, render the whole bottom chrome as a colored banner.
pub fn toast_title(app: &App) -> Option<Line<'static>> {
    let message = app.status_message.as_deref()?;
    let t = &app.theme;

    let (bg, marker) = match app.status_message_severity {
        StatusSeverity::Success => (t.success, "✓"),
        StatusSeverity::Info => (t.info, "ℹ"),
        StatusSeverity::Warning => (t.warning, "!"),
        StatusSeverity::Error => (t.error, "✖"),
    };

    Some(Line::from(vec![
        Span::styled(
            format!(" {} ", marker),
            Style::default()
                .bg(bg)
                .fg(t.fg_badge)
                .add_modifier(Modifier::BOLD),
        ),
        Span::styled(
            format!(" {} ", message),
            Style::default().bg(bg).fg(t.fg_badge),
        ),
    ]))
}

/// Left-aligned bottom-border title: a sequence of `[key] desc` pairs
/// appropriate to the current tab / mode.
pub fn bottom_left_title(app: &App) -> Line<'static> {
    let t = &app.theme;
    let pairs = current_pairs(app);

    let key_style = Style::default()
        .bg(t.bg_key_badge)
        .fg(t.highlight)
        .add_modifier(Modifier::BOLD);
    let desc_style = Style::default().fg(t.help_hint);

    let mut spans: Vec<Span<'static>> = vec![Span::raw(" ")];
    for (key, desc) in pairs {
        spans.push(Span::styled(format!(" {} ", key), key_style));
        spans.push(Span::styled(format!(" {}", desc), desc_style));
        spans.push(Span::raw(" "));
    }
    Line::from(spans)
}

/// Right-aligned bottom-border title: `ws 3/5`, `job 2/7`, etc.
pub fn bottom_right_title(app: &App) -> Line<'static> {
    let t = &app.theme;
    let text = position_text(app);
    Line::from(vec![Span::styled(
        format!(" {} ", text),
        Style::default().fg(t.fg_dim),
    )])
}

fn position_text(app: &App) -> String {
    match app.selected_tab {
        0 => {
            let total = app.workflows.len();
            let idx = app.workflow_list_state.selected().unwrap_or(0);
            if total == 0 {
                "no workflows".to_string()
            } else if app.job_selection_mode {
                let jobs = app.available_jobs.len().max(1);
                format!("job {}/{}", app.selected_job_index + 1, jobs)
            } else {
                format!("ws {}/{}", idx + 1, total)
            }
        }
        1 => {
            if app.detailed_view {
                let step = app.step_table_state.selected().unwrap_or(0) + 1;
                format!("step {}", step)
            } else {
                let job = app.job_list_state.selected().unwrap_or(0) + 1;
                format!("job {}", job)
            }
        }
        2 => {
            let n = app.processed_logs.len();
            if n == 0 {
                "no logs".to_string()
            } else {
                format!("log {}/{}", (app.log_scroll + 1).min(n), n)
            }
        }
        _ => String::new(),
    }
}

fn current_pairs(app: &App) -> &'static [(&'static str, &'static str)] {
    match app.selected_tab {
        0 => {
            if app.job_selection_mode {
                &[
                    ("↵", "run"),
                    ("a", "all"),
                    ("Esc", "back"),
                    ("?", "help"),
                    ("q", "quit"),
                ]
            } else if app.diff_filter_active {
                &[
                    ("Space", "toggle"),
                    ("↵", "run"),
                    ("r", "queue"),
                    ("t", "trig"),
                    ("d", "diff:on"),
                    ("?", "help"),
                    ("q", "quit"),
                ]
            } else {
                &[
                    ("Space", "toggle"),
                    ("↵", "run"),
                    ("r", "queue"),
                    ("t", "trig"),
                    ("d", "diff"),
                    ("?", "help"),
                    ("q", "quit"),
                ]
            }
        }
        1 => {
            if app.detailed_view {
                &[
                    ("Esc", "back"),
                    ("↑↓", "steps"),
                    ("?", "help"),
                    ("q", "quit"),
                ]
            } else {
                &[
                    ("↵", "details"),
                    ("↑↓", "jobs"),
                    ("?", "help"),
                    ("q", "quit"),
                ]
            }
        }
        2 => &[
            ("↑↓", "scroll"),
            ("s", "search"),
            ("f", "filter"),
            ("c", "clear"),
            ("?", "help"),
            ("q", "quit"),
        ],
        _ => &[("?", "help"), ("q", "quit")],
    }
}
