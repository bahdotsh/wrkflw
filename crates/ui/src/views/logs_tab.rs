// Logs tab. By default it tells the run's story: one line per job and
// step from the executor's events, with failures expanded. `o` switches to
// the raw log, every line with its time and level, following new output
// until the user scrolls away from the end.
use crate::app::App;
use crate::models::{LogFilterLevel, TimelineEntry};
use crate::text;
use crate::theme::{self, COLORS};
use ratatui::{
    layout::{Constraint, Direction, Layout, Rect},
    style::{Modifier, Style},
    text::{Line, Span},
    widgets::{HighlightSpacing, Paragraph, Table, TableState},
    Frame,
};

pub fn render_logs_tab(f: &mut Frame<'_>, app: &mut App, area: Rect) {
    if app.log_story {
        render_story(f, app, area);
        return;
    }
    let show_search = app.log_search_active || !app.log_search_query.is_empty();
    let chunks = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Length(if show_search { 3 } else { 0 }),
            Constraint::Min(0),
        ])
        .split(area);

    if show_search {
        render_search_bar(f, app, chunks[0]);
    }

    let logs = &app.processed_logs;
    let mut badge = if logs.is_empty() {
        "0".to_string()
    } else {
        let at = if app.log_follow {
            logs.len()
        } else {
            app.log_scroll.min(logs.len() - 1) + 1
        };
        format!("{at}/{}", logs.len())
    };
    if let Some(level) = app
        .log_filter_level
        .as_ref()
        .filter(|l| **l != LogFilterLevel::All)
    {
        badge = format!("{} only · {badge}", level.as_str().to_lowercase());
    }
    if app.log_follow && !logs.is_empty() {
        badge = format!("following · {badge}");
    }
    let block = theme::with_badge(
        theme::block_focused("Raw log"),
        format!("{badge} · o story"),
    );

    if logs.is_empty() {
        let inner = block.inner(chunks[1]);
        f.render_widget(block, chunks[1]);
        let msg = if app.log_filter_level.is_some() || !app.log_search_query.is_empty() {
            "No lines match the search or filter"
        } else {
            "No log lines yet"
        };
        theme::empty_state(
            f,
            inner,
            msg,
            vec![Span::styled(
                "Run a workflow and its output lands here",
                theme::muted_style(),
            )],
        );
        return;
    }
    let rows = logs.iter().map(|l| l.to_row());
    let widths = [
        Constraint::Length(8),
        Constraint::Length(9),
        Constraint::Min(0),
    ];
    let total = logs.len();
    let table = Table::new(rows, widths)
        .block(block)
        .row_highlight_style(theme::selected_style())
        .highlight_symbol(theme::SELECTED_MARK)
        .highlight_spacing(HighlightSpacing::Always);

    app.sync_log_cursor();
    let mut state = TableState::default()
        .with_offset(app.log_offset)
        .with_selected(Some(app.log_scroll));
    f.render_stateful_widget(table, chunks[1], &mut state);
    app.log_offset = state.offset();

    let viewport = chunks[1].height.saturating_sub(2) as usize;
    theme::scrollbar(f, chunks[1], total, state.offset(), viewport);
}

fn render_search_bar(f: &mut Frame<'_>, app: &App, area: Rect) {
    let block = theme::panel("Search", app.log_search_active);
    let matches = if !app.log_search_matches.is_empty() {
        format!(
            "{}/{} matches",
            app.log_search_match_idx
                .min(app.log_search_matches.len() - 1)
                + 1,
            app.log_search_matches.len()
        )
    } else if !app.log_search_query.is_empty() {
        "no matches".to_string()
    } else {
        String::new()
    };
    let block = theme::with_badge(block, matches);

    // Keep the end of a long query, where the cursor is, in view.
    let room = (area.width as usize).saturating_sub(8);
    let query = if app.log_search_active {
        text::truncate_left(&app.log_search_query, room).into_owned()
    } else {
        text::truncate(&app.log_search_query, room).into_owned()
    };
    let mut spans = vec![
        Span::styled("/ ", Style::default().fg(theme::current_accent())),
        Span::styled(
            query,
            Style::default()
                .fg(COLORS.text)
                .add_modifier(Modifier::BOLD),
        ),
    ];
    if app.log_search_active {
        spans.push(Span::styled(
            "▏",
            Style::default().fg(theme::current_accent()),
        ));
    }
    f.render_widget(Paragraph::new(Line::from(spans)).block(block), area);
}

/// One job's part of the story, in the order it started.
struct JobStory<'a> {
    name: &'a str,
    steps: Vec<&'a TimelineEntry>,
    finished: Option<&'a TimelineEntry>,
    /// The OS it asked for and the one it ran as, when they differ.
    runner: Option<(&'a str, &'a str)>,
}

