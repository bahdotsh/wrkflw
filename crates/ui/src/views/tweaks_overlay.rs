// Tweaks overlay: the accent picker, as a centred modal.
//
// We wire up the knobs that actually plumb through to the theme and
// layouts. Anything we *can't* back up end-to-end (e.g. switching
// between the dark and light themes mid-session, which log entries
// cached with the old palette would not follow) is omitted rather than
// rendered as a dead toggle — matches the rule from PR #104: "A UI
// without backing data is worse than no UI."
//
// The key dispatch in `app/mod.rs` treats the overlay as modal:
// while `tweaks_open` is true, unmatched keys are swallowed instead
// of falling through to the global handler. The one exception is `q`,
// which always quits — swallowing quit silently is a discoverability
// trap, and quit is universally modal-safe in this TUI.
//
// Controls:
//   - `a` / `A` : cycle accent forwards (wraps)
//   - `esc` / `,` : close
//   - `q` : quit (same as anywhere else)

use crate::app::{Accent, App};
use crate::theme::{self, COLORS};
use ratatui::{
    layout::{Constraint, Direction, Layout, Rect},
    style::{Color, Modifier, Style},
    text::{Line, Span},
    widgets::Paragraph,
    Frame,
};

/// Fits the accent row: label, five swatches and the accent's name.
const PANEL_W: u16 = 44;
const PANEL_H: u16 = 6;

pub fn render_tweaks_overlay(f: &mut Frame<'_>, app: &App, _area: Rect) {
    // A centred modal over a muted backdrop, the same as Help, so it never
    // lands on top of a pane's border and reads as part of that pane.
    let panel_rect = theme::modal(f, PANEL_W, PANEL_H);

    let block = theme::modal_frame("Tweaks");
    let inner = block.inner(panel_rect);
    f.render_widget(block, panel_rect);

    let rows = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Length(2),
            Constraint::Length(1),
            Constraint::Min(0),
        ])
        .split(inner);

    render_accent_row(f, app, rows[0]);
    render_shortcut_hint(f, rows[1]);
}

fn render_accent_row(f: &mut Frame<'_>, app: &App, area: Rect) {
    let swatch = |c: Accent, active: bool| -> Span<'_> {
        let (r, g, b) = c.rgb();
        let bg = Color::Rgb(r, g, b);
        let label = format!(" {} ", if active { "●" } else { " " });
        Span::styled(
            label,
            Style::default()
                .bg(bg)
                .fg(COLORS.bg_dark)
                .add_modifier(if active {
                    Modifier::BOLD
                } else {
                    Modifier::empty()
                }),
        )
    };
    let mut spans: Vec<Span> = vec![Span::styled(
        "accent  ",
        Style::default().fg(COLORS.text_muted),
    )];
    for c in [
        Accent::Blueprint,
        Accent::Cyan,
        Accent::Green,
        Accent::Violet,
        Accent::Coral,
    ] {
        spans.push(swatch(c, app.tweaks_accent == c));
        spans.push(Span::raw(" "));
    }
    spans.push(Span::styled(
        app.tweaks_accent.as_str(),
        Style::default().fg(COLORS.text),
    ));
    f.render_widget(Paragraph::new(Line::from(spans)), area);
}

fn render_shortcut_hint(f: &mut Frame<'_>, area: Rect) {
    let spans = vec![
        theme::key_chip("a"),
        Span::raw(" "),
        Span::styled("cycle accent", Style::default().fg(COLORS.text_dim)),
        Span::raw("   "),
        theme::key_chip(","),
        Span::raw(" "),
        Span::styled("close", Style::default().fg(COLORS.text_dim)),
    ];
    f.render_widget(Paragraph::new(Line::from(spans)), area);
}
