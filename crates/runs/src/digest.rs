//! The compact answer to "why did this run fail?".

use crate::record::{JobRecord, RunRecord, RunStatus, StepRecord, SCHEMA_VERSION};
use crate::store::RunStore;
use serde::Serialize;

/// Lines of output kept per failed step.
pub const DEFAULT_TAIL_LINES: usize = 40;

#[derive(Debug, Clone, Serialize)]
pub struct FailureDigest {
    pub schema_version: u32,
    pub run_id: String,
    pub workflow: String,
    pub status: RunStatus,
    /// Why nothing could run, when the failure is not in a step.
    pub error: Option<String>,
    pub failures: Vec<FailedStep>,
    pub replay: String,
}

#[derive(Debug, Clone, Serialize)]
pub struct FailedStep {
    pub job: String,
    /// `None` when the job failed outside any step, such as a timeout.
    pub step: Option<String>,
    pub step_index: Option<usize>,
    pub command: Option<String>,
    pub uses: Option<String>,
    pub exit_code: Option<i32>,
    pub output_tail: Vec<String>,
    pub output_lines: usize,
    /// Variables earlier steps in this job exported through `GITHUB_ENV`.
    pub env_from_earlier_steps: Vec<(String, String)>,
    /// Absolute path of the full log.
    pub log_path: Option<String>,
}

fn tail(text: &str, lines: usize) -> (Vec<String>, usize) {
    let all: Vec<&str> = text.lines().collect();
    let kept = all[all.len().saturating_sub(lines)..]
        .iter()
        .map(|line| line.to_string())
        .collect();
    (kept, all.len())
}

/// The digest of `run`, or `None` if it did not fail.
pub fn failure_digest(
    store: &RunStore,
    run: &RunRecord,
    tail_lines: usize,
) -> Option<FailureDigest> {
    if !matches!(
        run.status,
        RunStatus::Failure | RunStatus::Error | RunStatus::Interrupted
    ) {
        return None;
    }
    let failures = run
        .jobs
        .iter()
        .filter(|job| job.status == "failure" && !job.failure_allowed)
        .map(|job| failed_step(store, run, job, tail_lines))
        .collect();
    Some(FailureDigest {
        schema_version: SCHEMA_VERSION,
        run_id: run.id.clone(),
        workflow: run.workflow.clone(),
        status: run.status,
        error: run.error.clone(),
        failures,
        replay: format!("wrkflw runs replay {}", run.id),
    })
}

fn failed_step(
    store: &RunStore,
    run: &RunRecord,
    job: &JobRecord,
    tail_lines: usize,
) -> FailedStep {
    let log_path = |relative: &str| {
        store
            .run_dir(&run.id)
            .join(relative)
            .to_string_lossy()
            .into_owned()
    };
    let read = |relative: &Option<String>| {
        relative
            .as_deref()
            .and_then(|path| store.read_log(&run.id, path).ok())
            .unwrap_or_default()
    };
    let failed: Option<&StepRecord> = job.steps.iter().find(|s| s.conclusion == "failure");
    let Some(step) = failed else {
        let (output_tail, output_lines) = tail(&read(&job.log), tail_lines);
        return FailedStep {
            job: job.name.clone(),
            step: None,
            step_index: None,
            command: None,
            uses: None,
            exit_code: None,
            output_tail,
            output_lines,
            env_from_earlier_steps: Vec::new(),
            log_path: job.log.as_deref().map(log_path),
        };
    };
    let (output_tail, output_lines) = tail(&read(&step.log), tail_lines);
    let mut env: Vec<(String, String)> = Vec::new();
    for earlier in job.steps.iter().filter(|s| s.index < step.index) {
        for (key, value) in &earlier.env_set {
            env.retain(|(existing, _)| existing != key);
            env.push((key.clone(), value.clone()));
        }
    }
    FailedStep {
        job: job.name.clone(),
        step: Some(step.name.clone()),
        step_index: Some(step.index),
        command: step.command.clone(),
        uses: step.uses.clone(),
        exit_code: step.exit_code,
        output_tail,
        output_lines,
        env_from_earlier_steps: env,
        log_path: step.log.as_deref().map(log_path),
    }
}

/// Plain-text rendering, for the terminal and for pasting into a prompt.
pub fn render_digest(digest: &FailureDigest) -> String {
    let mut out = format!(
        "Run {} {} ({})\n",
        digest.run_id,
        digest.status.as_str(),
        digest.workflow
    );
    if let Some(error) = &digest.error {
        out.push_str(&format!("Error: {error}\n"));
    }
    for failure in &digest.failures {
        let step = failure.step.as_deref().unwrap_or("(no step)");
        out.push_str(&format!("\nFailed: {} / {}", failure.job, step));
        if let Some(code) = failure.exit_code {
            out.push_str(&format!(" (exit {code})"));
        }
        out.push('\n');
        if let Some(uses) = &failure.uses {
            out.push_str(&format!("Action: {uses}\n"));
        }
        if let Some(command) = &failure.command {
            out.push_str("Script:\n");
            for line in command.lines() {
                out.push_str(&format!("  {line}\n"));
            }
        }
        if !failure.env_from_earlier_steps.is_empty() {
            out.push_str("Env from earlier steps:\n");
            for (key, value) in &failure.env_from_earlier_steps {
                out.push_str(&format!("  {key}={value}\n"));
            }
        }
        if !failure.output_tail.is_empty() {
            let shown = failure.output_tail.len();
            out.push_str(&format!(
                "Output (last {shown} of {} lines):\n",
                failure.output_lines
            ));
            for line in &failure.output_tail {
                out.push_str(&format!("  {line}\n"));
            }
        }
        if let Some(path) = &failure.log_path {
            out.push_str(&format!("Log: {path}\n"));
        }
    }
    if !digest.replay.is_empty() {
        out.push_str(&format!("\nRerun: {}\n", digest.replay));
    }
    out
}
