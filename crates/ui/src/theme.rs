// Centralized theme for wrkflw TUI
//
// The dark palette and symbol set match the design handoff in
// `wrkflw TUI.html`. The light palette keeps the same roles with colors
// that stay readable on a light terminal background. View files import
// from this module instead of hardcoding.

use ratatui::{
    layout::{Margin, Rect},
    style::{Color, Modifier, Style},
    text::{Line, Span},
    widgets::{
        Block, BorderType, Borders, Padding, Scrollbar, ScrollbarOrientation, ScrollbarState,
    },
    Frame,
};
use std::cell::Cell;
use std::ops::Deref;
use std::str::FromStr;
use std::sync::atomic::{AtomicBool, Ordering};

thread_local! {
    /// Per-render accent override. Populated by `set_accent_override`
    /// at the top of [`crate::views::render_ui`] and consulted by
    /// [`current_accent`] inside widget builders. Thread-local so each
    /// ratatui backend thread gets its own value; a `Cell<Option<_>>`
    /// because renders are single-frame and we never need interior
    /// sharing — just a scalar handoff.
    static ACCENT_OVERRIDE: Cell<Option<Color>> = const { Cell::new(None) };
}

/// Install an accent color for the current frame. Pass `None` to
/// fall back to [`COLORS.accent`].
pub fn set_accent_override(color: Option<Color>) {
    ACCENT_OVERRIDE.with(|c| c.set(color));
}

/// Read the active accent color. Falls back to the static palette
/// value when no override is installed.
pub fn current_accent() -> Color {
    ACCENT_OVERRIDE.with(|c| c.get()).unwrap_or(COLORS.accent)
}

// ── Color Palette ──────────────────────────────────────────────────

/// Which palette the TUI draws with.
#[derive(Copy, Clone, Debug, Default, PartialEq, Eq)]
pub enum ThemeMode {
    #[default]
    Dark,
    Light,
}

impl ThemeMode {
    fn colors(self) -> &'static Colors {
        match self {
            ThemeMode::Dark => &DARK,
            ThemeMode::Light => &LIGHT,
        }
    }
}

/// How the user asked for the theme to be chosen.
#[derive(Copy, Clone, Debug, Default, PartialEq, Eq)]
pub enum ThemePreference {
    /// Follow the terminal's background color.
    #[default]
    Auto,
    Dark,
    Light,
}

impl FromStr for ThemePreference {
    type Err = String;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        match s.trim().to_ascii_lowercase().as_str() {
            "auto" => Ok(ThemePreference::Auto),
            "dark" => Ok(ThemePreference::Dark),
            "light" => Ok(ThemePreference::Light),
            other => Err(format!(
                "unknown theme '{other}' (expected auto, dark or light)"
            )),
        }
    }
}

/// Environment variable that selects the theme when `--theme` is absent.
const THEME_ENV: &str = "WRKFLW_THEME";

/// Process-wide rather than thread-local like the accent override: the
/// log worker thread builds styles too, and the mode never changes once
/// the TUI has started.
static LIGHT_MODE: AtomicBool = AtomicBool::new(false);

/// The palette in use.
pub fn mode() -> ThemeMode {
    if LIGHT_MODE.load(Ordering::Relaxed) {
        ThemeMode::Light
    } else {
        ThemeMode::Dark
    }
}

/// Choose the palette for this run. `flag` wins over `WRKFLW_THEME`, and
/// anything else follows the terminal.
///
/// Must run before the TUI enters raw mode: detection reads the
/// terminal's reply from the tty, which would race the event loop.
pub fn install(flag: Option<ThemePreference>) {
    let env = std::env::var(THEME_ENV).ok();
    let preference = flag
        .or_else(|| env.and_then(|value| value.parse().ok()))
        .unwrap_or_default();
    let mode = match preference {
        ThemePreference::Dark => ThemeMode::Dark,
        ThemePreference::Light => ThemeMode::Light,
        ThemePreference::Auto => detect_mode(),
    };
    LIGHT_MODE.store(mode == ThemeMode::Light, Ordering::Relaxed);
}

