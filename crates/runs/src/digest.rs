//! The compact answer to "why did this run fail?".

use crate::record::{JobRecord, RunRecord, RunStatus, StepRecord, SCHEMA_VERSION};
use crate::store::RunStore;
use serde::Serialize;

/// Lines of output kept per failed step.
pub const DEFAULT_TAIL_LINES: usize = 40;

/// Lines from above the tail that look like the cause.
const MAX_ERROR_LINES: usize = 10;
const ERROR_WORDS: [&str; 5] = ["error", "fatal", "panic", "traceback", "failed"];

#[derive(Debug, Clone, Serialize)]
pub struct FailureDigest {
    pub schema_version: u32,
    pub run_id: String,
    pub workflow: String,
    pub status: RunStatus,
    /// Why nothing could run, when the failure is not in a step.
    pub error: Option<String>,
    /// What to keep in mind when reading this result.
    pub note: Option<String>,
    pub failures: Vec<FailedStep>,
    pub replay: String,
}

#[derive(Debug, Clone, Serialize)]
pub struct FailedStep {
    pub job: String,
    /// Other legs of a matrix job that failed in exactly the same way.
    pub also_failed_in: Vec<String>,
    /// `None` when the job failed outside any step, such as a timeout.
    pub step: Option<String>,
    pub step_index: Option<usize>,
    pub command: Option<String>,
    pub uses: Option<String>,
    pub exit_code: Option<i32>,
    pub output_tail: Vec<String>,
    /// Lines above `output_tail` that look like errors. The cause of a
    /// failure is often followed by more output than the tail holds.
    pub error_lines: Vec<String>,
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

/// Up to [`MAX_ERROR_LINES`] of the lines above the last `tail_lines` that
/// mention an error. wrkflw prints the script ahead of the output, so a word
/// in the script is not evidence and everything before the output is ignored.
fn error_lines(text: &str, tail_lines: usize) -> Vec<String> {
    let all: Vec<&str> = text.lines().collect();
    let above_tail = &all[..all.len().saturating_sub(tail_lines)];
    let output_start = above_tail
        .iter()
        .position(|line| matches!(*line, "Standard Output:" | "Standard Error:"))
        .unwrap_or(0);
    let found: Vec<&&str> = above_tail[output_start..]
        .iter()
        .filter(|line| {
            let lower = line.to_lowercase();
            ERROR_WORDS.iter().any(|word| lower.contains(word))
        })
        .collect();
    found[found.len().saturating_sub(MAX_ERROR_LINES)..]
        .iter()
        .map(|line| line.to_string())
        .collect()
}

/// `line` with durations blanked. Two legs rarely take equally long, and
/// `Ran 2 tests in 0.001s` shouldn't tell their failures apart. Other numbers
/// stay, because in `5 != 6` they are the failure.
fn without_durations(line: &str) -> String {
    let is_duration = |word: &str| {
        let word = word.trim_matches(|c| matches!(c, '(' | ')' | '[' | ']' | ','));
        word.strip_suffix("ms")
            .or_else(|| word.strip_suffix('s'))
            .is_some_and(|number| number.parse::<f64>().is_ok())
    };
    let words: Vec<&str> = line
        .split_whitespace()
        .map(|word| {
            if is_duration(word) {
                "<duration>"
            } else {
                word
            }
        })
        .collect();
    words.join(" ")
}

/// Whether two failures are one leg's failure repeated by another.
fn same_failure(a: &FailedStep, b: &FailedStep) -> bool {
    let tail = |failure: &FailedStep| -> Vec<String> {
        failure
            .output_tail
            .iter()
            .map(|line| without_durations(line))
            .collect()
    };
    (&a.step, &a.command, &a.uses, a.exit_code) == (&b.step, &b.command, &b.uses, b.exit_code)
        && tail(a) == tail(b)
}

/// The failures of `run`, with the legs of a matrix job that failed the same
/// way folded into one entry: a wide matrix would otherwise repeat one bug
/// until the digest is no longer compact.
fn failed_steps(store: &RunStore, run: &RunRecord, tail_lines: usize) -> Vec<FailedStep> {
    let mut failures: Vec<(&str, FailedStep)> = Vec::new();
    let failed_jobs = run
        .jobs
        .iter()
        .filter(|job| job.status == "failure" && !job.failure_allowed);
    for job in failed_jobs {
        let failure = failed_step(store, run, job, tail_lines);
        let repeat_of = failures.iter_mut().find(|(canonical, earlier)| {
            *canonical == job.canonical_name && same_failure(earlier, &failure)
        });
        match repeat_of {
            Some((_, earlier)) => earlier.also_failed_in.push(failure.job),
            None => failures.push((&job.canonical_name, failure)),
        }
    }
    failures.into_iter().map(|(_, failure)| failure).collect()
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
    let note = run.from_step.as_ref().map(|step| {
        format!(
            "Started at step '{step}' with --from-step, so earlier steps did not run: \
             their outputs, env and files are missing."
        )
    });
    let failures = failed_steps(store, run, tail_lines);
    Some(FailureDigest {
        schema_version: SCHEMA_VERSION,
        run_id: run.id.clone(),
        workflow: run.workflow.clone(),
        status: run.status,
        error: run.error.clone(),
        note,
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
        let text = read(&job.log);
        let (output_tail, output_lines) = tail(&text, tail_lines);
        return FailedStep {
            job: job.name.clone(),
            also_failed_in: Vec::new(),
            step: None,
            step_index: None,
            command: None,
            uses: None,
            exit_code: None,
            output_tail,
            error_lines: error_lines(&text, tail_lines),
            output_lines,
            env_from_earlier_steps: Vec::new(),
            log_path: job.log.as_deref().map(log_path),
        };
    };
    let text = read(&step.log);
    let (output_tail, output_lines) = tail(&text, tail_lines);
    let mut env: Vec<(String, String)> = Vec::new();
    for earlier in job.steps.iter().filter(|s| s.index < step.index) {
        for (key, value) in &earlier.env_set {
            env.retain(|(existing, _)| existing != key);
            env.push((key.clone(), value.clone()));
        }
    }
    FailedStep {
        job: job.name.clone(),
        also_failed_in: Vec::new(),
        step: Some(step.name.clone()),
        step_index: Some(step.index),
        command: step.command.clone(),
        uses: step.uses.clone(),
        exit_code: step.exit_code,
        output_tail,
        error_lines: error_lines(&text, tail_lines),
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
    if let Some(note) = &digest.note {
        out.push_str(&format!("Note: {note}\n"));
    }
    for failure in &digest.failures {
        let step = failure.step.as_deref().unwrap_or("(no step)");
        out.push_str(&format!("\nFailed: {} / {}", failure.job, step));
        if let Some(code) = failure.exit_code {
            out.push_str(&format!(" (exit {code})"));
        }
        out.push('\n');
        if !failure.also_failed_in.is_empty() {
            out.push_str(&format!(
                "Also failed in: {}\n",
                failure.also_failed_in.join(", ")
            ));
        }
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
        if !failure.error_lines.is_empty() {
            out.push_str("Errors above the output below:\n");
            for line in &failure.error_lines {
                out.push_str(&format!("  {line}\n"));
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

#[cfg(test)]
mod tests {
    use super::*;

    fn failure(output: &[&str]) -> FailedStep {
        FailedStep {
            job: "test (n: 1)".to_string(),
            also_failed_in: Vec::new(),
            step: Some("run".to_string()),
            step_index: Some(0),
            command: None,
            uses: None,
            exit_code: Some(1),
            output_tail: output.iter().map(|line| line.to_string()).collect(),
            error_lines: Vec::new(),
            output_lines: output.len(),
            env_from_earlier_steps: Vec::new(),
            log_path: None,
        }
    }

    #[test]
    fn failures_differing_only_in_duration_are_the_same() {
        assert!(same_failure(
            &failure(&["Ran 2 tests in 0.001s"]),
            &failure(&["Ran 2 tests in 0.000s"]),
        ));
        assert!(same_failure(
            &failure(&["done (12ms)"]),
            &failure(&["done (9ms)"]),
        ));
    }

    #[test]
    fn failures_differing_in_any_other_number_are_not() {
        assert!(!same_failure(
            &failure(&["assert 5 != 6"]),
            &failure(&["assert 7 != 8"]),
        ));
    }

    #[test]
    fn error_lines_come_from_above_the_tail_and_not_from_the_script() {
        let text = "Exit code: 1\nCommand: echo error in script\n\nStandard Output:\n\
                    compiling\nerror: the cause\nnoise\nerror: inside the tail\n";
        assert_eq!(error_lines(text, 2), ["error: the cause"]);
        assert!(error_lines(text, 100).is_empty());
    }

    #[test]
    fn error_lines_keep_the_latest_when_there_are_many() {
        let text: Vec<String> = (0..30).map(|n| format!("error {n}")).collect();
        let found = error_lines(&text.join("\n"), 1);
        assert_eq!(found.len(), MAX_ERROR_LINES);
        assert_eq!(found.last().unwrap(), "error 28");
    }
}
