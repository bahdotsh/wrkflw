// DAG full view — screen 4 from the design.
//
// Two modes behind a single tab (toggled with `g`):
//
//   - Graph: jobs laid out in topological columns, each column prefixed
//     with a stage label. Edges are drawn on the left gutter of each
//     column so a user can see `needs:` at a glance without us having
//     to pretend we're an SVG canvas.
//   - List: topological stages as headers with jobs listed under each —
//     the same data as the design's `TopoList`. Denser, read-easier on
//     narrow terminals.
//
// The workflow shown is the one currently focused in the Workflows
// tab. This deliberately mirrors the design (no workflow picker on
// this screen) — the Workflows tab is the selector.

use crate::app::App;
use crate::components::dag::{self, NodeState};
use crate::models::WorkflowStatus;
use crate::theme::{self, BadgeKind, COLORS};
use ratatui::{
    layout::{Alignment, Constraint, Direction, Layout, Rect},
    style::{Modifier, Style},
    text::{Line, Span},
    widgets::{Paragraph, Wrap},
    Frame,
};
use wrkflw_executor::JobStatus;
use wrkflw_parser::workflow::WorkflowDefinition;

pub fn render_dag_tab(f: &mut Frame<'_>, app: &App, area: Rect) {
    let Some(idx) = app.workflow_list_state.selected() else {
        render_empty_state(
            f,
            area,
            "No workflow selected — pick one on the Workflows tab.",
        );
        return;
    };
    let Some(workflow) = app.workflows.get(idx) else {
        render_empty_state(f, area, "Workflow selection out of range.");
        return;
    };
    let Some(def) = workflow.definition.as_ref() else {
        render_empty_state(
            f,
            area,
            &format!("Couldn't parse {} — DAG unavailable.", workflow.name),
        );
        return;
    };

    let outer = Layout::default()
        .direction(Direction::Vertical)
        .constraints([Constraint::Length(2), Constraint::Min(0)])
        .split(area);

    render_header(f, app, workflow, outer[0]);

    let body = Layout::default()
        .direction(Direction::Horizontal)
        .constraints([Constraint::Min(0), Constraint::Length(30)])
        .split(outer[1]);

    if app.dag_list_view {
        render_topo_list(f, app, def, workflow, idx, body[0]);
    } else {
        render_graph(f, app, def, workflow, idx, body[0]);
    }
    render_legend(f, app, body[1]);
}

fn render_empty_state(f: &mut Frame<'_>, area: Rect, msg: &str) {
    let block = theme::block("DAG");
    let inner = block.inner(area);
    f.render_widget(block, area);
    f.render_widget(
        Paragraph::new(msg).style(Style::default().fg(COLORS.text_muted)),
        inner,
    );
}

fn render_header(f: &mut Frame<'_>, app: &App, workflow: &crate::models::Workflow, area: Rect) {
    let view_label = if app.dag_list_view { "list" } else { "graph" };
    let spans = vec![
        Span::styled(
            workflow.name.clone(),
            Style::default()
                .fg(COLORS.text)
                .add_modifier(Modifier::BOLD),
        ),
        Span::styled(
            "  ·  dependency graph  ·  ",
            Style::default().fg(COLORS.text_muted),
        ),
        theme::badge_outline(view_label, BadgeKind::Info),
        Span::raw("  "),
        Span::styled(
            "press `g` to toggle",
            Style::default().fg(COLORS.text_muted),
        ),
    ];
    f.render_widget(
        Paragraph::new(Line::from(spans)).alignment(Alignment::Left),
        area,
    );
}

/// State lookup for every named job — consults the current
/// `WorkflowExecution` so a running nightly shows `build` as
/// `Running`, mirroring the design's live DAG. `workflow_idx` is the
/// position of this workflow in `app.workflows`; it's threaded from
/// the caller so we don't have to reach for pointer identity here.
fn state_for_job(
    app: &App,
    workflow: &crate::models::Workflow,
    workflow_idx: usize,
    name: &str,
) -> NodeState {
    if !matches!(
        workflow.status,
        WorkflowStatus::Running | WorkflowStatus::Success | WorkflowStatus::Failed
    ) {
        return NodeState::Pending;
    }
    let Some(exec) = workflow.execution_details.as_ref() else {
        return NodeState::Pending;
    };
    match exec.jobs.iter().find(|j| j.name == name) {
        Some(j) => match j.status {
            JobStatus::Success => NodeState::Success,
            JobStatus::Failure => NodeState::Failure,
            JobStatus::Skipped => NodeState::Skipped,
        },
        None => {
            if app.current_execution == Some(workflow_idx) {
                NodeState::Running
            } else {
                NodeState::Pending
            }
        }
    }
}