/// Ask the terminal for its background, then fall back to `COLORFGBG`
/// for terminals that don't answer, then to the dark palette.
fn detect_mode() -> ThemeMode {
    use terminal_colorsaurus::{theme_mode, QueryOptions};

    match theme_mode(QueryOptions::default()) {
        Ok(terminal_colorsaurus::ThemeMode::Dark) => ThemeMode::Dark,
        Ok(terminal_colorsaurus::ThemeMode::Light) => ThemeMode::Light,
        Err(_) => std::env::var("COLORFGBG")
            .ok()
            .and_then(|value| mode_from_colorfgbg(&value))
            .unwrap_or_default(),
    }
}

/// `COLORFGBG` is `fg;bg` or `fg;default;bg`, with ANSI color indexes.
/// Indexes 0-6 and 8 are the dark ones.
fn mode_from_colorfgbg(value: &str) -> Option<ThemeMode> {
    let background: u8 = value.rsplit(';').next()?.parse().ok()?;
    match background {
        0..=6 | 8 => Some(ThemeMode::Dark),
        7 | 9..=15 => Some(ThemeMode::Light),
        _ => None,
    }
}

pub struct Colors {
    // Brand / accent
    pub accent: Color,
    pub highlight: Color,

    // Semantic status
    pub success: Color,
    pub error: Color,
    pub warning: Color,
    pub info: Color,
    pub trigger: Color,

    // Text hierarchy
    pub text: Color,
    pub text_dim: Color,
    pub text_muted: Color,

    // Borders
    pub border: Color,
    pub border_focused: Color,

    // Backgrounds
    pub bg_selected: Color,
    pub bg_panel: Color,
    pub bg_bar: Color,
    pub bg_dark: Color,

    // The menu and key bars, drawn inverted like a DOS menu bar:
    // `chrome_text` on `chrome`, hotkeys in `chrome_key`.
    pub chrome: Color,
    pub chrome_text: Color,
    pub chrome_key: Color,
    /// Drop shadows under raised buttons and floating windows.
    pub shadow: Color,

    // Runtime badges
    pub runtime_docker: Color,
    pub runtime_podman: Color,
    pub runtime_kubernetes: Color,
    pub runtime_emulation: Color,
    pub runtime_secure: Color,

    // Tool output: the two basic ANSI colours that have no semantic
    // status of their own.
    pub ansi_blue: Color,
    pub ansi_magenta: Color,
}

/// The active palette. Dereferences to [`Colors`], so `COLORS.text`
/// always reads from the palette selected by [`install`].
pub struct ActiveColors;

pub static COLORS: ActiveColors = ActiveColors;

impl Deref for ActiveColors {
    type Target = Colors;

    fn deref(&self) -> &Colors {
        mode().colors()
    }
}

// "Blueprint": a cyanotype. Prussian-blue paper, chalk-white lines and a
// yellow drafting pencil for the accent, the way a schematic is marked up.
// Status colors stay clear of the pencil so a failure still reads at a
// glance.
static DARK: Colors = Colors {
    accent: Color::Rgb(0xf5, 0xcf, 0x4a),
    highlight: Color::Rgb(0xff, 0xe5, 0x8a),

    success: Color::Rgb(0x6f, 0xd6, 0xa0),
    error: Color::Rgb(0xff, 0x7b, 0x6e),
    warning: Color::Rgb(0xf7, 0xa6, 0x5a),
    info: Color::Rgb(0x7f, 0xc4, 0xf2),
    trigger: Color::Rgb(0xc3, 0xa6, 0xff),

    text: Color::Rgb(0xea, 0xf0, 0xf4),
    text_dim: Color::Rgb(0xb0, 0xc2, 0xd2),
    text_muted: Color::Rgb(0x93, 0xa9, 0xbd),

    border: Color::Rgb(0x4a, 0x6f, 0x92),
    border_focused: Color::Rgb(0xf5, 0xcf, 0x4a),

    bg_selected: Color::Rgb(0x1b, 0x3b, 0x5c),
    bg_panel: Color::Rgb(0x0e, 0x21, 0x34),
    bg_bar: Color::Rgb(0x12, 0x2a, 0x42),
    bg_dark: Color::Rgb(0x09, 0x17, 0x26),

    chrome: Color::Rgb(0xb4, 0xc3, 0xd1),
    chrome_text: Color::Rgb(0x0a, 0x1a, 0x2b),
    chrome_key: Color::Rgb(0x8a, 0x1c, 0x12),
    shadow: Color::Rgb(0x02, 0x07, 0x0d),

    runtime_docker: Color::Rgb(0x7f, 0xb2, 0xf5),
    runtime_podman: Color::Rgb(0xa3, 0xd9, 0xe8),
    runtime_kubernetes: Color::Rgb(0xad, 0xa6, 0xff),
    runtime_emulation: Color::Rgb(0xf5, 0xcf, 0x4a),
    runtime_secure: Color::Rgb(0x6f, 0xd6, 0xa0),
    ansi_blue: Color::Rgb(0x7f, 0xb2, 0xf5),
    ansi_magenta: Color::Rgb(0xe0, 0x9c, 0xeb),
};

