// UI Views module
mod execution_tab;
mod help_overlay;
mod job_detail;
mod logs_tab;
mod status_bar;
mod title_bar;
mod workflows_tab;

use crate::app::App;
use ratatui::{backend::CrosstermBackend, Frame};
use std::io;

// Main render function for the UI
pub fn render_ui(f: &mut Frame<CrosstermBackend<io::Stdout>>, app: &mut App) {
    // Check if help should be shown as an overlay
    if app.show_help {
        help_overlay::render_help_overlay(f);
        return;
    }

    let size = f.size();

    // Create main layout
    let main_chunks = ratatui::layout::Layout::default()
        .direction(ratatui::layout::Direction::Vertical)
        .constraints(
            [
                ratatui::layout::Constraint::Length(3), // Title bar and tabs
                ratatui::layout::Constraint::Min(5),    // Main content
                ratatui::layout::Constraint::Length(2), // Status bar
            ]
            .as_ref(),
        )
        .split(size);

    // Render title bar with tabs
    title_bar::render_title_bar(f, app, main_chunks[0]);

    // Render main content based on selected tab
    match app.selected_tab {
        0 => workflows_tab::render_workflows_tab(f, app, main_chunks[1]),
        1 => {
            if app.detailed_view {
                job_detail::render_job_detail_view(f, app, main_chunks[1])
            } else {
                execution_tab::render_execution_tab(f, app, main_chunks[1])
            }
        }
        2 => logs_tab::render_logs_tab(f, app, main_chunks[1]),
        3 => help_overlay::render_help_tab(f, main_chunks[1]),
        _ => {}
    }

    // Render status bar
    status_bar::render_status_bar(f, app, main_chunks[2]);
}
