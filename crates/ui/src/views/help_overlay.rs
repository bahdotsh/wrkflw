// Help overlay rendering
use crate::theme::{self, Theme};
use ratatui::{
    layout::{Constraint, Direction, Layout, Rect},
    style::{Modifier, Style},
    text::{Line, Span},
    widgets::{Block, BorderType, Borders, Paragraph, Wrap},
    Frame,
};

fn section_header<'a>(t: &Theme, title: &'a str) -> Vec<Line<'a>> {
    vec![
        Line::from(Span::styled(
            title,
            Style::default()
                .fg(t.accent)
                .add_modifier(Modifier::BOLD | Modifier::UNDERLINED),
        )),
        Line::from(Span::styled(
            theme::symbols::HRULE.repeat(title.len()),
            Style::default().fg(t.fg_muted),
        )),
    ]
}

fn key_line<'a>(t: &Theme, key: &'a str, desc: &'a str) -> Line<'a> {
    Line::from(vec![
        Span::styled(
            format!(" {:16}", key),
            Style::default()
                .fg(t.highlight)
                .add_modifier(Modifier::BOLD),
        ),
        Span::styled(desc, Style::default().fg(t.fg_dim)),
    ])
}

// Render the help tab with scroll support
pub fn render_help_content(f: &mut Frame<'_>, t: &Theme, area: Rect, scroll_offset: usize) {
    let chunks = Layout::default()
        .direction(Direction::Horizontal)
        .constraints([Constraint::Percentage(50), Constraint::Percentage(50)].as_ref())
        .split(area);

    // Left column
    let mut left_lines: Vec<Line> = Vec::new();

    left_lines.extend(section_header(t, "NAVIGATION"));
    left_lines.push(Line::from(""));
    left_lines.push(key_line(t, "Tab / Shift+Tab", "Switch between tabs"));
    left_lines.push(key_line(t, "1-4 / w,x,l,h", "Jump to specific tab"));
    left_lines.push(key_line(t, "\u{2191}/\u{2193} or k/j", "Navigate lists"));
    left_lines.push(key_line(t, "Enter", "Select / View details"));
    left_lines.push(key_line(t, "Esc", "Back / Exit help"));
    left_lines.push(Line::from(""));

    left_lines.extend(section_header(t, "WORKFLOW MANAGEMENT"));
    left_lines.push(Line::from(""));
    left_lines.push(key_line(t, "Space", "Toggle workflow selection"));
    left_lines.push(key_line(t, "r", "Run selected workflows"));
    left_lines.push(key_line(t, "a", "Select all workflows"));
    left_lines.push(key_line(t, "n", "Deselect all workflows"));
    left_lines.push(key_line(t, "Shift+R", "Reset workflow status"));
    left_lines.push(key_line(t, "t", "Trigger remote workflow"));
    left_lines.push(Line::from(""));

    left_lines.extend(section_header(t, "JOB SELECTION"));
    left_lines.push(Line::from(""));
    left_lines.push(key_line(t, "Shift+J", "View jobs in workflow"));
    left_lines.push(key_line(t, "Enter (in jobs)", "Run selected job"));
    left_lines.push(key_line(t, "a (in jobs)", "Run all jobs"));
    left_lines.push(key_line(t, "Esc (in jobs)", "Back to workflow list"));
    left_lines.push(Line::from(""));

    left_lines.extend(section_header(t, "EXECUTION MODES"));
    left_lines.push(Line::from(""));
    left_lines.push(key_line(t, "e", "Toggle emulation mode"));
    left_lines.push(key_line(t, "v", "Toggle validation mode"));
    left_lines.push(key_line(t, "d", "Toggle diff-aware trigger filter"));
    left_lines.push(key_line(t, "D", "Cycle diff filter event (push/PR/...)"));
    left_lines.push(Line::from(""));
    left_lines.push(Line::from(Span::styled(
        "Runtime Modes:",
        Style::default()
            .fg(t.fg_bright)
            .add_modifier(Modifier::BOLD),
    )));
    left_lines.push(Line::from(vec![
        Span::raw("  \u{2022} "),
        Span::styled("Docker", Style::default().fg(t.runtime_docker)),
        Span::styled(
            " \u{2500} Container isolation (default)",
            theme::dim_style(t),
        ),
    ]));
    left_lines.push(Line::from(vec![
        Span::raw("  \u{2022} "),
        Span::styled("Podman", Style::default().fg(t.runtime_podman)),
        Span::styled(" \u{2500} Rootless containers", theme::dim_style(t)),
    ]));
    left_lines.push(Line::from(vec![
        Span::raw("  \u{2022} "),
        Span::styled("Emulation", Style::default().fg(t.runtime_emulation)),
        Span::styled(" \u{2500} Process mode (UNSAFE)", theme::dim_style(t)),
    ]));
    left_lines.push(Line::from(vec![
        Span::raw("  \u{2022} "),
        Span::styled("Secure Emulation", Style::default().fg(t.runtime_secure)),
        Span::styled(" \u{2500} Sandboxed processes", theme::dim_style(t)),
    ]));

    // Right column
    let mut right_lines: Vec<Line> = Vec::new();

    right_lines.extend(section_header(t, "LOGS & SEARCH"));
    right_lines.push(Line::from(""));
    right_lines.push(key_line(t, "s", "Toggle log search"));
    right_lines.push(key_line(t, "f", "Toggle log filter"));
    right_lines.push(key_line(t, "c", "Clear search & filter"));
    right_lines.push(key_line(t, "n", "Next search match"));
    right_lines.push(key_line(t, "\u{2191}/\u{2193}", "Scroll logs / Navigate"));
    right_lines.push(Line::from(""));

    right_lines.extend(section_header(t, "TAB OVERVIEW"));
    right_lines.push(Line::from(""));
    right_lines.push(Line::from(vec![
        Span::styled(
            "1. Workflows",
            Style::default().fg(t.accent).add_modifier(Modifier::BOLD),
        ),
        Span::styled(" \u{2500} Browse & select workflows", theme::dim_style(t)),
    ]));
    right_lines.push(Line::from(Span::styled(
        "   View, select, and run workflows",
        theme::muted_style(t),
    )));
    right_lines.push(Line::from(""));
    right_lines.push(Line::from(vec![
        Span::styled(
            "2. Execution",
            Style::default().fg(t.success).add_modifier(Modifier::BOLD),
        ),
        Span::styled(" \u{2500} Monitor job progress", theme::dim_style(t)),
    ]));
    right_lines.push(Line::from(Span::styled(
        "   Track jobs, steps, and output",
        theme::muted_style(t),
    )));
    right_lines.push(Line::from(""));
    right_lines.push(Line::from(vec![
        Span::styled(
            "3. Logs",
            Style::default().fg(t.info).add_modifier(Modifier::BOLD),
        ),
        Span::styled(" \u{2500} View execution logs", theme::dim_style(t)),
    ]));
    right_lines.push(Line::from(Span::styled(
        "   Search, filter, real-time streaming",
        theme::muted_style(t),
    )));
    right_lines.push(Line::from(""));
    right_lines.push(Line::from(vec![
        Span::styled(
            "4. Help",
            Style::default()
                .fg(t.highlight)
                .add_modifier(Modifier::BOLD),
        ),
        Span::styled(" \u{2500} This guide", theme::dim_style(t)),
    ]));
    right_lines.push(Line::from(""));

    right_lines.extend(section_header(t, "QUICK ACTIONS"));
    right_lines.push(Line::from(""));
    right_lines.push(key_line(t, "?", "Toggle help overlay"));
    right_lines.push(key_line(t, "q", "Quit application"));
    right_lines.push(Line::from(""));

    right_lines.extend(section_header(t, "TIPS"));
    right_lines.push(Line::from(""));
    right_lines.push(Line::from(vec![
        Span::raw("\u{2022} Use "),
        Span::styled("emulation mode", Style::default().fg(t.runtime_emulation)),
        Span::styled(" when containers are unavailable", theme::dim_style(t)),
    ]));
    right_lines.push(Line::from(vec![
        Span::raw("\u{2022} "),
        Span::styled("Secure emulation", Style::default().fg(t.runtime_secure)),
        Span::styled(
            " provides sandboxing for untrusted workflows",
            theme::dim_style(t),
        ),
    ]));
    right_lines.push(Line::from(vec![
        Span::raw("\u{2022} Use "),
        Span::styled("validation mode", Style::default().fg(t.success)),
        Span::styled(" to check workflows without execution", theme::dim_style(t)),
    ]));
    right_lines.push(Line::from(""));
    right_lines.push(Line::from(Span::styled(
        "\u{2191}\u{2193} scroll \u{2502} ? close",
        theme::muted_style(t),
    )));

    // Apply scroll offset
    let left_lines: Vec<Line> = if scroll_offset < left_lines.len() {
        left_lines.into_iter().skip(scroll_offset).collect()
    } else {
        vec![Line::from("")]
    };

    let right_lines: Vec<Line> = if scroll_offset < right_lines.len() {
        right_lines.into_iter().skip(scroll_offset).collect()
    } else {
        vec![Line::from("")]
    };

    let left_widget = Paragraph::new(left_lines)
        .block(
            Block::default()
                .borders(Borders::ALL)
                .border_type(BorderType::Rounded)
                .border_style(Style::default().fg(t.border))
                .title(Span::styled(" Controls & Features ", theme::title_style(t))),
        )
        .wrap(Wrap { trim: true });

    let right_widget = Paragraph::new(right_lines)
        .block(
            Block::default()
                .borders(Borders::ALL)
                .border_type(BorderType::Rounded)
                .border_style(Style::default().fg(t.border))
                .title(Span::styled(" Interface Guide ", theme::title_style(t))),
        )
        .wrap(Wrap { trim: true });

    f.render_widget(left_widget, chunks[0]);
    f.render_widget(right_widget, chunks[1]);
}

// Render a help overlay
pub fn render_help_overlay(f: &mut Frame<'_>, t: &Theme, scroll_offset: usize) {
    let size = f.area();

    let width = (size.width * 9 / 10).min(120);
    let height = (size.height * 9 / 10).min(40);
    let x = (size.width - width) / 2;
    let y = (size.height - height) / 2;

    let help_area = Rect {
        x,
        y,
        width,
        height,
    };

    // Dim the rest of the screen behind the overlay
    let dim = Block::default().style(Style::default().bg(t.bg_modal_dim));
    f.render_widget(dim, size);

    // Overlay border
    let overlay_block = Block::default()
        .borders(Borders::ALL)
        .border_type(BorderType::Double)
        .style(Style::default().bg(t.bg_modal).fg(t.border_focused))
        .title(Span::styled(
            " Press ? or Esc to close ",
            theme::muted_style(t),
        ));

    f.render_widget(overlay_block, help_area);

    let inner_area = Rect {
        x: help_area.x + 1,
        y: help_area.y + 1,
        width: help_area.width.saturating_sub(2),
        height: help_area.height.saturating_sub(2),
    };

    render_help_content(f, t, inner_area, scroll_offset);
}
