// Centralized theme for wrkflw TUI
//
// Palette and symbol set match the design handoff in `wrkflw TUI.html`.
// View files import from this module instead of hardcoding.

use ratatui::{
    style::{Color, Modifier, Style},
    text::Span,
    widgets::{Block, BorderType, Borders},
};

// ── Color Palette ──────────────────────────────────────────────────

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
    pub runtime_emulation: Color,
    pub runtime_secure: Color,
}

pub const COLORS: Colors = Colors {
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
    runtime_emulation: Color::Rgb(0xff, 0x99, 0x66),
    runtime_secure: Color::Rgb(0x8f, 0xce, 0x8f),
};

// ── Symbols ────────────────────────────────────────────────────────

pub use wrkflw_logging::symbols;

// ── Style Helpers ──────────────────────────────────────────────────

pub fn title_style() -> Style {
    Style::default()
        .fg(COLORS.highlight)
        .add_modifier(Modifier::BOLD)
}

pub fn brand_style() -> Style {
    Style::default()
        .fg(COLORS.accent)
        .add_modifier(Modifier::BOLD)
}

pub fn label_style() -> Style {
    Style::default().fg(COLORS.accent)
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
        .border_style(Style::default().fg(COLORS.border_focused))
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
            BadgeKind::Accent => COLORS.accent,
            BadgeKind::Highlight => COLORS.highlight,
            BadgeKind::Docker => COLORS.runtime_docker,
            BadgeKind::Podman => COLORS.runtime_podman,
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

/// Solid badge: dark text on a colored background.
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
