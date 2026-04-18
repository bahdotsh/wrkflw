// Workflows tab rendering
use crate::app::App;
use crate::models::TriggerMatchStatus;
use crate::theme;
use crate::utils::truncate_path;
use ratatui::{
    layout::{Constraint, Rect},
    style::{Modifier, Style},
    widgets::{Cell, Row, Table, TableState},
    Frame,
};

// Render the workflow list tab
pub fn render_workflows_tab(f: &mut Frame<'_>, app: &mut App, area: Rect) {
    if app.job_selection_mode {
        render_job_selection(f, app, area);
    } else {
        render_workflow_list(f, app, area);
    }
}

fn render_workflow_list(f: &mut Frame<'_>, app: &mut App, area: Rect) {
    let t = &app.theme;

    let header_cells = if app.diff_filter_active {
        vec!["", "Status", "Trigger", "Workflow Name", "Path"]
    } else {
        vec!["", "Status", "Workflow Name", "Path"]
    };
    let header = Row::new(
        header_cells
            .iter()
            .map(|h| Cell::from(*h).style(theme::header_style(t))),
    )
    .height(1);

    // Reserve space for the highlight_symbol ("▌ " = 2 cols) plus the four
    // leading fixed columns when sizing the path column's usable width.
    let path_budget = (area.width as usize)
        .saturating_sub(2 + 5 + 3 + if app.diff_filter_active { 3 } else { 0 })
        / 2;

    let diff_active = app.diff_filter_active;
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
                theme::workflow_status_animated(t, &workflow.status, spinner_frame);

            let path_display = workflow.path.to_string_lossy();
            let path_shortened = truncate_path(&path_display, path_budget.max(10));

            let mut cells = vec![
                Cell::from(checkbox).style(Style::default().fg(t.success)),
                Cell::from(status_symbol).style(status_style),
            ];

            if diff_active {
                let (trigger_symbol, trigger_style) = match &workflow.trigger_match {
                    Some(TriggerMatchStatus::Matched(_)) => {
                        ("\u{25cf}", Style::default().fg(t.success))
                    }
                    Some(TriggerMatchStatus::Skipped(_)) => {
                        ("\u{25cb}", Style::default().fg(t.fg_muted))
                    }
                    None => ("-", Style::default().fg(t.fg_muted)),
                };
                cells.push(Cell::from(trigger_symbol).style(trigger_style));
            }

            cells.push(
                Cell::from(workflow.name.clone()).style(
                    Style::default()
                        .fg(t.fg_bright)
                        .add_modifier(Modifier::BOLD),
                ),
            );
            cells.push(Cell::from(path_shortened).style(theme::muted_style(t)));

            Row::new(cells)
        })
        .collect();

    let widths: Vec<Constraint> = if app.diff_filter_active {
        vec![
            Constraint::Length(5),      // Checkbox
            Constraint::Length(3),      // Status
            Constraint::Length(3),      // Trigger
            Constraint::Percentage(42), // Name
            Constraint::Percentage(42), // Path
        ]
    } else {
        vec![
            Constraint::Length(5),      // Checkbox
            Constraint::Length(3),      // Status
            Constraint::Percentage(45), // Name
            Constraint::Percentage(45), // Path
        ]
    };

    let workflows_table = Table::new(rows, widths)
        .header(header)
        .row_highlight_style(theme::selected_style(t))
        .highlight_symbol("\u{258c} "); // ▌

    let mut table_state = TableState::default();
    table_state.select(app.workflow_list_state.selected());

    // One-row margin on each side so rows don't crowd the outer frame border.
    let inset = inset(area, 1, 0);
    f.render_stateful_widget(workflows_table, inset, &mut table_state);

    app.workflow_list_state.select(table_state.selected());
}

fn render_job_selection(f: &mut Frame<'_>, app: &mut App, area: Rect) {
    let t = &app.theme;
    let workflow_name = app
        .workflow_list_state
        .selected()
        .and_then(|idx| app.workflows.get(idx))
        .map(|w| w.name.as_str())
        .unwrap_or("Unknown");

    let header_cells = ["#", "Job Name"]
        .iter()
        .map(|h| Cell::from(*h).style(theme::header_style(t)));
    let header = Row::new(header_cells).height(1);

    let rows: Vec<Row> = app
        .available_jobs
        .iter()
        .enumerate()
        .map(|(i, job_name)| {
            Row::new(vec![
                Cell::from(format!("{}", i + 1)).style(theme::muted_style(t)),
                Cell::from(job_name.clone()).style(Style::default().fg(t.fg_bright)),
            ])
        })
        .collect();

    let widths = [Constraint::Length(4), Constraint::Percentage(90)];
    let jobs_table = Table::new(rows, widths)
        .header(header)
        .row_highlight_style(theme::selected_style(t))
        .highlight_symbol("\u{258c} ");

    let mut table_state = TableState::default();
    table_state.select(Some(app.selected_job_index));

    // Render a small "Jobs in '<workflow>'" header line above the table,
    // then the table in the remaining area.
    use ratatui::text::{Line, Span};
    use ratatui::widgets::Paragraph;
    let heading = Paragraph::new(Line::from(vec![
        Span::styled("  Jobs in ", theme::muted_style(t)),
        Span::styled(
            format!("'{}'", workflow_name),
            Style::default()
                .fg(t.fg_bright)
                .add_modifier(Modifier::BOLD),
        ),
    ]));
    let (heading_rect, body_rect) = split_top(area, 1);
    f.render_widget(heading, heading_rect);
    f.render_stateful_widget(jobs_table, inset(body_rect, 1, 0), &mut table_state);
}

/// Inset `r` by `dx` columns horizontally and `dy` rows vertically.
fn inset(r: Rect, dx: u16, dy: u16) -> Rect {
    Rect {
        x: r.x + dx,
        y: r.y + dy,
        width: r.width.saturating_sub(dx * 2),
        height: r.height.saturating_sub(dy * 2),
    }
}

/// Split `r` into a top strip of `top_h` rows and the remainder below it.
fn split_top(r: Rect, top_h: u16) -> (Rect, Rect) {
    let top = Rect {
        x: r.x,
        y: r.y,
        width: r.width,
        height: top_h.min(r.height),
    };
    let rest = Rect {
        x: r.x,
        y: r.y + top.height,
        width: r.width,
        height: r.height.saturating_sub(top.height),
    };
    (top, rest)
}