fn group_by_job(timeline: &[TimelineEntry]) -> Vec<JobStory<'_>> {
    let mut jobs: Vec<JobStory> = Vec::new();
    for entry in timeline {
        let name = match entry {
            TimelineEntry::JobStarted { job, .. }
            | TimelineEntry::StepFinished { job, .. }
            | TimelineEntry::JobFinished { job, .. }
            | TimelineEntry::RunnerSubstituted { job, .. } => job.as_str(),
        };
        let at = match jobs.iter().position(|j| j.name == name) {
            Some(at) => at,
            None => {
                jobs.push(JobStory {
                    name,
                    steps: Vec::new(),
                    finished: None,
                    runner: None,
                });
                jobs.len() - 1
            }
        };
        match entry {
            TimelineEntry::StepFinished { .. } => jobs[at].steps.push(entry),
            TimelineEntry::JobFinished { .. } => jobs[at].finished = Some(entry),
            TimelineEntry::RunnerSubstituted {
                requested, ran_as, ..
            } => jobs[at].runner = Some((requested.as_str(), ran_as.as_str())),
            TimelineEntry::JobStarted { .. } => {}
        }
    }
    jobs
}

/// `left` and `right` on one line, `left` cut to leave room for `right`.
fn spread(left: Vec<Span<'static>>, right: Vec<Span<'static>>, w: usize) -> Line<'static> {
    let right_w = text::spans_width(&right);
    let room = w.saturating_sub(right_w + 1);
    let mut spans = text::fit_spans(left, room);
    spans.push(Span::raw(" "));
    spans.extend(right);
    Line::from(spans)
}

/// The story lines for a run, `w` cells wide. `settled` once the run has
/// ended: a job that never reported finishing then did not finish, rather
/// than still running (a run can end with an error before a job's finish
/// event, as when a matrix leg's runner image can't be resolved).
pub(crate) fn story_lines(
    timeline: &[TimelineEntry],
    live: Option<&crate::models::LiveTails>,
    w: usize,
    spinner_frame: usize,
    settled: bool,
) -> Vec<Line<'static>> {
    use crate::views::execution_tab::format_ms;
    let mut lines = Vec::new();
    for job in group_by_job(timeline) {
        let (glyph, glyph_style, right) = match job.finished {
            Some(TimelineEntry::JobFinished {
                status,
                failure_allowed,
                duration_ms,
                repeated_from,
                ..
            }) => {
                let (g, st) = if *failure_allowed && *status == wrkflw_executor::JobStatus::Failure
                {
                    (theme::symbols::WARNING, Style::default().fg(COLORS.warning))
                } else {
                    theme::job_status(status)
                };
                let right = match (status, repeated_from) {
                    // Not run: say whose result it reports, not "0ms".
                    (_, Some(original)) => {
                        text::truncate(&text::sanitize(&format!("same as {original}")), w / 2)
                            .into_owned()
                    }
                    (wrkflw_executor::JobStatus::Skipped, None) => "skipped".to_string(),
                    _ => format_ms(*duration_ms),
                };
                (g, st, right)
            }
            _ if settled => {
                let (g, st) = theme::job_status(&wrkflw_executor::JobStatus::Failure);
                (g, st, "did not finish".to_string())
            }
            _ => (
                theme::spinner(spinner_frame),
                Style::default().fg(COLORS.info),
                "running".to_string(),
            ),
        };
        let mut left = vec![
            Span::styled(format!("{glyph} "), glyph_style),
            Span::styled(
                text::sanitize(job.name).into_owned(),
                Style::default()
                    .fg(COLORS.text)
                    .add_modifier(Modifier::BOLD),
            ),
        ];
        // A pass here doesn't cover OS-specific steps, so say so where the
        // job's result is shown, not only in the log.
        if let Some((requested, ran_as)) = job.runner {
            left.push(Span::styled(
                format!(
                    "  {} ran as {} ({})",
                    theme::symbols::WARNING,
                    text::sanitize(ran_as),
                    text::sanitize(requested)
                ),
                Style::default().fg(COLORS.warning),
            ));
        }
        lines.push(spread(
            left,
            vec![Span::styled(right, Style::default().fg(COLORS.text_dim))],
            w,
        ));
        for step in &job.steps {
            let TimelineEntry::StepFinished {
                step,
                status,
                exit_code,
                duration_ms,
                tail,
                ..
            } = step
            else {
                continue;
            };
            let (g, st) = theme::step_status(status);
            let mut right = Vec::new();
            if let Some(code) = exit_code.filter(|c| *c != 0) {
                right.push(Span::styled(
                    format!("exit {code}  "),
                    Style::default().fg(COLORS.error),
                ));
            }
            right.push(Span::styled(format_ms(*duration_ms), theme::muted_style()));
            lines.push(spread(
                vec![
                    Span::raw("  "),
                    Span::styled(format!("{g} "), st),
                    Span::styled(
                        text::sanitize(step).into_owned(),
                        if *status == wrkflw_executor::StepStatus::Skipped {
                            theme::muted_style()
                        } else {
                            Style::default().fg(COLORS.text)
                        },
                    ),
                ],
                right,
                w,
            ));
            for line in tail {
                let room = w.saturating_sub(6);
                lines.push(Line::from(vec![
                    Span::styled("    │ ", Style::default().fg(COLORS.error)),
                    Span::styled(
                        text::truncate(&text::sanitize(line), room).into_owned(),
                        Style::default().fg(COLORS.text_dim),
                    ),
                ]));
            }
        }
        // A running job shows its current step and the last lines it
        // printed, so a long step visibly makes progress.
        if job.finished.is_none() && !settled {
            if let Some(tail) = live.and_then(|live| live.jobs.get(job.name)) {
                lines.push(Line::from(vec![
                    Span::raw("  "),
                    Span::styled(
                        format!("{} ", theme::spinner(spinner_frame)),
                        Style::default().fg(COLORS.info),
                    ),
                    Span::styled(
                        text::sanitize(&tail.step).into_owned(),
                        Style::default().fg(COLORS.text),
                    ),
                ]));
                let room = w.saturating_sub(6);
                for line in tail
                    .lines
                    .iter()
                    .skip(tail.lines.len().saturating_sub(STORY_LIVE_LINES))
                {
                    let plain: String = text::ansi_line(line, Style::default())
                        .spans
                        .iter()
                        .map(|span| span.content.as_ref())
                        .collect();
                    lines.push(Line::from(vec![
                        Span::styled("    │ ", Style::default().fg(COLORS.info)),
                        Span::styled(
                            text::truncate(&text::sanitize(&plain), room).into_owned(),
                            Style::default().fg(COLORS.text_dim),
                        ),
                    ]));
                }
            }
        }
        lines.push(Line::from(""));
    }
    lines
}

