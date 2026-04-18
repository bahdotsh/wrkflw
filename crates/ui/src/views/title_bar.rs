// Title bar rendering
use crate::app::App;
use crate::theme;
use ratatui::{
    layout::{Alignment, Rect},
    style::{Modifier, Style},
    text::{Line, Span},
    widgets::{Block, BorderType, Borders, Tabs},
    Frame,
};

// Render the title bar with tabs
pub fn render_title_bar(f: &mut Frame<'_>, app: &App, area: Rect) {
    let t = &app.theme;
    let tab_labels = [
        "1\u{00B7}Workflows",
        "2\u{00B7}Execution",
        "3\u{00B7}Logs",
        "4\u{00B7}Help",
    ];
    let tab_lines: Vec<Line> = tab_labels
        .iter()
        .enumerate()
        .map(|(i, label)| {
            let style = if i == app.selected_tab {
                Style::default()
                    .fg(t.highlight)
                    .add_modifier(Modifier::BOLD)
            } else {
                Style::default().fg(t.fg_dim)
            };
            Line::from(Span::styled(*label, style))
        })
        .collect();
    let tabs = Tabs::new(tab_lines)
        .block(
            Block::default()
                .borders(Borders::ALL)
                .border_type(BorderType::Rounded)
                .border_style(Style::default().fg(t.border))
                .title(Span::styled(" wrkflw ", theme::brand_style(t)))
                .title_alignment(Alignment::Center),
        )
        .highlight_style(
            Style::default()
                .bg(t.bg_selection)
                .fg(t.highlight)
                .add_modifier(Modifier::BOLD),
        )
        .select(app.selected_tab)
        .divider(Span::styled(
            theme::symbols::TAB_DIVIDER,
            Style::default().fg(t.fg_muted),
        ));

    f.render_widget(tabs, area);
}
