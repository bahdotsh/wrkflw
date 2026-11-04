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
use crate::text;
use crate::theme::{self, BadgeKind, COLORS};
use ratatui::{
    layout::{Constraint, Direction, Layout, Rect},
    style::{Modifier, Style},
    text::{Line, Span},
    widgets::{Paragraph, Wrap},
    Frame,
};

pub fn render_trigger_tab(f: &mut Frame<'_>, app: &mut App, area: Rect) {
    let body = if area.width >= 100 {
        Layout::default()
            .direction(Direction::Horizontal)
            .constraints([Constraint::Percentage(55), Constraint::Min(0)])
            .split(area)
    } else {
        Layout::default()
            .direction(Direction::Vertical)
            .constraints([Constraint::Min(0), Constraint::Length(9)])
            .split(area)
    };

    render_target_pane(f, app, body[0]);
    render_preview_pane(f, app, body[1]);
}

fn token_is_set(var: &str) -> bool {
    std::env::var(var).ok().is_some_and(|v| !v.is_empty())
}

fn render_target_pane(f: &mut Frame<'_>, app: &mut App, area: Rect) {
    let token_var = match app.trigger_platform {
        TriggerPlatform::Github => "GITHUB_TOKEN",
        TriggerPlatform::Gitlab => "GITLAB_TOKEN",
    };
    let token_set = token_is_set(token_var);
    let block = theme::with_badge(
        theme::block_focused("Dispatch remotely"),
        if token_set { "ready" } else { "token missing" },
    );
    let inner = block.inner(area);
    f.render_widget(block, area);
    let w = inner.width as usize;

    let target = app.trigger_tab_target().clone();
    let mut lines: Vec<Line> = Vec::new();

    let pill = |label: &'static str, kind: BadgeKind, active: bool| -> Span<'static> {
        if active {
            theme::badge_solid(label, kind)
        } else {
            Span::styled(format!(" {label} "), theme::muted_style())
        }
    };
    lines.push(Line::from(vec![
        pill(
            "GitHub",
            BadgeKind::Trigger,
            matches!(app.trigger_platform, TriggerPlatform::Github),
        ),
        Span::raw(" "),
        pill(
            "GitLab",
            BadgeKind::Warning,
            matches!(app.trigger_platform, TriggerPlatform::Gitlab),
        ),
    ]));
    lines.push(Line::from(""));

    let value_w = w.saturating_sub(LABEL_W);
    lines.push(field_row("Repository", &target.repo_label, value_w, false));
    let wf_label = app
        .trigger_selected_workflow_name()
        .unwrap_or("no workflows")
        .to_string();
    let position = if app.workflows.is_empty() {
        String::new()
    } else {
        format!(
            "  {}/{}",
            (app.trigger_workflow_idx + 1).min(app.workflows.len()),
            app.workflows.len()
        )
    };
    let mut wf = field_row(
        "Workflow",
        &wf_label,
        value_w.saturating_sub(position.len()),
        true,
    );
    wf.spans.push(Span::styled(position, theme::muted_style()));
    lines.push(wf);

    let branch_display = if app.trigger_branch.is_empty() && !app.trigger_branch_focused {
        format!("{} (default)", target.default_branch)
    } else {
        app.trigger_branch.clone()
    };
    if app.trigger_branch_focused {
        lines.push(Line::from(vec![
            label_span("Branch"),
            Span::styled(
                format!(
                    "{}▏",
                    text::truncate_left(&branch_display, value_w.saturating_sub(2))
                ),
                editing_style(),
            ),
        ]));
    } else {
        lines.push(field_row("Branch", &branch_display, value_w, false));
    }
    lines.push(Line::from(vec![
        label_span("Token"),
        if token_set {
            Span::styled(
                format!("{} ${token_var}", theme::symbols::SUCCESS),
                Style::default().fg(COLORS.success),
            )
        } else {
            Span::styled(
                format!("{} ${token_var} is not set", theme::symbols::FAILURE),
                Style::default().fg(COLORS.error),
            )
        },
    ]));

    if let Some(note) = target.note {
        lines.push(Line::from(""));
        lines.push(Line::from(vec![
            Span::styled(
                format!("{} ", theme::symbols::WARNING),
                Style::default().fg(COLORS.warning),
            ),
            Span::styled(
                text::truncate(&note, w.saturating_sub(2)).into_owned(),
                Style::default().fg(COLORS.text_dim),
            ),
        ]));
    }

    lines.push(Line::from(""));
    lines.push(theme::section(format!(
        "Inputs ({})",
        app.trigger_inputs.len()
    )));
    let inputs_top = lines.len();
    if app.trigger_inputs.is_empty() {
        lines.push(Line::from(vec![
            Span::styled("none · press ", theme::muted_style()),
            Span::styled(
                "+",
                Style::default()
                    .fg(theme::current_accent())
                    .add_modifier(Modifier::BOLD),
            ),
            Span::styled(" to add key=value", theme::muted_style()),
        ]));
    } else {
        let half = w.saturating_sub(5) / 2;
        // Align the `=` column on the longest key that fits.
        let key_w = app
            .trigger_inputs
            .iter()
            .map(|(k, _)| text::width(k).max(3) + 1)
            .max()
            .unwrap_or(4)
            .min(half);
        for (i, (k, v)) in app.trigger_inputs.iter().enumerate() {
            let editing = app.trigger_input_cursor == Some(i);
            let k_focus = editing && !app.trigger_input_on_value;
            let v_focus = editing && app.trigger_input_on_value;
            let shown = |s: &str, placeholder: &str, focus: bool, cells: usize| -> String {
                let s = text::sanitize(s);
                let raw = if s.is_empty() && !focus {
                    placeholder.to_string()
                } else if focus {
                    format!("{}▏", text::truncate_left(&s, cells.saturating_sub(1)))
                } else {
                    text::truncate(&s, cells).into_owned()
                };
                text::fit(&raw, cells)
            };
            let k_style = if k_focus {
                editing_style()
            } else if k.is_empty() {
                theme::muted_style()
            } else {
                Style::default().fg(theme::current_accent())
            };
            let v_style = if v_focus {
                editing_style()
            } else if v.is_empty() {
                theme::muted_style()
            } else {
                Style::default().fg(COLORS.text)
            };
            lines.push(Line::from(vec![
                Span::styled(
                    if editing { theme::SELECTED_MARK } else { "  " },
                    Style::default().fg(theme::current_accent()),
                ),
                Span::styled(shown(k, "key", k_focus, key_w), k_style),
                Span::styled(" = ", theme::muted_style()),
                Span::styled(
                    shown(v, "value", v_focus, w.saturating_sub(key_w + 5)),
                    v_style,
                ),
            ]));
        }
    }

    // Keep the input being edited on screen when the list is long.
    let viewport = inner.height as usize;
    let offset = app
        .trigger_input_cursor
        .map(|c| (inputs_top + c).saturating_sub(viewport.saturating_sub(1)))
        .unwrap_or(0);
    let total = lines.len();
    f.render_widget(Paragraph::new(lines).scroll((offset as u16, 0)), inner);
    theme::scrollbar(f, area, total, offset, viewport);
}

