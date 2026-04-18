// Help modal.
//
// Single centered overlay; `?` / `Esc` dismisses. Sections have a bullet
// header and an underline; each row is `[ key ] description` with the key
// rendered as an inverse-video badge (mdterm/giff pattern).

use crate::app::App;
use crate::theme::{self, Theme};
use ratatui::{
    layout::{Alignment, Constraint, Direction, Layout, Rect},
    style::{Modifier, Style},
    text::{Line, Span, Text},
    widgets::{Block, BorderType, Borders, Clear, Paragraph},
    Frame,
};

const MAX_MODAL_WIDTH: u16 = 68;
const MAX_MODAL_HEIGHT: u16 = 34;

/// Render the help modal centered over the current frame.
pub fn render_help_modal(f: &mut Frame<'_>, app: &App) {
    let t = &app.theme;
    let area = f.area();

    // Dim the rest of the frame so the modal reads as foreground.
    let dim = Block::default().style(Style::default().bg(t.bg_modal_dim));
    f.render_widget(dim, area);

    let modal = centered_rect(MAX_MODAL_WIDTH, MAX_MODAL_HEIGHT, area);
    f.render_widget(Clear, modal);

    let block = Block::default()
        .borders(Borders::ALL)
        .border_type(BorderType::Rounded)
        .border_style(Style::default().fg(t.border_focused))
        .style(Style::default().bg(t.bg_modal))
        .title(Span::styled(
            " Keybindings ",
            Style::default()
                .fg(t.accent)
                .add_modifier(Modifier::BOLD),
        ))
        .title_bottom(
            Line::from(vec![
                Span::styled(" ? ", key_badge_style(t)),
                Span::raw(" "),
                Span::styled(" Esc ", key_badge_style(t)),
                Span::styled(" to close ", Style::default().fg(t.help_hint)),
            ])
            .alignment(Alignment::Center),
        );

    let inner = block.inner(modal);
    f.render_widget(block, modal);

    let inner_width = inner.width as usize;
    let lines = build_lines(t, inner_width, app);
    let visible = inner.height as usize;
    let scroll = app.help_scroll.min(lines.len().saturating_sub(visible));
    let visible_lines: Vec<Line> = lines.into_iter().skip(scroll).take(visible).collect();

    let paragraph =
        Paragraph::new(Text::from(visible_lines)).style(Style::default().bg(t.bg_modal));
    f.render_widget(paragraph, inner);
}

fn build_lines(t: &Theme, width: usize, app: &App) -> Vec<Line<'static>> {
    let mut out = Vec::new();

    out.push(Line::from(""));
    out.extend(section(t, "Navigation"));
    out.push(row(t, "Tab / S-Tab", "Cycle tabs"));
    out.push(row(t, "1 · 2 · 3", "Jump to tab"));
    out.push(row(t, "↑↓ / k j", "Navigate lists"));
    out.push(row(t, "Enter", "Select / view details"));
    out.push(row(t, "Esc", "Back / exit"));
    out.push(separator(t, width));

    out.extend(section(t, "Workflows"));
    out.push(row(t, "Space", "Toggle selection"));
    out.push(row(t, "r", "Run selected"));
    out.push(row(t, "a", "Select all"));
    out.push(row(t, "n", "Deselect all"));
    out.push(row(t, "S-R", "Reset status"));
    out.push(row(t, "t", "Trigger remote"));
    out.push(row(t, "S-J", "View jobs"));
    out.push(separator(t, width));

    out.extend(section(t, "Modes"));
    out.push(row(t, "e", "Toggle emulation"));
    out.push(row(t, "v", "Toggle validation"));
    out.push(row(t, "d", "Toggle diff filter"));
    out.push(row(t, "D", "Cycle diff event"));
    out.push(row(t, "T", "Toggle light / dark"));
    out.push(separator(t, width));

    out.extend(section(t, "Logs"));
    out.push(row(t, "s", "Search"));
    out.push(row(t, "f", "Filter"));
    out.push(row(t, "c", "Clear search & filter"));
    out.push(row(t, "n", "Next match"));
    out.push(separator(t, width));

    out.extend(section(t, "Runtimes"));
    out.push(runtime_row(t, "Docker", t.runtime_docker, "container isolation"));
    out.push(runtime_row(t, "Podman", t.runtime_podman, "rootless containers"));
    out.push(runtime_row(t, "Secure", t.runtime_secure, "sandboxed processes"));
    out.push(runtime_row(t, "Emul.", t.runtime_emulation, "process mode (unsafe)"));
    out.push(separator(t, width));

    out.extend(section(t, "General"));
    out.push(row(t, "?", "Toggle this help"));
    out.push(row(t, "q", "Quit"));

    // Include selected-tab for the help_scroll accounting.
    let _ = app.selected_tab;
    out
}

fn section(t: &Theme, title: &str) -> Vec<Line<'static>> {
    vec![
        Line::from(""),
        Line::from(vec![
            Span::styled("  ● ", Style::default().fg(t.accent)),
            Span::styled(
                title.to_string(),
                Style::default()
                    .fg(t.accent)
                    .add_modifier(Modifier::BOLD),
            ),
        ]),
        Line::from(""),
    ]
}

fn row(t: &Theme, key: &str, desc: &str) -> Line<'static> {
    Line::from(vec![
        Span::raw("    "),
        Span::styled(format!(" {:^10} ", key), key_badge_style(t)),
        Span::raw("  "),
        Span::styled(desc.to_string(), Style::default().fg(t.fg_normal)),
    ])
}

fn runtime_row(t: &Theme, name: &str, color: ratatui::style::Color, desc: &str) -> Line<'static> {
    Line::from(vec![
        Span::raw("    "),
        Span::styled(
            format!(" {:^10} ", name),
            Style::default().bg(color).fg(t.fg_badge).add_modifier(Modifier::BOLD),
        ),
        Span::raw("  "),
        Span::styled(desc.to_string(), Style::default().fg(t.fg_normal)),
    ])
}

fn separator(t: &Theme, width: usize) -> Line<'static> {
    let inner = width.saturating_sub(2);
    Line::from(Span::styled(
        format!(" {} ", theme::symbols::HRULE.repeat(inner)),
        Style::default().fg(t.fg_separator),
    ))
}

fn key_badge_style(t: &Theme) -> Style {
    Style::default()
        .bg(t.bg_key_badge)
        .fg(t.highlight)
        .add_modifier(Modifier::BOLD)
}

fn centered_rect(width: u16, height: u16, r: Rect) -> Rect {
    if r.width == 0 || r.height == 0 {
        return r;
    }
    let height = height.min(r.height);
    let width = width.min(r.width);

    let vert_margin = 100u16.saturating_sub(height * 100 / r.height) / 2;
    let horiz_margin = 100u16.saturating_sub(width * 100 / r.width) / 2;

    let vlayout = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Percentage(vert_margin),
            Constraint::Length(height),
            Constraint::Percentage(vert_margin),
        ])
        .split(r);

    Layout::default()
        .direction(Direction::Horizontal)
        .constraints([
            Constraint::Percentage(horiz_margin),
            Constraint::Length(width),
            Constraint::Percentage(horiz_margin),
        ])
        .split(vlayout[1])[1]
}
