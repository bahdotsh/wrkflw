// Dashboard layout — workflows table on the left, preview / trigger filter /
// quick actions on the right. Mirrors `DashboardScreen` from the design.
use crate::app::App;
use crate::models::TriggerMatchStatus;
use crate::text;
use crate::theme::{self, BadgeKind, COLORS};
use ratatui::{
    layout::{Constraint, Direction, Layout, Rect},
    style::{Modifier, Style},
    text::{Line, Span},
    widgets::{Cell, HighlightSpacing, Paragraph, Row, Table, TableState},
    Frame,
};

pub fn render_workflows_tab(f: &mut Frame<'_>, app: &mut App, area: Rect) {
    if app.job_selection_mode {
        render_job_selection(f, app, area);
        return;
    }

    // The side column is context, not controls: below ~76 columns the
    // table needs every cell, so the column goes rather than squeezing
    // the list into unreadable slivers.
    let side = match area.width {
        w if w >= 110 => 42,
        w if w >= 76 => 32,
        _ => 0,
    };
    let chunks = Layout::default()
        .direction(Direction::Horizontal)
        .constraints([Constraint::Min(0), Constraint::Length(side)])
        .split(area);

    render_workflow_list(f, app, chunks[0]);
    if side > 0 {
        render_right_column(f, app, chunks[1]);
    }
}

fn render_workflow_list(f: &mut Frame<'_>, app: &mut App, area: Rect) {
    let selected_count = app.workflows.iter().filter(|w| w.selected).count();
    let mut block = theme::block_focused("Workflows");
    let mut badge = format!("{} total", app.workflows.len());
    if selected_count > 0 {
        badge = format!("{selected_count} selected · {badge}");
    }
    if app.diff_filter_active {
        badge = format!("diff: {} · {badge}", app.diff_filter_event);
    }
    block = theme::with_badge(block, badge);
    let inner = block.inner(area);

    if app.workflows.is_empty() {
        f.render_widget(block, area);
        theme::empty_state(
            f,
            inner,
            "No workflows found",
            vec![Span::styled(
                "Add .github/workflows/*.yml or pass a path: wrkflw tui <dir>",
                theme::muted_style(),
            )],
        );
        return;
    }

    let show_trigger =
        app.diff_filter_active || app.workflows.iter().any(|w| w.trigger_match.is_some());
    // Fixed columns: mark(2) checkbox(3) status(1) trigger(1) jobs(6),
    // plus one cell of spacing between each.
    let fixed = 2 + 3 + 1 + 6 + 4 + if show_trigger { 2 } else { 0 };
    let flexible = (inner.width as usize).saturating_sub(fixed);
    let show_path = flexible >= 40;
    let (name_w, path_w) = if show_path {
        let name_w = (flexible * 45 / 100).clamp(16, 48);
        (name_w, flexible.saturating_sub(name_w + 1))
    } else {
        (flexible, 0)
    };

    let mut header_cells = vec!["", ""];
    if show_trigger {
        header_cells.push("");
    }
    header_cells.push("NAME");
    if show_path {
        header_cells.push("PATH");
    }
    header_cells.push("  JOBS");
    let header = Row::new(
        header_cells
            .into_iter()
            .map(|h| Cell::from(h).style(theme::header_style())),
    )
    .height(1);

    let spinner_frame = app.spinner_frame;
    let rows: Vec<Row> = app
        .workflows
        .iter()
        .map(|workflow| {
            let (checkbox, checkbox_color) = if workflow.selected {
                (theme::symbols::CHECKBOX_ON, COLORS.success)
            } else {
                (theme::symbols::CHECKBOX_OFF, COLORS.text_muted)
            };
            let (status_symbol, status_style) =
                theme::workflow_status_animated(&workflow.status, spinner_frame);
            let mut cells = vec![
                Cell::from(checkbox).style(Style::default().fg(checkbox_color)),
                Cell::from(status_symbol).style(status_style),
            ];
            if show_trigger {
                let (sym, style) = match &workflow.trigger_match {
                    Some(TriggerMatchStatus::Matched(_)) => {
                        ("●", Style::default().fg(COLORS.success))
                    }
                    Some(TriggerMatchStatus::Skipped(_)) => ("○", theme::muted_style()),
                    None => (" ", theme::muted_style()),
                };
                cells.push(Cell::from(sym).style(style));
            }
            let name_style = if workflow.definition.is_some() {
                Style::default().fg(COLORS.text)
            } else {
                Style::default().fg(COLORS.error)
            };
            cells.push(
                Cell::from(text::truncate(&text::sanitize(&workflow.name), name_w).into_owned())
                    .style(name_style),
            );
            if show_path {
                let path = workflow.path.to_string_lossy();
                cells.push(
                    Cell::from(text::truncate_left(&text::sanitize(&path), path_w).into_owned())
                        .style(theme::muted_style()),
                );
            }
            // Unparseable files show a red ✖ instead of a misleading 0.
            let (jobs, jobs_style) = if workflow.definition.is_none() {
                (
                    theme::symbols::FAILURE.to_string(),
                    Style::default().fg(COLORS.error),
                )
            } else {
                (workflow.job_names.len().to_string(), theme::muted_style())
            };
            cells.push(Cell::from(text::pad_left(&jobs, 6)).style(jobs_style));
            Row::new(cells)
        })
        .collect();

    let mut widths = vec![Constraint::Length(3), Constraint::Length(1)];
    if show_trigger {
        widths.push(Constraint::Length(1));
    }
    widths.push(Constraint::Length(name_w as u16));
    if show_path {
        widths.push(Constraint::Length(path_w as u16));
    }
    widths.push(Constraint::Length(6));

    let total = rows.len();
    let table = Table::new(rows, widths)
        .header(header)
        .block(block)
        .row_highlight_style(theme::selected_style())
        .highlight_symbol(theme::SELECTED_MARK)
        .highlight_spacing(HighlightSpacing::Always);

    // Carry the scroll offset across frames so the list doesn't jump
    // back to the top-anchored position every redraw.
    let mut table_state = TableState::default()
        .with_offset(app.workflow_list_state.offset())
        .with_selected(app.workflow_list_state.selected());
    f.render_stateful_widget(table, area, &mut table_state);
    app.workflow_list_state.select(table_state.selected());
    *app.workflow_list_state.offset_mut() = table_state.offset();

    let viewport = inner.height.saturating_sub(1) as usize;
    theme::scrollbar(f, area, total, table_state.offset(), viewport);
}

