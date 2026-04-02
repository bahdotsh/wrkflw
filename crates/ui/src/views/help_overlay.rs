// Help overlay rendering
use ratatui::{
    backend::CrosstermBackend,
    layout::{Constraint, Direction, Layout, Rect},
    style::{Color, Modifier, Style},
    text::{Line, Span},
    widgets::{Block, BorderType, Borders, Paragraph, Wrap},
    Frame,
};
use std::io;

// Render the help tab with scroll support
pub fn render_help_content(
    f: &mut Frame<CrosstermBackend<io::Stdout>>,
    area: Rect,
    scroll_offset: usize,
) {
    // Split the area into columns for better organization
    let chunks = Layout::default()
        .direction(Direction::Horizontal)
        .constraints([Constraint::Percentage(50), Constraint::Percentage(50)].as_ref())
        .split(area);

    // Left column content
    let left_help_text = vec![
        Line::from(Span::styled(
            "🗂  NAVIGATION",
            Style::default()
                .fg(Color::Cyan)
                .add_modifier(Modifier::BOLD),
        )),
        Line::from(""),
        Line::from(vec![
            Span::styled(
                "Tab / Shift+Tab",
                Style::default()
                    .fg(Color::Yellow)
                    .add_modifier(Modifier::BOLD),
            ),
            Span::raw(" - Switch between tabs"),
        ]),
        Line::from(vec![
            Span::styled(
                "1-4 / w,x,l,h",
                Style::default()
                    .fg(Color::Yellow)
                    .add_modifier(Modifier::BOLD),
            ),
            Span::raw(" - Jump to specific tab"),
        ]),
        Line::from(vec![
            Span::styled(
                "↑/↓ or k/j",
                Style::default()
                    .fg(Color::Yellow)
                    .add_modifier(Modifier::BOLD),
            ),
            Span::raw(" - Navigate lists"),
        ]),
        Line::from(vec![
            Span::styled(
                "Enter",
                Style::default()
                    .fg(Color::Yellow)
                    .add_modifier(Modifier::BOLD),
            ),
            Span::raw(" - Select/View details"),
        ]),
        Line::from(vec![
            Span::styled(
                "Esc",
                Style::default()
                    .fg(Color::Yellow)
                    .add_modifier(Modifier::BOLD),
            ),
            Span::raw(" - Back/Exit help"),
        ]),
        Line::from(""),
        Line::from(Span::styled(
            "🚀 WORKFLOW MANAGEMENT",
            Style::default()
                .fg(Color::Green)
                .add_modifier(Modifier::BOLD),
        )),
        Line::from(""),
        Line::from(vec![
            Span::styled(
                "Space",
                Style::default()
                    .fg(Color::Yellow)
                    .add_modifier(Modifier::BOLD),
            ),
            Span::raw(" - Toggle workflow selection"),
        ]),
        Line::from(vec![
            Span::styled(
                "r",
                Style::default()
                    .fg(Color::Yellow)
                    .add_modifier(Modifier::BOLD),
            ),
            Span::raw(" - Run selected workflows"),
        ]),
        Line::from(vec![
            Span::styled(
                "a",
                Style::default()
                    .fg(Color::Yellow)
                    .add_modifier(Modifier::BOLD),
            ),
            Span::raw(" - Select all workflows"),
        ]),
        Line::from(vec![
            Span::styled(
                "n",
                Style::default()
                    .fg(Color::Yellow)
                    .add_modifier(Modifier::BOLD),
            ),
            Span::raw(" - Deselect all workflows"),
        ]),
        Line::from(vec![
            Span::styled(
                "Shift+R",
                Style::default()
                    .fg(Color::Yellow)
                    .add_modifier(Modifier::BOLD),
            ),
            Span::raw(" - Reset workflow status"),
        ]),
        Line::from(vec![
            Span::styled(
                "t",
                Style::default()
                    .fg(Color::Yellow)
                    .add_modifier(Modifier::BOLD),
            ),
            Span::raw(" - Trigger remote workflow"),
        ]),
        Line::from(""),
        Line::from(Span::styled(
            "🎯 JOB SELECTION",
            Style::default()
                .fg(Color::Blue)
                .add_modifier(Modifier::BOLD),
        )),
        Line::from(""),
        Line::from(vec![
            Span::styled(
                "Shift+J",
                Style::default()
                    .fg(Color::Yellow)
                    .add_modifier(Modifier::BOLD),
            ),
            Span::raw(" - View jobs in workflow"),
        ]),
        Line::from(vec![
            Span::styled(
                "Enter (in jobs)",
                Style::default()
                    .fg(Color::Yellow)
                    .add_modifier(Modifier::BOLD),
            ),
            Span::raw(" - Run selected job"),
        ]),
        Line::from(vec![
            Span::styled(
                "a (in jobs)",
                Style::default()
                    .fg(Color::Yellow)
                    .add_modifier(Modifier::BOLD),
            ),
            Span::raw(" - Run all jobs"),
        ]),
        Line::from(vec![
            Span::styled(
                "Esc (in jobs)",
                Style::default()
                    .fg(Color::Yellow)
                    .add_modifier(Modifier::BOLD),
            ),
            Span::raw(" - Back to workflow list"),
        ]),
        Line::from(""),
        Line::from(Span::styled(
            "🔧 EXECUTION MODES",
            Style::default()
                .fg(Color::Magenta)
                .add_modifier(Modifier::BOLD),
        )),
        Line::from(""),
        Line::from(vec![
            Span::styled(
                "e",
                Style::default()
                    .fg(Color::Yellow)
                    .add_modifier(Modifier::BOLD),
            ),
            Span::raw(" - Toggle emulation mode"),
        ]),
        Line::from(vec![
            Span::styled(
                "v",
                Style::default()
                    .fg(Color::Yellow)
                    .add_modifier(Modifier::BOLD),
            ),
            Span::raw(" - Toggle validation mode"),
        ]),
        Line::from(""),
        Line::from(vec![Span::styled(
            "Runtime Modes:",
            Style::default()
                .fg(Color::White)
                .add_modifier(Modifier::BOLD),
        )]),
        Line::from(vec![
            Span::raw("  • "),
            Span::styled("Docker", Style::default().fg(Color::Blue)),
            Span::raw(" - Container isolation (default)"),
        ]),
        Line::from(vec![
            Span::raw("  • "),
            Span::styled("Podman", Style::default().fg(Color::Blue)),
            Span::raw(" - Rootless containers"),
        ]),
        Line::from(vec![
            Span::raw("  • "),
            Span::styled("Emulation", Style::default().fg(Color::Red)),
            Span::raw(" - Process mode (UNSAFE)"),
        ]),
        Line::from(vec![
            Span::raw("  • "),
            Span::styled("Secure Emulation", Style::default().fg(Color::Yellow)),
            Span::raw(" - Sandboxed processes"),
        ]),
    ];

    // Right column content
    let right_help_text = vec![
        Line::from(Span::styled(
            "📄 LOGS & SEARCH",
            Style::default()
                .fg(Color::Blue)
                .add_modifier(Modifier::BOLD),
        )),
        Line::from(""),
        Line::from(vec![
            Span::styled(
                "s",
                Style::default()
                    .fg(Color::Yellow)
                    .add_modifier(Modifier::BOLD),
            ),
            Span::raw(" - Toggle log search"),
        ]),
        Line::from(vec![
            Span::styled(
                "f",
                Style::default()
                    .fg(Color::Yellow)
                    .add_modifier(Modifier::BOLD),
            ),
            Span::raw(" - Toggle log filter"),
        ]),
        Line::from(vec![
            Span::styled(
                "c",
                Style::default()
                    .fg(Color::Yellow)
                    .add_modifier(Modifier::BOLD),
            ),
            Span::raw(" - Clear search & filter"),
        ]),
        Line::from(vec![
            Span::styled(
                "n",
                Style::default()
                    .fg(Color::Yellow)
                    .add_modifier(Modifier::BOLD),
            ),
            Span::raw(" - Next search match"),
        ]),
        Line::from(vec![
            Span::styled(
                "↑/↓",
                Style::default()
                    .fg(Color::Yellow)
                    .add_modifier(Modifier::BOLD),
            ),
            Span::raw(" - Scroll logs/Navigate"),
        ]),
        Line::from(""),
        Line::from(Span::styled(
            "ℹ️  TAB OVERVIEW",
            Style::default()
                .fg(Color::White)
                .add_modifier(Modifier::BOLD),
        )),
        Line::from(""),
        Line::from(vec![
            Span::styled(
                "1. Workflows",
                Style::default()
                    .fg(Color::Cyan)
                    .add_modifier(Modifier::BOLD),
            ),
            Span::raw(" - Browse & select workflows"),
        ]),
        Line::from(vec![Span::raw("   • View workflow files")]),
        Line::from(vec![Span::raw("   • Select multiple for batch execution")]),
        Line::from(vec![Span::raw("   • Trigger remote workflows")]),
        Line::from(""),
        Line::from(vec![
            Span::styled(
                "2. Execution",
                Style::default()
                    .fg(Color::Green)
                    .add_modifier(Modifier::BOLD),
            ),
            Span::raw(" - Monitor job progress"),
        ]),
        Line::from(vec![Span::raw("   • View job status and details")]),
        Line::from(vec![Span::raw("   • Enter job details with Enter")]),
        Line::from(vec![Span::raw("   • Navigate step execution")]),
        Line::from(""),
        Line::from(vec![
            Span::styled(
                "3. Logs",
                Style::default()
                    .fg(Color::Blue)
                    .add_modifier(Modifier::BOLD),
            ),
            Span::raw(" - View execution logs"),
        ]),
        Line::from(vec![Span::raw("   • Search and filter logs")]),
        Line::from(vec![Span::raw("   • Real-time log streaming")]),
        Line::from(vec![Span::raw("   • Navigate search results")]),
        Line::from(""),
        Line::from(vec![
            Span::styled(
                "4. Help",
                Style::default()
                    .fg(Color::Yellow)
                    .add_modifier(Modifier::BOLD),
            ),
            Span::raw(" - This comprehensive guide"),
        ]),
        Line::from(""),
        Line::from(Span::styled(
            "🎯 QUICK ACTIONS",
            Style::default().fg(Color::Red).add_modifier(Modifier::BOLD),
        )),
        Line::from(""),
        Line::from(vec![
            Span::styled(
                "?",
                Style::default()
                    .fg(Color::Yellow)
                    .add_modifier(Modifier::BOLD),
            ),
            Span::raw(" - Toggle help overlay"),
        ]),
        Line::from(vec![
            Span::styled(
                "q",
                Style::default()
                    .fg(Color::Yellow)
                    .add_modifier(Modifier::BOLD),
            ),
            Span::raw(" - Quit application"),
        ]),
        Line::from(""),
        Line::from(Span::styled(
            "💡 TIPS",
            Style::default()
                .fg(Color::Yellow)
                .add_modifier(Modifier::BOLD),
        )),
        Line::from(""),
        Line::from(vec![
            Span::raw("• Use "),
            Span::styled("emulation mode", Style::default().fg(Color::Red)),
            Span::raw(" when containers"),
        ]),
        Line::from(vec![Span::raw("  are unavailable or for quick testing")]),
        Line::from(""),
        Line::from(vec![
            Span::raw("• "),
            Span::styled("Secure emulation", Style::default().fg(Color::Yellow)),
            Span::raw(" provides sandboxing"),
        ]),
        Line::from(vec![Span::raw("  for untrusted workflows")]),
        Line::from(""),
        Line::from(vec![
            Span::raw("• Use "),
            Span::styled("validation mode", Style::default().fg(Color::Green)),
            Span::raw(" to check"),
        ]),
        Line::from(vec![Span::raw("  workflows without execution")]),
        Line::from(""),
        Line::from(vec![
            Span::raw("• "),
            Span::styled("Preserve containers", Style::default().fg(Color::Blue)),
            Span::raw(" on failure"),
        ]),
        Line::from(vec![Span::raw("  for debugging (Docker/Podman only)")]),
    ];

    // Apply scroll offset to the content
    let left_help_text = if scroll_offset < left_help_text.len() {
        left_help_text.into_iter().skip(scroll_offset).collect()
    } else {
        vec![Line::from("")]
    };

    let right_help_text = if scroll_offset < right_help_text.len() {
        right_help_text.into_iter().skip(scroll_offset).collect()
    } else {
        vec![Line::from("")]
    };

    // Render left column
    let left_widget = Paragraph::new(left_help_text)
        .block(
            Block::default()
                .borders(Borders::ALL)
                .border_type(BorderType::Rounded)
                .title(Span::styled(
                    " WRKFLW Help - Controls & Features ",
                    Style::default()
                        .fg(Color::Yellow)
                        .add_modifier(Modifier::BOLD),
                )),
        )
        .wrap(Wrap { trim: true });

    // Render right column
    let right_widget = Paragraph::new(right_help_text)
        .block(
            Block::default()
                .borders(Borders::ALL)
                .border_type(BorderType::Rounded)
                .title(Span::styled(
                    " Interface Guide & Tips ",
                    Style::default()
                        .fg(Color::Cyan)
                        .add_modifier(Modifier::BOLD),
                )),
        )
        .wrap(Wrap { trim: true });

    f.render_widget(left_widget, chunks[0]);
    f.render_widget(right_widget, chunks[1]);
}