// Blueprint on drafting paper: blueprint-blue ink on cream. `bg_dark`
// keeps its role: it is the text on solid chips, so it stays the color
// furthest from the foregrounds.
static LIGHT: Colors = Colors {
    accent: Color::Rgb(0x1c, 0x4f, 0xa1),
    highlight: Color::Rgb(0x8a, 0x5b, 0x00),

    success: Color::Rgb(0x1e, 0x6b, 0x43),
    error: Color::Rgb(0xb0, 0x2a, 0x1f),
    warning: Color::Rgb(0x8a, 0x5b, 0x00),
    info: Color::Rgb(0x1d, 0x5e, 0x7a),
    trigger: Color::Rgb(0x6b, 0x3f, 0xa0),

    text: Color::Rgb(0x10, 0x2a, 0x43),
    text_dim: Color::Rgb(0x34, 0x50, 0x6b),
    text_muted: Color::Rgb(0x4d, 0x63, 0x77),

    border: Color::Rgb(0x7d, 0x8b, 0x99),
    border_focused: Color::Rgb(0x1c, 0x4f, 0xa1),

    bg_selected: Color::Rgb(0xdc, 0xe4, 0xec),
    bg_panel: Color::Rgb(0xf3, 0xf0, 0xe6),
    bg_bar: Color::Rgb(0xe9, 0xe4, 0xd6),
    bg_dark: Color::Rgb(0xfb, 0xfa, 0xf5),

    chrome: Color::Rgb(0x1c, 0x3a, 0x5e),
    chrome_text: Color::Rgb(0xf3, 0xf0, 0xe6),
    chrome_key: Color::Rgb(0xf5, 0xcf, 0x4a),
    shadow: Color::Rgb(0xa8, 0xa2, 0x8f),

    runtime_docker: Color::Rgb(0x1f, 0x55, 0xa8),
    runtime_podman: Color::Rgb(0x1d, 0x5e, 0x7a),
    runtime_kubernetes: Color::Rgb(0x4e, 0x3f, 0xa8),
    runtime_emulation: Color::Rgb(0x8a, 0x5b, 0x00),
    runtime_secure: Color::Rgb(0x1e, 0x6b, 0x43),
    ansi_blue: Color::Rgb(0x1f, 0x55, 0xa8),
    ansi_magenta: Color::Rgb(0x8a, 0x2f, 0x8f),
};

pub use wrkflw_logging::symbols;

// ── Style Helpers ──────────────────────────────────────────────────

pub fn title_style() -> Style {
    Style::default()
        .fg(COLORS.highlight)
        .add_modifier(Modifier::BOLD)
}

pub fn selected_style() -> Style {
    Style::default()
        .bg(COLORS.bg_selected)
        .add_modifier(Modifier::BOLD)
}

/// Gutter marker drawn beside the selected row of every list.
pub const SELECTED_MARK: &str = "▌ ";

pub fn header_style() -> Style {
    Style::default()
        .fg(COLORS.text_muted)
        .add_modifier(Modifier::BOLD)
}

pub fn search_highlight() -> Style {
    Style::default()
        .bg(COLORS.highlight)
        .fg(COLORS.bg_dark)
        .add_modifier(Modifier::BOLD)
}

pub fn dim_style() -> Style {
    Style::default().fg(COLORS.text_dim)
}

pub fn muted_style() -> Style {
    Style::default().fg(COLORS.text_muted)
}

pub fn key_style() -> Style {
    Style::default().fg(COLORS.highlight)
}

pub fn hint_style() -> Style {
    Style::default().fg(COLORS.text_dim)
}

pub fn panel_style() -> Style {
    Style::default().bg(COLORS.bg_panel)
}

// ── Status Styles ──────────────────────────────────────────────────

