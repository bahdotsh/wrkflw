// Logs tab rendering
use crate::app::App;
use crate::theme::{self, COLORS};
use ratatui::{
    layout::{Alignment, Constraint, Direction, Layout, Rect},
    style::Style,
    text::{Line, Span},
    widgets::{Cell, Paragraph, Row, Table, TableState},
    Frame,
};

// Render the logs tab
pub fn render_logs_tab(f: &mut Frame<'_>, app: &App, area: Rect) {
    let show_search_bar =
        app.log_search_active || !app.log_search_query.is_empty() || app.log_filter_level.is_some();

    let chunks = Layout::default()
        .direction(Direction::Vertical)
        .constraints(
            [
                Constraint::Length(if show_search_bar { 3 } else { 0 }), // Search bar (optional)
                Constraint::Min(3),                                      // Logs content
            ]
            .as_ref(),
        )
        .margin(1)
        .split(area);

    // Render search bar if active
    if show_search_bar {
        let search_text = if app.log_search_active {
            format!("Search: {}\u{2588}", app.log_search_query) // █ cursor
        } else {
            format!("Search: {}", app.log_search_query)
        };

        let filter_text = match &app.log_filter_level {
            Some(level) => format!("Filter: {}", level.to_string()),
            None => "No filter".to_string(),
        };

        let match_info = if !app.log_search_matches.is_empty() {
            format!(
                "Matches: {}/{}",
                app.log_search_match_idx + 1,
                app.log_search_matches.len()
            )
        } else if !app.log_search_query.is_empty() {
            "No matches".to_string()
        } else {
            String::new()
        };

        let filter_style = match &app.log_filter_level {
            Some(crate::models::LogFilterLevel::Error) => Style::default().fg(COLORS.error),
            Some(crate::models::LogFilterLevel::Warning) => Style::default().fg(COLORS.warning),
            Some(crate::models::LogFilterLevel::Info) => Style::default().fg(COLORS.info),
            Some(crate::models::LogFilterLevel::Success) => Style::default().fg(COLORS.success),
            Some(crate::models::LogFilterLevel::Trigger) => Style::default().fg(COLORS.trigger),
            Some(crate::models::LogFilterLevel::All) | None => theme::dim_style(),
        };

        let search_info = Line::from(vec![
            Span::styled(&search_text, Style::default().fg(COLORS.text)),
            Span::raw("   "),
            Span::styled(filter_text, filter_style),
            Span::raw("   "),
            Span::styled(match_info, Style::default().fg(COLORS.trigger)),
        ]);

        let search_block = if app.log_search_active {
            theme::block_focused("Search & Filter")
        } else {
            theme::block("Search & Filter")
        };

        let search_widget = Paragraph::new(search_info)
            .block(search_block)
            .alignment(Alignment::Left);

        f.render_widget(search_widget, chunks[0]);
    }

    // Log table
    let filtered_logs = &app.processed_logs;

    let header_cells = ["Time", "Type", "Message"]
        .iter()
        .map(|h| Cell::from(*h).style(theme::header_style()));

    let header = Row::new(header_cells).height(1);

    let rows = filtered_logs
        .iter()
        .map(|processed_log| processed_log.to_row());

    let content_idx = if show_search_bar { 1 } else { 0 };

    let log_title = format!(
        "Logs ({}/{})",
        if filtered_logs.is_empty() {
            0
        } else {
            app.log_scroll + 1
        },
        filtered_logs.len()
    );

    let widths = [
        Constraint::Length(10),     // Timestamp column
        Constraint::Length(9),      // Log type column (wider for badges)
        Constraint::Percentage(80), // Message column
    ];
    let log_table = Table::new(rows, widths)
        .header(header)
        .block(theme::block(&log_title))
        .highlight_style(theme::selected_style());

    let mut log_table_state = TableState::default();

    if !filtered_logs.is_empty() {
        log_table_state.select(Some(app.log_scroll.min(filtered_logs.len() - 1)));
    }

    f.render_stateful_widget(log_table, chunks[content_idx], &mut log_table_state);
}