// Render a help overlay
pub fn render_help_overlay(f: &mut Frame<CrosstermBackend<io::Stdout>>, scroll_offset: usize) {
    let size = f.size();

    // Create a larger centered modal to accommodate comprehensive help content
    let width = (size.width * 9 / 10).min(120); // Use 90% of width, max 120 chars
    let height = (size.height * 9 / 10).min(40); // Use 90% of height, max 40 lines
    let x = (size.width - width) / 2;
    let y = (size.height - height) / 2;

    let help_area = Rect {
        x,
        y,
        width,
        height,
    };

    // Create a semi-transparent dark background for better visibility
    let clear = Block::default().style(Style::default().bg(Color::Black));
    f.render_widget(clear, size);

    // Add a border around the entire overlay for better visual separation
    let overlay_block = Block::default()
        .borders(Borders::ALL)
        .border_type(BorderType::Double)
        .style(Style::default().bg(Color::Black).fg(Color::White))
        .title(Span::styled(
            " Press ? or Esc to close help ",
            Style::default()
                .fg(Color::Gray)
                .add_modifier(Modifier::ITALIC),
        ));

    f.render_widget(overlay_block, help_area);

    // Create inner area for content
    let inner_area = Rect {
        x: help_area.x + 1,
        y: help_area.y + 1,
        width: help_area.width.saturating_sub(2),
        height: help_area.height.saturating_sub(2),
    };

    // Render the help content with scroll support
    render_help_content(f, inner_area, scroll_offset);
}