use crate::models::WorkflowStatus;
use wrkflw_executor::{JobStatus, StepStatus};

pub fn workflow_status(status: &WorkflowStatus) -> (&'static str, Style) {
    match status {
        WorkflowStatus::NotStarted => {
            (symbols::NOT_STARTED, Style::default().fg(COLORS.text_muted))
        }
        WorkflowStatus::Running => (symbols::RUNNING, Style::default().fg(COLORS.info)),
        WorkflowStatus::Success => (symbols::SUCCESS, Style::default().fg(COLORS.success)),
        WorkflowStatus::Failed => (symbols::FAILURE, Style::default().fg(COLORS.error)),
        WorkflowStatus::Skipped => (symbols::SKIPPED, Style::default().fg(COLORS.warning)),
    }
}

pub fn spinner(frame: usize) -> &'static str {
    symbols::SPINNER[frame % symbols::SPINNER.len()]
}

pub fn workflow_status_animated(
    status: &WorkflowStatus,
    spinner_frame: usize,
) -> (&'static str, Style) {
    match status {
        WorkflowStatus::Running => (spinner(spinner_frame), Style::default().fg(COLORS.info)),
        other => workflow_status(other),
    }
}

pub fn job_state(state: crate::models::JobState, spinner_frame: usize) -> (&'static str, Style) {
    use crate::models::JobState;
    match state {
        JobState::Success => (symbols::SUCCESS, Style::default().fg(COLORS.success)),
        JobState::Failure => (symbols::FAILURE, Style::default().fg(COLORS.error)),
        JobState::FailureAllowed => (symbols::FAILURE, Style::default().fg(COLORS.warning)),
        JobState::Skipped => (symbols::SKIPPED, Style::default().fg(COLORS.text_muted)),
        JobState::Running => (spinner(spinner_frame), Style::default().fg(COLORS.info)),
        JobState::Pending => (symbols::NOT_STARTED, Style::default().fg(COLORS.text_muted)),
    }
}

pub fn job_status(status: &JobStatus) -> (&'static str, Style) {
    match status {
        JobStatus::Success => (symbols::SUCCESS, Style::default().fg(COLORS.success)),
        JobStatus::Failure => (symbols::FAILURE, Style::default().fg(COLORS.error)),
        JobStatus::Skipped => (symbols::SKIPPED, Style::default().fg(COLORS.text_muted)),
    }
}

pub fn step_status(status: &StepStatus) -> (&'static str, Style) {
    match status {
        StepStatus::Success => (symbols::SUCCESS, Style::default().fg(COLORS.success)),
        StepStatus::Failure => (symbols::FAILURE, Style::default().fg(COLORS.error)),
        StepStatus::Skipped => (symbols::SKIPPED, Style::default().fg(COLORS.text_muted)),
    }
}

// ── Block Helpers ──────────────────────────────────────────────────

pub fn block<'a>(title: &'a str) -> Block<'a> {
    panel(title, false)
}

pub fn block_focused<'a>(title: &'a str) -> Block<'a> {
    panel(title, true)
}

/// The one panel style every pane shares: a full box, as in a DOS text
/// UI. The focused pane gets a double line in the accent and the rest a
/// single line, so focus reads from across the room. The title sits in
/// the top edge between tee joints, `╡ Title ╞`. Text gets a cell of
/// padding on each side; the scrollbar draws over the right edge.
pub fn panel<'a>(title: &'a str, focused: bool) -> Block<'a> {
    let (edge, border_type, title_style) = if focused {
        (
            current_accent(),
            BorderType::Double,
            Style::default()
                .fg(current_accent())
                .add_modifier(Modifier::BOLD),
        )
    } else {
        (
            COLORS.border,
            BorderType::Plain,
            Style::default()
                .fg(COLORS.text)
                .add_modifier(Modifier::BOLD),
        )
    };
    let block = Block::default()
        .borders(Borders::ALL)
        .border_type(border_type)
        .border_style(Style::default().fg(edge))
        .padding(Padding::horizontal(1));
    if title.is_empty() {
        return block;
    }
    block.title(boxed_title(title, border_type, edge, title_style))
}

