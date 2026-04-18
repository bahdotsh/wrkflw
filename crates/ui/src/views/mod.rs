// UI Views module.
//
// All chrome (brand, tab breadcrumb, mode pills, key hints, position
// indicator) is embedded in a single outer frame's top and bottom borders
// — the mdterm pattern. This reclaims the rows a dedicated title/status
// bar would otherwise consume.

mod execution_tab;
mod help_overlay;
mod job_detail;
mod logs_tab;
mod status_bar;
mod title_bar;
mod workflows_tab;

use crate::app::App;
use ratatui::{
    layout::Alignment,
    style::Style,
    widgets::{Block, BorderType, Borders},
    Frame,
};

/// Main render entry point.
pub fn render_ui(f: &mut Frame<'_>, app: &mut App) {
    let t = &app.theme;
    let size = f.area();

    // Apply the theme's base background across the whole frame — matters for
    // the light theme, where a non-Reset bg gives a cohesive look.
    let bg_block = Block::default().style(Style::default().bg(t.bg_default));
    f.render_widget(bg_block, size);

    // Build the outer frame with chrome on the top/bottom borders.
    let mut outer = Block::default()
        .borders(Borders::ALL)
        .border_type(BorderType::Rounded)
        .border_style(Style::default().fg(t.border))
        .title(title_bar::top_left_title(app).alignment(Alignment::Left))
        .title(title_bar::top_right_title(app).alignment(Alignment::Right));

    // Bottom chrome: toast when present, otherwise keys + position.
    if let Some(toast) = status_bar::toast_title(app) {
        outer = outer.title_bottom(toast.alignment(Alignment::Center));
    } else {
        outer = outer
            .title_bottom(status_bar::bottom_left_title(app).alignment(Alignment::Left))
            .title_bottom(status_bar::bottom_right_title(app).alignment(Alignment::Right));
    }

    let inner = outer.inner(size);
    f.render_widget(outer, size);

    // Help modal draws on top of the outer frame.
    if app.show_help {
        help_overlay::render_help_modal(f, app);
        return;
    }

    // Render main content in the inner rect based on selected tab.
    match app.selected_tab {
        0 => workflows_tab::render_workflows_tab(f, app, inner),
        1 => {
            if app.detailed_view {
                job_detail::render_job_detail_view(f, app, inner);
            } else {
                execution_tab::render_execution_tab(f, app, inner);
            }
        }
        2 => logs_tab::render_logs_tab(f, app, inner),
        _ => {}
    }
}
