// DAG tab — the selected workflow's jobs as stage columns of status
// cards (or an indented list with `g`). ←/→ scroll when the stages don't
// all fit, ↑/↓ switch workflow, Enter opens the run on Execution.

use crate::app::App;
use crate::components::dag::{self, matrix_size_label};
use crate::models::{JobState, Workflow};
use crate::text;
use crate::theme::{self, COLORS};
use ratatui::{
    layout::{Constraint, Direction, Layout, Rect},
    style::{Modifier, Style},
    text::{Line, Span},
    widgets::Paragraph,
    Frame,
};
use std::collections::{HashMap, HashSet};
use std::sync::Arc;
use wrkflw_parser::workflow::WorkflowDefinition;

/// Job name → matrix run count label, e.g. `6` or `?`.
type MatrixLabels = HashMap<String, String>;

const MIN_CARD_W: u16 = 16;
const MAX_CARD_W: u16 = 28;
const GAP: u16 = 3;
const CARD_H: u16 = 3;

pub fn render_dag_tab(f: &mut Frame<'_>, app: &mut App, area: Rect) {
    let legend_w = if area.width >= 96 { 24 } else { 0 };
    let body = Layout::default()
        .direction(Direction::Horizontal)
        .constraints([Constraint::Min(0), Constraint::Length(legend_w)])
        .split(area);
    if legend_w > 0 {
        render_legend(f, app, body[1]);
    }

    // The run's workflow while one runs, as the Jobs and Log views show;
    // otherwise ↑↓ browse every workflow's graph.
    let workflow = app.shown_workflow().and_then(|i| app.workflows.get(i));
    let Some(workflow) = workflow else {
        let block = theme::block_focused("Graph");
        let inner = block.inner(body[0]);
        f.render_widget(block, body[0]);
        theme::empty_state(f, inner, "No workflow selected", vec![]);
        return;
    };
    let Some(def) = workflow.definition.clone() else {
        let block = theme::block_focused("Graph");
        let inner = block.inner(body[0]);
        f.render_widget(block, body[0]);
        theme::empty_state(
            f,
            inner,
            "Couldn't parse this workflow",
            vec![Span::styled(
                "Validate it from the Workflows tab to see why",
                theme::muted_style(),
            )],
        );
        return;
    };

    // Expanding a matrix materialises every combination; do it once per
    // parsed workflow, not for every card on every frame.
    let fresh = matches!(&app.dag_matrix_labels, Some((d, _)) if Arc::ptr_eq(d, &def));
    if !fresh {
        let labels: MatrixLabels = def
            .jobs
            .iter()
            .filter_map(|(name, job)| {
                let label = job.matrix_config().and_then(matrix_size_label)?;
                Some((name.clone(), label))
            })
            .collect();
        app.dag_matrix_labels = Some((Arc::clone(&def), labels));
    }
    let labels = app
        .dag_matrix_labels
        .as_ref()
        .map(|(_, l)| l)
        .expect("labels were just filled");

    let levels = dag::topo_levels(&def);
    let scroll = if app.dag_list_view {
        render_list(f, app, workflow, &def, labels, &levels, body[0])
    } else {
        render_graph(f, app, workflow, &def, labels, &levels, body[0])
    };
    app.dag_scroll = scroll;
}

/// How long each finished job took, and the jobs that set the run's
/// length: the slowest of each stage.
struct Timing {
    durations: HashMap<String, u64>,
    critical: HashSet<String>,
    /// The run has ended, so every job that will report a time has.
    settled: bool,
    /// Jobs with a leg still running, whose time may yet grow.
    unfinished: HashSet<String>,
}

impl Timing {
    fn of(workflow: &Workflow, levels: &[Vec<String>]) -> Self {
        let exec = workflow.execution_details.as_ref();
        let durations = exec.map(|e| e.job_durations_ms()).unwrap_or_default();
        let settled = exec.is_some_and(|e| e.end_time.is_some());
        let unfinished = exec
            .map(|e| e.unfinished_jobs(&workflow.all_matrix_legs()))
            .unwrap_or_default();
        // Mid-run, unfinished jobs would count as taking no time, and the
        // path would move as they finish; it is only drawn once it's known.
        let critical = if settled {
            critical_path(levels, &durations)
        } else {
            HashSet::new()
        };
        Timing {
            durations,
            critical,
            settled,
            unfinished,
        }
    }

