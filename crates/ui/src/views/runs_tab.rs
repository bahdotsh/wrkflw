// Runs tab: the run history, including who started each run and why.

use crate::app::App;
use crate::text;
use crate::theme::{self, COLORS};
use ratatui::{
    layout::{Constraint, Direction, Layout, Rect},
    style::{Modifier, Style},
    text::{Line, Span},
    widgets::{Paragraph, Wrap},
    Frame,
};
use wrkflw_runs::{RunRecord, RunStatus};

pub(crate) fn status_glyph(status: RunStatus) -> (&'static str, ratatui::style::Color) {
    match status {
        RunStatus::Success => (theme::symbols::SUCCESS, COLORS.success),
        RunStatus::Failure => (theme::symbols::FAILURE, COLORS.error),
        RunStatus::Error => (theme::symbols::WARNING, COLORS.error),
        RunStatus::Running => (theme::symbols::RUNNING, COLORS.info),
        RunStatus::Skipped => (theme::symbols::SKIPPED, COLORS.text_muted),
        RunStatus::Interrupted => (theme::symbols::BLOCKED, COLORS.warning),
    }
}

/// Glyph and colour for a job or step status stored as text.
fn text_status(status: &str) -> (&'static str, ratatui::style::Color) {
    match status {
        "success" => (theme::symbols::SUCCESS, COLORS.success),
        "failure" => (theme::symbols::FAILURE, COLORS.error),
        "skipped" => (theme::symbols::SKIPPED, COLORS.text_muted),
        _ => (theme::symbols::NOT_STARTED, COLORS.text_dim),
    }
}

pub(crate) fn when(started_at: &str) -> String {
    chrono::DateTime::parse_from_rfc3339(started_at)
        .map(|t| {
            let local = t.with_timezone(&chrono::Local);
            if local.date_naive() == chrono::Local::now().date_naive() {
                local.format("today %H:%M").to_string()
            } else {
                local.format("%b %d %H:%M").to_string()
            }
        })
        .unwrap_or_default()
}

pub(crate) fn duration(ms: Option<u64>) -> String {
    ms.map(|ms| {
        if ms < 1000 {
            format!("{ms}ms")
        } else {
            super::execution_tab::format_duration((ms / 1000) as i64)
        }
    })
    .unwrap_or_default()
}

fn run_row(run: &RunRecord, w: usize, selected: bool) -> Line<'static> {
    let (glyph, color) = status_glyph(run.status);
    let workflow = run.workflow.rsplit('/').next().unwrap_or("").to_string();
    let when = when(&run.started_at);
    let took = duration(run.duration_ms);
    let meta = format!("{when:>12}  {took:>7}");
    let show_meta = w >= 40;
    let name_w = w.saturating_sub(4 + if show_meta { meta.len() + 1 } else { 0 });
    let row = if selected {
        theme::selected_style()
    } else {
        Style::default()
    };
    let mut spans = vec![
        Span::styled(
            if selected { theme::SELECTED_MARK } else { "  " },
            Style::default().fg(theme::current_accent()).patch(row),
        ),
        Span::styled(format!("{glyph} "), Style::default().fg(color).patch(row)),
        Span::styled(
            text::fit(&workflow, name_w),
            Style::default().fg(COLORS.text).patch(row),
        ),
    ];
    if show_meta {
        spans.push(Span::styled(
            format!(" {meta}"),
            theme::muted_style().patch(row),
        ));
    }
    Line::from(spans)
}

