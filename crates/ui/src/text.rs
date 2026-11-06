// Display-width-aware text helpers for the views.
//
// Terminal cells are not chars: CJK and most emoji take two cells and
// combining marks take none, so anything that pads or truncates by
// `chars().count()` misaligns columns or overflows its pane. Every
// helper here measures with `unicode-width`.
use crate::theme::COLORS;
use ratatui::{
    style::{Color, Modifier, Style},
    text::{Line, Span},
};
use std::borrow::Cow;
use unicode_width::{UnicodeWidthChar, UnicodeWidthStr};

const ELLIPSIS: char = '…';
const TAB_STOP: usize = 4;

/// Cells `s` occupies on screen.
pub fn width(s: &str) -> usize {
    UnicodeWidthStr::width(s)
}

/// Replaces tabs with spaces up to the next tab stop, line breaks with a
/// single space, and drops other control characters, which ratatui would
/// otherwise render as nothing and silently glue neighbouring words
/// together.
pub fn sanitize(s: &str) -> Cow<'_, str> {
    if !s.chars().any(|c| c.is_control()) {
        return Cow::Borrowed(s);
    }
    let mut out = String::with_capacity(s.len() + 8);
    let mut col = 0;
    for c in s.chars() {
        match c {
            '\t' => {
                let n = TAB_STOP - col % TAB_STOP;
                out.extend(std::iter::repeat_n(' ', n));
                col += n;
            }
            '\n' | '\r' => {
                out.push(' ');
                col += 1;
            }
            c if c.is_control() => {}
            c => {
                out.push(c);
                col += c.width().unwrap_or(0);
            }
        }
    }
    Cow::Owned(out)
}

/// Cuts `s` to at most `max` cells, ending in `…` when anything was cut.
pub fn truncate(s: &str, max: usize) -> Cow<'_, str> {
    if width(s) <= max {
        return Cow::Borrowed(s);
    }
    if max == 0 {
        return Cow::Borrowed("");
    }
    let mut out = String::new();
    let mut used = 0;
    for c in s.chars() {
        let w = c.width().unwrap_or(0);
        if used + w > max - 1 {
            break;
        }
        out.push(c);
        used += w;
    }
    out.push(ELLIPSIS);
    Cow::Owned(out)
}

/// Like [`truncate`] but keeps the end, for paths whose file name is the
/// part worth reading.
pub fn truncate_left(s: &str, max: usize) -> Cow<'_, str> {
    if width(s) <= max {
        return Cow::Borrowed(s);
    }
    if max == 0 {
        return Cow::Borrowed("");
    }
    let mut kept: Vec<char> = Vec::new();
    let mut used = 0;
    for c in s.chars().rev() {
        let w = c.width().unwrap_or(0);
        if used + w > max - 1 {
            break;
        }
        kept.push(c);
        used += w;
    }
    let mut out = String::from(ELLIPSIS);
    out.extend(kept.into_iter().rev());
    Cow::Owned(out)
}

/// Truncates to `cells` and right-pads with spaces to exactly `cells`.
pub fn fit(s: &str, cells: usize) -> String {
    let t = truncate(s, cells);
    let pad = cells.saturating_sub(width(&t));
    let mut out = t.into_owned();
    out.extend(std::iter::repeat_n(' ', pad));
    out
}

/// Left-pads with spaces to `cells`; never truncates.
pub fn pad_left(s: &str, cells: usize) -> String {
    let pad = cells.saturating_sub(width(s));
    let mut out: String = std::iter::repeat_n(' ', pad).collect();
    out.push_str(s);
    out
}

