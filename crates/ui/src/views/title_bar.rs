// Title bar rendering
use crate::app::App;
use ratatui::{
    backend::CrosstermBackend,
    layout::{Alignment, Rect},
    style::{Color, Modifier, Style},
    text::{Line, Span},
    widgets::{Block, BorderType, Borders, Tabs},
    Frame,
};
use std::io;

// Render the title bar with tabs
pub fn render_title_bar(f: &mut Frame<CrosstermBackend<io::Stdout>>, app: &App, area: Rect) {
    let titles = ["Workflows", "Execution", "Logs", "Help"];
    let tabs = Tabs::new(
        titles
            .iter()
            .enumerate()
            .map(|(i, t)| {
                if i == 1 {
                    // Special case for "Execution"
                    let e_part = &t[0..1]; // "E"
                    let x_part = &t[1..2]; // "x"
                    let rest = &t[2..]; // "ecution"
                    Line::from(vec![
                        Span::styled(e_part, Style::default().fg(Color::White)),
                        Span::styled(
                            x_part,
                            Style::default()
                                .fg(Color::Yellow)
                                .add_modifier(Modifier::UNDERLINED),
                        ),
                        Span::styled(rest, Style::default().fg(Color::White)),
                    ])
                } else {
                    // Original styling for other tabs
                    let (first, rest) = t.split_at(1);
                    Line::from(vec![
                        Span::styled(
                            first,
                            Style::default()
                                .fg(Color::Yellow)
                                .add_modifier(Modifier::UNDERLINED),
                        ),
                        Span::styled(rest, Style::default().fg(Color::White)),
                    ])
                }
            })
            .collect(),
    )
    .block(
        Block::default()
            .borders(Borders::ALL)
            .border_type(BorderType::Rounded)
            .title(Span::styled(
                " wrkflw ",
                Style::default()
                    .fg(Color::Cyan)
                    .add_modifier(Modifier::BOLD),
            ))
            .title_alignment(Alignment::Center),
    )
    .highlight_style(
        Style::default()
            .bg(Color::DarkGray)
            .fg(Color::Yellow)
            .add_modifier(Modifier::BOLD),
    )
    .select(app.selected_tab)
    .divider(Span::raw("|"));

    f.render_widget(tabs, area);
}
