// Runs tab: the run history, including who started each run and why.

use crate::app::App;
use crate::theme::COLORS;
use ratatui::{
    layout::{Constraint, Direction, Layout, Rect},
    style::{Modifier, Style},
    text::{Line, Span},
    widgets::{Block, Borders, List, ListItem, ListState, Paragraph, Wrap},
    Frame,
};
use wrkflw_runs::{RunRecord, RunStatus};

fn status_color(status: RunStatus) -> ratatui::style::Color {
    match status {
        RunStatus::Success => COLORS.success,
        RunStatus::Failure | RunStatus::Error => COLORS.error,
        _ => COLORS.text_dim,
    }
}

fn run_row(run: &RunRecord) -> ListItem<'static> {
    let workflow = run.workflow.rsplit('/').next().unwrap_or("").to_string();
    let agent = run.agent.clone().unwrap_or_else(|| run.source.clone());
    ListItem::new(Line::from(vec![
        Span::styled(
            format!("{:<11} ", run.status.as_str()),
            Style::default().fg(status_color(run.status)),
        ),
        Span::raw(format!("{workflow}  ")),
        Span::styled(agent, Style::default().fg(COLORS.text_dim)),
    ]))
}

fn detail_lines(run: &RunRecord) -> Vec<Line<'static>> {
    let mut lines = vec![
        Line::from(run.id.clone()),
        Line::from(format!("workflow: {}", run.workflow)),
        Line::from(format!("runtime:  {}", run.runtime)),
        Line::from(format!("source:   {}", run.source)),
    ];
    if let Some(agent) = &run.agent {
        lines.push(Line::from(format!("agent:    {agent}")));
    }
    if let Some(intent) = &run.intent {
        lines.push(Line::from(format!("intent:   {intent}")));
    }
    if let Some(session) = &run.session {
        lines.push(Line::from(format!("session:  {session}")));
    }
    if let Some(sha) = &run.git_sha {
        lines.push(Line::from(format!(
            "commit:   {}",
            &sha[..sha.len().min(10)]
        )));
    }
    if let Some(error) = &run.error {
        lines.push(Line::from(format!("error:    {error}")));
    }
    lines.push(Line::from(""));
    for job in &run.jobs {
        lines.push(Line::from(Span::styled(
            format!("{} {}", job.status, job.name),
            Style::default().add_modifier(Modifier::BOLD),
        )));
        for step in &job.steps {
            let code = step
                .exit_code
                .map(|c| format!(" (exit {c})"))
                .unwrap_or_default();
            lines.push(Line::from(format!(
                "  {} {}{}",
                step.conclusion, step.name, code
            )));
        }
    }
    lines
}

pub fn render_runs_tab(f: &mut Frame<'_>, app: &mut App, area: Rect) {
    let panes = Layout::default()
        .direction(Direction::Horizontal)
        .constraints([Constraint::Percentage(50), Constraint::Min(0)])
        .split(area);

    let list_block = Block::default().borders(Borders::ALL).title(" Runs ");
    if app.runs.is_empty() {
        let empty = Paragraph::new("No recorded runs yet.")
            .style(Style::default().fg(COLORS.text_dim))
            .block(list_block);
        f.render_widget(empty, panes[0]);
        return;
    }

    let items: Vec<ListItem> = app.runs.iter().map(run_row).collect();
    let list = List::new(items)
        .block(list_block)
        .highlight_style(Style::default().add_modifier(Modifier::REVERSED));
    let mut state = ListState::default();
    state.select(Some(app.runs_selected));
    f.render_stateful_widget(list, panes[0], &mut state);

    if let Some(run) = app.runs.get(app.runs_selected) {
        let detail = Paragraph::new(detail_lines(run))
            .wrap(Wrap { trim: false })
            .block(Block::default().borders(Borders::ALL).title(" Details "));
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
