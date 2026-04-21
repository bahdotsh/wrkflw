// Dashboard layout — workflows table on the left, preview / trigger filter /
// quick actions on the right. Mirrors `DashboardScreen` from the design.
use crate::app::App;
use crate::models::{TriggerMatchStatus, WorkflowStatus};
use crate::theme::{self, BadgeKind, COLORS};
use ratatui::{
    layout::{Constraint, Direction, Layout, Rect},
    style::{Modifier, Style},
    text::{Line, Span},
    widgets::{Cell, Paragraph, Row, Table, TableState, Wrap},
    Frame,
};

pub fn render_workflows_tab(f: &mut Frame<'_>, app: &mut App, area: Rect) {
    if app.job_selection_mode {
        render_job_selection(f, app, area);
        return;
    }

    let chunks = Layout::default()
        .direction(Direction::Horizontal)
        .constraints([Constraint::Min(40), Constraint::Length(40)])
        .margin(1)
        .split(area);

    render_workflow_list(f, app, chunks[0]);
    render_right_column(f, app, chunks[1]);
}

fn render_workflow_list(f: &mut Frame<'_>, app: &mut App, area: Rect) {
    let selected_count = app.workflows.iter().filter(|w| w.selected).count();
    let diff_indicator = if app.diff_filter_active {
        format!("  [DIFF: {}]", app.diff_filter_event)
    } else {
        String::new()
    };
    let block_title = format!("Workflows ({} selected){}", selected_count, diff_indicator);

    let header_cells = ["", "ST", "TR", "NAME", "PATH", "JOBS"];
    let header = Row::new(
        header_cells
            .iter()
            .map(|h| Cell::from(*h).style(theme::header_style())),
    )
    .height(1);

    let spinner_frame = app.spinner_frame;
    let rows: Vec<Row> = app
        .workflows
        .iter()
        .map(|workflow| {
            let checkbox = if workflow.selected {
                theme::symbols::CHECKBOX_ON
            } else {
                theme::symbols::CHECKBOX_OFF
            };
            let (status_symbol, status_style) =
                theme::workflow_status_animated(&workflow.status, spinner_frame);
            let (trigger_symbol, trigger_style) = match &workflow.trigger_match {
                Some(TriggerMatchStatus::Matched(_)) => ("●", Style::default().fg(COLORS.success)),
                Some(TriggerMatchStatus::Skipped(_)) => {
                    ("○", Style::default().fg(COLORS.text_muted))
                }
                None => (" ", Style::default().fg(COLORS.text_muted)),
            };

            let path_display = workflow.path.to_string_lossy();
            let path_shortened = if path_display.len() > 40 {
                let start = path_display
                    .char_indices()
                    .rev()
                    .nth(39)
                    .map(|(i, _)| i)
                    .unwrap_or(0);
                format!("…{}", &path_display[start..])
            } else {
                path_display.to_string()
            };

            let jobs_n = workflow.job_names.len();
            Row::new(vec![
                Cell::from(checkbox).style(Style::default().fg(if workflow.selected {
                    COLORS.success
                } else {
                    COLORS.text_muted
                })),
                Cell::from(status_symbol).style(status_style),
                Cell::from(trigger_symbol).style(trigger_style),
                Cell::from(workflow.name.clone()).style(
                    Style::default()
                        .fg(COLORS.text)
                        .add_modifier(Modifier::BOLD),
                ),
                Cell::from(path_shortened).style(theme::muted_style()),
                Cell::from(format!("{}", jobs_n)).style(Style::default().fg(COLORS.text_muted)),
            ])
        })
        .collect();

    let widths = [
        Constraint::Length(3), // checkbox
        Constraint::Length(2), // status
        Constraint::Length(2), // trigger
        Constraint::Min(16),   // name
        Constraint::Min(16),   // path
        Constraint::Length(5), // jobs
    ];

    let table = Table::new(rows, widths)
        .header(header)
        .block(theme::block_focused(&block_title))
        .highlight_style(theme::selected_style())
        .highlight_symbol(theme::symbols::SELECTED);

    let mut table_state = TableState::default();
    table_state.select(app.workflow_list_state.selected());

    f.render_stateful_widget(table, area, &mut table_state);
    app.workflow_list_state.select(table_state.selected());
}

