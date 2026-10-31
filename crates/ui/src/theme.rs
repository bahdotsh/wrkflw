// Centralized theme for wrkflw TUI
//
// The dark palette and symbol set match the design handoff in
// `wrkflw TUI.html`. The light palette keeps the same roles with colors
// that stay readable on a light terminal background. View files import
// from this module instead of hardcoding.

use ratatui::{
    style::{Color, Modifier, Style},
    text::Span,
    widgets::{Block, BorderType, Borders},
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

    // Runtime badges
    pub runtime_docker: Color,
    pub runtime_podman: Color,
    pub runtime_kubernetes: Color,
    pub runtime_emulation: Color,
    pub runtime_secure: Color,
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

static DARK: Colors = Colors {
    accent: Color::Rgb(0x5f, 0xd3, 0xf3),
    highlight: Color::Rgb(0xf5, 0xd7, 0x6e),

    success: Color::Rgb(0x8f, 0xce, 0x8f),
    error: Color::Rgb(0xff, 0x7a, 0x7a),
    warning: Color::Rgb(0xf5, 0xd7, 0x6e),
    info: Color::Rgb(0x5f, 0xd3, 0xf3),
    trigger: Color::Rgb(0xd6, 0x8c, 0xff),

    text: Color::Rgb(0xd7, 0xdd, 0xe4),
    text_dim: Color::Rgb(0x88, 0x90, 0xa0),
    text_muted: Color::Rgb(0x56, 0x5e, 0x6b),

    border: Color::Rgb(0x2a, 0x2f, 0x38),
    border_focused: Color::Rgb(0x5f, 0xd3, 0xf3),

    bg_selected: Color::Rgb(0x1a, 0x20, 0x28),
    bg_panel: Color::Rgb(0x0f, 0x12, 0x16),
    bg_bar: Color::Rgb(0x14, 0x17, 0x1c),
    bg_dark: Color::Rgb(0x07, 0x09, 0x0b),

    runtime_docker: Color::Rgb(0x5f, 0xa3, 0xff),
    runtime_podman: Color::Rgb(0x5f, 0xd3, 0xf3),
    runtime_kubernetes: Color::Rgb(0x9f, 0x8f, 0xff),
    runtime_emulation: Color::Rgb(0xff, 0x99, 0x66),
    runtime_secure: Color::Rgb(0x8f, 0xce, 0x8f),
};

// `bg_dark` keeps its role here: it is the bar background and the text on
// solid chips, so it stays the color furthest from the foregrounds.
static LIGHT: Colors = Colors {
    accent: Color::Rgb(0x00, 0x6d, 0x87),
    highlight: Color::Rgb(0x85, 0x55, 0x00),

    success: Color::Rgb(0x1f, 0x75, 0x1f),
    error: Color::Rgb(0xbb, 0x22, 0x2a),
    warning: Color::Rgb(0x85, 0x55, 0x00),
    info: Color::Rgb(0x00, 0x6d, 0x87),
    trigger: Color::Rgb(0x7b, 0x2f, 0xbf),

    text: Color::Rgb(0x1f, 0x23, 0x28),
    text_dim: Color::Rgb(0x4a, 0x52, 0x5e),
    text_muted: Color::Rgb(0x5c, 0x64, 0x70),

    border: Color::Rgb(0xc3, 0xc9, 0xd1),
    border_focused: Color::Rgb(0x00, 0x6d, 0x87),

    bg_selected: Color::Rgb(0xe4, 0xe8, 0xed),
    bg_panel: Color::Rgb(0xf4, 0xf5, 0xf7),
    bg_bar: Color::Rgb(0xec, 0xee, 0xf1),
    bg_dark: Color::Rgb(0xf7, 0xf8, 0xfa),

    runtime_docker: Color::Rgb(0x1f, 0x5f, 0xbf),
    runtime_podman: Color::Rgb(0x00, 0x6d, 0x87),
    runtime_kubernetes: Color::Rgb(0x4b, 0x3f, 0xb3),
    runtime_emulation: Color::Rgb(0xa8, 0x47, 0x00),
    runtime_secure: Color::Rgb(0x1f, 0x75, 0x1f),
};

// ── Symbols ────────────────────────────────────────────────────────

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

pub fn header_style() -> Style {
    Style::default()
        .fg(COLORS.highlight)
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
    Block::default()
        .borders(Borders::ALL)
        .border_type(BorderType::Rounded)
        .border_style(Style::default().fg(COLORS.border))
        .title(Span::styled(format!(" {} ", title), title_style()))
}

pub fn block_focused<'a>(title: &'a str) -> Block<'a> {
    Block::default()
        .borders(Borders::ALL)
        .border_type(BorderType::Rounded)
        .border_style(Style::default().fg(current_accent()))
        .title(Span::styled(format!(" {} ", title), title_style()))
}

// ── Badge Helpers ──────────────────────────────────────────────────

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
            .bg(COLORS.bg_bar)
            .fg(COLORS.highlight)
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
            Accent::Amber,
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