fn render_graph(
    f: &mut Frame<'_>,
    app: &App,
    def: &WorkflowDefinition,
    workflow: &crate::models::Workflow,
    workflow_idx: usize,
    area: Rect,
) {
    let block = theme::block_focused("DAG · topological columns");
    let inner = block.inner(area);
    f.render_widget(block, area);

    let levels = dag::topo_levels(def);
    if levels.is_empty() {
        f.render_widget(
            Paragraph::new("no jobs").style(Style::default().fg(COLORS.text_muted)),
            inner,
        );
        return;
    }

    // Node cards are a fixed 18 cells wide ("╭────────────────╮").
    // If we just split the area into `levels.len()` equal columns, each
    // column can shrink below 18 on narrow terminals and the box-drawing
    // characters wrap — which looks broken. Instead, clamp the visible
    // column count to what fits, and render a trailing "… +N more
    // stages" marker so the user knows they're seeing a subset.
    const NODE_CARD_W: u16 = 18;
    const OVERFLOW_W: u16 = 16; // width reserved for "… +N more" column
    let total_stages = levels.len();
    let max_visible = (inner.width / NODE_CARD_W).max(1) as usize;
    let (visible_stages, truncated) = if total_stages > max_visible {
        // Reserve space for the overflow column by dropping one more
        // stage; ensures the tail marker has somewhere to live.
        let capped = max_visible.saturating_sub(1).max(1);
        (capped, total_stages - capped)
    } else {
        (total_stages, 0)
    };

    let mut constraints: Vec<Constraint> = (0..visible_stages)
        .map(|_| Constraint::Length(NODE_CARD_W))
        .collect();
    if truncated > 0 {
        constraints.push(Constraint::Length(OVERFLOW_W));
    }
    constraints.push(Constraint::Min(0)); // trailing slack

    let cols = Layout::default()
        .direction(Direction::Horizontal)
        .constraints(constraints)
        .split(inner);

    // Column labels read straight off the topology ("Stage N"). The
    // design handoff uses semantic names like "build" / "test", but
    // no such grouping exists in GitHub Actions' workflow YAML today,
    // so putting a name here would mean *inventing* one.
    for (li, layer) in levels.iter().take(visible_stages).enumerate() {
        let col = cols[li];
        let mut lines: Vec<Line> = Vec::new();
        lines.push(Line::from(vec![Span::styled(
            format!("Stage {}", li + 1),
            Style::default()
                .fg(COLORS.highlight)
                .add_modifier(Modifier::BOLD),
        )]));
        lines.push(Line::from(""));
        for name in layer {
            let st = state_for_job(app, workflow, workflow_idx, name);
            let color = match st {
                NodeState::Success => COLORS.success,
                NodeState::Failure => COLORS.error,
                NodeState::Skipped => COLORS.warning,
                NodeState::Running => COLORS.info,
                NodeState::Pending => COLORS.text_muted,
            };
            let glyph = match st {
                NodeState::Success => theme::symbols::SUCCESS,
                NodeState::Failure => theme::symbols::FAILURE,
                NodeState::Skipped => theme::symbols::SKIPPED,
                NodeState::Running => theme::spinner(app.spinner_frame),
                NodeState::Pending => theme::symbols::NOT_STARTED,
            };
            // Node card: two-line "┌─ name ─┐" style in plain text.
            lines.push(Line::from(vec![Span::styled(
                "╭────────────────╮",
                Style::default().fg(color),
            )]));
            // Distinct name from the outer `truncated` stage-count
            // binding so the shadow doesn't mislead a future reader.
            let short_name = truncate(name, 12);
            let padding = 12usize.saturating_sub(short_name.chars().count());
            lines.push(Line::from(vec![
                Span::styled("│ ", Style::default().fg(color)),
                Span::styled(glyph.to_string(), Style::default().fg(color)),
                Span::raw(" "),
                Span::styled(
                    short_name,
                    Style::default()
                        .fg(if matches!(st, NodeState::Running) {
                            COLORS.text
                        } else {
                            color
                        })
                        .add_modifier(if matches!(st, NodeState::Running) {
                            Modifier::BOLD
                        } else {
                            Modifier::empty()
                        }),
                ),
                Span::styled(" ".repeat(padding), Style::default().fg(color)),
                Span::styled(" │", Style::default().fg(color)),
            ]));
            // Matrix badge for nodes that carry a strategy.matrix.
            let matrix_axes = def
                .jobs
                .get(name)
                .and_then(|j| j.matrix_config())
                .map(|m| m.parameters.len())
                .unwrap_or(0);
            if matrix_axes > 0 {
                lines.push(Line::from(vec![
                    Span::styled("│ ", Style::default().fg(color)),
                    theme::badge_outline(format!("matrix×{}", matrix_axes), BadgeKind::Info),
                    Span::styled("         │", Style::default().fg(color)),
                ]));
            }
            lines.push(Line::from(vec![Span::styled(
                "╰────────────────╯",
                Style::default().fg(color),
            )]));
        }
        f.render_widget(Paragraph::new(lines).wrap(Wrap { trim: false }), col);
    }

    if truncated > 0 {
        let overflow_col = cols[visible_stages];
        let mut overflow_lines: Vec<Line> = Vec::new();
        overflow_lines.push(Line::from(vec![Span::styled(
            format!("+{}", truncated),
            Style::default()
                .fg(COLORS.text_muted)
                .add_modifier(Modifier::BOLD),
        )]));
        overflow_lines.push(Line::from(Span::styled(
            "more stages",
            Style::default().fg(COLORS.text_dim),
        )));
        overflow_lines.push(Line::from(""));
        overflow_lines.push(Line::from(Span::styled(
            "press `g`",
            Style::default().fg(COLORS.text_dim),
        )));
        overflow_lines.push(Line::from(Span::styled(
            "for list view",
            Style::default().fg(COLORS.text_dim),
        )));
        f.render_widget(
            Paragraph::new(overflow_lines).wrap(Wrap { trim: false }),
            overflow_col,
        );
    }
}