fn render_right_column(f: &mut Frame<'_>, app: &App, area: Rect) {
    let chunks = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Length(11),
            Constraint::Length(7),
            Constraint::Min(0),
        ])
        .split(area);

    render_preview(f, app, chunks[0]);
    render_trigger_filter(f, app, chunks[1]);
    render_quick_actions(f, chunks[2]);
}

fn render_preview(f: &mut Frame<'_>, app: &App, area: Rect) {
    let block = theme::block("Preview");
    let inner_area = block.inner(area);
    f.render_widget(block, area);

    let mut lines: Vec<Line> = Vec::new();
    let selected = app
        .workflow_list_state
        .selected()
        .and_then(|i| app.workflows.get(i));

    let Some(wf) = selected else {
        lines.push(Line::from(Span::styled(
            "(no workflow selected)",
            Style::default().fg(COLORS.text_muted),
        )));
        f.render_widget(Paragraph::new(lines), inner_area);
        return;
    };

    if let Some(def) = wf.definition.as_ref() {
        lines.push(Line::from(vec![
            Span::styled("name: ", Style::default().fg(COLORS.accent)),
            Span::styled(
                def.name.clone(),
                Style::default()
                    .fg(COLORS.text)
                    .add_modifier(Modifier::BOLD),
            ),
        ]));
        let triggers = if def.on.is_empty() {
            "—".to_string()
        } else {
            def.on.join(", ")
        };
        lines.push(Line::from(vec![
            Span::styled("on:   ", Style::default().fg(COLORS.accent)),
            Span::styled(triggers, Style::default().fg(COLORS.text_dim)),
        ]));
        lines.push(Line::from(vec![
            Span::styled("jobs: ", Style::default().fg(COLORS.accent)),
            Span::styled(
                format!("{}", def.jobs.len()),
                Style::default().fg(COLORS.text_dim),
            ),
        ]));
        let chain = build_chain(def);
        lines.push(Line::from(vec![Span::styled(
            chain,
            Style::default().fg(COLORS.text_muted),
        )]));
        lines.push(Line::from(""));
        let mut badges: Vec<Span> = Vec::new();
        let needs_count = def
            .jobs
            .values()
            .filter(|j| j.needs.as_ref().is_some_and(|n| !n.is_empty()))
            .count();
        if needs_count > 0 {
            badges.push(theme::badge_outline(
                format!("needs: {}", needs_count),
                BadgeKind::Success,
            ));
            badges.push(Span::raw(" "));
        }
        let matrix_jobs = def.jobs.values().filter(|j| j.strategy.is_some()).count();
        if matrix_jobs > 0 {
            badges.push(theme::badge_outline(
                format!("matrix: {}", matrix_jobs),
                BadgeKind::Info,
            ));
            badges.push(Span::raw(" "));
        }
        let uses_count = def
            .jobs
            .values()
            .flat_map(|j| j.steps.iter())
            .filter(|s| s.uses.is_some())
            .count();
        if uses_count > 0 {
            badges.push(theme::badge_outline(
                format!("uses: {}", uses_count),
                BadgeKind::Warning,
            ));
        }
        if !badges.is_empty() {
            lines.push(Line::from(badges));
        }
    } else {
        lines.push(Line::from(vec![Span::styled(
            wf.name.clone(),
            Style::default()
                .fg(COLORS.text)
                .add_modifier(Modifier::BOLD),
        )]));
        lines.push(Line::from(Span::styled(
            "(failed to parse — check YAML)",
            Style::default().fg(COLORS.text_muted),
        )));
        lines.push(Line::from(vec![
            Span::styled("jobs: ", Style::default().fg(COLORS.accent)),
            Span::styled(
                format!("{}", wf.job_names.len()),
                Style::default().fg(COLORS.text_dim),
            ),
        ]));
    }

    f.render_widget(Paragraph::new(lines).wrap(Wrap { trim: false }), inner_area);
}