fn render_right_column(f: &mut Frame<'_>, app: &App, area: Rect) {
    let show_actions = area.height >= 30;
    let chunks = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Min(6),
            Constraint::Length(5),
            Constraint::Length(if show_actions { 8 } else { 0 }),
        ])
        .split(area);

    render_preview(f, app, chunks[0]);
    render_trigger_filter(f, app, chunks[1]);
    if show_actions {
        render_quick_actions(f, chunks[2]);
    }
}

fn render_preview(f: &mut Frame<'_>, app: &App, area: Rect) {
    let block = theme::block("Preview");
    let inner_area = block.inner(area);
    f.render_widget(block, area);
    let w = inner_area.width as usize;

    let mut lines: Vec<Line> = Vec::new();
    let selected = app
        .workflow_list_state
        .selected()
        .and_then(|i| app.workflows.get(i));

    let Some(wf) = selected else {
        theme::empty_state(f, inner_area, "No workflow selected", vec![]);
        return;
    };

    let label = |k: &'static str| Span::styled(format!("{k:<6}"), theme::muted_style());

    if let Some(def) = wf.definition.as_ref() {
        let title = if def.name.is_empty() {
            &wf.name
        } else {
            &def.name
        };
        lines.push(Line::from(Span::styled(
            text::truncate(&text::sanitize(title), w).into_owned(),
            Style::default()
                .fg(COLORS.text)
                .add_modifier(Modifier::BOLD),
        )));
        let triggers = if def.on.is_empty() {
            "—".to_string()
        } else {
            def.on.join(", ")
        };
        lines.push(Line::from(vec![
            label("on"),
            Span::styled(
                text::truncate(&triggers, w.saturating_sub(6)).into_owned(),
                Style::default().fg(COLORS.text_dim),
            ),
        ]));
        lines.push(Line::from(vec![
            label("jobs"),
            Span::styled(
                format!("{}", def.jobs.len()),
                Style::default().fg(COLORS.text_dim),
            ),
        ]));

        let mut badges: Vec<Span> = Vec::new();
        let needs_count = def
            .jobs
            .values()
            .filter(|j| j.needs.as_ref().is_some_and(|n| !n.is_empty()))
            .count();
        if needs_count > 0 {
            badges.push(theme::badge_outline(
                format!("needs {}", needs_count),
                BadgeKind::Success,
            ));
        }
        let matrix_jobs = def.jobs.values().filter(|j| j.strategy.is_some()).count();
        if matrix_jobs > 0 {
            badges.push(theme::badge_outline(
                format!("matrix {}", matrix_jobs),
                BadgeKind::Info,
            ));
        }
        let uses_count = def
            .jobs
            .values()
            .flat_map(|j| j.steps.iter())
            .filter(|s| s.uses.is_some())
            .count();
        if uses_count > 0 {
            badges.push(theme::badge_outline(
                format!("uses {}", uses_count),
                BadgeKind::Warning,
            ));
        }
        if !badges.is_empty() && text::spans_width(&badges) <= w {
            lines.push(Line::from(badges));
        }

        if let Some(run) = app
            .workflow_list_state
            .selected()
            .and_then(|i| app.last_run(i))
        {
            lines.push(Line::from(""));
            lines.push(theme::section("Last run"));
            lines.push(last_run_line(run, w));
        }

        let levels = crate::components::dag::topo_levels(def);
        if !levels.is_empty() {
            lines.push(Line::from(""));
            lines.push(theme::section("Stages"));
            let room = (inner_area.height as usize).saturating_sub(lines.len());
            let shown = if levels.len() > room {
                room.saturating_sub(1)
            } else {
                levels.len()
            };
            for (i, layer) in levels.iter().take(shown).enumerate() {
                lines.push(Line::from(vec![
                    Span::styled(format!("{:>2} ", i + 1), theme::muted_style()),
                    Span::styled(
                        text::truncate(&layer.join(" · "), w.saturating_sub(3)).into_owned(),
                        Style::default().fg(COLORS.text_dim),
                    ),
                ]));
            }
            if shown < levels.len() {
                lines.push(Line::from(Span::styled(
                    format!("   +{} more stages", levels.len() - shown),
                    theme::muted_style(),
                )));
            }
        }
    } else {
        lines.push(Line::from(Span::styled(
            text::truncate(&text::sanitize(&wf.name), w).into_owned(),
            Style::default()
                .fg(COLORS.text)
                .add_modifier(Modifier::BOLD),
        )));
        lines.push(Line::from(vec![
            Span::styled(theme::symbols::FAILURE, Style::default().fg(COLORS.error)),
            Span::styled(
                " couldn't parse this file",
                Style::default().fg(COLORS.error),
            ),
        ]));
        lines.push(Line::from(""));
        lines.push(Line::from(vec![
            Span::styled("press ", theme::muted_style()),
            Span::styled(
                "v",
                Style::default()
                    .fg(theme::current_accent())
                    .add_modifier(Modifier::BOLD),
            ),
            Span::styled(" then ", theme::muted_style()),
            Span::styled(
                "Enter",
                Style::default()
                    .fg(theme::current_accent())
                    .add_modifier(Modifier::BOLD),
            ),
            Span::styled(" to see why", theme::muted_style()),
        ]));
    }

    f.render_widget(Paragraph::new(lines), inner_area);
}

