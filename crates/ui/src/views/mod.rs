// UI Views module
mod dag_tab;
mod execution_tab;
mod help_overlay;
mod job_detail;
mod logs_tab;
mod secrets_tab;
mod status_bar;
mod title_bar;
mod trigger_tab;
mod tweaks_overlay;
mod workflows_tab;

pub use secrets_tab::secrets_provider_count;
pub use title_bar::TAB_COUNT;

use crate::app::App;
use ratatui::Frame;

// Main render function for the UI
pub fn render_ui(f: &mut Frame<'_>, app: &mut App) {
    // Plumb the Tweaks accent into the theme's thread-local so
    // anything that calls `theme::current_accent()` or uses
    // `block_focused` picks up the user's choice. Set it before
    // any widget is built and we don't need to pass `app` down.
    let (r, g, b) = app.tweaks_accent.rgb();
    crate::theme::set_accent_override(Some(ratatui::style::Color::Rgb(r, g, b)));

    // Check if help should be shown as an overlay
    if app.show_help {
        help_overlay::render_help_overlay(f, app.help_scroll);
        return;
    }

    let size = f.area();

    // Create main layout
    let main_chunks = ratatui::layout::Layout::default()
        .direction(ratatui::layout::Direction::Vertical)
        .constraints(
            [
                ratatui::layout::Constraint::Length(1), // Title bar and tabs
                ratatui::layout::Constraint::Min(5),    // Main content
                ratatui::layout::Constraint::Length(1), // Status bar
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
        2 => dag_tab::render_dag_tab(f, app, main_chunks[1]),
        3 => logs_tab::render_logs_tab(f, app, main_chunks[1]),
        4 => trigger_tab::render_trigger_tab(f, app, main_chunks[1]),
        5 => secrets_tab::render_secrets_tab(f, app, main_chunks[1]),
        6 => help_overlay::render_help_content(f, main_chunks[1], app.help_scroll),
        _ => {}
    }

    // Render status bar
    status_bar::render_status_bar(f, app, main_chunks[2]);

    // Tweaks overlay is rendered last so it sits above the main view
    // (matches the floating `TweaksPanel` in the design's bottom-right).
    if app.tweaks_open {
        tweaks_overlay::render_tweaks_overlay(f, app, size);
    }
}