/// Lines of a running step's output shown under it in the story.
const STORY_LIVE_LINES: usize = 3;

fn render_story(f: &mut Frame<'_>, app: &mut App, area: Rect) {
    let idx = app.shown_workflow();
    let workflow = idx.map(|i| &app.workflows[i]);
    let exec = workflow.and_then(|w| w.execution_details.as_ref());
    // The Run header already names the workflow.
    let block = theme::with_badge(theme::block_focused("Story"), "o raw");
    let inner = block.inner(area);
    f.render_widget(block, area);

    let Some(exec) = exec.filter(|e| !e.timeline.is_empty()) else {
        theme::empty_state(
            f,
            inner,
            match exec {
                // Validation and remote dispatch report results without
                // events, so there is no story, only the jobs view.
                Some(exec) if exec.end_time.is_some() && !exec.jobs.is_empty() => {
                    "This run has no story; its results are in the jobs view (x)"
                }
                Some(exec) if exec.end_time.is_some() => "The run ended before any job started",
                Some(_) => "Waiting for the first job to start",
                None => "Nothing has run yet",
            },
            vec![
                Span::styled("Run a workflow from ", theme::muted_style()),
                theme::key_chip("1"),
                Span::styled(" or press ", theme::muted_style()),
                theme::key_chip("o"),
                Span::styled(" for the raw log", theme::muted_style()),
            ],
        );
        return;
    };

    let w = inner.width.saturating_sub(1) as usize;
    let mut lines = story_lines(
        &exec.timeline,
        Some(&exec.live),
        w,
        app.spinner_frame,
        exec.end_time.is_some(),
    );
    if let Some(end) = exec.end_time {
        let ms = end
            .signed_duration_since(exec.start_time)
            .num_milliseconds();
        lines.push(Line::from(Span::styled(
            format!(
                "finished in {}",
                crate::views::execution_tab::format_run_time(ms, true)
            ),
            theme::muted_style(),
        )));
    }

    let viewport = inner.height as usize;
    let max = lines.len().saturating_sub(viewport);
    if app.story_follow || app.story_scroll >= max {
        app.story_scroll = max;
        app.story_follow = true;
    }
    let total = lines.len();
    f.render_widget(
        Paragraph::new(lines).scroll((app.story_scroll as u16, 0)),
        inner,
    );
    theme::scrollbar(f, area, total, app.story_scroll, viewport);
}

