// Mouse text selection: drag to highlight, release to copy to the clipboard.
//
// Selection works on the rendered frame rather than on view models, so
// every tab gets it without knowing about it, and what lands on the
// clipboard is exactly what was highlighted on screen.
use crate::theme::COLORS;
use base64::{engine::general_purpose::STANDARD, Engine as _};
use crossterm::event::{KeyCode, MouseButton, MouseEvent, MouseEventKind};
use ratatui::{buffer::Buffer, layout::Rect, style::Style};
use std::io::{self, Write};
use std::process::{Command, Stdio};
use std::time::{Duration, Instant};
use unicode_width::UnicodeWidthStr;

/// How long a clipboard tool gets before we stop waiting on it. This
/// runs on the UI thread, so a wedged tool must not freeze the TUI.
const CLIPBOARD_TOOL_TIMEOUT: Duration = Duration::from_millis(500);

const VERTICAL_BORDERS: &str = "│┃║";
const BORDER_JOINS: &str = "╭╮╰╯┌┐└┘├┤┬┴┼╔╗╚╝╠╣╦╩╬┏┓┗┛┣┫┳┻╋";

/// An in-progress or finished drag, in screen cells (column, row).
#[derive(Copy, Clone, Debug, PartialEq, Eq)]
pub struct Selection {
    anchor: (u16, u16),
    head: (u16, u16),
    dragged: bool,
    copy_pending: bool,
    /// The pane the drag started in. Needs a rendered frame to work
    /// out, so it is filled in by the first `apply` after the press.
    bounds: Option<Rect>,
}

impl Selection {
    fn new(anchor: (u16, u16)) -> Self {
        Self {
            anchor,
            head: anchor,
            dragged: false,
            copy_pending: false,
            bounds: None,
        }
    }

    /// One `(row, first_col, last_col)` per selected row, inclusive,
    /// in reading order: the first and last rows are partial and the
    /// rows between run the full width of the pane.
    fn row_spans(&self, bounds: Rect) -> Vec<(u16, u16, u16)> {
        let clamp = |(col, row): (u16, u16)| {
            (
                row.clamp(bounds.top(), bounds.bottom() - 1),
                col.clamp(bounds.left(), bounds.right() - 1),
            )
        };
        let (a, b) = (clamp(self.anchor), clamp(self.head));
        let (start, end) = (a.min(b), a.max(b));
        (start.0..=end.0)
            .map(|row| {
                let first = if row == start.0 {
                    start.1
                } else {
                    bounds.left()
                };
                let last = if row == end.0 {
                    end.1
                } else {
                    bounds.right() - 1
                };
                (row, first, last)
            })
            .collect()
    }

    fn text(&self, buf: &Buffer, bounds: Rect) -> String {
        let lines: Vec<String> = self
            .row_spans(bounds)
            .into_iter()
            .map(|(row, first, last)| {
                let mut line = String::new();
                let mut col = first;
                while col <= last {
                    let symbol = buf[(col, row)].symbol();
                    line.push_str(symbol);
                    // A wide glyph owns the blank cell after it.
                    col += symbol.width().max(1) as u16;
                }
                line.trim_end().to_string()
            })
            .collect();
        lines.join("\n").trim_end().to_string()
    }
}

/// Applies a mouse event to the selection. Wheel events come back as
/// the arrow key they stand for: capturing the mouse stops the terminal
/// from translating the wheel into arrows itself, and this keeps it
/// scrolling whatever the arrows scroll on the current tab.
pub fn handle_mouse(selection: &mut Option<Selection>, mouse: MouseEvent) -> Option<KeyCode> {
    let position = (mouse.column, mouse.row);
    match mouse.kind {
        MouseEventKind::Down(MouseButton::Left) => *selection = Some(Selection::new(position)),
        MouseEventKind::Drag(MouseButton::Left) => {
            if let Some(selection) = selection {
                selection.head = position;
                selection.dragged = true;
            }
        }
        MouseEventKind::Up(MouseButton::Left) => match selection {
            Some(current) if current.dragged => current.copy_pending = true,
            // A plain click selects nothing; it only clears what was there.
            _ => *selection = None,
        },
        MouseEventKind::ScrollUp => return Some(KeyCode::Up),
        MouseEventKind::ScrollDown => return Some(KeyCode::Down),
        _ => {}
    }
    None
}

