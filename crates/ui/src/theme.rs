// Centralized theme for wrkflw TUI.
//
// The `Theme` struct carries every color the TUI renders; `App` owns one
// instance. Call sites take `&Theme` rather than reading a global `COLORS`
// so dark/light can switch at runtime.
//
// Palette is Catppuccin Mocha (dark) / Latte (light) — the same RGB values
// ship in giff and mdterm, two sibling TUIs. Keeping them aligned means a
// user who runs multiple tools sees a single visual identity.

use ratatui::{
    style::{Color, Modifier, Style},
    text::Span,
    widgets::{Block, BorderType, Borders},
};

/// Re-export of the shared symbol constants from `wrkflw_logging::symbols`.
/// All crates use a single source of truth for Unicode symbols.
pub use wrkflw_logging::symbols;

// ── Theme ──────────────────────────────────────────────────────────

#[derive(Debug, Clone, PartialEq)]
pub struct Theme {
    pub is_dark: bool,

    // Brand / accent
    pub accent: Color,
    pub highlight: Color,

    // Semantic status
    pub success: Color,
    pub error: Color,
    pub warning: Color,
    pub info: Color,
    pub trigger: Color,

    // Text hierarchy (brightest → dimmest)
    pub fg_bright: Color,
    pub fg_normal: Color,
    pub fg_dim: Color,
    pub fg_muted: Color,
    pub help_hint: Color,

    // Borders
    pub border: Color,
    pub border_focused: Color,
    pub border_dim: Color,

    // Backgrounds
    pub bg_default: Color,
    pub bg_selection: Color,
    pub bg_header: Color,
    pub bg_modal: Color,
    pub bg_modal_dim: Color,
    pub bg_key_badge: Color,
    pub bg_bar: Color,
    pub bg_dark: Color,

    // Ancillary
    pub fg_separator: Color,
    pub fg_badge: Color,
    pub scrollbar_track: Color,
    pub scrollbar_thumb: Color,

    // Runtime badge colors
    pub runtime_docker: Color,
    pub runtime_podman: Color,
    pub runtime_emulation: Color,
    pub runtime_secure: Color,
}

impl Theme {
    /// Catppuccin Mocha — the default dark theme.
    pub fn dark() -> Self {
        Self {
            is_dark: true,

            accent: Color::Rgb(137, 180, 250),   // blue
            highlight: Color::Rgb(249, 226, 175), // yellow

            success: Color::Rgb(166, 227, 161), // green
            error: Color::Rgb(243, 139, 168),   // red
            warning: Color::Rgb(250, 179, 135), // peach
            info: Color::Rgb(137, 220, 235),    // sky
            trigger: Color::Rgb(203, 166, 247), // mauve

            fg_bright: Color::Rgb(205, 214, 244), // text
            fg_normal: Color::Rgb(186, 194, 222), // subtext1
            fg_dim: Color::Rgb(127, 132, 156),    // overlay2
            fg_muted: Color::Rgb(108, 112, 134),  // overlay1
            help_hint: Color::Rgb(88, 91, 112),   // surface2

            border: Color::Rgb(69, 71, 90),         // surface1
            border_focused: Color::Rgb(137, 180, 250), // blue
            border_dim: Color::Rgb(49, 50, 68),     // surface0

            bg_default: Color::Reset,
            bg_selection: Color::Rgb(49, 50, 68),   // surface0
            bg_header: Color::Rgb(24, 24, 37),      // crust
            bg_modal: Color::Rgb(30, 30, 46),       // base
            bg_modal_dim: Color::Rgb(17, 17, 27),   // mantle
            bg_key_badge: Color::Rgb(49, 50, 68),   // surface0
            bg_bar: Color::Rgb(24, 24, 37),         // crust
            bg_dark: Color::Rgb(17, 17, 27),        // mantle

            fg_separator: Color::Rgb(49, 50, 68),     // surface0
            fg_badge: Color::Rgb(24, 24, 37),         // crust (dark text on light badges)
            scrollbar_track: Color::Rgb(49, 50, 68),  // surface0
            scrollbar_thumb: Color::Rgb(127, 132, 156), // overlay2

            runtime_docker: Color::Rgb(137, 180, 250),    // blue
            runtime_podman: Color::Rgb(137, 220, 235),    // sky
            runtime_emulation: Color::Rgb(243, 139, 168), // red
            runtime_secure: Color::Rgb(166, 227, 161),    // green
        }
    }

