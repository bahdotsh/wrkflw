//! Live execution events and per-run hooks.
//!
//! The engine reports job and step progress here so that structured output
//! and run history can follow a run as it happens. Hooks travel in a
//! task-local rather than on `ExecutionConfig`: every job of a run executes
//! on one task (`join_all`, never `spawn`, because the executor future is not
//! `Send`), and the many `ExecutionConfig` literals stay untouched.

use crate::engine::{JobStatus, StepStatus};
use std::future::Future;
use std::sync::Arc;
use wrkflw_parser::workflow::Step;

/// Progress of a run. Every text field is already masked.
#[derive(Debug, Clone)]
pub enum ExecutionEvent {
    JobStarted {
        job: String,
        canonical_name: String,
    },
    StepStarted {
        job: String,
        step: String,
        index: usize,
    },
    StepFinished {
        job: String,
        step: String,
        index: usize,
        status: StepStatus,
        outcome: StepStatus,
        conclusion: StepStatus,
        exit_code: Option<i32>,
        duration_ms: u64,
        output: String,
        /// The `run:` script as written, with `${{ }}` unexpanded.
        command: Option<String>,
        uses: Option<String>,
        /// Variables this step exported through `GITHUB_ENV`.
        env_set: Vec<(String, String)>,
    },
    JobFinished {
        job: String,
        status: JobStatus,
        failure_allowed: bool,
        duration_ms: u64,
        logs: String,
    },
}

/// Receives events on the executor's task, so it must not block for long.
pub trait ExecutionObserver: Send + Sync {
    fn on_event(&self, event: &ExecutionEvent);
}

/// The name a step reports under: its `name:`, its action, or its 1-based
/// position.
pub(crate) fn step_display_name(step: &Step, index: usize) -> String {
    step.name
        .clone()
        .or_else(|| step.uses.clone())
        .unwrap_or_else(|| format!("Step {}", index + 1))
}

/// Where a job should start, from `--from-step`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FromStep {
    pub job: String,
    /// A step's reported name, or a 1-based position.
    pub step: String,
}

impl FromStep {
    /// Index of the step to start from, among `steps`.
    pub fn resolve(&self, steps: &[Step]) -> Option<usize> {
        // The name a run reports has to work here, so that a step named in
        // a failure can be passed straight back. Repeated names resolve to
        // the first, and a position tells them apart.
        let by_name = steps
            .iter()
            .enumerate()
            .position(|(index, step)| step_display_name(step, index) == self.step);
        by_name.or_else(|| {
            let position: usize = self.step.parse().ok()?;
            (1..=steps.len()).contains(&position).then(|| position - 1)
        })
    }
}

/// Everything a caller can attach to one run.
#[derive(Clone, Default)]
pub struct RunHooks {
    pub observers: Vec<Arc<dyn ExecutionObserver>>,
    pub from_step: Option<FromStep>,
}

tokio::task_local! {
    static HOOKS: RunHooks;
}

/// Run `future` with `hooks` active for everything it executes.
pub async fn with_run_hooks<F: Future>(hooks: RunHooks, future: F) -> F::Output {
    HOOKS.scope(hooks, future).await
}

/// Send an event to the active observers. `build` only runs if one exists.
///
/// Outside `with_run_hooks` this does nothing, which is correct for a run
/// nobody observes. It also means a `tokio::spawn` inside the executor loses
/// its events without an error, because task-locals don't cross into new
/// tasks, so anything that emits has to stay on the run's own task.
pub(crate) fn emit(build: impl FnOnce() -> ExecutionEvent) {
    let _ = HOOKS.try_with(|hooks| {
        if hooks.observers.is_empty() {
            return;
        }
        let event = build();
        for observer in &hooks.observers {
            observer.on_event(&event);
        }
    });
}

/// The index `--from-step` starts `job` at, or 0 when it doesn't apply.
pub(crate) fn first_step_for(job: &str, steps: &[Step]) -> usize {
    HOOKS
        .try_with(|hooks| {
            hooks
                .from_step
                .as_ref()
                .filter(|from| from.job == job)
                .and_then(|from| from.resolve(steps))
                .unwrap_or(0)
        })
        .unwrap_or(0)
}

/// Whether `--from-step` skips this step. Checkout still runs: each run gets
/// a fresh workspace, so without it the resumed step would find no files.
pub(crate) fn skipped_by_from_step(index: usize, first_step: usize, step: &Step) -> bool {
    let is_checkout = step
        .uses
        .as_deref()
        .is_some_and(|uses| uses.starts_with("actions/checkout"));
    index < first_step && !is_checkout
}

#[cfg(test)]
mod tests {
    use super::*;

    fn step(name: Option<&str>, uses: Option<&str>) -> Step {
        serde_yaml::from_value(
            serde_yaml::to_value(serde_json::json!({
                "name": name,
                "uses": uses,
            }))
            .unwrap(),
        )
        .unwrap()
    }

    fn resolve(wanted: &str, steps: &[Step]) -> Option<usize> {
        FromStep {
            job: "build".to_string(),
            step: wanted.to_string(),
        }
        .resolve(steps)
    }

    #[test]
    fn every_reported_step_name_resolves_to_its_step() {
        let steps = [
            step(None, Some("actions/checkout@v4")),
            step(Some("build"), None),
            step(None, None),
        ];
        for (index, step) in steps.iter().enumerate() {
            assert_eq!(
                resolve(&step_display_name(step, index), &steps),
                Some(index)
            );
        }
    }

    #[test]
    fn a_position_reaches_a_repeated_action() {
        let steps = [
            step(None, Some("actions/cache@v4")),
            step(None, Some("actions/cache@v4")),
        ];
        assert_eq!(resolve("actions/cache@v4", &steps), Some(0));
        assert_eq!(resolve("2", &steps), Some(1));
        assert_eq!(resolve("3", &steps), None);
    }
}