fn build_chain(def: &wrkflw_parser::workflow::WorkflowDefinition) -> String {
    use crate::components::dag::topo_levels;
    let levels = topo_levels(def);
    if levels.is_empty() {
        return String::new();
    }
    levels
        .into_iter()
        .map(|layer| layer.join(","))
        .collect::<Vec<_>>()
        .join(" → ")
}

fn render_trigger_filter(f: &mut Frame<'_>, app: &App, area: Rect) {
    let block = theme::block_focused("Trigger filter");
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
                Span::styled("event: ", Style::default().fg(COLORS.text_dim)),
                theme::badge_solid(app.diff_filter_event.clone(), BadgeKind::Info),
            ]),
            Line::from(vec![
                Span::styled("●", Style::default().fg(COLORS.success)),
                Span::raw(" matches → "),
                Span::styled(
                    format!("{}", matched),
                    Style::default()
                        .fg(COLORS.text)
                        .add_modifier(Modifier::BOLD),
                ),
                Span::raw("   "),
                Span::styled("○", Style::default().fg(COLORS.text_muted)),
                Span::raw(" skipped → "),
                Span::styled(
                    format!("{}", skipped),
                    Style::default()
                        .fg(COLORS.text)
                        .add_modifier(Modifier::BOLD),
                ),
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
                Span::raw("  "),
                Span::styled(label.to_string(), Style::default().fg(COLORS.text)),
                Span::raw("  "),
                Span::styled("▸", Style::default().fg(kind.fg())),
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
        .map(|w| w.name.as_str())
        .unwrap_or("Unknown");

    let block_title = format!("Jobs in '{}'", workflow_name);

    let header_cells = ["#", "Job Name"]
        .iter()
        .map(|h| Cell::from(*h).style(theme::header_style()));

    let header = Row::new(header_cells).height(1);

    let rows = app.available_jobs.iter().enumerate().map(|(i, job_name)| {
        Row::new(vec![
            Cell::from(format!("{}", i + 1)).style(theme::muted_style()),
            Cell::from(job_name.clone()).style(Style::default().fg(COLORS.text)),
        ])
    });

    let widths = [Constraint::Length(4), Constraint::Percentage(90)];
    let jobs_table = Table::new(rows, widths)
        .header(header)
        .block(theme::block_focused(&block_title))
        .highlight_style(theme::selected_style())
        .highlight_symbol(theme::symbols::SELECTED);

    let mut table_state = TableState::default();
    table_state.select(Some(app.selected_job_index));

    let inner = Layout::default()
        .direction(Direction::Vertical)
        .constraints([Constraint::Min(0)].as_ref())
        .margin(1)
        .split(area);

    f.render_stateful_widget(jobs_table, inner[0], &mut table_state);
}

// ── Helpers used by other tabs (kept here so badges around statuses stay
//    consistent with the dashboard styling). Currently unused publicly; kept
//    as a marker that this is the canonical location for new dashboard helpers.
#[allow(dead_code)]
fn workflow_status_badge(status: &WorkflowStatus) -> Span<'static> {
    match status {
        WorkflowStatus::Running => theme::badge_solid("RUNNING", BadgeKind::Info),
        WorkflowStatus::Success => theme::badge_outline("SUCCESS", BadgeKind::Success),
        WorkflowStatus::Failed => theme::badge_outline("FAILED", BadgeKind::Error),
        WorkflowStatus::Skipped => theme::badge_outline("SKIPPED", BadgeKind::Warning),
        WorkflowStatus::NotStarted => theme::badge_outline("IDLE", BadgeKind::Dim),
    }
}