/// Paints the selection onto a rendered frame. Returns the selected
/// text once, on the frame after the mouse is released.
pub fn apply(selection: &mut Option<Selection>, buf: &mut Buffer) -> Option<String> {
    let selection = selection.as_mut()?;
    let bounds = *selection
        .bounds
        .get_or_insert_with(|| enclosing_pane(buf, selection.anchor));
    if bounds.is_empty() {
        return None;
    }
    let copied = if selection.copy_pending {
        selection.copy_pending = false;
        Some(selection.text(buf, bounds)).filter(|text| !text.is_empty())
    } else {
        None
    };
    if selection.dragged {
        let style = Style::default().bg(COLORS.accent).fg(COLORS.bg_dark);
        for (row, first, last) in selection.row_spans(bounds) {
            buf.set_style(Rect::new(first, row, last - first + 1, 1), style);
        }
    }
    copied
}

fn symbol_in(buf: &Buffer, col: u16, row: u16, set: &str) -> bool {
    buf[(col, row)].symbol().chars().all(|c| set.contains(c))
        && !buf[(col, row)].symbol().is_empty()
}

/// True for a cell of a pane's vertical border. A lone `│` in a log
/// line has no border above or below it, so it doesn't count.
fn is_wall(buf: &Buffer, col: u16, row: u16) -> bool {
    let joins = |row: u16| {
        symbol_in(buf, col, row, VERTICAL_BORDERS) || symbol_in(buf, col, row, BORDER_JOINS)
    };
    symbol_in(buf, col, row, VERTICAL_BORDERS)
        && ((row > buf.area.top() && joins(row - 1))
            || (row + 1 < buf.area.bottom() && joins(row + 1)))
}

/// The inside of the bordered pane around `(col, row)`, or the whole
/// screen if there is none. Clamping to it is what keeps a multi-line
/// drag from picking up borders and the panes beside it.
fn enclosing_pane(buf: &Buffer, (col, row): (u16, u16)) -> Rect {
    let area = buf.area;
    if !area.intersects(Rect::new(col, row, 1, 1)) || is_wall(buf, col, row) {
        return area;
    }
    let left = (area.left()..col).rev().find(|&c| is_wall(buf, c, row));
    let right = (col + 1..area.right()).find(|&c| is_wall(buf, c, row));

    // Follow a side border up and down to find where the pane ends.
    let (mut top, mut bottom) = (area.top(), area.bottom());
    if let Some(wall) = left.or(right) {
        let on_wall = |r: u16| symbol_in(buf, wall, r, VERTICAL_BORDERS);
        top = (area.top()..row)
            .rev()
            .find(|&r| !on_wall(r))
            .map_or(top, |r| r + 1);
        bottom = (row + 1..area.bottom())
            .find(|&r| !on_wall(r))
            .unwrap_or(bottom);
    }
    let x = left.map_or(area.left(), |c| c + 1);
    let width = right.unwrap_or(area.right()) - x;
    Rect::new(x, top, width, bottom - top)
}

/// How the text reached the clipboard, which decides what we can
/// honestly tell the user.
#[derive(Copy, Clone, Debug, PartialEq, Eq)]
pub enum CopyMethod {
    /// A system clipboard tool accepted the text.
    Native,
    /// Only the terminal was asked, and terminals never acknowledge
    /// OSC 52, so this may or may not have worked.
    Terminal,
}

/// Puts `text` on the clipboard both ways. OSC 52 is what reaches the
/// user's own clipboard over SSH; the native tools cover terminals that
/// ignore it, such as Terminal.app, and tmux, which blocks it by default.
pub fn copy_to_clipboard(text: &str) -> io::Result<CopyMethod> {
    let mut stdout = io::stdout();
    stdout.write_all(osc52(text, std::env::var_os("TMUX").is_some()).as_bytes())?;
    stdout.flush()?;

    let copied = native_tools()
        .iter()
        .any(|(program, args)| run_tool(program, args, text).is_ok());
    Ok(if copied {
        CopyMethod::Native
    } else {
        CopyMethod::Terminal
    })
}

