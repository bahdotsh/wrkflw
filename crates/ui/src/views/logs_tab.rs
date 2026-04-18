// Logs tab rendering
use crate::app::App;
use crate::theme;
use ratatui::{
    layout::{Constraint, Direction, Layout, Rect},
    style::{Modifier, Style},
    text::{Line, Span},
    widgets::{
        Cell, Paragraph, Row, Scrollbar, ScrollbarOrientation, ScrollbarState, Table, TableState,
    },
    Frame,
};

pub fn render_logs_tab(f: &mut Frame<'_>, app: &App, area: Rect) {
    let show_search_bar =
        app.log_search_active || !app.log_search_query.is_empty() || app.log_filter_level.is_some();

    // 1-line inline search bar when active, otherwise no header row.
    let chunks = Layout::default()
        .direction(Direction::Vertical)
        .constraints(
            [
                Constraint::Length(if show_search_bar { 1 } else { 0 }),
                Constraint::Min(3),
            ]
            .as_ref(),
        )
        .split(area);

    if show_search_bar {
        render_search_bar(f, app, chunks[0]);
    }

    render_log_table(f, app, chunks[1]);
}

fn render_search_bar(f: &mut Frame<'_>, app: &App, area: Rect) {
    let t = &app.theme;

    // Search: <query>_    Filter: ERROR   3/42
    let mut spans: Vec<Span<'static>> = Vec::new();

    spans.push(Span::styled("  Search ", theme::muted_style(t)));
    spans.push(Span::styled(
        app.log_search_query.clone(),
        Style::default()
            .fg(t.fg_bright)
            .add_modifier(Modifier::BOLD),
    ));
    if app.log_search_active {
        // Blinking-like cursor placeholder.
        spans.push(Span::styled(
            "\u{2588}".to_string(),
            Style::default().fg(t.accent),
        ));
    }

    spans.push(Span::raw("   "));
    let (filter_label, filter_style) = match &app.log_filter_level {
        Some(crate::models::LogFilterLevel::Error) => ("ERROR", Style::default().fg(t.error)),
        Some(crate::models::LogFilterLevel::Warning) => ("WARN", Style::default().fg(t.warning)),
        Some(crate::models::LogFilterLevel::Info) => ("INFO", Style::default().fg(t.info)),
        Some(crate::models::LogFilterLevel::Success) => ("SUCCESS", Style::default().fg(t.success)),
        Some(crate::models::LogFilterLevel::Trigger) => ("TRIG", Style::default().fg(t.trigger)),
        Some(crate::models::LogFilterLevel::All) => ("ALL", theme::dim_style(t)),
        None => ("none", theme::dim_style(t)),
    };
    spans.push(Span::styled("filter ", theme::muted_style(t)));
    spans.push(Span::styled(
        filter_label.to_string(),
        filter_style.add_modifier(Modifier::BOLD),
    ));

    if !app.log_search_matches.is_empty() {
        spans.push(Span::raw("   "));
        spans.push(Span::styled(
            format!(
                "{}/{}",
                app.log_search_match_idx + 1,
                app.log_search_matches.len()
            ),
            Style::default().fg(t.trigger),
        ));
    } else if !app.log_search_query.is_empty() {
        spans.push(Span::raw("   "));
        spans.push(Span::styled("no match", Style::default().fg(t.error)));
    }

    f.render_widget(Paragraph::new(Line::from(spans)), area);
}

fn render_log_table(f: &mut Frame<'_>, app: &App, area: Rect) {
    let t = &app.theme;
    let filtered_logs = &app.processed_logs;

    let header_cells = ["Time", "Type", "Message"]
        .iter()
        .map(|h| Cell::from(*h).style(theme::header_style(t)));
    let header = Row::new(header_cells).height(1);

    let rows = filtered_logs.iter().map(|processed_log| processed_log.to_row(t));

    let widths = [
        Constraint::Length(10),
        Constraint::Length(9),
        Constraint::Percentage(80),
    ];
    let log_table = Table::new(rows, widths)
        .header(header)
        .row_highlight_style(theme::selected_style(t));

    let mut log_table_state = TableState::default();
    if !filtered_logs.is_empty() {
        log_table_state.select(Some(app.log_scroll.min(filtered_logs.len() - 1)));
    }

    // Inset content by 1 col to avoid hugging the outer frame.
    let content_area = Rect {
        x: area.x + 1,
        y: area.y,
        width: area.width.saturating_sub(1),
        height: area.height,
    };
    f.render_stateful_widget(log_table, content_area, &mut log_table_state);

    // Scrollbar on the far right when there is more than fits.
    let visible = content_area.height.saturating_sub(1) as usize; // minus header row
    if filtered_logs.len() > visible && visible > 0 {
        let max_scroll = filtered_logs.len().saturating_sub(visible);
        let mut state = ScrollbarState::new(max_scroll).position(app.log_scroll.min(max_scroll));
        let scrollbar_area = Rect {
            x: area.x + area.width.saturating_sub(1),
            y: content_area.y + 1, // skip header row
            width: 1,
            height: content_area.height.saturating_sub(1),
        };
        f.render_stateful_widget(
            Scrollbar::new(ScrollbarOrientation::VerticalRight)
                .begin_symbol(None)
                .end_symbol(None)
                .track_style(Style::default().fg(t.scrollbar_track))
                .thumb_style(Style::default().fg(t.scrollbar_thumb)),
            scrollbar_area,
            &mut state,
        );
    }
}