fn render_topo_list(
    f: &mut Frame<'_>,
    app: &App,
    def: &WorkflowDefinition,
    workflow: &crate::models::Workflow,
    workflow_idx: usize,
    area: Rect,
) {
    let block = theme::block_focused("Jobs · topological order");
    let inner = block.inner(area);
    f.render_widget(block, area);

    let levels = dag::topo_levels(def);
    let mut lines: Vec<Line> = Vec::new();

    for (li, layer) in levels.iter().enumerate() {
        lines.push(Line::from(vec![Span::styled(
            format!(" Stage {}", li + 1),
            Style::default()
                .fg(COLORS.highlight)
                .add_modifier(Modifier::BOLD),
        )]));
        for name in layer {
            let st = state_for_job(app, workflow, workflow_idx, name);
            let (glyph, style) = match st {
                NodeState::Success => {
                    (theme::symbols::SUCCESS, Style::default().fg(COLORS.success))
                }
                NodeState::Failure => (theme::symbols::FAILURE, Style::default().fg(COLORS.error)),
                NodeState::Skipped => {
                    (theme::symbols::SKIPPED, Style::default().fg(COLORS.warning))
                }
                NodeState::Running => (
                    theme::spinner(app.spinner_frame),
                    Style::default().fg(COLORS.info),
                ),
                NodeState::Pending => (
                    theme::symbols::NOT_STARTED,
                    Style::default().fg(COLORS.text_muted),
                ),
            };
            let needs: String = def
                .jobs
                .get(name)
                .and_then(|j| j.needs.as_ref())
                .map(|n| {
                    if n.is_empty() {
                        "—".to_string()
                    } else {
                        n.join(", ")
                    }
                })
                .unwrap_or_else(|| "—".to_string());
            let matrix_badge = def
                .jobs
                .get(name)
                .and_then(|j| j.matrix_config())
                .map(|m| format!(" matrix×{}", m.parameters.len()))
                .unwrap_or_default();
            lines.push(Line::from(vec![
                Span::raw("   "),
                Span::styled(glyph.to_string(), style),
                Span::raw(" "),
                Span::styled(
                    name.clone(),
                    Style::default()
                        .fg(COLORS.text)
                        .add_modifier(Modifier::BOLD),
                ),
                Span::styled(matrix_badge, Style::default().fg(COLORS.info)),
                Span::styled("  needs: ", Style::default().fg(COLORS.text_muted)),
                Span::styled(needs, Style::default().fg(COLORS.text_dim)),
            ]));
        }
        lines.push(Line::from(""));
    }
    f.render_widget(Paragraph::new(lines).wrap(Wrap { trim: false }), inner);
}

fn render_legend(f: &mut Frame<'_>, app: &App, area: Rect) {
    let block = theme::block("Legend");
    let inner = block.inner(area);
    f.render_widget(block, area);

    let lines = vec![
        Line::from(vec![
            Span::styled(theme::symbols::SUCCESS, Style::default().fg(COLORS.success)),
            Span::raw("  success"),
        ]),
        Line::from(vec![
            Span::styled(
                theme::spinner(app.spinner_frame),
                Style::default().fg(COLORS.info),
            ),
            Span::raw("  running"),
        ]),
        Line::from(vec![
            Span::styled(
                theme::symbols::NOT_STARTED,
                Style::default().fg(COLORS.text_muted),
            ),
            Span::raw("  pending"),
        ]),
        Line::from(vec![
            Span::styled(theme::symbols::SKIPPED, Style::default().fg(COLORS.warning)),
            Span::raw("  skipped"),
        ]),
        Line::from(vec![
            Span::styled(theme::symbols::FAILURE, Style::default().fg(COLORS.error)),
            Span::raw("  failed"),
        ]),
        Line::from(""),
        Line::from(vec![Span::styled(
            "SHORTCUTS",
            Style::default()
                .fg(COLORS.highlight)
                .add_modifier(Modifier::BOLD),
        )]),
        Line::from(vec![
            theme::key_chip("g"),
            Span::raw(" "),
            Span::styled("toggle graph/list", Style::default().fg(COLORS.text_dim)),
        ]),
        Line::from(vec![
            theme::key_chip("enter"),
            Span::raw(" "),
            Span::styled("open Execution", Style::default().fg(COLORS.text_dim)),
        ]),
    ];
    f.render_widget(Paragraph::new(lines), inner);
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