/// `╡ title ╞` for a double edge, `┤ title ├` for a single one.
fn boxed_title<'a>(title: &str, border_type: BorderType, edge: Color, style: Style) -> Line<'a> {
    let (open, close) = match border_type {
        BorderType::Double => ("╡", "╞"),
        _ => ("┤", "├"),
    };
    Line::from(vec![
        Span::styled(open, Style::default().fg(edge)),
        Span::styled(format!(" {title} "), style),
        Span::styled(close, Style::default().fg(edge)),
    ])
}

/// A double-lined window for floating overlays (Help, Tweaks). [`modal`]
/// gives it a drop shadow.
pub fn modal_frame<'a>(title: &'a str) -> Block<'a> {
    let accent = current_accent();
    Block::default()
        .borders(Borders::ALL)
        .border_type(BorderType::Double)
        .border_style(Style::default().fg(accent))
        .padding(Padding::horizontal(1))
        .title(boxed_title(
            title,
            BorderType::Double,
            accent,
            Style::default().fg(accent).add_modifier(Modifier::BOLD),
        ))
        .title_alignment(ratatui::layout::Alignment::Center)
}

/// Adds a dim right-aligned label to a panel's top border, for counts
/// and positions like `3/40`.
pub fn with_badge<'a>(block: Block<'a>, badge: impl Into<String>) -> Block<'a> {
    let badge = badge.into();
    if badge.is_empty() {
        return block;
    }
    block.title_top(Line::from(Span::styled(format!(" {badge} "), muted_style())).right_aligned())
}

/// Heading for a group of rows inside a panel: `── LAST RUN`, a short
/// rule leading into the title like a section mark on a form.
pub fn section<'a>(title: impl Into<String>) -> Line<'a> {
    Line::from(vec![
        Span::styled("── ", Style::default().fg(COLORS.border)),
        Span::styled(
            title.into().to_uppercase(),
            Style::default()
                .fg(COLORS.highlight)
                .add_modifier(Modifier::BOLD),
        ),
    ])
}

/// Builds `key desc  key desc …`, dropping whole hints from the end
/// rather than cutting one in half when the row is too narrow.
pub fn hint_spans<'a>(hints: &[(&str, &str)], max_width: usize) -> Vec<Span<'a>> {
    let mut spans: Vec<Span> = Vec::new();
    let mut used = 0;
    for (i, (key, desc)) in hints.iter().enumerate() {
        let sep = if i == 0 { 0 } else { 3 };
        let w = sep + crate::text::width(key) + 1 + crate::text::width(desc);
        if used + w > max_width {
            break;
        }
        if sep > 0 {
            spans.push(Span::styled("   ", muted_style()));
        }
        spans.push(Span::styled(
            key.to_string(),
            Style::default()
                .fg(current_accent())
                .add_modifier(Modifier::BOLD),
        ));
        spans.push(Span::styled(format!(" {desc}"), hint_style()));
        used += w;
    }
    spans
}

/// Two rows of raised push buttons, drawn the way text-mode OSes drew
/// them: a solid face, then a half-block drop shadow, `▄` at the face's
/// right end and `▀▀▀` under it, one cell to the right:
///
/// ```text
///  Enter run ▄
///   ▀▀▀▀▀▀▀▀▀▀
/// ```
///
/// Each button is `(key, label, lit)`; a lit button takes the accent.
/// Whole buttons are dropped from the end rather than cut in half when
/// the row is too narrow. Returns the face row and the shadow row.
pub fn raised_buttons<'a>(
    buttons: &[(&str, &str, bool)],
    max_width: usize,
) -> (Vec<Span<'a>>, Vec<Span<'a>>) {
    let shadow = Style::default().fg(COLORS.shadow);
    let (mut faces, mut shadows) = (Vec::new(), Vec::new());
    let mut used = 0;
    for (key, label, lit) in buttons {
        // A button with no label is just its key: ` 2 `.
        let label = if label.is_empty() {
            String::new()
        } else {
            format!(" {label}")
        };
        let face_w = crate::text::width(key) + crate::text::width(&label) + 2;
        // The face, its shadow column and a one-cell gap.
        if used + face_w + 2 > max_width {
            break;
        }
        let (face, key_style) = if *lit {
            let lit = Style::default()
                .bg(current_accent())
                .fg(COLORS.bg_dark)
                .add_modifier(Modifier::BOLD);
            (lit, lit)
        } else {
            let face = Style::default().bg(COLORS.chrome).fg(COLORS.chrome_text);
            (
                face,
                face.fg(COLORS.chrome_key).add_modifier(Modifier::BOLD),
            )
        };
        faces.push(Span::styled(format!(" {key}"), key_style));
        faces.push(Span::styled(format!("{label} "), face));
        faces.push(Span::styled("▄", shadow));
        faces.push(Span::raw(" "));
        shadows.push(Span::raw(" "));
        shadows.push(Span::styled("▀".repeat(face_w), shadow));
        shadows.push(Span::raw(" "));
        used += face_w + 2;
    }
    (faces, shadows)
}