    /// Wall time of a stage: its jobs run side by side, so the slowest.
    /// While the run goes on, only once every job in the stage has finished,
    /// every matrix leg included.
    fn stage_ms(&self, layer: &[String]) -> Option<u64> {
        let times: Vec<u64> = layer
            .iter()
            .filter_map(|j| self.durations.get(j).copied())
            .collect();
        let pending =
            times.len() < layer.len() || layer.iter().any(|j| self.unfinished.contains(j));
        if !self.settled && pending {
            return None;
        }
        times.into_iter().max()
    }
}

/// The jobs that set the run's length: the slowest of each stage. Stages
/// run one after another and the jobs in a stage side by side, as the
/// executor schedules them, so a run takes the sum of these jobs' times.
/// Empty until the run has timings, and for single-stage workflows, where
/// the slowest job is already plain to see.
fn critical_path(levels: &[Vec<String>], durations: &HashMap<String, u64>) -> HashSet<String> {
    if durations.is_empty() || levels.len() < 2 {
        return HashSet::new();
    }
    levels
        .iter()
        .filter_map(|layer| {
            layer
                .iter()
                .filter_map(|job| durations.get(job).map(|ms| (*ms, job)))
                // Ties go to the first name, so the path doesn't flicker.
                .max_by_key(|(ms, job)| (*ms, std::cmp::Reverse(job.as_str())))
                .map(|(_, job)| job.clone())
        })
        .collect()
}

fn matrix_suffix(labels: &MatrixLabels, job: &str) -> String {
    labels
        .get(job)
        .map(|n| format!(" ×{n}"))
        .unwrap_or_default()
}

fn state_color(state: JobState) -> ratatui::style::Color {
    match state {
        JobState::Success => COLORS.success,
        JobState::Failure => COLORS.error,
        JobState::FailureAllowed => COLORS.warning,
        JobState::Running => COLORS.info,
        JobState::Skipped | JobState::Pending => COLORS.text_muted,
    }
}

/// Draws the stage columns from `app.dag_scroll`, returning the scroll
/// clamped to the stages that exist.
fn render_graph(
    f: &mut Frame<'_>,
    app: &App,
    workflow: &Workflow,
    def: &WorkflowDefinition,
    labels: &MatrixLabels,
    levels: &[Vec<String>],
    area: Rect,
) -> usize {
    // The Run header names the workflow.
    let title = "Graph";
    let block = theme::block_focused(title);
    let inner = block.inner(area);

    if levels.is_empty() {
        f.render_widget(block, area);
        theme::empty_state(f, inner, "No jobs", vec![]);
        return 0;
    }

    let fit = ((inner.width + GAP) / (MIN_CARD_W + GAP)).max(1) as usize;
    let visible = fit.min(levels.len());
    let first = app.dag_scroll.min(levels.len() - visible);
    let card_w = ((inner.width + GAP) / visible as u16)
        .saturating_sub(GAP)
        .clamp(MIN_CARD_W.min(inner.width), MAX_CARD_W);

    let mut badge = format!("{} stages", levels.len());
    if visible < levels.len() {
        badge = format!(
            "{}{}–{} of {}{}",
            if first > 0 { "‹ " } else { "" },
            first + 1,
            first + visible,
            levels.len(),
            if first + visible < levels.len() {
                " ›"
            } else {
                ""
            },
        );
    }
    f.render_widget(theme::with_badge(block, badge), area);

    let timing = Timing::of(workflow, levels);
    let max_cards = (inner.height.saturating_sub(2) / CARD_H).max(1) as usize;
    for (col, (li, layer)) in levels
        .iter()
        .enumerate()
        .skip(first)
        .take(visible)
        .enumerate()
    {
        let x = inner.x + col as u16 * (card_w + GAP);
        if x + card_w > inner.right() {
            break;
        }
        let head = Rect::new(x, inner.y, card_w, 1);
        f.render_widget(
            Paragraph::new(Line::from(vec![
                Span::styled(
                    format!("Stage {}", li + 1),
                    Style::default()
                        .fg(COLORS.text_dim)
                        .add_modifier(Modifier::BOLD),
                ),
                Span::styled(format!("  {}", layer.len()), theme::muted_style()),
                Span::styled(
                    timing
                        .stage_ms(layer)
                        .map(|ms| format!(" · {}", crate::views::execution_tab::format_ms(ms)))
                        .unwrap_or_default(),
                    Style::default().fg(COLORS.text_dim),
                ),
            ])),
            head,
        );

        let overflow = layer.len() > max_cards;
        let shown = if overflow {
            max_cards.saturating_sub(1).max(1)
        } else {
            layer.len()
        };
        for (row, name) in layer.iter().take(shown).enumerate() {
            let y = inner.y + 2 + row as u16 * CARD_H;
            if y + CARD_H > inner.bottom() {
                break;
            }
            let card = Rect::new(x, y, card_w, CARD_H);
            render_card(f, app, workflow, labels, &timing, name, card);

            let has_needs = def
                .jobs
                .get(name)
                .and_then(|j| j.needs.as_ref())
                .is_some_and(|n| !n.is_empty());
            if has_needs && col > 0 {
                f.render_widget(
                    Paragraph::new(Span::styled("─▶ ", theme::muted_style())),
                    Rect::new(x - GAP, y + 1, GAP, 1),
                );
            }
        }
        if overflow {
            let y = inner.y + 2 + shown as u16 * CARD_H;
            if y < inner.bottom() {
                f.render_widget(
                    Paragraph::new(Span::styled(
                        format!("  +{} more", layer.len() - shown),
                        theme::muted_style(),
                    )),
                    Rect::new(x, y, card_w, 1),
                );
            }
        }
    }
    first
}

