// UI Views module
mod dag_tab;
mod execution_tab;
mod help_overlay;
mod job_detail;
mod logs_tab;
mod run_header;
mod runs_tab;
mod secrets_tab;
mod status_bar;
mod title_bar;
mod trigger_tab;
mod tweaks_overlay;
mod workflows_tab;

pub use job_detail::StepOutputCache;

/// Number of sub-tabs in the step inspector, for Tab cycling.
pub const INSPECTOR_TABS: usize = job_detail::INSPECTOR_TABS.len();

pub use title_bar::{
    tab_of, SCREEN_COUNT, TABS, TAB_COUNT, TAB_DAG, TAB_EXECUTION, TAB_LOGS, TAB_RUN, TAB_RUNS,
    TAB_SECRETS, TAB_TRIGGER, TAB_WORKFLOWS,
};

use crate::app::App;
use ratatui::Frame;

/// RAII guard that installs an accent override on construction and
/// clears it on drop. Scoping the thread-local to one render pass
/// stops later code (tests, alternate backends) from inheriting stale
/// state — the thread-local is a handoff, not a setting.
struct AccentScope;

impl AccentScope {
    fn install(color: ratatui::style::Color) -> Self {
        crate::theme::set_accent_override(Some(color));
        Self
    }
}

impl Drop for AccentScope {
    fn drop(&mut self) {
        crate::theme::set_accent_override(None);
    }
}

// Main render function for the UI
pub fn render_ui(f: &mut Frame<'_>, app: &mut App) {
    // Plumb the Tweaks accent into the theme's thread-local so
    // anything that calls `theme::current_accent()` or uses
    // `block_focused` picks up the user's choice. The guard clears
    // the override on drop so the override lives for exactly this
    // frame.
    let (r, g, b) = app.tweaks_accent.rgb();
    let _accent = AccentScope::install(ratatui::style::Color::Rgb(r, g, b));

    let size = f.area();
    // Paint the palette's own background so the theme looks the same in
    // every terminal instead of inheriting its colours.
    f.render_widget(
        ratatui::widgets::Paragraph::new("")
            .style(ratatui::style::Style::default().bg(crate::theme::COLORS.bg_panel)),
        size,
    );

    // Create main layout
    let main_chunks = ratatui::layout::Layout::default()
        .direction(ratatui::layout::Direction::Vertical)
        .constraints([
            ratatui::layout::Constraint::Length(2), // Title bar: tab buttons and their shadows
            ratatui::layout::Constraint::Min(0),    // Main content
            ratatui::layout::Constraint::Length(2), // Key bar and its button shadows
        ])
        .split(size);

    // Render title bar with tabs
    title_bar::render_title_bar(f, app, main_chunks[0]);

    // The Run tab's views share a header: the run's summary and the
    // Jobs · Graph · Log switcher.
    let mut content = main_chunks[1];
    if tab_of(app.selected_tab) == TAB_RUN && !app.detailed_view {
        let parts = ratatui::layout::Layout::default()
            .direction(ratatui::layout::Direction::Vertical)
            .constraints([
                ratatui::layout::Constraint::Length(3),
                ratatui::layout::Constraint::Min(0),
            ])
            .split(content);
        run_header::render_run_header(f, app, parts[0]);
        content = parts[1];
    }

    // Render main content based on selected tab
    match app.selected_tab {
        TAB_WORKFLOWS => workflows_tab::render_workflows_tab(f, app, content),
        TAB_EXECUTION => {
            if app.detailed_view {
                job_detail::render_job_detail_view(f, app, content)
            } else {
                execution_tab::render_execution_tab(f, app, content)
            }
        }
        TAB_DAG => dag_tab::render_dag_tab(f, app, content),
        TAB_LOGS => logs_tab::render_logs_tab(f, app, content),
        TAB_TRIGGER => trigger_tab::render_trigger_tab(f, app, content),
        TAB_SECRETS => secrets_tab::render_secrets_tab(f, app, content),
        TAB_RUNS => runs_tab::render_runs_tab(f, app, content),
        _ => {}
    }

    // Render status bar
    status_bar::render_status_bar(f, app, main_chunks[2]);

    // Tweaks overlay is rendered last so it sits above the main view
    // (matches the floating `TweaksPanel` in the design's bottom-right).
    if app.tweaks_open {
        tweaks_overlay::render_tweaks_overlay(f, app, size);
    }

    // Help floats over the current screen so it stays in context.
    if app.show_help {
        help_overlay::render_help_overlay(f, &mut app.help_scroll);
    }
}

#[cfg(test)]
mod tests {
    use super::AccentScope;
    use crate::theme::{self, COLORS};
    use ratatui::style::Color;

    #[test]
    fn accent_scope_clears_thread_local_on_drop() {
        // Regression guard: the thread-local accent override is a
        // per-frame handoff, not a setting. Installing it and dropping
        // the guard must restore `current_accent()` to the static
        // palette so later code (tests, alternate backends, the next
        // frame) doesn't inherit stale state. Without this contract a
        // test that renders with a Tweaks accent installed could leak
        // the override into every subsequent test on the same thread.
        assert_eq!(theme::current_accent(), COLORS.accent);
        {
            let _guard = AccentScope::install(Color::Rgb(0xff, 0x00, 0x00));
            assert_eq!(theme::current_accent(), Color::Rgb(0xff, 0x00, 0x00));
        }
        assert_eq!(theme::current_accent(), COLORS.accent);
    }
}
