// Progress bar component
use ratatui::{
    style::{Color, Style},
    widgets::Gauge,
};

/// A simple progress bar component for the TUI
pub struct ProgressBar {
    pub progress: f64,
    pub label: Option<String>,
    pub color: Color,
}

impl ProgressBar {
    /// Create a new progress bar
    pub fn new(progress: f64) -> Self {
        ProgressBar {
            progress: progress.clamp(0.0, 1.0),
            label: None,
            color: Color::Blue,
        }
    }

    /// Set label
    pub fn label(mut self, label: &str) -> Self {
        self.label = Some(label.to_string());
        self
    }

    /// Set color
    pub fn color(mut self, color: Color) -> Self {
        self.color = color;
        self
    }

    /// Update progress value
    pub fn update(&mut self, progress: f64) {
        self.progress = progress.clamp(0.0, 1.0);
    }

    /// Render the progress bar
    pub fn render(&self) -> Gauge {
        let label = match &self.label {
            Some(lbl) => format!("{} {:.0}%", lbl, self.progress * 100.0),
            None => format!("{:.0}%", self.progress * 100.0),
        };

        Gauge::default()
            .gauge_style(Style::default().fg(self.color).bg(Color::Black))
            .label(label)
            .ratio(self.progress)
    }
}