/// Draws a DOS-style scrollbar (`▲ ░░█░ ▼`) over the right border of `area` when
/// `total` rows don't fit in `viewport`.
pub fn scrollbar(f: &mut Frame<'_>, area: Rect, total: usize, position: usize, viewport: usize) {
    if total <= viewport || area.height < 3 {
        return;
    }
    let mut state = ScrollbarState::new(total.saturating_sub(viewport))
        .position(position.min(total.saturating_sub(viewport)))
        .viewport_content_length(viewport);
    let bar = Scrollbar::new(ScrollbarOrientation::VerticalRight)
        .begin_symbol(Some("▲"))
        .end_symbol(Some("▼"))
        .track_symbol(Some("░"))
        .thumb_symbol("█")
        .track_style(Style::default().fg(COLORS.border))
        .thumb_style(Style::default().fg(current_accent()));
    f.render_stateful_widget(
        bar,
        area.inner(Margin {
            vertical: 1,
            horizontal: 0,
        }),
        &mut state,
    );
}

/// Opens a modal: mutes everything already drawn, then clears and returns
/// a `width`×`height` rect centred on the screen, shrunk to fit. Muting the
/// colours rather than relying on the terminal's dim attribute keeps the
/// backdrop consistent across terminals and in both palettes.
pub fn modal(f: &mut Frame<'_>, width: u16, height: u16) -> Rect {
    let screen = f.area();
    f.buffer_mut().set_style(
        screen,
        Style::default()
            .fg(COLORS.border)
            .remove_modifier(Modifier::BOLD),
    );
    let area = screen.centered(
        ratatui::layout::Constraint::Length(width.min(screen.width)),
        ratatui::layout::Constraint::Length(height.min(screen.height)),
    );
    // A Turbo Vision drop shadow: two cells to the right, one below.
    let shadow = Style::default().bg(COLORS.shadow).fg(COLORS.text_muted);
    for rect in [
        Rect::new(area.right(), area.y + 1, 2, area.height),
        Rect::new(area.x + 2, area.bottom(), area.width, 1),
    ] {
        let rect = rect.intersection(screen);
        if !rect.is_empty() {
            f.buffer_mut().set_style(rect, shadow);
        }
    }
    f.render_widget(ratatui::widgets::Clear, area);
    f.render_widget(
        ratatui::widgets::Paragraph::new("").style(Style::default().bg(COLORS.bg_bar)),
        area,
    );
    area
}

/// Centred, muted message for a pane with nothing to show yet.
pub fn empty_state(f: &mut Frame<'_>, area: Rect, title: &str, hint: Vec<Span<'_>>) {
    let top = area.height.saturating_sub(2) / 2;
    let mut lines: Vec<Line> = (0..top).map(|_| Line::from("")).collect();
    lines.push(Line::from(Span::styled(
        title.to_string(),
        Style::default()
            .fg(COLORS.text_dim)
            .add_modifier(Modifier::BOLD),
    )));
    if !hint.is_empty() {
        lines.push(Line::from(hint));
    }
    f.render_widget(
        ratatui::widgets::Paragraph::new(lines).alignment(ratatui::layout::Alignment::Center),
        area,
    );
}

#[derive(Copy, Clone, Debug)]
pub enum BadgeKind {
    Info,
    Success,
    Error,
    Warning,
    Trigger,
    Dim,
    Accent,
    Highlight,
    Docker,
    Podman,
    Kubernetes,
    Emulation,
    Secure,
}