    /// Catppuccin Latte — the light theme.
    pub fn light() -> Self {
        Self {
            is_dark: false,

            accent: Color::Rgb(30, 102, 245),    // blue
            highlight: Color::Rgb(223, 142, 29), // yellow

            success: Color::Rgb(64, 160, 43),  // green
            error: Color::Rgb(210, 15, 57),    // red
            warning: Color::Rgb(254, 100, 11), // peach
            info: Color::Rgb(4, 165, 229),     // sky
            trigger: Color::Rgb(136, 57, 239), // mauve

            fg_bright: Color::Rgb(30, 32, 48),    // text
            fg_normal: Color::Rgb(76, 79, 105),   // subtext1
            fg_dim: Color::Rgb(124, 127, 147),    // overlay2
            fg_muted: Color::Rgb(140, 143, 161),  // overlay1
            help_hint: Color::Rgb(172, 176, 190), // surface2

            border: Color::Rgb(188, 192, 204),       // surface1
            border_focused: Color::Rgb(30, 102, 245), // blue
            border_dim: Color::Rgb(204, 208, 218),   // surface0

            bg_default: Color::Rgb(239, 241, 245),  // base
            bg_selection: Color::Rgb(220, 224, 232), // surface0
            bg_header: Color::Rgb(220, 224, 232),    // surface0
            bg_modal: Color::Rgb(239, 241, 245),     // base
            bg_modal_dim: Color::Rgb(230, 233, 239), // mantle
            bg_key_badge: Color::Rgb(220, 224, 232), // surface0
            bg_bar: Color::Rgb(230, 233, 239),       // mantle
            bg_dark: Color::Rgb(230, 233, 239),      // mantle

            fg_separator: Color::Rgb(220, 224, 232),   // surface0
            fg_badge: Color::Rgb(239, 241, 245),       // base (light text on dark badges)
            scrollbar_track: Color::Rgb(220, 224, 232), // surface0
            scrollbar_thumb: Color::Rgb(140, 143, 161), // overlay1

            runtime_docker: Color::Rgb(30, 102, 245),   // blue
            runtime_podman: Color::Rgb(4, 165, 229),    // sky
            runtime_emulation: Color::Rgb(210, 15, 57), // red
            runtime_secure: Color::Rgb(64, 160, 43),    // green
        }
    }

    /// Flip between dark and light. Used by the `T` keybind.
    pub fn toggle(&self) -> Self {
        if self.is_dark {
            Self::light()
        } else {
            Self::dark()
        }
    }

    pub fn by_name(name: &str) -> Option<Self> {
        match name {
            "dark" => Some(Self::dark()),
            "light" => Some(Self::light()),
            _ => None,
        }
    }

    pub fn name(&self) -> &'static str {
        if self.is_dark { "dark" } else { "light" }
    }
}

impl Default for Theme {
    fn default() -> Self {
        Self::dark()
    }
}

/// Parse a `#RRGGBB` hex color. Returns `None` on any malformed input.
pub fn parse_color(s: &str) -> Option<Color> {
    let hex = s.strip_prefix('#')?;
    if hex.len() != 6 {
        return None;
    }
    let r = u8::from_str_radix(&hex[0..2], 16).ok()?;
    let g = u8::from_str_radix(&hex[2..4], 16).ok()?;
    let b = u8::from_str_radix(&hex[4..6], 16).ok()?;
    Some(Color::Rgb(r, g, b))
}

// ── Style Helpers ──────────────────────────────────────────────────

pub fn title_style(t: &Theme) -> Style {
    Style::default()
        .fg(t.highlight)
        .add_modifier(Modifier::BOLD)
}

pub fn brand_style(t: &Theme) -> Style {
    Style::default().fg(t.accent).add_modifier(Modifier::BOLD)
}

pub fn label_style(t: &Theme) -> Style {
    Style::default().fg(t.accent)
}

pub fn selected_style(t: &Theme) -> Style {
    Style::default().bg(t.bg_selection).add_modifier(Modifier::BOLD)
}

pub fn header_style(t: &Theme) -> Style {
    Style::default()
        .fg(t.highlight)
        .add_modifier(Modifier::BOLD | Modifier::UNDERLINED)
}

pub fn search_highlight(t: &Theme) -> Style {
    Style::default()
        .bg(t.highlight)
        .fg(t.fg_badge)
        .add_modifier(Modifier::BOLD)
}

pub fn dim_style(t: &Theme) -> Style {
    Style::default().fg(t.fg_dim)
}

pub fn muted_style(t: &Theme) -> Style {
    Style::default().fg(t.fg_muted)
}

pub fn key_style(t: &Theme) -> Style {
    Style::default().fg(t.highlight)
}

pub fn hint_style(t: &Theme) -> Style {
    Style::default().fg(t.help_hint)
}

// ── Status styles ──────────────────────────────────────────────────

use crate::models::WorkflowStatus;
use wrkflw_executor::{JobStatus, StepStatus};

pub fn workflow_status(t: &Theme, status: &WorkflowStatus) -> (&'static str, Style) {
    match status {
        WorkflowStatus::NotStarted => (symbols::NOT_STARTED, Style::default().fg(t.fg_dim)),
        WorkflowStatus::Running => (symbols::RUNNING, Style::default().fg(t.info)),
        WorkflowStatus::Success => (symbols::SUCCESS, Style::default().fg(t.success)),
        WorkflowStatus::Failed => (symbols::FAILURE, Style::default().fg(t.error)),
        WorkflowStatus::Skipped => (symbols::SKIPPED, Style::default().fg(t.warning)),
    }
}

