// Step-progress dots strip — one short bar per step, coloured by status.
// Mirrors the design's `<ProgressDots/>` component in screens-core.jsx.

use crate::models::WorkflowStatus;
use crate::theme::COLORS;
use ratatui::{
    layout::Rect,
    style::Style,
    text::{Line, Span},
    widgets::Paragraph,
    Frame,
};
use wrkflw_executor::StepStatus;

/// State of a single dot. Live state isn't carried by `StepStatus` (which
/// only models terminal outcomes), so callers pass a synthesised tag.
#[derive(Copy, Clone, Debug)]
pub enum DotState {
    Success,
    Failure,
    Skipped,
    Running,
    Pending,
}

impl DotState {
    pub fn from_step(s: &StepStatus) -> Self {
        match s {
            StepStatus::Success => DotState::Success,
            StepStatus::Failure => DotState::Failure,
            StepStatus::Skipped => DotState::Skipped,
        }
    }
}

fn dot_style(state: DotState) -> Style {
    let c = match state {
        DotState::Success => COLORS.success,
        DotState::Failure => COLORS.error,
        DotState::Skipped => COLORS.warning,
        DotState::Running => COLORS.info,
        DotState::Pending => COLORS.border,
    };
    Style::default().fg(c)
}

/// Render a horizontal strip of progress segments and a `done/total` counter.
pub fn render(frame: &mut Frame<'_>, area: Rect, dots: &[DotState], done: usize, total: usize) {
    if total == 0 {
        return;
    }
    let mut spans: Vec<Span> = Vec::with_capacity(dots.len() * 2 + 2);
    for d in dots {
        spans.push(Span::styled("▆", dot_style(*d)));
        spans.push(Span::raw(" "));
    }
    spans.push(Span::styled(
        format!(" {}/{} ", done, total),
        Style::default().fg(COLORS.text_dim),
    ));
    frame.render_widget(Paragraph::new(Line::from(spans)), area);
}

/// Convenience: build a `Vec<DotState>` from terminal step statuses, padding
/// with `Pending` and marking the next-pending slot `Running` if the workflow
/// is currently active.
pub fn synthesise(
    completed: &[StepStatus],
    total: usize,
    workflow_status: &WorkflowStatus,
) -> Vec<DotState> {
    let mut out: Vec<DotState> = completed.iter().map(DotState::from_step).collect();
    let pending = total.saturating_sub(out.len());
    if pending == 0 {
        return out;
    }
    let next_is_running = matches!(workflow_status, WorkflowStatus::Running);
    for i in 0..pending {
        if i == 0 && next_is_running {
            out.push(DotState::Running);
        } else {
            out.push(DotState::Pending);
        }
    }
    out
}