impl BadgeKind {
    pub fn fg(self) -> Color {
        match self {
            BadgeKind::Info => COLORS.info,
            BadgeKind::Success => COLORS.success,
            BadgeKind::Error => COLORS.error,
            BadgeKind::Warning => COLORS.warning,
            BadgeKind::Trigger => COLORS.trigger,
            BadgeKind::Dim => COLORS.text_dim,
            // Resolve dynamically so Tweaks accent changes recolor
            // `BadgeKind::Accent` call sites in the same frame.
            BadgeKind::Accent => current_accent(),
            BadgeKind::Highlight => COLORS.highlight,
            BadgeKind::Docker => COLORS.runtime_docker,
            BadgeKind::Podman => COLORS.runtime_podman,
            BadgeKind::Kubernetes => COLORS.runtime_kubernetes,
            BadgeKind::Emulation => COLORS.runtime_emulation,
            BadgeKind::Secure => COLORS.runtime_secure,
        }
    }
}

/// Outline badge: small chip with colored text on the panel background.
pub fn badge_outline<'a>(text: impl Into<String>, kind: BadgeKind) -> Span<'a> {
    Span::styled(
        format!(" {} ", text.into()),
        Style::default().fg(kind.fg()).add_modifier(Modifier::BOLD),
    )
}

/// Solid badge: `bg_dark` text on a colored background.
pub fn badge_solid<'a>(text: impl Into<String>, kind: BadgeKind) -> Span<'a> {
    Span::styled(
        format!(" {} ", text.into()),
        Style::default()
            .bg(kind.fg())
            .fg(COLORS.bg_dark)
            .add_modifier(Modifier::BOLD),
    )
}

/// Legacy badge helper kept for callers that already pass explicit colors.
pub fn badge<'a>(text: &'a str, bg: Color, fg: Color) -> Span<'a> {
    Span::styled(format!(" {} ", text), Style::default().bg(bg).fg(fg))
}

/// Render a key hint as a keycap chip: `[K]` in highlight on bg_bar.
pub fn key_chip<'a>(key: impl Into<String>) -> Span<'a> {
    Span::styled(
        format!(" {} ", key.into()),
        Style::default()
            .bg(COLORS.bg_selected)
            .fg(current_accent())
            .add_modifier(Modifier::BOLD),
    )
}

/// Pulsing style for the LIVE indicator dot.
pub fn pulse_style(frame: usize) -> Style {
    let dim = (frame / 4).is_multiple_of(2);
    let mut s = Style::default().fg(COLORS.error);
    if dim {
        s = s.add_modifier(Modifier::DIM);
    } else {
        s = s.add_modifier(Modifier::BOLD);
    }
    s
}

