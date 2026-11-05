// Run header — the two lines above every Run view: the run's summary,
// then the Jobs · Graph · Log buttons with the key for each, and a row
// for the buttons' shadows.
use crate::app::App;
use crate::theme;
use ratatui::{
    layout::{Constraint, Direction, Layout, Rect},
    text::{Line, Span},
    widgets::Paragraph,
    Frame,
};

use super::{execution_tab, TAB_DAG, TAB_EXECUTION, TAB_LOGS};

const VIEWS: [(usize, &str, &str); 3] = [
    (TAB_EXECUTION, "x", "Jobs"),
    (TAB_DAG, "g", "Graph"),
    (TAB_LOGS, "l", "Log"),
];

pub fn render_run_header(f: &mut Frame<'_>, app: &App, area: Rect) {
    let rows = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Length(1),
            Constraint::Length(1),
            Constraint::Length(1),
        ])
        .split(area.inner(ratatui::layout::Margin {
            horizontal: 1,
            vertical: 0,
        }));

    // The graph follows the Workflows cursor; jobs and log follow the run.
    let idx = if app.selected_tab == TAB_DAG {
        app.workflow_list_state.selected()
    } else {
        app.shown_workflow()
    };
    match idx.and_then(|i| app.workflows.get(i)) {
        Some(workflow) => execution_tab::render_summary_line(f, app, workflow, rows[0]),
        None => f.render_widget(
            Paragraph::new(Span::styled("No workflow selected", theme::muted_style())),
            rows[0],
        ),
    }

    // Raised push buttons; the current view's is lit. The header's third
    // row holds their shadows.
    let buttons: Vec<(&str, &str, bool)> = VIEWS
        .iter()
        .map(|(screen, key, label)| (*key, *label, app.selected_tab == *screen))
        .collect();
    let (spans, shadows) = theme::raised_buttons(&buttons, rows[1].width as usize);
    f.render_widget(Paragraph::new(Line::from(shadows)), rows[2]);
    f.render_widget(Paragraph::new(Line::from(spans)), rows[1]);
}