/// The OSC 52 escape sequence for `text`. Inside tmux it goes out twice:
/// plain for `set-clipboard on`, wrapped for `allow-passthrough on`.
fn osc52(text: &str, in_tmux: bool) -> String {
    let sequence = format!("\x1b]52;c;{}\x07", STANDARD.encode(text));
    if in_tmux {
        let escaped = sequence.replace('\x1b', "\x1b\x1b");
        format!("{sequence}\x1bPtmux;{escaped}\x1b\\")
    } else {
        sequence
    }
}

fn native_tools() -> Vec<(&'static str, &'static [&'static str])> {
    // Over SSH these would fill the remote machine's clipboard, not
    // the user's, and report success for it.
    if std::env::var_os("SSH_TTY").is_some() {
        return Vec::new();
    }
    if cfg!(target_os = "macos") {
        return vec![("pbcopy", &[])];
    }
    let mut tools: Vec<(&'static str, &'static [&'static str])> = Vec::new();
    if std::env::var_os("WAYLAND_DISPLAY").is_some() {
        tools.push(("wl-copy", &[]));
    }
    if std::env::var_os("DISPLAY").is_some() {
        tools.push(("xclip", &["-selection", "clipboard"]));
        tools.push(("xsel", &["--clipboard", "--input"]));
    }
    tools
}

fn run_tool(program: &str, args: &[&str], text: &str) -> io::Result<()> {
    let mut child = Command::new(program)
        .args(args)
        .stdin(Stdio::piped())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn()?;
    if let Some(mut stdin) = child.stdin.take() {
        stdin.write_all(text.as_bytes())?;
    }
    let deadline = Instant::now() + CLIPBOARD_TOOL_TIMEOUT;
    loop {
        if let Some(status) = child.try_wait()? {
            return if status.success() {
                Ok(())
            } else {
                Err(io::Error::other(format!("{program} failed: {status}")))
            };
        }
        if Instant::now() >= deadline {
            let _ = child.kill();
            let _ = child.wait();
            return Err(io::ErrorKind::TimedOut.into());
        }
        std::thread::sleep(Duration::from_millis(5));
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crossterm::event::KeyModifiers;
    use ratatui::layout::Position;

    fn mouse(kind: MouseEventKind, column: u16, row: u16) -> MouseEvent {
        MouseEvent {
            kind,
            column,
            row,
            modifiers: KeyModifiers::NONE,
        }
    }

    fn drag(from: (u16, u16), to: (u16, u16)) -> Option<Selection> {
        let mut selection = None;
        let left = MouseButton::Left;
        handle_mouse(
            &mut selection,
            mouse(MouseEventKind::Down(left), from.0, from.1),
        );
        handle_mouse(
            &mut selection,
            mouse(MouseEventKind::Drag(left), to.0, to.1),
        );
        handle_mouse(&mut selection, mouse(MouseEventKind::Up(left), to.0, to.1));
        selection
    }

    fn screen(lines: &[&str]) -> Buffer {
        Buffer::with_lines(lines.iter().map(|line| line.to_string()))
    }

    fn pane_screen() -> Buffer {
        screen(&[
            "title bar             ",
            "╭ Left ──╮╭ Right ───╮",
            "│ one    ││ alpha    │",
            "│ two    ││ beta     │",
            "╰────────╯╰──────────╯",
            "status bar            ",
        ])
    }

    #[test]
    fn multi_line_drag_stays_inside_the_pane_it_started_in() {
        let mut buf = pane_screen();
        let mut selection = drag((12, 2), (16, 3));
        assert_eq!(
            apply(&mut selection, &mut buf).as_deref(),
            Some("alpha\n beta")
        );
    }

    #[test]
    fn drag_past_the_pane_clamps_to_its_edges() {
        let mut buf = pane_screen();
        let mut selection = drag((2, 2), (21, 5));
        assert_eq!(
            apply(&mut selection, &mut buf).as_deref(),
            Some("one\n two")
        );
    }

    #[test]
    fn backwards_drag_selects_the_same_text() {
        let mut buf = pane_screen();
        let mut selection = drag((5, 3), (2, 2));
        assert_eq!(
            apply(&mut selection, &mut buf).as_deref(),
            Some("one\n two")
        );
    }

    #[test]
    fn a_lone_box_character_in_text_is_not_a_pane_border() {
        let mut buf = screen(&["left │ right", "second line "]);
        let mut selection = drag((0, 0), (11, 0));
        assert_eq!(
            apply(&mut selection, &mut buf).as_deref(),
            Some("left │ right")
        );
    }

    #[test]
    fn selection_outside_any_pane_uses_the_whole_screen() {
        let mut buf = pane_screen();
        let mut selection = drag((0, 0), (4, 0));
        assert_eq!(apply(&mut selection, &mut buf).as_deref(), Some("title"));
    }

    #[test]
    fn wide_characters_are_copied_once() {
        let mut buf = screen(&["日本語 ok"]);
        let mut selection = drag((0, 0), (8, 0));
        assert_eq!(
            apply(&mut selection, &mut buf).as_deref(),
            Some("日本語 ok")
        );
    }

    #[test]
    fn text_is_handed_over_once_and_stays_highlighted() {
        let mut buf = pane_screen();
        let mut selection = drag((2, 2), (4, 2));
        assert!(apply(&mut selection, &mut buf).is_some());
        assert_eq!(buf[Position::new(3, 2)].bg, COLORS.accent);
        assert_eq!(apply(&mut selection, &mut buf), None);
        assert!(selection.is_some());
    }

    #[test]
    fn highlight_is_painted_during_the_drag_without_copying() {
        let mut buf = pane_screen();
        let mut selection = None;
        let left = MouseButton::Left;
        handle_mouse(&mut selection, mouse(MouseEventKind::Down(left), 2, 2));
        handle_mouse(&mut selection, mouse(MouseEventKind::Drag(left), 4, 2));
        assert_eq!(apply(&mut selection, &mut buf), None);
        assert_eq!(buf[Position::new(3, 2)].bg, COLORS.accent);
    }

    #[test]
    fn click_without_drag_clears_and_copies_nothing() {
        let mut selection = drag((2, 2), (4, 2));
        let left = MouseButton::Left;
        handle_mouse(&mut selection, mouse(MouseEventKind::Down(left), 9, 9));
        handle_mouse(&mut selection, mouse(MouseEventKind::Up(left), 9, 9));
        assert_eq!(selection, None);
    }

    #[test]
    fn selecting_only_blank_cells_copies_nothing() {
        let mut buf = screen(&["          "]);
        let mut selection = drag((2, 0), (8, 0));
        assert_eq!(apply(&mut selection, &mut buf), None);
    }

    #[test]
    fn press_outside_the_frame_does_not_panic() {
        let mut buf = pane_screen();
        let mut selection = drag((500, 500), (600, 600));
        assert_eq!(apply(&mut selection, &mut buf), None);
    }

    #[test]
    fn wheel_maps_to_arrow_keys() {
        let mut selection = None;
        assert_eq!(
            handle_mouse(&mut selection, mouse(MouseEventKind::ScrollUp, 0, 0)),
            Some(KeyCode::Up)
        );
        assert_eq!(
            handle_mouse(&mut selection, mouse(MouseEventKind::ScrollDown, 0, 0)),
            Some(KeyCode::Down)
        );
    }

    #[test]
    fn osc52_encodes_text_and_wraps_for_tmux() {
        assert_eq!(osc52("hi", false), "\x1b]52;c;aGk=\x07");
        assert_eq!(
            osc52("hi", true),
            "\x1b]52;c;aGk=\x07\x1bPtmux;\x1b\x1b]52;c;aGk=\x07\x1b\\"
        );
    }
}
