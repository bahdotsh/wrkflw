// Top-chrome helpers.
//
// wrkflw embeds both the title bar and the status bar in the outer frame's
// rounded border (mdterm pattern). This file builds the two `Line`s that sit
// on the top border: a left-aligned identity strip (brand · summary · mode
// pill) and a right-aligned tab breadcrumb.

use crate::app::App;
use crate::models::WorkflowStatus;
use ratatui::{
    style::{Modifier, Style},
    text::{Line, Span},
};
use wrkflw_executor::RuntimeType;

/// Left-aligned top-border title: `  wrkflw · N workflows · EXECUTION `.
pub fn top_left_title(app: &App) -> Line<'static> {
    let t = &app.theme;
    let mut spans: Vec<Span<'static>> = Vec::new();

    spans.push(Span::styled(
        "  wrkflw ".to_string(),
        Style::default().fg(t.accent).add_modifier(Modifier::BOLD),
    ));
    spans.push(Span::styled(
        "│ ".to_string(),
        Style::default().fg(t.border_dim),
    ));

    // Workflow summary: count + selected
    let total = app.workflows.len();
    let selected = app.workflows.iter().filter(|w| w.selected).count();
    let running = app
        .workflows
        .iter()
        .filter(|w| matches!(w.status, WorkflowStatus::Running))
        .count();
    let summary = if running > 0 {
        format!("{} workflows · {} running", total, running)
    } else if selected > 0 {
        format!("{} workflows · {} selected", total, selected)
    } else {
        format!("{} workflows", total)
    };
    spans.push(Span::styled(summary, Style::default().fg(t.fg_normal)));

    // Mode pill: VALIDATION | EXECUTION
    spans.push(Span::raw(" "));
    let (label, bg) = if app.validation_mode {
        (" VALIDATION ", t.warning)
    } else {
        (" EXECUTION ", t.success)
    };
    spans.push(Span::styled(
        label.to_string(),
        Style::default()
            .bg(bg)
            .fg(t.fg_badge)
            .add_modifier(Modifier::BOLD),
    ));

    // Runtime pill next to it
    spans.push(Span::raw(" "));
    let runtime_bg = match app.runtime_type {
        RuntimeType::Docker => t.runtime_docker,
        RuntimeType::Podman => t.runtime_podman,
        RuntimeType::SecureEmulation => t.runtime_secure,
        RuntimeType::Emulation => t.runtime_emulation,
    };
    let runtime_label = match app.runtime_type {
        RuntimeType::Docker => " DOCKER ",
        RuntimeType::Podman => " PODMAN ",
        RuntimeType::SecureEmulation => " SECURE ",
        RuntimeType::Emulation => " EMULATION ",
    };
    spans.push(Span::styled(
        runtime_label.to_string(),
        Style::default()
            .bg(runtime_bg)
            .fg(t.fg_badge)
            .add_modifier(Modifier::BOLD),
    ));

    // Diff-filter indicator when active
    if app.diff_filter_active {
        spans.push(Span::raw(" "));
        spans.push(Span::styled(
            format!(" DIFF:{} ", app.diff_filter_event),
            Style::default().bg(t.trigger).fg(t.fg_badge),
        ));
    }

    spans.push(Span::raw(" "));
    Line::from(spans)
}

/// Right-aligned top-border title: `Workflows · Execution · Logs `.
/// The active tab is rendered in `highlight`-bold; the others in `fg_dim`.
pub fn top_right_title(app: &App) -> Line<'static> {
    let t = &app.theme;
    let labels = ["Workflows", "Execution", "Logs"];
    let mut spans: Vec<Span<'static>> = Vec::new();

    for (i, label) in labels.iter().enumerate() {
        if i > 0 {
            spans.push(Span::styled(
                " · ".to_string(),
                Style::default().fg(t.fg_muted),
            ));
        }
        let n = format!("{}", i + 1);
        let style = if i == app.selected_tab {
            Style::default()
                .fg(t.highlight)
                .add_modifier(Modifier::BOLD)
        } else {
            Style::default().fg(t.fg_dim)
        };
        spans.push(Span::styled(n, Style::default().fg(t.fg_muted)));
        spans.push(Span::styled("·".to_string(), Style::default().fg(t.fg_muted)));
        spans.push(Span::styled((*label).to_string(), style));
    }
    spans.push(Span::raw(" "));
    Line::from(spans)
}