fn render_card(
    f: &mut Frame<'_>,
    app: &App,
    workflow: &Workflow,
    labels: &MatrixLabels,
    timing: &Timing,
    name: &str,
    area: Rect,
) {
    let state = workflow.job_state(name);
    let color = state_color(state);
    let (glyph, glyph_style) = theme::job_state(state, app.spinner_frame);
    let matrix = matrix_suffix(labels, name);

    // The critical path gets a heavy border; the colour still says how
    // the job went.
    let block = ratatui::widgets::Block::bordered()
        .border_type(if timing.critical.contains(name) {
            ratatui::widgets::BorderType::Thick
        } else {
            ratatui::widgets::BorderType::Rounded
        })
        .border_style(Style::default().fg(if state == JobState::Pending {
            COLORS.border
        } else {
            color
        }));
    let inner = block.inner(area);
    let took = timing
        .durations
        .get(name)
        .map(|ms| crate::views::execution_tab::format_ms(*ms))
        .unwrap_or_default();
    let room = (inner.width as usize).saturating_sub(3 + matrix.len());
    // Show the time only when the name still has room to be read.
    // The time keeps a cell of margin from the border, like the glyph.
    let (room, took) = if !took.is_empty() && room >= text::width(&took) + 9 {
        (room - text::width(&took) - 2, format!("{took} "))
    } else {
        (room, String::new())
    };
    let shown_name = text::truncate(&text::sanitize(name), room).into_owned();
    // Right-align the time: pad what the name left of its room.
    let pad = if took.is_empty() {
        0
    } else {
        room.saturating_sub(text::width(&shown_name)) + 1
    };
    let name_style = match state {
        JobState::Pending | JobState::Skipped => theme::muted_style(),
        _ => Style::default()
            .fg(COLORS.text)
            .add_modifier(Modifier::BOLD),
    };
    f.render_widget(
        Paragraph::new(Line::from(vec![
            Span::raw(" "),
            Span::styled(glyph, glyph_style),
            Span::raw(" "),
            Span::styled(shown_name.clone(), name_style),
            Span::styled(matrix, Style::default().fg(COLORS.info)),
            Span::raw(" ".repeat(pad)),
            Span::styled(took, theme::muted_style()),
        ]))
        .block(block),
        area,
    );
}