fn render_preview_pane(f: &mut Frame<'_>, app: &App, area: Rect) {
    let block = theme::block("curl equivalent");
    let inner = block.inner(area);
    f.render_widget(block, area);

    let lines: Vec<Line> = app
        .trigger_curl_preview()
        .split('\n')
        .map(|s| {
            let style = if s.trim_start().starts_with("-H") {
                theme::muted_style()
            } else {
                Style::default().fg(COLORS.text)
            };
            Line::from(Span::styled(s.to_string(), style))
        })
        .collect();
    f.render_widget(Paragraph::new(lines).wrap(Wrap { trim: false }), inner);
}

const LABEL_W: usize = 12;

fn label_span(label: &str) -> Span<'static> {
    Span::styled(format!("{label:<LABEL_W$}"), theme::muted_style())
}

fn editing_style() -> Style {
    Style::default()
        .fg(COLORS.text)
        .bg(COLORS.bg_selected)
        .add_modifier(Modifier::BOLD)
}

fn field_row(label: &str, value: &str, room: usize, bold: bool) -> Line<'static> {
    let mut style = Style::default().fg(COLORS.text);
    if bold {
        style = style.add_modifier(Modifier::BOLD);
    }
    Line::from(vec![
        label_span(label),
        Span::styled(
            text::truncate(&text::sanitize(value), room).into_owned(),
            style,
        ),
    ])
}
