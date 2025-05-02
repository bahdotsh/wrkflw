// Workflows tab rendering
use crate::app::App;
use crate::models::WorkflowStatus;
use ratatui::{
    backend::CrosstermBackend,
    layout::{Alignment, Constraint, Direction, Layout, Rect},
    style::{Color, Modifier, Style},
    text::{Line, Span},
    widgets::{Block, BorderType, Borders, Cell, Paragraph, Row, Table, TableState},
    Frame,
};
use std::io;

// Render the workflow list tab
pub fn render_workflows_tab(
    f: &mut Frame<CrosstermBackend<io::Stdout>>,
    app: &mut App,
    area: Rect,
) {
    // Create a more structured layout for the workflow tab
    let chunks = Layout::default()
        .direction(Direction::Vertical)
        .constraints(
            [
                Constraint::Length(3), // Header with instructions
                Constraint::Min(5),    // Workflow list
            ]
            .as_ref(),
        )
        .margin(1)
        .split(area);

    // Render header with instructions
    let header_text = vec![
        Line::from(vec![Span::styled(
            "Available Workflows",
            Style::default()
                .fg(Color::Yellow)
                .add_modifier(Modifier::BOLD),
        )]),
        Line::from(vec![
            Span::styled("Space", Style::default().fg(Color::Cyan)),
            Span::raw(": Toggle selection   "),
            Span::styled("Enter", Style::default().fg(Color::Cyan)),
            Span::raw(": Run   "),
            Span::styled("t", Style::default().fg(Color::Cyan)),
            Span::raw(": Trigger remotely"),
        ]),
    ];

    let header = Paragraph::new(header_text)
        .block(
            Block::default()
                .borders(Borders::ALL)
                .border_type(BorderType::Rounded),
        )
        .alignment(Alignment::Center);

    f.render_widget(header, chunks[0]);

    // Create a table for workflows instead of a list for better organization
    let selected_style = Style::default()
        .bg(Color::DarkGray)
        .add_modifier(Modifier::BOLD);

    // Normal style definition removed as it was unused

    let header_cells = ["", "Status", "Workflow Name", "Path"]
        .iter()
        .map(|h| Cell::from(*h).style(Style::default().fg(Color::Yellow)));

    let header = Row::new(header_cells)
        .style(Style::default().add_modifier(Modifier::BOLD))
        .height(1);

    let rows = app.workflows.iter().map(|workflow| {
        // Create cells for each column
        let checkbox = if workflow.selected { "✓" } else { " " };

        let (status_symbol, status_style) = match workflow.status {
            WorkflowStatus::NotStarted => ("○", Style::default().fg(Color::Gray)),
            WorkflowStatus::Running => ("⟳", Style::default().fg(Color::Cyan)),
            WorkflowStatus::Success => ("✅", Style::default().fg(Color::Green)),
            WorkflowStatus::Failed => ("❌", Style::default().fg(Color::Red)),
            WorkflowStatus::Skipped => ("⏭", Style::default().fg(Color::Yellow)),
        };

        let path_display = workflow.path.to_string_lossy();
        let path_shortened = if path_display.len() > 30 {
            format!("...{}", &path_display[path_display.len() - 30..])
        } else {
            path_display.to_string()
        };

        Row::new(vec![
            Cell::from(checkbox).style(Style::default().fg(Color::Green)),
            Cell::from(status_symbol).style(status_style),
            Cell::from(workflow.name.clone()),
            Cell::from(path_shortened).style(Style::default().fg(Color::DarkGray)),
        ])
    });

    let workflows_table = Table::new(rows)
        .header(header)
        .block(
            Block::default()
                .borders(Borders::ALL)
                .border_type(BorderType::Rounded)
                .title(Span::styled(
                    " Workflows ",
                    Style::default().fg(Color::Yellow),
                )),
        )
        .highlight_style(selected_style)
        .highlight_symbol("» ")
        .widths(&[
            Constraint::Length(3),      // Checkbox column
            Constraint::Length(4),      // Status icon column
            Constraint::Percentage(45), // Name column
            Constraint::Percentage(45), // Path column
        ]);

    // We need to convert ListState to TableState
    let mut table_state = TableState::default();
    table_state.select(app.workflow_list_state.selected());

    f.render_stateful_widget(workflows_table, chunks[1], &mut table_state);

    // Update the app list state to match the table state
    app.workflow_list_state.select(table_state.selected());
}
