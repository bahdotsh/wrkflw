// Workflows tab rendering
use crate::app::App;
use crate::theme::{self, COLORS};
use ratatui::{
    layout::{Constraint, Direction, Layout, Rect},
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
    let selected_count = app.workflows.iter().filter(|w| w.selected).count();
    let block_title = format!("Workflows ({} selected)", selected_count);

    let header_cells = ["", "Status", "Workflow Name", "Path"]
        .iter()
        .map(|h| Cell::from(*h).style(theme::header_style()));

    let header = Row::new(header_cells).height(1);

    let rows = app.workflows.iter().map(|workflow| {
        let checkbox = if workflow.selected {
            theme::symbols::CHECKBOX_ON
        } else {
            theme::symbols::CHECKBOX_OFF
        };

        let (status_symbol, status_style) =
            theme::workflow_status_animated(&workflow.status, app.spinner_frame);

        let path_display = workflow.path.to_string_lossy();
        let path_shortened = if path_display.len() > 30 {
            let start = path_display
                .char_indices()
                .rev()
                .nth(29)
                .map(|(i, _)| i)
                .unwrap_or(0);
            format!("\u{2026}{}", &path_display[start..])
        } else {
            path_display.to_string()
        };

        Row::new(vec![
            Cell::from(checkbox).style(Style::default().fg(COLORS.success)),
            Cell::from(status_symbol).style(status_style),
            Cell::from(workflow.name.clone()).style(
                Style::default()
                    .fg(COLORS.text)
                    .add_modifier(Modifier::BOLD),
            ),
            Cell::from(path_shortened).style(theme::muted_style()),
        ])
    });

    let widths = [
        Constraint::Length(5),      // Checkbox column
        Constraint::Length(3),      // Status icon column
        Constraint::Percentage(45), // Name column
        Constraint::Percentage(45), // Path column
    ];
    let workflows_table = Table::new(rows, widths)
        .header(header)
        .block(theme::block(&block_title))
        .highlight_style(theme::selected_style())
        .highlight_symbol(theme::symbols::SELECTED);

    let mut table_state = TableState::default();
    table_state.select(app.workflow_list_state.selected());

    // Use inner area with margin for consistent spacing
    let inner = Layout::default()
        .direction(Direction::Vertical)
        .constraints([Constraint::Min(0)].as_ref())
        .margin(1)
        .split(area);

    f.render_stateful_widget(workflows_table, inner[0], &mut table_state);

    // Update the app list state to match the table state
    app.workflow_list_state.select(table_state.selected());
}

fn render_job_selection(f: &mut Frame<'_>, app: &mut App, area: Rect) {
    // Get workflow name for the header
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

    let widths = [
        Constraint::Length(4),      // Number column
        Constraint::Percentage(90), // Job name column
    ];
    let jobs_table = Table::new(rows, widths)
        .header(header)
        .block(theme::block(&block_title))
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
