// Mini job-dependency view.
//
// The design's full DAG is a free-form SVG; in a terminal we render a tighter
// columns-by-topological-level layout using box-drawing chars. When `needs:`
// data is unavailable we fall back to a single linear column so the panel
// is never empty.

use crate::theme::{self, BadgeKind, COLORS};
use ratatui::{
    layout::Rect,
    style::{Modifier, Style},
    text::{Line, Span},
    widgets::Paragraph,
    Frame,
};
use std::collections::{HashMap, HashSet};
use wrkflw_parser::workflow::WorkflowDefinition;

/// Status of a job *as it appears live* — synthesised from `WorkflowExecution`.
#[derive(Copy, Clone, Debug, PartialEq, Eq)]
pub enum NodeState {
    Success,
    Failure,
    Skipped,
    Running,
    Pending,
}

fn state_color(s: NodeState) -> ratatui::style::Color {
    match s {
        NodeState::Success => COLORS.success,
        NodeState::Failure => COLORS.error,
        NodeState::Skipped => COLORS.warning,
        NodeState::Running => COLORS.info,
        NodeState::Pending => COLORS.text_muted,
    }
}

fn state_glyph(s: NodeState, spinner_frame: usize) -> &'static str {
    match s {
        NodeState::Success => theme::symbols::SUCCESS,
        NodeState::Failure => theme::symbols::FAILURE,
        NodeState::Skipped => theme::symbols::SKIPPED,
        NodeState::Running => theme::spinner(spinner_frame),
        NodeState::Pending => theme::symbols::NOT_STARTED,
    }
}

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

/// Render the mini DAG into `area`. `state_of` resolves a job name to its
/// current node state.
pub fn render<F: Fn(&str) -> NodeState>(
    frame: &mut Frame<'_>,
    area: Rect,
    def: Option<&WorkflowDefinition>,
    state_of: F,
    spinner_frame: usize,
) {
    let mut lines: Vec<Line> = Vec::new();

    let levels: Vec<Vec<String>> = match def {
        Some(d) => topo_levels(d),
        None => Vec::new(),
    };

    if levels.is_empty() {
        lines.push(Line::from(Span::styled(
            "no parsed workflow",
            Style::default().fg(COLORS.text_muted),
        )));
        frame.render_widget(Paragraph::new(lines), area);
        return;
    }

    // Render one row per (column, name) up to a few — narrow text panel.
    // Format:  L1: setup   ✓
    //          L2: fmt     ✓
    //              clippy  ✓
    //          L3: build   ⠋  ◀
    let stage_labels = ["setup", "lint", "build", "test/docs", "publish"];
    for (li, layer) in levels.iter().enumerate() {
        // Stage header
        let stage = stage_labels.get(li).copied().unwrap_or("stage");
        lines.push(Line::from(vec![
            Span::styled(
                format!("L{} ", li + 1),
                Style::default().fg(COLORS.text_muted),
            ),
            Span::styled(
                stage.to_string(),
                Style::default()
                    .fg(COLORS.highlight)
                    .add_modifier(Modifier::BOLD),
            ),
        ]));
        for name in layer {
            let st = state_of(name);
            let color = state_color(st);
            let glyph = state_glyph(st, spinner_frame);
            let active_marker = if matches!(st, NodeState::Running) {
                "  ◀"
            } else {
                ""
            };
            lines.push(Line::from(vec![
                Span::raw("  "),
                Span::styled(glyph.to_string(), Style::default().fg(color)),
                Span::raw(" "),
                Span::styled(
                    truncate(name, 14),
                    Style::default().fg(if matches!(st, NodeState::Running) {
                        COLORS.text
                    } else {
                        color
                    }),
                ),
                Span::styled(active_marker.to_string(), Style::default().fg(COLORS.info)),
            ]));
        }
    }

    // Tiny legend strip (matches the design's "active critical path" idea).
    lines.push(Line::from(vec![Span::styled(
        " ",
        Style::default().fg(COLORS.text_muted),
    )]));
    lines.push(Line::from(vec![
        theme::badge_outline("running", BadgeKind::Info),
        Span::raw(" "),
        theme::badge_outline("done", BadgeKind::Success),
        Span::raw(" "),
        theme::badge_outline("pending", BadgeKind::Dim),
    ]));

    frame.render_widget(Paragraph::new(lines), area);
}

fn truncate(s: &str, n: usize) -> String {
    if s.chars().count() <= n {
        s.to_string()
    } else {
        let mut out: String = s.chars().take(n.saturating_sub(1)).collect();
        out.push('…');
        out
    }
}
