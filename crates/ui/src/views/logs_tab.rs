// Logs tab — every log line with its time and level, following new
// output until the user scrolls away from the end.
use crate::app::App;
use crate::models::LogFilterLevel;
use crate::text;
use crate::theme::{self, COLORS};
use ratatui::{
    layout::{Constraint, Direction, Layout, Rect},
    style::{Modifier, Style},
    text::{Line, Span},
    widgets::{HighlightSpacing, Paragraph, Table, TableState},
    Frame,
};

pub fn render_logs_tab(f: &mut Frame<'_>, app: &mut App, area: Rect) {
    let show_search = app.log_search_active || !app.log_search_query.is_empty();
    let chunks = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Length(if show_search { 3 } else { 0 }),
            Constraint::Min(0),
        ])
        .split(area);

    if show_search {
        render_search_bar(f, app, chunks[0]);
    }

    let logs = &app.processed_logs;
    let mut badge = if logs.is_empty() {
        "0".to_string()
    } else {
        let at = if app.log_follow {
            logs.len()
        } else {
            app.log_scroll.min(logs.len() - 1) + 1
        };
        format!("{at}/{}", logs.len())
    };
    if let Some(level) = app
        .log_filter_level
        .as_ref()
        .filter(|l| **l != LogFilterLevel::All)
    {
        badge = format!("{} only · {badge}", level.as_str().to_lowercase());
    }
    if app.log_follow && !logs.is_empty() {
        badge = format!("following · {badge}");
    }
    let block = theme::with_badge(theme::block_focused("Logs"), badge);

    if logs.is_empty() {
        let inner = block.inner(chunks[1]);
        f.render_widget(block, chunks[1]);
        let msg = if app.log_filter_level.is_some() || !app.log_search_query.is_empty() {
            "No lines match the search or filter"
        } else {
            "No log lines yet"
        };
        theme::empty_state(
            f,
            inner,
            msg,
            vec![Span::styled(
                "Run a workflow and its output lands here",
                theme::muted_style(),
            )],
        );
        return;
    }
    let rows = logs.iter().map(|l| l.to_row());
    let widths = [
        Constraint::Length(8),
        Constraint::Length(9),
        Constraint::Min(0),
    ];
    let total = logs.len();
    let table = Table::new(rows, widths)
        .block(block)
        .row_highlight_style(theme::selected_style())
        .highlight_symbol(theme::SELECTED_MARK)
        .highlight_spacing(HighlightSpacing::Always);

    if app.log_follow {
        app.log_scroll = total - 1;
    }
    app.log_scroll = app.log_scroll.min(total - 1);
    let mut state = TableState::default()
        .with_offset(app.log_offset)
        .with_selected(Some(app.log_scroll));
    f.render_stateful_widget(table, chunks[1], &mut state);
    app.log_offset = state.offset();

    let viewport = chunks[1].height.saturating_sub(2) as usize;
    theme::scrollbar(f, chunks[1], total, state.offset(), viewport);
}

fn render_search_bar(f: &mut Frame<'_>, app: &App, area: Rect) {
    let block = theme::panel("Search", app.log_search_active);
    let matches = if !app.log_search_matches.is_empty() {
        format!(
            "{}/{} matches",
            app.log_search_match_idx
                .min(app.log_search_matches.len() - 1)
                + 1,
            app.log_search_matches.len()
        )
    } else if !app.log_search_query.is_empty() {
        "no matches".to_string()
    } else {
        String::new()
    };
    let block = theme::with_badge(block, matches);

    // Keep the end of a long query, where the cursor is, in view.
    let room = (area.width as usize).saturating_sub(8);
    let query = if app.log_search_active {
        text::truncate_left(&app.log_search_query, room).into_owned()
    } else {
        text::truncate(&app.log_search_query, room).into_owned()
    };
    let mut spans = vec![
        Span::styled("/ ", Style::default().fg(theme::current_accent())),
        Span::styled(
            query,
            Style::default()
                .fg(COLORS.text)
                .add_modifier(Modifier::BOLD),
        ),
    ];
    if app.log_search_active {
        spans.push(Span::styled(
            "▏",
            Style::default().fg(theme::current_accent()),
        ));
    }
    f.render_widget(Paragraph::new(Line::from(spans)).block(block), area);
}
