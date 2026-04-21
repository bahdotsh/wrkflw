// Remote trigger — screen 8 from the design.
//
// Two-pane layout:
//   - Left:  target form (platform · repo · workflow · branch · token · inputs)
//   - Right: live curl-equivalent preview of the POST we'd send
//
// Backing features that already exist and this tab binds to:
//   - wrkflw_github::get_repo_info (git `origin` → owner/repo/default_branch)
//   - wrkflw_github::trigger_workflow (workflow_dispatch)
//   - wrkflw_gitlab::get_repo_info  (same, GitLab flavour)
//   - wrkflw_gitlab::trigger_pipeline
//
// Repo info is resolved once per platform and cached on `App`
// (`trigger_tab_target`) so we don't re-shell `git remote` on every
// render. The cache is invalidated on platform toggle.

use crate::app::{App, TriggerPlatform};
use crate::theme::{self, BadgeKind, COLORS};
use ratatui::{
    layout::{Alignment, Constraint, Direction, Layout, Rect},
    style::{Modifier, Style},
    text::{Line, Span},
    widgets::{Paragraph, Wrap},
    Frame,
};

pub fn render_trigger_tab(f: &mut Frame<'_>, app: &mut App, area: Rect) {
    let outer = Layout::default()
        .direction(Direction::Vertical)
        .constraints([Constraint::Length(2), Constraint::Min(0)])
        .split(area);

    render_header(f, app, outer[0]);

    let body = Layout::default()
        .direction(Direction::Horizontal)
        .constraints([Constraint::Percentage(50), Constraint::Min(0)])
        .split(outer[1]);

    render_target_pane(f, app, body[0]);
    render_preview_pane(f, app, body[1]);
}

/// An env token we treat as "set". Empty string doesn't count — users
/// occasionally `export GITHUB_TOKEN=` to clear the value without
/// unsetting the var, and calling that "authenticated" would mislead.
fn token_is_set(var: &str) -> bool {
    std::env::var(var).ok().is_some_and(|v| !v.is_empty())
}

fn render_header(f: &mut Frame<'_>, app: &App, area: Rect) {
    let auth_state = match app.trigger_platform {
        TriggerPlatform::Github => {
            if token_is_set("GITHUB_TOKEN") {
                ("authenticated", BadgeKind::Success)
            } else {
                ("GITHUB_TOKEN missing", BadgeKind::Error)
            }
        }
        TriggerPlatform::Gitlab => {
            if token_is_set("GITLAB_TOKEN") {
                ("authenticated", BadgeKind::Success)
            } else {
                ("GITLAB_TOKEN missing", BadgeKind::Error)
            }
        }
    };

    let header = Line::from(vec![
        Span::styled(
            "TRIGGER REMOTE",
            Style::default()
                .fg(COLORS.trigger)
                .add_modifier(Modifier::BOLD),
        ),
        Span::styled(
            format!(
                "  ·  dispatch workflow on {}  ·  ",
                app.trigger_platform.as_str()
            ),
            Style::default().fg(COLORS.text_muted),
        ),
        theme::badge_outline(auth_state.0, auth_state.1),
    ]);
    f.render_widget(Paragraph::new(header).alignment(Alignment::Left), area);
}