/// Draws the indented list from line `app.dag_scroll`, returning the
/// clamped scroll.
fn render_list(
    f: &mut Frame<'_>,
    app: &App,
    workflow: &Workflow,
    def: &WorkflowDefinition,
    labels: &MatrixLabels,
    levels: &[Vec<String>],
    area: Rect,
) -> usize {
    // The Run header names the workflow.
    let title = "Graph";
    let block = theme::with_badge(theme::block_focused(title), "list");
    let inner = block.inner(area);
    f.render_widget(block, area);
    let w = inner.width as usize;

    let mut lines: Vec<Line> = Vec::new();
    for (li, layer) in levels.iter().enumerate() {
        if li > 0 {
            lines.push(Line::from(""));
        }
        lines.push(theme::section(format!("Stage {}", li + 1)));
        for name in layer {
            let state = workflow.job_state(name);
            let (glyph, style) = theme::job_state(state, app.spinner_frame);
            let job = def.jobs.get(name);
            let needs = job
                .and_then(|j| j.needs.as_ref())
                .filter(|n| !n.is_empty())
                .map(|n| format!("  ← {}", n.join(", ")))
                .unwrap_or_default();
            let matrix = matrix_suffix(labels, name);
            let name = text::sanitize(name).into_owned();
            let mut spans = vec![
                Span::styled(glyph, style),
                Span::raw(" "),
                Span::styled(
                    name.clone(),
                    Style::default()
                        .fg(COLORS.text)
                        .add_modifier(Modifier::BOLD),
                ),
                Span::styled(matrix.clone(), Style::default().fg(COLORS.info)),
            ];
            let used = 2 + text::width(&name) + text::width(&matrix);
            spans.push(Span::styled(
                text::truncate(&needs, w.saturating_sub(used)).into_owned(),
                theme::muted_style(),
            ));
            lines.push(Line::from(spans));
        }
    }

    let viewport = inner.height as usize;
    let total = lines.len();
    let scroll = app.dag_scroll.min(total.saturating_sub(viewport));
    f.render_widget(Paragraph::new(lines).scroll((scroll as u16, 0)), inner);
    theme::scrollbar(f, area, total, scroll, viewport);
    scroll
}

fn render_legend(f: &mut Frame<'_>, app: &App, area: Rect) {
    let block = theme::block("Legend");
    let inner = block.inner(area);
    f.render_widget(block, area);

    let mut lines: Vec<Line> = [
        JobState::Success,
        JobState::Failure,
        JobState::FailureAllowed,
        JobState::Running,
        JobState::Skipped,
        JobState::Pending,
    ]
    .into_iter()
    .map(|state| {
        let (glyph, style) = theme::job_state(state, app.spinner_frame);
        let label = match state {
            JobState::Success => "passed",
            JobState::Failure => "failed",
            JobState::FailureAllowed => "failed, allowed",
            JobState::Running => "running",
            JobState::Skipped => "skipped",
            JobState::Pending => "not run",
        };
        Line::from(vec![
            Span::styled(glyph, style),
            Span::styled(format!("  {label}"), Style::default().fg(COLORS.text_dim)),
        ])
    })
    .collect();
    lines.push(Line::from(vec![
        Span::styled("×6", Style::default().fg(COLORS.info)),
        Span::styled(" matrix runs", Style::default().fg(COLORS.text_dim)),
    ]));
    lines.push(Line::from(vec![
        Span::styled("┃", Style::default().fg(COLORS.text_dim)),
        Span::styled("  critical path", Style::default().fg(COLORS.text_dim)),
    ]));
    f.render_widget(Paragraph::new(lines), inner);
}

#[cfg(test)]
mod timing_tests {
    use super::*;

    fn def(yaml: &str) -> WorkflowDefinition {
        serde_yaml::from_str(yaml).expect("test workflow parses")
    }

    #[test]
    fn critical_path_follows_the_slowest_chain() {
        let def = def(
            "name: d\non: push\njobs:\n  checkout:\n    runs-on: x\n    steps: [{run: a}]\n  lint:\n    runs-on: x\n    needs: checkout\n    steps: [{run: a}]\n  test:\n    runs-on: x\n    needs: checkout\n    steps: [{run: a}]\n  package:\n    runs-on: x\n    needs: [lint, test]\n    steps: [{run: a}]\n",
        );
        let levels = dag::topo_levels(&def);
        let durations: HashMap<String, u64> = [
            ("checkout", 1_000),
            ("lint", 2_000),
            ("test", 9_000),
            ("package", 500),
        ]
        .into_iter()
        .map(|(k, v)| (k.to_string(), v))
        .collect();
        let path = critical_path(&levels, &durations);
        let expected: HashSet<String> = ["checkout", "test", "package"]
            .into_iter()
            .map(String::from)
            .collect();
        assert_eq!(path, expected);
        assert!(critical_path(&levels, &HashMap::new()).is_empty());
    }

