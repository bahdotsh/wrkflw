// Button component
use ratatui::{
    style::{Color, Modifier, Style},
    text::{Line, Span},
    widgets::Paragraph,
};

/// A simple button component for the TUI
pub struct Button {
    pub label: String,
    pub is_selected: bool,
    pub is_active: bool,
}

impl Button {
    /// Create a new button
    pub fn new(label: &str) -> Self {
        Button {
            label: label.to_string(),
            is_selected: false,
            is_active: true,
        }
    }

    /// Set selected state
    pub fn selected(mut self, is_selected: bool) -> Self {
        self.is_selected = is_selected;
        self
    }

    /// Set active state
    pub fn active(mut self, is_active: bool) -> Self {
        self.is_active = is_active;
        self
    }

    /// Render the button
    pub fn render(&self) -> Paragraph {
        let (fg, bg) = match (self.is_selected, self.is_active) {
            (true, true) => (Color::Black, Color::Yellow),
            (true, false) => (Color::Black, Color::DarkGray),
            (false, true) => (Color::White, Color::Blue),
            (false, false) => (Color::DarkGray, Color::Black),
        };

        let style = Style::default().fg(fg).bg(bg).add_modifier(Modifier::BOLD);

        Paragraph::new(Line::from(vec![Span::styled(
            format!(" {} ", self.label),
            style,
        )]))
    }
}
