// Help overlay rendering
use ratatui::{
    backend::CrosstermBackend,
    layout::Rect,
    style::{Color, Modifier, Style},
    text::{Line, Span},
    widgets::{Block, BorderType, Borders, Paragraph, Wrap},
    Frame,
};
use std::io;

// Render the help tab
pub fn render_help_tab(f: &mut Frame<CrosstermBackend<io::Stdout>>, area: Rect) {
    let help_text = vec![
        Line::from(Span::styled(
            "Keyboard Controls",
            Style::default()
                .fg(Color::Cyan)
                .add_modifier(Modifier::BOLD),
        )),
        Line::from(""),
        Line::from(vec![
            Span::styled(
                "Tab",
                Style::default()
                    .fg(Color::Yellow)
                    .add_modifier(Modifier::BOLD),
            ),
            Span::raw(" - Switch between tabs"),
        ]),
        // More help text would follow...
    ];

    let help_widget = Paragraph::new(help_text)
        .block(
            Block::default()
                .borders(Borders::ALL)
                .border_type(BorderType::Rounded)
                .title(Span::styled(" Help ", Style::default().fg(Color::Yellow))),
        )
        .wrap(Wrap { trim: true });

    f.render_widget(help_widget, area);
}

// Render a help overlay
pub fn render_help_overlay(f: &mut Frame<CrosstermBackend<io::Stdout>>) {
    let size = f.size();

    // Create a slightly smaller centered modal
    let width = size.width.min(60);
    let height = size.height.min(20);
    let x = (size.width - width) / 2;
    let y = (size.height - height) / 2;

    let help_area = Rect {
        x,
        y,
        width,
        height,
    };

    // Create a clear background
    let clear = Block::default().style(Style::default().bg(Color::Black));
    f.render_widget(clear, size);

    // Render the help content
    render_help_tab(f, help_area);
}