fn render_trigger_filter(f: &mut Frame<'_>, app: &App, area: Rect) {
    let block = theme::block_focused("Diff filter");
    let inner_area = block.inner(area);
    f.render_widget(block, area);

    if app.diff_filter_active {
        let matched = app
            .workflows
            .iter()
            .filter(|w| matches!(w.trigger_match, Some(TriggerMatchStatus::Matched(_))))
            .count();
        let skipped = app
            .workflows
            .iter()
            .filter(|w| matches!(w.trigger_match, Some(TriggerMatchStatus::Skipped(_))))
            .count();

        let lines = vec![
            Line::from(vec![
                Span::styled("event  ", theme::muted_style()),
                theme::badge_solid(app.diff_filter_event.clone(), BadgeKind::Info),
                Span::styled("  D to change", theme::muted_style()),
            ]),
            Line::from(vec![
                Span::styled(
                    format!("● {matched}"),
                    Style::default()
                        .fg(COLORS.success)
                        .add_modifier(Modifier::BOLD),
                ),
                Span::styled(" would run   ", theme::muted_style()),
                Span::styled(
                    format!("○ {skipped}"),
                    Style::default()
                        .fg(COLORS.text_dim)
                        .add_modifier(Modifier::BOLD),
                ),
                Span::styled(" skip", theme::muted_style()),
            ]),
        ];
        f.render_widget(Paragraph::new(lines), inner_area);
    } else {
        let lines = vec![
            Line::from(Span::styled(
                "diff filter disabled",
                Style::default().fg(COLORS.text_muted),
            )),
            Line::from(vec![
                Span::styled("press ", Style::default().fg(COLORS.text_dim)),
                theme::key_chip("d"),
                Span::styled(" to enable", Style::default().fg(COLORS.text_dim)),
            ]),
        ];
        f.render_widget(Paragraph::new(lines), inner_area);
    }
}