    #[test]
    fn critical_path_is_the_slowest_job_of_each_stage() {
        // Stage times add up, so the run takes a (10s) + d (10s), although
        // neither dependency chain, a → c or b → d, is longer than 11s.
        let def = def(
            "name: d\non: push\njobs:\n  a:\n    runs-on: x\n    steps: [{run: a}]\n  b:\n    runs-on: x\n    steps: [{run: a}]\n  c:\n    runs-on: x\n    needs: a\n    steps: [{run: a}]\n  d:\n    runs-on: x\n    needs: b\n    steps: [{run: a}]\n",
        );
        let levels = dag::topo_levels(&def);
        let durations: HashMap<String, u64> =
            [("a", 10_000), ("b", 1_000), ("c", 1_000), ("d", 10_000)]
                .into_iter()
                .map(|(k, v)| (k.to_string(), v))
                .collect();
        let expected: HashSet<String> = ["a", "d"].into_iter().map(String::from).collect();
        assert_eq!(critical_path(&levels, &durations), expected);
    }

    #[test]
    fn equal_chains_pick_the_same_path_every_time() {
        let def = def(
            "name: d\non: push\njobs:\n  a:\n    runs-on: x\n    steps: [{run: a}]\n  b:\n    runs-on: x\n    steps: [{run: a}]\n  c:\n    runs-on: x\n    needs: a\n    steps: [{run: a}]\n  d:\n    runs-on: x\n    needs: b\n    steps: [{run: a}]\n",
        );
        let levels = dag::topo_levels(&def);
        let durations: HashMap<String, u64> = ["a", "b", "c", "d"]
            .into_iter()
            .map(|k| (k.to_string(), 1_000))
            .collect();
        let expected: HashSet<String> = ["a", "c"].into_iter().map(String::from).collect();
        for _ in 0..20 {
            assert_eq!(critical_path(&levels, &durations), expected);
        }
    }

    #[test]
    fn stage_times_wait_for_the_stage_while_the_run_goes_on() {
        let layer = vec!["lint".to_string(), "test".to_string()];
        let mut timing = Timing {
            durations: HashMap::from([("lint".to_string(), 2_000)]),
            critical: HashSet::new(),
            settled: false,
            unfinished: HashSet::new(),
        };
        assert_eq!(timing.stage_ms(&layer), None);
        timing.durations.insert("test".to_string(), 9_000);
        assert_eq!(timing.stage_ms(&layer), Some(9_000));
        // Once the run has ended, a job that never reported doesn't hide it.
        timing.durations.remove("test");
        timing.settled = true;
        assert_eq!(timing.stage_ms(&layer), Some(2_000));
    }

    #[test]
    fn a_matrix_stage_waits_for_its_last_leg() {
        use crate::models::TimelineEntry as T;
        use wrkflw_executor::JobStatus;
        let started = |job: &str| T::JobStarted {
            job: job.into(),
            canonical_name: "build".into(),
        };
        let finished = |job: &str, ms| T::JobFinished {
            job: job.into(),
            status: JobStatus::Success,
            failure_allowed: false,
            duration_ms: ms,
            repeated_from: None,
        };
        let mut exec = crate::models::WorkflowExecution {
            jobs: Vec::new(),
            start_time: chrono::Local::now(),
            end_time: None,
            progress: 0.0,
            timeline: vec![started("build (os: a)"), started("build (os: b)")],
            live: Default::default(),
        };
        let layer = vec!["build".to_string()];
        let timing = |exec: &crate::models::WorkflowExecution| Timing {
            durations: exec.job_durations_ms(),
            critical: HashSet::new(),
            settled: false,
            unfinished: exec.unfinished_jobs(&HashMap::new()),
        };

        exec.timeline.push(finished("build (os: a)", 900));
        assert_eq!(timing(&exec).stage_ms(&layer), None, "one leg still runs");

        exec.timeline.push(finished("build (os: b)", 1_400));
        assert_eq!(timing(&exec).stage_ms(&layer), Some(1_400));

        // With `max-parallel: 2`, a four-leg matrix runs in two batches;
        // between them every started leg has ended, but two haven't begun.
        let legs = HashMap::from([("build".to_string(), 4)]);
        let batched = Timing {
            durations: exec.job_durations_ms(),
            critical: HashSet::new(),
            settled: false,
            unfinished: exec.unfinished_jobs(&legs),
        };
        assert_eq!(
            batched.stage_ms(&layer),
            None,
            "the next batch hasn't started"
        );
        for leg in ["build (os: c)", "build (os: d)"] {
            exec.timeline.push(started(leg));
            exec.timeline.push(finished(leg, 300));
        }
        assert!(exec.unfinished_jobs(&legs).is_empty());
    }
}