/// Fits a run of spans to exactly `cells`: the last span that overflows
/// is truncated with `…` and the rest dropped, and a short run is padded.
pub fn fit_spans(spans: Vec<Span<'static>>, cells: usize) -> Vec<Span<'static>> {
    let mut out = Vec::with_capacity(spans.len() + 1);
    let mut used = 0;
    for span in spans {
        let w = width(&span.content);
        if used + w <= cells {
            used += w;
            out.push(span);
            continue;
        }
        let cut = truncate(&span.content, cells - used).into_owned();
        used += width(&cut);
        out.push(Span::styled(cut, span.style));
        break;
    }
    if used < cells {
        out.push(Span::raw(" ".repeat(cells - used)));
    }
    out
}

/// Total cells of a run of spans.
pub fn spans_width(spans: &[Span<'_>]) -> usize {
    spans.iter().map(|s| width(&s.content)).sum()
}

/// Turns one line of program output into styled spans, honouring the
/// SGR colour codes tools like cargo and npm emit. Every other escape
/// sequence is dropped so it can't corrupt the screen.
pub fn ansi_line(raw: &str, base: Style) -> Line<'static> {
    let mut spans: Vec<Span<'static>> = Vec::new();
    let mut style = base;
    let mut buf = String::new();
    let mut chars = raw.chars().peekable();
    while let Some(c) = chars.next() {
        if c != '\x1b' {
            buf.push(c);
            continue;
        }
        match chars.peek() {
            Some('[') => {
                chars.next();
                let mut params = String::new();
                let mut fin = None;
                for n in chars.by_ref() {
                    if ('\x40'..='\x7e').contains(&n) {
                        fin = Some(n);
                        break;
                    }
                    params.push(n);
                }
                if fin == Some('m') {
                    if !buf.is_empty() {
                        spans.push(Span::styled(sanitize(&buf).into_owned(), style));
                        buf.clear();
                    }
                    style = apply_sgr(style, base, &params);
                }
            }
            // OSC (hyperlinks, titles): skip to BEL or ST.
            Some(']') => {
                chars.next();
                while let Some(n) = chars.next() {
                    if n == '\x07' {
                        break;
                    }
                    if n == '\x1b' && chars.peek() == Some(&'\\') {
                        chars.next();
                        break;
                    }
                }
            }
            _ => {
                chars.next();
            }
        }
    }
    if !buf.is_empty() {
        spans.push(Span::styled(sanitize(&buf).into_owned(), style));
    }
    Line::from(spans)
}

/// Applies one SGR parameter string (the part between `ESC[` and `m`).
///
/// Parameters are `;`-separated. A group may also carry `:`-separated
/// sub-parameters (ITU T.416 form, e.g. `38:2::r:g:b` or `4:3`), which
/// is self-contained and never borrows the following groups.
fn apply_sgr(mut style: Style, base: Style, params: &str) -> Style {
    if params.is_empty() {
        return base;
    }
    let groups: Vec<&str> = params.split(';').collect();
    let num = |s: &str| s.parse::<u16>().unwrap_or(0);
    let mut i = 0;
    while i < groups.len() {
        let group = groups[i];
        i += 1;
        if group.contains(':') {
            let sub: Vec<Option<u16>> = group.split(':').map(|p| p.parse().ok()).collect();
            match sub[0] {
                Some(n @ (38 | 48)) => {
                    let color = match sub.get(1).copied().flatten() {
                        Some(5) => sub.get(2).copied().flatten().map(indexed),
                        // `38:2:cs:r:g:b` names a colour space before the
                        // channels; `38:2:r:g:b` omits it.
                        Some(2) => {
                            let rgb = if sub.len() >= 6 {
                                &sub[3..6]
                            } else {
                                &sub[2..]
                            };
                            match rgb {
                                [Some(r), Some(g), Some(b), ..] => Some(rgb_color(*r, *g, *b)),
                                _ => None,
                            }
                        }
                        _ => None,
                    };
                    style = set_color(style, n, color);
                }
                // `4:0` turns underline off; `4:1`..`4:5` are styles of it.
                Some(4) if sub.get(1).copied().flatten() == Some(0) => {
                    style = style.remove_modifier(Modifier::UNDERLINED);
                }
                Some(n) => style = apply_code(style, base, n),
                None => {}
            }
            continue;
        }
        let code = num(group);
        if code == 38 || code == 48 {
            // Legacy form: `38;5;n` or `38;2;r;g;b` across groups.
            let color = match groups.get(i).map(|g| num(g)) {
                Some(5) => {
                    let c = groups.get(i + 1).map(|g| indexed(num(g)));
                    i += 2;
                    c
                }
                Some(2) => {
                    let rgb = (groups.get(i + 1), groups.get(i + 2), groups.get(i + 3));
                    i += 4;
                    match rgb {
                        (Some(r), Some(g), Some(b)) => Some(rgb_color(num(r), num(g), num(b))),
                        _ => None,
                    }
                }
                _ => None,
            };
            style = set_color(style, code, color);
            continue;
        }
        style = apply_code(style, base, code);
    }
    style
}

/// Applies a single SGR code that takes no arguments.
fn apply_code(style: Style, base: Style, code: u16) -> Style {
    let mut style = style;
    match code {
        0 => style = base,
        1 => style = style.add_modifier(Modifier::BOLD),
        2 => style = style.add_modifier(Modifier::DIM),
        3 => style = style.add_modifier(Modifier::ITALIC),
        4 => style = style.add_modifier(Modifier::UNDERLINED),
        7 => style = style.add_modifier(Modifier::REVERSED),
        9 => style = style.add_modifier(Modifier::CROSSED_OUT),
        22 => style = style.remove_modifier(Modifier::BOLD | Modifier::DIM),
        23 => style = style.remove_modifier(Modifier::ITALIC),
        24 => style = style.remove_modifier(Modifier::UNDERLINED),
        27 => style = style.remove_modifier(Modifier::REVERSED),
        29 => style = style.remove_modifier(Modifier::CROSSED_OUT),
        n @ 30..=37 => style = style.fg(ansi_color(n - 30)),
        n @ 90..=97 => style = style.fg(ansi_color(n - 90 + 8)),
        n @ 40..=47 => style = style.bg(ansi_color(n - 40)),
        n @ 100..=107 => style = style.bg(ansi_color(n - 100 + 8)),
        39 => style.fg = base.fg,
        49 => style.bg = base.bg,
        _ => {}
    }
    style
}

fn set_color(style: Style, code: u16, color: Option<Color>) -> Style {
    match (code, color) {
        (38, Some(c)) => style.fg(c),
        (48, Some(c)) => style.bg(c),
        _ => style,
    }
}

fn indexed(v: u16) -> Color {
    Color::Indexed(v.min(255) as u8)
}

fn rgb_color(r: u16, g: u16, b: u16) -> Color {
    Color::Rgb(r.min(255) as u8, g.min(255) as u8, b.min(255) as u8)
}

/// Maps the 16 basic ANSI colours onto the palette so tool output
/// matches the rest of the UI in both light and dark mode.
fn ansi_color(n: u16) -> Color {
    match n % 8 {
        0 => COLORS.text_muted,
        1 => COLORS.error,
        2 => COLORS.success,
        3 => COLORS.warning,
        4 => COLORS.ansi_blue,
        5 => COLORS.ansi_magenta,
        6 => COLORS.info,
        _ => COLORS.text,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn fit_spans_truncates_and_pads_to_exact_width() {
        let spans = vec![Span::raw("ab"), Span::raw("cdefgh")];
        let fitted = fit_spans(spans.clone(), 5);
        assert_eq!(spans_width(&fitted), 5);
        assert_eq!(fitted[1].content, "cd…");
        assert_eq!(spans_width(&fit_spans(spans, 12)), 12);
    }

    #[test]
    fn status_symbols_are_one_cell_wide() {
        use crate::theme::symbols::*;
        for (name, glyph) in [
            ("SUCCESS", SUCCESS),
            ("FAILURE", FAILURE),
            ("RUNNING", RUNNING),
            ("SKIPPED", SKIPPED),
            ("NOT_STARTED", NOT_STARTED),
            ("WARNING", WARNING),
            ("INFO", INFO),
            ("DEBUG", DEBUG),
            ("GEAR", GEAR),
            ("LOCK", LOCK),
            ("BLOCKED", BLOCKED),
            ("ARROW", ARROW),
            ("NESTED", NESTED),
        ] {
            assert_eq!(width(glyph), 1, "{name} ({glyph}) must take one cell");
        }
    }

    #[test]
    fn truncation_counts_cells_not_chars() {
        assert_eq!(truncate("日本語です", 5), "日本…");
        assert_eq!(width(&truncate("日本語です", 5)), 5);
        assert_eq!(truncate("short", 10), "short");
        assert_eq!(truncate("abc", 0), "");
        assert_eq!(truncate_left("a/b/c/file.yml", 9), "…file.yml");
        assert_eq!(fit("日本", 5), "日本 ");
    }

    #[test]
    fn sanitize_expands_tabs_and_drops_controls() {
        assert_eq!(sanitize("a\tb"), "a   b");
        assert_eq!(sanitize("ab\tc"), "ab  c");
        assert_eq!(sanitize("x\u{7}y"), "xy");
        assert_eq!(sanitize("one\ntwo\rthree"), "one two three");
    }

    #[test]
    fn ansi_colours_become_styles_and_escapes_vanish() {
        let line = ansi_line("plain \x1b[1;31merror\x1b[0m done\x1b[2K", Style::default());
        let text: String = line.spans.iter().map(|s| s.content.as_ref()).collect();
        assert_eq!(text, "plain error done");
        let err = line.spans.iter().find(|s| s.content == "error").unwrap();
        assert_eq!(err.style.fg, Some(COLORS.error));
        assert!(err.style.add_modifier.contains(Modifier::BOLD));
        let tail = line.spans.last().unwrap();
        assert_eq!(tail.style, Style::default());
    }

    #[test]
    fn ansi_colon_form_colours_keep_their_channels() {
        let fg = |raw: &str| ansi_line(raw, Style::default()).spans[0].style.fg;
        // Empty colour-space id, as kitty and recent cargo emit.
        assert_eq!(fg("\x1b[38:2::1:2:3mx"), Some(Color::Rgb(1, 2, 3)));
        assert_eq!(fg("\x1b[38:2:0:1:2:3mx"), Some(Color::Rgb(1, 2, 3)));
        assert_eq!(fg("\x1b[38:2:1:2:3mx"), Some(Color::Rgb(1, 2, 3)));
        assert_eq!(fg("\x1b[38:5:200mx"), Some(Color::Indexed(200)));
        // A colon group must not swallow the groups after it.
        let line = ansi_line("\x1b[38:5:1;1mx", Style::default());
        assert_eq!(line.spans[0].style.fg, Some(Color::Indexed(1)));
        assert!(line.spans[0].style.add_modifier.contains(Modifier::BOLD));
        // Out-of-range channels clamp instead of wrapping.
        assert_eq!(fg("\x1b[38;2;300;0;0mx"), Some(Color::Rgb(255, 0, 0)));
    }

    #[test]
    fn ansi_curly_underline_and_underline_off() {
        let s = ansi_line("\x1b[4:3mx", Style::default()).spans[0].style;
        assert!(s.add_modifier.contains(Modifier::UNDERLINED));
        let s = ansi_line("\x1b[4m\x1b[4:0mx", Style::default()).spans[0].style;
        assert!(!s.add_modifier.contains(Modifier::UNDERLINED));
    }

    #[test]
    fn ansi_handles_extended_colours_and_osc_links() {
        let line = ansi_line(
            "\x1b[38;2;1;2;3mrgb\x1b[38;5;200mix\x1b]8;;http://x\x07link\x1b]8;;\x07",
            Style::default(),
        );
        assert_eq!(line.spans[0].style.fg, Some(Color::Rgb(1, 2, 3)));
        assert_eq!(line.spans[1].style.fg, Some(Color::Indexed(200)));
        let text: String = line.spans.iter().map(|s| s.content.as_ref()).collect();
        assert_eq!(text, "rgbixlink");
    }
}