fn render_quick_actions(f: &mut Frame<'_>, area: Rect) {
    let block = theme::block("Quick actions");
    let inner_area = block.inner(area);
    f.render_widget(block, area);

    let actions = [
        ("r", "Run selected", BadgeKind::Success),
        ("v", "Validate only", BadgeKind::Info),
        ("t", "Trigger remote", BadgeKind::Trigger),
        ("e", "Cycle runtime", BadgeKind::Warning),
        ("d", "Toggle diff filter", BadgeKind::Accent),
        ("?", "Help", BadgeKind::Dim),
    ];

    let lines: Vec<Line> = actions
        .iter()
        .map(|(k, label, kind)| {
            Line::from(vec![
                theme::key_chip(*k),
                Span::raw(" "),
                Span::styled(label.to_string(), Style::default().fg(kind.fg())),
            ])
        })
        .collect();

    f.render_widget(Paragraph::new(lines), inner_area);
}

fn render_job_selection(f: &mut Frame<'_>, app: &mut App, area: Rect) {
    let workflow_name = app
        .workflow_list_state
        .selected()
        .and_then(|idx| app.workflows.get(idx))
        .map(|w| text::sanitize(&w.name).into_owned())
        .unwrap_or_else(|| "Unknown".to_string());

    let title = format!("Run one job · {}", workflow_name);
    let block = theme::with_badge(
        theme::block_focused(&title),
        format!("{} jobs", app.available_jobs.len()),
    );
    let inner = block.inner(area);

    if app.available_jobs.is_empty() {
        f.render_widget(block, area);
        theme::empty_state(f, inner, "This workflow has no jobs", vec![]);
        return;
    }

    let name_w = (inner.width as usize).saturating_sub(7);
    let rows = app.available_jobs.iter().enumerate().map(|(i, job_name)| {
        Row::new(vec![
            Cell::from(format!("{:>3}", i + 1)).style(theme::muted_style()),
            Cell::from(text::truncate(&text::sanitize(job_name), name_w).into_owned())
                .style(Style::default().fg(COLORS.text)),
        ])
    });

    let total = app.available_jobs.len();
    let jobs_table = Table::new(rows, [Constraint::Length(3), Constraint::Min(0)])
        .block(block)
        .row_highlight_style(theme::selected_style())
        .highlight_symbol(theme::SELECTED_MARK)
        .highlight_spacing(HighlightSpacing::Always);

    let mut table_state = TableState::default()
        .with_selected(Some(app.selected_job_index.min(total.saturating_sub(1))));
    f.render_stateful_widget(jobs_table, area, &mut table_state);
    theme::scrollbar(f, area, total, table_state.offset(), inner.height as usize);
}

/// One line about a recorded run: outcome, when, how long, and the first
/// job that failed.
fn last_run_line(run: &wrkflw_runs::RunRecord, w: usize) -> Line<'static> {
    use crate::views::runs_tab::{duration, status_glyph, when};
    let (glyph, color) = status_glyph(run.status);
    let mut spans = vec![
        Span::styled(format!("{glyph} "), Style::default().fg(color)),
        Span::styled(
            run.status.as_str().to_string(),
            Style::default().fg(color).add_modifier(Modifier::BOLD),
        ),
    ];
    let mut facts = vec![when(&run.started_at)];
    let took = duration(run.duration_ms);
    if !took.is_empty() {
        facts.push(took);
    }
    if let Some(job) = run
        .jobs
        .iter()
        .find(|j| j.status == "failure" && !j.failure_allowed)
    {
        facts.push(format!("in {}", job.name));
    }
    for fact in facts.into_iter().filter(|f| !f.is_empty()) {
        spans.push(Span::styled(" · ", theme::muted_style()));
        spans.push(Span::styled(fact, Style::default().fg(COLORS.text_dim)));
    }
    let mut fitted = text::fit_spans(spans, w);
    // Drop the padding: the line sits in a paragraph, not a column.
    if fitted.last().is_some_and(|s| s.content.trim().is_empty()) {
        fitted.pop();
    }
    Line::from(fitted)
}
