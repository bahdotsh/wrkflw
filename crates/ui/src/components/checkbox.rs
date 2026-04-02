// Checkbox component
use crate::theme::{self, COLORS};
use ratatui::{
    style::{Modifier, Style},
    text::{Line, Span},
    widgets::Paragraph,
};

/// A simple checkbox component for the TUI
pub struct Checkbox {
    pub label: String,
    pub is_checked: bool,
    pub is_selected: bool,
}

impl Checkbox {
    /// Create a new checkbox
    pub fn new(label: &str) -> Self {
        Checkbox {
            label: label.to_string(),
            is_checked: false,
            is_selected: false,
        }
    }

    /// Set checked state
    pub fn checked(mut self, is_checked: bool) -> Self {
        self.is_checked = is_checked;
        self
    }

    /// Set selected state
    pub fn selected(mut self, is_selected: bool) -> Self {
        self.is_selected = is_selected;
        self
    }

    /// Toggle checked state
    pub fn toggle(&mut self) {
        self.is_checked = !self.is_checked;
    }

    /// Render the checkbox
    pub fn render(&self) -> Paragraph<'_> {
        let checkbox = if self.is_checked {
            theme::symbols::CHECKBOX_ON
        } else {
            theme::symbols::CHECKBOX_OFF
        };

        let style = if self.is_selected {
            Style::default()
                .fg(COLORS.highlight)
                .add_modifier(Modifier::BOLD)
        } else {
            Style::default().fg(COLORS.text)
        };

        Paragraph::new(Line::from(vec![
            Span::styled(checkbox, style),
            Span::raw(" "),
            Span::styled(&self.label, style),
        ]))
    }
}
