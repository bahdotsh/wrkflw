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
// Repo info is resolved lazily per draw (cheap: reads `git remote`)
// so the pane reflects the current working directory even if the
// user cd's between runs. If either resolution fails — e.g. the user
// is outside a git repo, or has no `origin` — we surface the error
// in place rather than fake a plausible-looking repo name.

use crate::app::{App, TriggerPlatform};
use crate::theme::{self, BadgeKind, COLORS};
use ratatui::{
    layout::{Alignment, Constraint, Direction, Layout, Rect},
    style::{Modifier, Style},
    text::{Line, Span},
    widgets::{Paragraph, Wrap},
    Frame,
};

pub fn render_trigger_tab(f: &mut Frame<'_>, app: &App, area: Rect) {
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

fn render_header(f: &mut Frame<'_>, app: &App, area: Rect) {
    let auth_state = match app.trigger_platform {
        TriggerPlatform::Github => std::env::var("GITHUB_TOKEN")
            .ok()
            .map(|_| ("authenticated", BadgeKind::Success))
            .unwrap_or(("GITHUB_TOKEN missing", BadgeKind::Error)),
        TriggerPlatform::Gitlab => std::env::var("GITLAB_TOKEN")
            .ok()
            .map(|_| ("authenticated", BadgeKind::Success))
            .unwrap_or(("GITLAB_TOKEN missing", BadgeKind::Error)),
    };

    let header = Line::from(vec![
        Span::styled(
            "TRIGGER REMOTE",
            Style::default()
                .fg(COLORS.trigger)
                .add_modifier(Modifier::BOLD),
        ),
        Span::styled(
            format!("  ·  dispatch workflow on {}  ·  ", app.trigger_platform.as_str()),
            Style::default().fg(COLORS.text_muted),
        ),
        theme::badge_outline(auth_state.0, auth_state.1),
    ]);
    f.render_widget(
        Paragraph::new(header).alignment(Alignment::Left),
        area,
    );
}

/// Resolved repo info; falls back to a placeholder on error so the
/// tab stays readable while informing the user what's wrong.
struct Target {
    platform_label: String,
    repo_label: String,
    default_branch: String,
    /// Non-fatal note surfaced in the UI when repo resolution fails.
    note: Option<String>,
}

fn resolve_target(app: &App) -> Target {
    match app.trigger_platform {
        TriggerPlatform::Github => match wrkflw_github::get_repo_info() {
            Ok(info) => Target {
                platform_label: "GitHub".to_string(),
                repo_label: format!("{}/{}", info.owner, info.repo),
                default_branch: info.default_branch,
                note: None,
            },
            Err(e) => Target {
                platform_label: "GitHub".to_string(),
                repo_label: "<unresolved>".to_string(),
                default_branch: "main".to_string(),
                note: Some(e.to_string()),
            },
        },
        TriggerPlatform::Gitlab => match wrkflw_gitlab::get_repo_info() {
            Ok(info) => Target {
                platform_label: "GitLab".to_string(),
                repo_label: format!("{}/{}", info.namespace, info.project),
                default_branch: info.default_branch,
                note: None,
            },
            Err(e) => Target {
                platform_label: "GitLab".to_string(),
                repo_label: "<unresolved>".to_string(),
                default_branch: "main".to_string(),
                note: Some(e.to_string()),
            },
        },
    }
}

fn render_target_pane(f: &mut Frame<'_>, app: &App, area: Rect) {
    let block = theme::block_focused("Target");
    let inner = block.inner(area);
    f.render_widget(block, area);

    let target = resolve_target(app);
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
        Span::styled("   press `p` to toggle", Style::default().fg(COLORS.text_muted)),
    ]));
    lines.push(Line::from(""));

    // Target rows.
    lines.push(field_row("Platform", &target.platform_label));
    lines.push(field_row("Repository", &target.repo_label));
    let wf_label = app
        .trigger_selected_workflow_name()
        .unwrap_or("<no workflow — add one>");
    lines.push(field_row_hl(
        "Workflow",
        wf_label,
        format!(
            "{}/{}",
            app.trigger_workflow_idx + 1,
            app.workflows.len().max(1)
        ),
    ));
    let branch_display = if app.trigger_branch.is_empty() {
        format!("(default: {})", target.default_branch)
    } else {
        app.trigger_branch.clone()
    };
    lines.push(field_row("Branch / ref", &branch_display));
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
            let editing = app.trigger_input_cursor == i;
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
                    .bg(COLORS.accent)
                    .add_modifier(Modifier::BOLD)
            } else {
                Style::default().fg(COLORS.accent)
            };
            let v_style = if v_focus {
                Style::default()
                    .fg(COLORS.bg_dark)
                    .bg(COLORS.accent)
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

    let lines: Vec<Line> = app
        .trigger_curl_preview()
        .split(" \\")
        .map(|s| Line::from(Span::styled(
            s.trim().to_string(),
            Style::default().fg(COLORS.text),
        )))
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

fn field_row_hl<'a>(label: &'a str, value: &'a str, hint: String) -> Line<'a> {
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
        Span::styled(format!("  [{}]", hint), Style::default().fg(COLORS.text_dim)),
    ])
}