#[cfg(test)]
mod tests {
    use super::*;
    use wrkflw_executor::{JobStatus, StepStatus};

    fn text_of(lines: &[Line<'_>]) -> Vec<String> {
        lines
            .iter()
            .map(|l| {
                l.spans
                    .iter()
                    .map(|s| s.content.as_ref())
                    .collect::<String>()
            })
            .collect()
    }

    #[test]
    fn story_groups_steps_under_their_job_with_failure_tail() {
        let timeline = vec![
            TimelineEntry::JobStarted {
                job: "build".into(),
                canonical_name: "build".into(),
            },
            TimelineEntry::StepFinished {
                job: "build".into(),
                step: "cargo test".into(),
                status: StepStatus::Failure,
                exit_code: Some(101),
                duration_ms: 37_000,
                tail: vec!["test failed".into()],
            },
            TimelineEntry::JobFinished {
                job: "build".into(),
                status: JobStatus::Failure,
                failure_allowed: false,
                duration_ms: 37_200,
                repeated_from: None,
            },
            TimelineEntry::JobStarted {
                job: "lint".into(),
                canonical_name: "lint".into(),
            },
        ];
        let lines = text_of(&story_lines(&timeline, None, 60, 0, false));
        assert!(lines[0].starts_with("✖ build") && lines[0].ends_with("37.2s"));
        assert!(lines[1].contains("cargo test") && lines[1].contains("exit 101"));
        assert!(lines[2].contains("│ test failed"));
        assert!(lines[4].contains("lint") && lines[4].ends_with("running"));
        for line in &lines {
            assert!(text::width(line) <= 60, "{line:?} overflows");
        }
    }

    #[test]
    fn a_job_run_as_another_os_says_so_on_its_line() {
        let job = "build (os: windows-latest)";
        let timeline = vec![
            TimelineEntry::JobStarted {
                job: job.into(),
                canonical_name: "build".into(),
            },
            TimelineEntry::RunnerSubstituted {
                job: job.into(),
                requested: "windows-latest".into(),
                ran_as: "Linux".into(),
            },
            TimelineEntry::JobFinished {
                job: job.into(),
                status: JobStatus::Success,
                failure_allowed: false,
                duration_ms: 1_200,
                repeated_from: None,
            },
        ];
        let lines = text_of(&story_lines(&timeline, None, 80, 0, false));
        assert!(
            lines[0].contains("ran as Linux (windows-latest)"),
            "{lines:?}"
        );
        assert!(lines[0].ends_with("1.2s"));
        let narrow = text_of(&story_lines(&timeline, None, 30, 0, false));
        assert!(text::width(&narrow[0]) <= 30, "{narrow:?} overflows");
    }

    #[test]
    fn a_running_job_shows_the_step_that_is_running() {
        let timeline = vec![
            TimelineEntry::JobStarted {
                job: "build".into(),
                canonical_name: "build".into(),
            },
            TimelineEntry::StepFinished {
                job: "build".into(),
                step: "compile".into(),
                status: StepStatus::Success,
                exit_code: Some(0),
                duration_ms: 10_000,
                tail: Vec::new(),
            },
        ];
        let mut live = crate::models::LiveTails::default();
        live.push("build", 0, "compile", vec!["Finished dev".into()]);
        // The next step has started and printed nothing yet.
        live.push("build", 1, "test", Vec::new());
        let lines = text_of(&story_lines(&timeline, Some(&live), 60, 0, false));
        assert_eq!(lines[2].trim(), format!("{} test", theme::spinner(0)));
        assert!(
            !lines.iter().any(|l| l.contains("Finished dev")),
            "{lines:?}"
        );
    }

    #[test]
    fn a_job_that_never_finished_is_settled_once_the_run_ends() {
        let timeline = vec![TimelineEntry::JobStarted {
            job: "build (os: a)".into(),
            canonical_name: "build".into(),
        }];
        let mut live = crate::models::LiveTails::default();
        live.push("build (os: a)", 0, "compile", vec!["half done".into()]);

        let running = text_of(&story_lines(&timeline, Some(&live), 60, 0, false));
        assert!(running[0].ends_with("running"), "{running:?}");
        assert!(running.iter().any(|l| l.contains("half done")));

        let settled = text_of(&story_lines(&timeline, Some(&live), 60, 0, true));
        let (failed, _) = theme::job_status(&JobStatus::Failure);
        assert!(settled[0].starts_with(failed), "{settled:?}");
        assert!(settled[0].ends_with("did not finish"), "{settled:?}");
        assert!(
            !settled
                .iter()
                .any(|l| l.contains("half done") || l.contains("compile")),
            "a settled run still shows a live tail: {settled:?}"
        );
    }
}
