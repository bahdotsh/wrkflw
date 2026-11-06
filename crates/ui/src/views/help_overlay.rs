// Help — every key binding, grouped by where it applies. Floats over
// the current screen (? or h). Two columns when
// there's room, one otherwise; scrolling is clamped to the content.
use crate::text;
use crate::theme::{self, COLORS};
use ratatui::{
    layout::{Constraint, Direction, Layout, Rect},
    style::{Modifier, Style},
    text::{Line, Span},
    widgets::Paragraph,
    Frame,
};

type Section = (&'static str, Vec<(String, &'static str)>);

fn sections() -> Vec<Section> {
    let k = |s: &str| s.to_string();
    vec![
        (
            "Navigation",
            vec![
                (k("Tab / Shift+Tab"), "next / previous tab"),
                (format!("1–{}", super::TAB_COUNT), "jump to a tab"),
                (k("w"), "Workflows"),
                (k("x g l"), "Run: jobs · graph · log"),
                (k("↑↓ or k j"), "move in lists"),
                (k("Enter"), "open / run"),
                (k("Esc"), "back"),
                (k("? or h"), "this help"),
                (k(","), "tweaks (accent colour)"),
                (k("q or Ctrl+C"), "quit"),
            ],
        ),
        (
            "Workflows",
            vec![
                (k("Space"), "check or uncheck"),
                (k("a / n"), "check all / none"),
                (k("Enter"), "run the highlighted workflow"),
                (k("r"), "run checked, else highlighted"),
                (k("Home / End"), "first / last workflow"),
                (k("Shift+J"), "pick one job to run"),
                (k("Shift+R"), "reset statuses"),
                (k("t"), "dispatch on GitHub / GitLab"),
                (k("e"), "cycle runtime"),
                (k("v"), "validate instead of run"),
                (k("d / Shift+D"), "diff filter on/off · event"),
            ],
        ),
        (
            "Run · jobs",
            vec![
                (k("↑↓"), "choose a job"),
                (k("Home / End"), "first / last job"),
                (k("Enter"), "inspect its steps"),
                (k("Tab"), "Output ↔ Matrix (inspector)"),
                (k("PgUp / PgDn"), "scroll step output"),
                (k("End"), "back to the tail"),
            ],
        ),
        (
            "Run · graph",
            vec![
                (k("g"), "graph ↔ list"),
                (k("← →"), "scroll stages"),
                (k("↑↓"), "previous / next workflow"),
                (k("Enter"), "back to jobs"),
            ],
        ),
        (
            "Run · log",
            vec![
                (k("o"), "story ↔ raw log"),
                (k("s"), "search the raw log"),
                (k("n / ↑↓"), "next / previous match"),
                (k("f"), "filter by level"),
                (k("c"), "clear search and filter"),
                (k("PgUp / PgDn"), "page"),
                (k("Home / End"), "first line / follow new lines"),
            ],
        ),
        (
            "Dispatch",
            vec![
                (k("p"), "GitHub ↔ GitLab"),
                (k("↑↓"), "workflow"),
                (k("b"), "edit branch"),
                (k("i"), "edit inputs"),
                (k("+"), "add an input"),
                (k("Tab"), "next field, while editing"),
                (k("Enter"), "dispatch / finish editing"),
                (k("Esc"), "stop editing"),
                (k("c"), "copy the curl command to the log"),
            ],
        ),
        (
            "Setup",
            vec![
                (k("e"), "cycle runtime (works on any tab)"),
                (k("↑↓"), "secret provider"),
            ],
        ),
        (
            "History",
            vec![
                (k("↑↓"), "choose a run"),
                (k("Home / End"), "newest / oldest"),
            ],
        ),
        (
            "Mouse",
            vec![
                (k("drag"), "select text, copied on release"),
                (k("wheel"), "scroll"),
            ],
        ),
    ]
}

const KEY_W: usize = 17;

fn section_lines(section: &Section, w: usize) -> Vec<Line<'static>> {
    let mut lines = vec![theme::section(section.0)];
    for (key, desc) in &section.1 {
        lines.push(Line::from(vec![
            Span::styled(
                text::fit(key, KEY_W),
                Style::default()
                    .fg(theme::current_accent())
                    .add_modifier(Modifier::BOLD),
            ),
            Span::styled(
                text::truncate(desc, w.saturating_sub(KEY_W)).into_owned(),
                Style::default().fg(COLORS.text_dim),
            ),
        ]));
    }
    lines.push(Line::from(""));
    lines
}

/// Renders the bindings into `area`, clamping `scroll` to the content.
pub fn render_help_content(f: &mut Frame<'_>, area: Rect, scroll: &mut usize) {
    let block = theme::modal_frame("Keys");
    let inner = block.inner(area);
    let two_columns = inner.width >= 90;
    let col_w = if two_columns {
        (inner.width as usize - 3) / 2
    } else {
        inner.width as usize
    };

    let all = sections();
    let mut left: Vec<Line> = Vec::new();
    let mut right: Vec<Line> = Vec::new();
    if two_columns {
        // Fill the left column to about half the rows, then the right.
        let total: usize = all.iter().map(|s| s.1.len() + 2).sum();
        for s in &all {
            if left.len() < total / 2 {
                left.extend(section_lines(s, col_w));
            } else {
                right.extend(section_lines(s, col_w));
            }
        }
    } else {
        for s in &all {
            left.extend(section_lines(s, col_w));
        }
    }

    let content = left.len().max(right.len());
    let viewport = inner.height as usize;
    *scroll = (*scroll).min(content.saturating_sub(viewport));
    let badge = if content > viewport {
        "↑↓ scroll"
    } else {
        ""
    };
    f.render_widget(theme::with_badge(block, badge), area);

    if two_columns {
        let cols = Layout::default()
            .direction(Direction::Horizontal)
            .constraints([
                Constraint::Length(col_w as u16),
                Constraint::Length(3),
                Constraint::Min(0),
            ])
            .split(inner);
        f.render_widget(Paragraph::new(left).scroll((*scroll as u16, 0)), cols[0]);
        f.render_widget(Paragraph::new(right).scroll((*scroll as u16, 0)), cols[2]);
    } else {
        f.render_widget(Paragraph::new(left).scroll((*scroll as u16, 0)), inner);
    }
    theme::scrollbar(f, area, content, *scroll, viewport);
}

/// Help floating over whatever screen is open.
pub fn render_help_overlay(f: &mut Frame<'_>, scroll: &mut usize) {
    let screen = f.area();
    let (width, height) = if screen.width < 60 || screen.height < 16 {
        (screen.width, screen.height)
    } else {
        (
            (screen.width * 9 / 10).min(120),
            (screen.height * 9 / 10).min(38),
        )
    };
    let area = theme::modal(f, width, height);
    render_help_content(f, area, scroll);
    // A muted close hint along the bottom border.
    if area.height > 2 && area.width > 20 {
        let hint = " ? or Esc to close ";
        let x = area.right().saturating_sub(text::width(hint) as u16 + 2);
        f.render_widget(
            Paragraph::new(Span::styled(hint, theme::muted_style())),
            Rect::new(x, area.bottom() - 1, text::width(hint) as u16, 1),
        );
    }
}