fn detail_lines(run: &RunRecord) -> Vec<Line<'static>> {
    let (glyph, color) = status_glyph(run.status);
    let kv = |k: &str, v: String| {
        Line::from(vec![
            Span::styled(format!("{k:<10}"), theme::muted_style()),
            Span::styled(v, Style::default().fg(COLORS.text)),
        ])
    };
    let mut lines = vec![
        Line::from(vec![
            Span::styled(format!("{glyph} "), Style::default().fg(color)),
            Span::styled(
                run.status.as_str().to_uppercase(),
                Style::default().fg(color).add_modifier(Modifier::BOLD),
            ),
            Span::styled(format!("   {}", run.id), theme::muted_style()),
        ]),
        Line::from(""),
        kv("workflow", run.workflow.clone()),
        kv("started", when(&run.started_at)),
    ];
    if run.duration_ms.is_some() {
        lines.push(kv("took", duration(run.duration_ms)));
    }
    lines.push(kv("runtime", run.runtime.clone()));
    lines.push(kv("source", run.source.clone()));
    if let Some(agent) = &run.agent {
        lines.push(kv("agent", agent.clone()));
    }
    if let Some(intent) = &run.intent {
        lines.push(kv("intent", intent.clone()));
    }
    if let Some(session) = &run.session {
        lines.push(kv("session", session.clone()));
    }
    if let Some(sha) = &run.git_sha {
        let mut commit = sha[..sha.len().min(10)].to_string();
        if let Some(branch) = &run.git_branch {
            commit = format!("{commit} on {branch}");
        }
        if run.git_dirty == Some(true) {
            commit.push_str(" (dirty)");
        }
        lines.push(kv("commit", commit));
    }
    if let Some(reason) = &run.skip_reason {
        lines.push(kv("skipped", reason.clone()));
    }
    if let Some(error) = &run.error {
        lines.push(Line::from(""));
        lines.push(Line::from(Span::styled(
            error.clone(),
            Style::default().fg(COLORS.error),
        )));
    }
    if !run.jobs.is_empty() {
        lines.push(Line::from(""));
        lines.push(theme::section(format!("Jobs ({})", run.jobs.len())));
    }
    for job in &run.jobs {
        let (glyph, color) = text_status(&job.status);
        lines.push(Line::from(vec![
            Span::styled(format!("{glyph} "), Style::default().fg(color)),
            Span::styled(
                job.name.clone(),
                Style::default()
                    .fg(COLORS.text)
                    .add_modifier(Modifier::BOLD),
            ),
            Span::styled(
                format!("  {}", duration(job.duration_ms)),
                theme::muted_style(),
            ),
        ]));
        for step in &job.steps {
            let (glyph, color) = text_status(&step.conclusion);
            let code = match step.exit_code {
                Some(c) if c != 0 => format!("  exit {c}"),
                _ => String::new(),
            };
            lines.push(Line::from(vec![
                Span::styled(format!("  {glyph} "), Style::default().fg(color)),
                Span::styled(step.name.clone(), Style::default().fg(COLORS.text_dim)),
                Span::styled(code, Style::default().fg(COLORS.error)),
                Span::styled(
                    format!("  {}", duration(step.duration_ms)),
                    theme::muted_style(),
                ),
            ]));
        }
    }
    lines
}

pub fn render_runs_tab(f: &mut Frame<'_>, app: &mut App, area: Rect) {
    let panes = if app.runs.is_empty() {
        Layout::default()
            .constraints([Constraint::Min(0)])
            .split(area)
    } else if area.width >= 90 {
        Layout::default()
            .direction(Direction::Horizontal)
            .constraints([Constraint::Percentage(50), Constraint::Min(0)])
            .split(area)
    } else {
        Layout::default()
            .direction(Direction::Vertical)
            .constraints([Constraint::Percentage(45), Constraint::Min(0)])
            .split(area)
    };

    let list_block = theme::with_badge(theme::block_focused("Runs"), app.runs.len().to_string());
    let inner = list_block.inner(panes[0]);
    f.render_widget(list_block, panes[0]);
    if app.runs.is_empty() {
        theme::empty_state(
            f,
            inner,
            "No recorded runs yet",
            vec![Span::styled(
                "Every wrkflw run is recorded here",
                theme::muted_style(),
            )],
        );
        return;
    }

    let selected = app.runs_selected.min(app.runs.len() - 1);
    let w = inner.width as usize;
    let viewport = inner.height as usize;
    let offset = selected.saturating_sub(viewport.saturating_sub(1));
    let lines: Vec<Line> = app
        .runs
        .iter()
        .enumerate()
        .skip(offset)
        .take(viewport)
        .map(|(i, run)| run_row(run, w, i == selected))
        .collect();
    f.render_widget(Paragraph::new(lines), inner);
    theme::scrollbar(f, panes[0], app.runs.len(), offset, viewport);

    if let Some(run) = app.runs.get(selected) {
        let block = theme::block("Details");
        let detail = Paragraph::new(detail_lines(run))
            .wrap(Wrap { trim: false })
            .block(block);
        f.render_widget(detail, panes[1]);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn details_show_who_started_a_run_and_why() {
        let run: RunRecord = serde_json::from_value(serde_json::json!({
            "schema_version": 1, "id": "r1", "status": "failure", "workflow": "ci.yml",
            "cwd": "/r", "repo_root": "/r", "git_sha": null, "git_branch": null,
            "git_dirty": null, "runtime": "docker", "event": null, "source": "mcp",
            "agent": "claude-code", "intent": "fix the cache", "replay_of": null,
            "argv": [], "variables": [], "pid": 1, "started_at": "t", "finished_at": null,
            "duration_ms": null, "error": null, "skip_reason": null, "jobs": [],
        }))
        .unwrap();
        let text: Vec<String> = detail_lines(&run).iter().map(|l| l.to_string()).collect();
        assert!(text.iter().any(|l| l.contains("claude-code")));
        assert!(text.iter().any(|l| l.contains("fix the cache")));
    }
}
