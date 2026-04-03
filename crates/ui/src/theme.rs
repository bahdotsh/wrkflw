// Centralized theme for wrkflw TUI
//
// All colors, styles, and symbols are defined here.
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
    pub bg_bar: Color,
    pub bg_dark: Color,

    // Runtime badges
    pub runtime_docker: Color,
    pub runtime_podman: Color,
    pub runtime_emulation: Color,
    pub runtime_secure: Color,
}

pub const COLORS: Colors = Colors {
    accent: Color::Cyan,
    highlight: Color::Yellow,

    success: Color::Green,
    error: Color::Red,
    warning: Color::Yellow,
    info: Color::Cyan,
    trigger: Color::Magenta,

    text: Color::White,
    text_dim: Color::Gray,
    text_muted: Color::DarkGray,

    border: Color::DarkGray,
    border_focused: Color::Cyan,

    bg_selected: Color::Rgb(40, 44, 52),
    bg_bar: Color::DarkGray,
    bg_dark: Color::Black,

    runtime_docker: Color::Blue,
    runtime_podman: Color::Cyan,
    runtime_emulation: Color::Red,
    runtime_secure: Color::Green,
};

// ── Symbols ────────────────────────────────────────────────────────

/// Re-export the shared symbol constants from `wrkflw_logging::symbols`.
/// All crates use a single source of truth for Unicode symbols.
pub use wrkflw_logging::symbols;

// ── Style Helpers ──────────────────────────────────────────────────

/// Style for section/block titles
pub fn title_style() -> Style {
    Style::default()
        .fg(COLORS.highlight)
        .add_modifier(Modifier::BOLD)
}

/// Style for the wrkflw brand title
pub fn brand_style() -> Style {
    Style::default()
        .fg(COLORS.accent)
        .add_modifier(Modifier::BOLD)
}

/// Style for field labels ("Workflow:", "Status:", etc.)
pub fn label_style() -> Style {
    Style::default().fg(COLORS.accent)
}

/// Style for selected/highlighted rows
pub fn selected_style() -> Style {
    Style::default()
        .bg(COLORS.bg_selected)
        .add_modifier(Modifier::BOLD)
}

/// Style for table/column headers
pub fn header_style() -> Style {
    Style::default()
        .fg(COLORS.highlight)
        .add_modifier(Modifier::BOLD | Modifier::UNDERLINED)
}

/// Style for search match highlighting
pub fn search_highlight() -> Style {
    Style::default()
        .bg(COLORS.highlight)
        .fg(Color::Black)
        .add_modifier(Modifier::BOLD)
}

/// Style for dimmed/secondary text
pub fn dim_style() -> Style {
    Style::default().fg(COLORS.text_dim)
}

/// Style for muted text (paths, timestamps)
pub fn muted_style() -> Style {
    Style::default().fg(COLORS.text_muted)
}

/// Style for key hints in status bar ("[Enter]", "[Space]")
pub fn key_style() -> Style {
    Style::default().fg(COLORS.highlight)
}

/// Style for hint descriptions in status bar
pub fn hint_style() -> Style {
    Style::default().fg(COLORS.text_dim)
}

// ── Status Styles ──────────────────────────────────────────────────

use crate::models::WorkflowStatus;
use wrkflw_executor::{JobStatus, StepStatus};

/// Get symbol and style for a WorkflowStatus
pub fn workflow_status(status: &WorkflowStatus) -> (&'static str, Style) {
    match status {
        WorkflowStatus::NotStarted => (symbols::NOT_STARTED, Style::default().fg(COLORS.text_dim)),
        WorkflowStatus::Running => (symbols::RUNNING, Style::default().fg(COLORS.info)),
        WorkflowStatus::Success => (symbols::SUCCESS, Style::default().fg(COLORS.success)),
        WorkflowStatus::Failed => (symbols::FAILURE, Style::default().fg(COLORS.error)),
        WorkflowStatus::Skipped => (symbols::SKIPPED, Style::default().fg(COLORS.warning)),
    }
}

/// Get animated spinner symbol for running state
pub fn spinner(frame: usize) -> &'static str {
    symbols::SPINNER[frame % symbols::SPINNER.len()]
}

/// Get symbol and style for a WorkflowStatus with spinner animation
pub fn workflow_status_animated(
    status: &WorkflowStatus,
    spinner_frame: usize,
) -> (&'static str, Style) {
    match status {
        WorkflowStatus::Running => (spinner(spinner_frame), Style::default().fg(COLORS.info)),
        other => workflow_status(other),
    }
}

/// Get symbol and style for a JobStatus
pub fn job_status(status: &JobStatus) -> (&'static str, Style) {
    match status {
        JobStatus::Success => (symbols::SUCCESS, Style::default().fg(COLORS.success)),
        JobStatus::Failure => (symbols::FAILURE, Style::default().fg(COLORS.error)),
        JobStatus::Skipped => (symbols::SKIPPED, Style::default().fg(COLORS.text_dim)),
    }
}

/// Get symbol and style for a StepStatus
pub fn step_status(status: &StepStatus) -> (&'static str, Style) {
    match status {
        StepStatus::Success => (symbols::SUCCESS, Style::default().fg(COLORS.success)),
        StepStatus::Failure => (symbols::FAILURE, Style::default().fg(COLORS.error)),
        StepStatus::Skipped => (symbols::SKIPPED, Style::default().fg(COLORS.text_dim)),
    }
}

// ── Block Helpers ──────────────────────────────────────────────────

/// Create a styled block with rounded borders and a title
pub fn block<'a>(title: &'a str) -> Block<'a> {
    Block::default()
        .borders(Borders::ALL)
        .border_type(BorderType::Rounded)
        .border_style(Style::default().fg(COLORS.border))
        .title(Span::styled(format!(" {} ", title), title_style()))
}

/// Create a styled block with focused (accent) border
pub fn block_focused<'a>(title: &'a str) -> Block<'a> {
    Block::default()
        .borders(Borders::ALL)
        .border_type(BorderType::Rounded)
        .border_style(Style::default().fg(COLORS.border_focused))
        .title(Span::styled(format!(" {} ", title), title_style()))
}

// ── Badge Helpers ──────────────────────────────────────────────────

/// Create a styled badge span (text with colored background)
pub fn badge<'a>(text: &'a str, bg: Color, fg: Color) -> Span<'a> {
    Span::styled(format!(" {} ", text), Style::default().bg(bg).fg(fg))
}

/// Log level badge styles
pub fn log_badge(level: &str) -> Style {
    match level {
        "ERROR" => Style::default().bg(COLORS.error).fg(COLORS.text),
        "WARN" => Style::default().bg(COLORS.warning).fg(Color::Black),
        "SUCCESS" => Style::default().fg(COLORS.success),
        "INFO" => Style::default().fg(COLORS.info),
        "TRIG" => Style::default().fg(COLORS.trigger),
        _ => Style::default().fg(COLORS.text_dim),
    }
}