fn render_target_pane(f: &mut Frame<'_>, app: &mut App, area: Rect) {
    let block = theme::block_focused("Target");
    let inner = block.inner(area);
    f.render_widget(block, area);

    // Clone out of the cache so we don't hold an immutable borrow of
    // `app` while the rest of this function reads other fields. The
    // clone is a few small strings; cheap.
    let target = app.trigger_tab_target().clone();
    let mut lines: Vec<Line> = Vec::new();

    // Platform row — pill group.
    lines.push(Line::from(vec![Span::styled(
        "PLATFORM",
        Style::default()
            .fg(COLORS.text_muted)
            .add_modifier(Modifier::BOLD),
    )]));
    let mk_pill = |label: &str, kind: BadgeKind, active: bool| -> Span<'_> {
        if active {
            theme::badge_solid(label.to_string(), kind)
        } else {
            theme::badge_outline(label.to_string(), kind)
        }
    };
    lines.push(Line::from(vec![
        mk_pill(
            "github",
            BadgeKind::Trigger,
            matches!(app.trigger_platform, TriggerPlatform::Github),
        ),
        Span::raw(" "),
        mk_pill(
            "gitlab",
            BadgeKind::Warning,
            matches!(app.trigger_platform, TriggerPlatform::Gitlab),
        ),
        Span::styled(
            "   press `p` to toggle",
            Style::default().fg(COLORS.text_muted),
        ),
    ]));
    lines.push(Line::from(""));

    // Target rows.
    lines.push(field_row("Platform", &target.platform_label));
    lines.push(field_row("Repository", &target.repo_label));
    let wf_label = app
        .trigger_selected_workflow_name()
        .unwrap_or("<no workflow — add one>");
    let wf_hint = format!(
        "{}/{}",
        app.trigger_workflow_idx + 1,
        app.workflows.len().max(1)
    );
    lines.push(field_row_hl("Workflow", wf_label, &wf_hint));
    let branch_display = if app.trigger_branch.is_empty() {
        if app.trigger_branch_focused {
            // Focused but no characters typed yet — show an empty
            // edit caret rather than the resolved default so the
            // user can see they're starting fresh.
            "_".to_string()
        } else {
            format!("(default: {})", target.default_branch)
        }
    } else {
        app.trigger_branch.clone()
    };
    if app.trigger_branch_focused {
        lines.push(Line::from(vec![
            Span::styled(
                format!("  {:<14}", "Branch / ref"),
                Style::default().fg(COLORS.text_muted),
            ),
            Span::styled(
                branch_display,
                Style::default()
                    .fg(COLORS.bg_dark)
                    .bg(theme::current_accent())
                    .add_modifier(Modifier::BOLD),
            ),
            Span::styled(
                "  (Enter/Esc to commit — Esc clears)",
                Style::default().fg(COLORS.text_dim),
            ),
        ]));
    } else {
        lines.push(field_row("Branch / ref", &branch_display));
    }
    lines.push(field_row(
        "Token",
        match app.trigger_platform {
            TriggerPlatform::Github => "$GITHUB_TOKEN",
            TriggerPlatform::Gitlab => "$GITLAB_TOKEN",
        },
    ));

    if let Some(note) = target.note {
        lines.push(Line::from(""));
        lines.push(Line::from(vec![
            theme::badge_outline("warn", BadgeKind::Warning),
            Span::raw(" "),
            Span::styled(note, Style::default().fg(COLORS.text_dim)),
        ]));
    }

    lines.push(Line::from(""));
    lines.push(Line::from(vec![Span::styled(
        "INPUTS",
        Style::default()
            .fg(COLORS.highlight)
            .add_modifier(Modifier::BOLD),
    )]));
    if app.trigger_inputs.is_empty() {
        lines.push(Line::from(vec![Span::styled(
            "  (none)  —  press `+` to add a key=value input",
            Style::default().fg(COLORS.text_muted),
        )]));
    } else {
        for (i, (k, v)) in app.trigger_inputs.iter().enumerate() {
            let editing = app.trigger_input_cursor == Some(i);
            let k_focus = editing && !app.trigger_input_on_value;
            let v_focus = editing && app.trigger_input_on_value;
            let k_display = if k.is_empty() && !k_focus {
                "<key>".to_string()
            } else {
                k.clone()
            };
            let v_display = if v.is_empty() && !v_focus {
                "<value>".to_string()
            } else {
                v.clone()
            };
            let k_style = if k_focus {
                Style::default()
                    .fg(COLORS.bg_dark)
                    .bg(theme::current_accent())
                    .add_modifier(Modifier::BOLD)
            } else {
                Style::default().fg(theme::current_accent())
            };
            let v_style = if v_focus {
                Style::default()
                    .fg(COLORS.bg_dark)
                    .bg(theme::current_accent())
                    .add_modifier(Modifier::BOLD)
            } else {
                Style::default().fg(COLORS.text)
            };
            lines.push(Line::from(vec![
                Span::raw("  "),
                Span::styled(k_display, k_style),
                Span::styled(" = ", Style::default().fg(COLORS.text_muted)),
                Span::styled(v_display, v_style),
            ]));
        }
    }

    lines.push(Line::from(""));
    lines.push(Line::from(vec![
        theme::key_chip("p"),
        Span::raw(" "),
        Span::styled("platform", Style::default().fg(COLORS.text_dim)),
        Span::raw("   "),
        theme::key_chip("↑↓"),
        Span::raw(" "),
        Span::styled("workflow", Style::default().fg(COLORS.text_dim)),
        Span::raw("   "),
        theme::key_chip("b"),
        Span::raw(" "),
        Span::styled("edit branch", Style::default().fg(COLORS.text_dim)),
        Span::raw("   "),
        theme::key_chip("+"),
        Span::raw(" "),
        Span::styled("add input", Style::default().fg(COLORS.text_dim)),
        Span::raw("   "),
        theme::key_chip("tab"),
        Span::raw(" "),
        Span::styled("next field", Style::default().fg(COLORS.text_dim)),
    ]));
    lines.push(Line::from(vec![
        theme::key_chip("enter"),
        Span::raw(" "),
        Span::styled(
            "dispatch (or commit edit)",
            Style::default().fg(COLORS.text_dim),
        ),
        Span::raw("   "),
        theme::key_chip("c"),
        Span::raw(" "),
        Span::styled("copy curl → logs", Style::default().fg(COLORS.text_dim)),
    ]));

    f.render_widget(Paragraph::new(lines).wrap(Wrap { trim: false }), inner);
}

fn render_preview_pane(f: &mut Frame<'_>, app: &App, area: Rect) {
    let block = theme::block("Preview · curl equivalent");
    let inner = block.inner(area);
    f.render_widget(block, area);

    // Each flag lives on its own line (joined in `trigger_curl_preview`
    // with ` \\\n`). Splitting on `\n` gives us one ratatui Line per
    // flag so a narrow pane doesn't soft-wrap mid-header.
    let lines: Vec<Line> = app
        .trigger_curl_preview()
        .split('\n')
        .map(|s| {
            Line::from(Span::styled(
                s.to_string(),
                Style::default().fg(COLORS.text),
            ))
        })
        .collect();
    f.render_widget(Paragraph::new(lines).wrap(Wrap { trim: false }), inner);
}

fn field_row<'a>(label: &'a str, value: &'a str) -> Line<'a> {
    Line::from(vec![
        Span::styled(
            format!("  {:<14}", label),
            Style::default().fg(COLORS.text_muted),
        ),
        Span::styled(value.to_string(), Style::default().fg(COLORS.text)),
    ])
}

fn field_row_hl<'a>(label: &'a str, value: &'a str, hint: &str) -> Line<'a> {
    Line::from(vec![
        Span::styled(
            format!("  {:<14}", label),
            Style::default().fg(COLORS.text_muted),
        ),
        Span::styled(
            value.to_string(),
            Style::default()
                .fg(COLORS.text)
                .add_modifier(Modifier::BOLD),
        ),
        Span::styled(
            format!("  [{}]", hint),
            Style::default().fg(COLORS.text_dim),
        ),
    ])
}