/// Log level badge styles
pub fn log_badge(level: &str) -> Style {
    match level {
        "ERROR" => Style::default().bg(COLORS.error).fg(COLORS.bg_dark),
        "WARN" => Style::default().bg(COLORS.warning).fg(COLORS.bg_dark),
        "SUCCESS" => Style::default().fg(COLORS.success),
        "INFO" => Style::default().fg(COLORS.info),
        "TRIG" => Style::default().fg(COLORS.trigger),
        _ => Style::default().fg(COLORS.text_dim),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::app::Accent;

    fn luminance(color: Color) -> f64 {
        let Color::Rgb(r, g, b) = color else {
            panic!("palette colors are RGB, got {color:?}");
        };
        let channel = |v: u8| {
            let v = f64::from(v) / 255.0;
            if v <= 0.03928 {
                v / 12.92
            } else {
                ((v + 0.055) / 1.055).powf(2.4)
            }
        };
        0.2126 * channel(r) + 0.7152 * channel(g) + 0.0722 * channel(b)
    }

    /// WCAG contrast ratio.
    fn contrast(a: Color, b: Color) -> f64 {
        let (a, b) = (luminance(a), luminance(b));
        (a.max(b) + 0.05) / (a.min(b) + 0.05)
    }

    fn light_foregrounds() -> Vec<(String, Color)> {
        let c = &LIGHT;
        let mut colors: Vec<(String, Color)> = [
            ("accent", c.accent),
            ("highlight", c.highlight),
            ("success", c.success),
            ("error", c.error),
            ("warning", c.warning),
            ("info", c.info),
            ("trigger", c.trigger),
            ("text", c.text),
            ("text_dim", c.text_dim),
            ("text_muted", c.text_muted),
            ("runtime_docker", c.runtime_docker),
            ("runtime_podman", c.runtime_podman),
            ("runtime_kubernetes", c.runtime_kubernetes),
            ("runtime_emulation", c.runtime_emulation),
            ("runtime_secure", c.runtime_secure),
        ]
        .into_iter()
        .map(|(name, color)| (name.to_string(), color))
        .collect();
        for accent in [
            Accent::Cyan,
            Accent::Blueprint,
            Accent::Green,
            Accent::Violet,
            Accent::Coral,
        ] {
            let (r, g, b) = accent.rgb_for(ThemeMode::Light);
            colors.push((format!("accent {}", accent.as_str()), Color::Rgb(r, g, b)));
        }
        colors
    }

    #[test]
    fn light_foregrounds_are_readable_on_every_light_background() {
        let backgrounds = [
            ("terminal white", Color::Rgb(0xff, 0xff, 0xff)),
            ("bg_selected", LIGHT.bg_selected),
            ("bg_panel", LIGHT.bg_panel),
            ("bg_bar", LIGHT.bg_bar),
            ("bg_dark", LIGHT.bg_dark),
        ];
        for (fg_name, fg) in light_foregrounds() {
            for (bg_name, bg) in backgrounds {
                let ratio = contrast(fg, bg);
                assert!(ratio >= 4.5, "{fg_name} on {bg_name} is only {ratio:.2}:1");
            }
        }
    }

    #[test]
    fn dark_foregrounds_are_readable_on_every_dark_background() {
        let c = &DARK;
        let foregrounds = [
            ("accent", c.accent),
            ("highlight", c.highlight),
            ("success", c.success),
            ("error", c.error),
            ("warning", c.warning),
            ("info", c.info),
            ("trigger", c.trigger),
            ("text", c.text),
            ("text_dim", c.text_dim),
            ("text_muted", c.text_muted),
            ("runtime_docker", c.runtime_docker),
            ("runtime_podman", c.runtime_podman),
            ("runtime_kubernetes", c.runtime_kubernetes),
        ];
        let backgrounds = [
            ("bg_selected", c.bg_selected),
            ("bg_panel", c.bg_panel),
            ("bg_bar", c.bg_bar),
            ("bg_dark", c.bg_dark),
        ];
        for (fg_name, fg) in foregrounds {
            for (bg_name, bg) in backgrounds {
                let ratio = contrast(fg, bg);
                assert!(ratio >= 4.5, "{fg_name} on {bg_name} is only {ratio:.2}:1");
            }
        }
    }

    #[test]
    fn menu_and_key_bars_are_readable() {
        for c in [&DARK, &LIGHT] {
            for (name, fg, bg) in [
                ("bar text", c.chrome_text, c.chrome),
                ("bar hotkey", c.chrome_key, c.chrome),
                ("key cell", c.text, c.bg_dark),
                ("open tab", c.accent, c.bg_panel),
            ] {
                let ratio = contrast(fg, bg);
                assert!(ratio >= 4.5, "{name} is only {ratio:.2}:1");
            }
        }
    }

    #[test]
    fn light_solid_chips_are_readable() {
        // Solid chips draw `bg_dark` text on a foreground color.
        for (name, color) in light_foregrounds() {
            if name == "text" || name == "text_dim" || name == "text_muted" {
                continue;
            }
            let ratio = contrast(LIGHT.bg_dark, color);
            assert!(ratio >= 4.5, "chip on {name} is only {ratio:.2}:1");
        }
    }

    #[test]
    fn theme_preference_parses_case_insensitively() {
        assert_eq!(" Light ".parse(), Ok(ThemePreference::Light));
        assert_eq!("DARK".parse(), Ok(ThemePreference::Dark));
        assert_eq!("auto".parse(), Ok(ThemePreference::Auto));
        assert!("solarized".parse::<ThemePreference>().is_err());
    }

    #[test]
    fn colorfgbg_uses_the_last_field_as_background() {
        assert_eq!(mode_from_colorfgbg("15;0"), Some(ThemeMode::Dark));
        assert_eq!(mode_from_colorfgbg("0;15"), Some(ThemeMode::Light));
        assert_eq!(mode_from_colorfgbg("0;7"), Some(ThemeMode::Light));
        assert_eq!(mode_from_colorfgbg("15;default;8"), Some(ThemeMode::Dark));
        assert_eq!(mode_from_colorfgbg("15;default"), None);
        assert_eq!(mode_from_colorfgbg(""), None);
        assert_eq!(mode_from_colorfgbg("0;255"), None);
    }
}
