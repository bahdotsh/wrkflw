// Job-dependency helpers: topological stages and the compact stage list
// the Execution tab shows beside a run.

use crate::models::{JobState, Workflow};
use crate::text;
use crate::theme::{self, COLORS};
use ratatui::{
    layout::Rect,
    style::{Modifier, Style},
    text::{Line, Span},
    widgets::Paragraph,
    Frame,
};
use std::collections::{HashMap, HashSet};
use wrkflw_parser::workflow::WorkflowDefinition;

/// Compute topological levels (Kahn's algorithm). Returns columns of job names.
/// Jobs not present in the dependency graph go into the first column.
pub fn topo_levels(def: &WorkflowDefinition) -> Vec<Vec<String>> {
    let names: Vec<String> = {
        let mut v: Vec<String> = def.jobs.keys().cloned().collect();
        v.sort();
        v
    };
    let name_set: HashSet<&str> = names.iter().map(|s| s.as_str()).collect();

    // Build "needs" map restricted to known jobs only.
    let mut needs: HashMap<String, Vec<String>> = HashMap::new();
    for n in &names {
        let job = &def.jobs[n];
        let req: Vec<String> = job
            .needs
            .as_ref()
            .map(|v| {
                v.iter()
                    .filter(|d| name_set.contains(d.as_str()))
                    .cloned()
                    .collect()
            })
            .unwrap_or_default();
        needs.insert(n.clone(), req);
    }

    let mut placed: HashSet<String> = HashSet::new();
    let mut levels: Vec<Vec<String>> = Vec::new();

    while placed.len() < names.len() {
        let mut layer: Vec<String> = Vec::new();
        for n in &names {
            if placed.contains(n) {
                continue;
            }
            let deps = &needs[n];
            if deps.iter().all(|d| placed.contains(d)) {
                layer.push(n.clone());
            }
        }
        if layer.is_empty() {
            // Cycle / unresolved — drop remaining jobs into a final column.
            let rest: Vec<String> = names
                .iter()
                .filter(|n| !placed.contains(*n))
                .cloned()
                .collect();
            placed.extend(rest.iter().cloned());
            levels.push(rest);
            break;
        }
        for n in &layer {
            placed.insert(n.clone());
        }
        levels.push(layer);
    }
    levels
}

/// Compact stage-by-stage job list for side panes.
pub fn render(frame: &mut Frame<'_>, area: Rect, workflow: &Workflow, spinner_frame: usize) {
    let Some(def) = workflow.definition.as_deref() else {
        theme::empty_state(frame, area, "Couldn't parse this workflow", vec![]);
        return;
    };
    let levels = topo_levels(def);
    if levels.is_empty() {
        theme::empty_state(frame, area, "No jobs", vec![]);
        return;
    }

    let w = area.width as usize;
    let mut lines: Vec<Line> = Vec::new();
    for (li, layer) in levels.iter().enumerate() {
        lines.push(Line::from(Span::styled(
            format!("Stage {}", li + 1),
            Style::default()
                .fg(COLORS.text_muted)
                .add_modifier(Modifier::BOLD),
        )));
        for name in layer {
            let state = workflow.job_state(name);
            let (glyph, style) = theme::job_state(state, spinner_frame);
            let name_style = match state {
                JobState::Pending | JobState::Skipped => Style::default().fg(COLORS.text_muted),
                JobState::Failure => Style::default().fg(COLORS.error),
                _ => Style::default().fg(COLORS.text),
            };
            lines.push(Line::from(vec![
                Span::styled("├ ", Style::default().fg(COLORS.border)),
                Span::styled(glyph.to_string(), style),
                Span::raw(" "),
                Span::styled(
                    text::truncate(&text::sanitize(name), w.saturating_sub(4)).into_owned(),
                    name_style,
                ),
            ]));
        }
    }
    frame.render_widget(Paragraph::new(lines), area);
}

/// How many runs a matrix job expands into, `?` when that is only known
/// once an expression is evaluated.
pub fn matrix_size_label(matrix: &wrkflw_matrix::MatrixConfig) -> Option<String> {
    use wrkflw_matrix::MatrixEntries;

    if matrix.expression.is_some() || matches!(matrix.include, MatrixEntries::Expression(_)) {
        return Some("?".to_string());
    }
    match wrkflw_matrix::expand_matrix(matrix) {
        Ok(combos) if !combos.is_empty() => Some(combos.len().to_string()),
        Ok(_) => None,
        Err(_) => Some("?".to_string()),
    }
}