pub fn spinner(frame: usize) -> &'static str {
    symbols::SPINNER[frame % symbols::SPINNER.len()]
}

pub fn workflow_status_animated(
    t: &Theme,
    status: &WorkflowStatus,
    spinner_frame: usize,
) -> (&'static str, Style) {
    match status {
        WorkflowStatus::Running => (spinner(spinner_frame), Style::default().fg(t.info)),
        other => workflow_status(t, other),
    }
}

pub fn job_status(t: &Theme, status: &JobStatus) -> (&'static str, Style) {
    match status {
        JobStatus::Success => (symbols::SUCCESS, Style::default().fg(t.success)),
        JobStatus::Failure => (symbols::FAILURE, Style::default().fg(t.error)),
        JobStatus::Skipped => (symbols::SKIPPED, Style::default().fg(t.fg_dim)),
    }
}

pub fn step_status(t: &Theme, status: &StepStatus) -> (&'static str, Style) {
    match status {
        StepStatus::Success => (symbols::SUCCESS, Style::default().fg(t.success)),
        StepStatus::Failure => (symbols::FAILURE, Style::default().fg(t.error)),
        StepStatus::Skipped => (symbols::SKIPPED, Style::default().fg(t.fg_dim)),
    }
}

// ── Block helpers ──────────────────────────────────────────────────

pub fn block<'a>(t: &Theme, title: &'a str) -> Block<'a> {
    Block::default()
        .borders(Borders::ALL)
        .border_type(BorderType::Rounded)
        .border_style(Style::default().fg(t.border))
        .title(Span::styled(format!(" {} ", title), title_style(t)))
}

pub fn block_focused<'a>(t: &Theme, title: &'a str) -> Block<'a> {
    Block::default()
        .borders(Borders::ALL)
        .border_type(BorderType::Rounded)
        .border_style(Style::default().fg(t.border_focused))
        .title(Span::styled(format!(" {} ", title), title_style(t)))
}

// ── Badge helpers ──────────────────────────────────────────────────

pub fn badge<'a>(text: &'a str, bg: Color, fg: Color) -> Span<'a> {
    Span::styled(format!(" {} ", text), Style::default().bg(bg).fg(fg))
}

pub fn log_badge(t: &Theme, level: &str) -> Style {
    match level {
        "ERROR" => Style::default().bg(t.error).fg(t.fg_badge),
        "WARN" => Style::default().bg(t.warning).fg(t.fg_badge),
        "SUCCESS" => Style::default().fg(t.success),
        "INFO" => Style::default().fg(t.info),
        "TRIG" => Style::default().fg(t.trigger),
        _ => Style::default().fg(t.fg_dim),
    }
}

// ── Tests ──────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_color_valid() {
        assert_eq!(parse_color("#FF0000"), Some(Color::Rgb(255, 0, 0)));
        assert_eq!(parse_color("#00ff00"), Some(Color::Rgb(0, 255, 0)));
        assert_eq!(parse_color("#1a2B3c"), Some(Color::Rgb(0x1a, 0x2b, 0x3c)));
    }

    #[test]
    fn parse_color_invalid() {
        assert_eq!(parse_color("FF0000"), None);
        assert_eq!(parse_color("#FFF"), None);
        assert_eq!(parse_color("#GGGGGG"), None);
        assert_eq!(parse_color(""), None);
    }

    #[test]
    fn by_name_known() {
        assert!(Theme::by_name("dark").unwrap().is_dark);
        assert!(!Theme::by_name("light").unwrap().is_dark);
    }

    #[test]
    fn by_name_unknown() {
        assert!(Theme::by_name("solarized").is_none());
    }

    #[test]
    fn toggle_flips() {
        let d = Theme::dark();
        assert!(d.is_dark);
        let l = d.toggle();
        assert!(!l.is_dark);
        let d2 = l.toggle();
        assert!(d2.is_dark);
    }

    #[test]
    fn name_reports_variant() {
        assert_eq!(Theme::dark().name(), "dark");
        assert_eq!(Theme::light().name(), "light");
    }

    #[test]
    fn default_is_dark() {
        assert!(Theme::default().is_dark);
    }

    #[test]
    fn dark_theme_structural_invariants() {
        let t = Theme::dark();
        assert_ne!(t.success, t.error);
        assert_ne!(t.info, t.trigger);
        assert_ne!(t.border, t.border_focused);
        assert_ne!(t.runtime_docker, t.runtime_emulation);
    }

    #[test]
    fn light_theme_structural_invariants() {
        let t = Theme::light();
        assert!(!t.is_dark);
        assert_ne!(t.success, t.error);
        assert_ne!(t.border, t.border_focused);
        // Light theme should paint a background rather than Reset
        assert_ne!(t.bg_default, Color::Reset);
    }
}
