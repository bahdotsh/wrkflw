//! The compact answer to "why did this run fail?".

use crate::record::{JobRecord, RunRecord, RunStatus, StepRecord, SCHEMA_VERSION};
use crate::store::RunStore;
use serde::Serialize;
use wrkflw_executor::{EXIT_CODE_LABEL, STDERR_HEADER, STDOUT_HEADER};

/// Lines of output kept per failed step.
pub const DEFAULT_TAIL_LINES: usize = 40;

/// Lines from above the tail that look like the cause.
const MAX_ERROR_LINES: usize = 10;

/// Words that mark a line as an error, matched as whole words so a crate
/// called `thiserror` or a flag like `-Werror` isn't one.
static ERROR_WORD: std::sync::LazyLock<regex::Regex> = std::sync::LazyLock::new(|| {
    regex::Regex::new(r"(?i)\b(error|errors|fatal|panic|panicked|traceback|failed|failures?)\b")
        .unwrap()
});

/// A count of zero in front of an error word, as in `0 failed` or
/// `no errors`: a summary saying nothing went wrong.
static ZERO_COUNT: std::sync::LazyLock<regex::Regex> =
    std::sync::LazyLock::new(|| regex::Regex::new(r"(?i)\b(0|no)\s*$").unwrap());

/// Terminal escape sequences: colours, cursor moves, and the OSC 8
/// hyperlinks cargo prints. An agent reads the digest as text.
static ANSI: std::sync::LazyLock<regex::Regex> = std::sync::LazyLock::new(|| {
    regex::Regex::new(r"\x1b\[[0-?]*[ -/]*[@-~]|\x1b\][^\x07\x1b]*(?:\x07|\x1b\\)|\x1b[@-Z\\-_]")
        .unwrap()
});

/// Output that says memory ran out, not that code failed.
static OUT_OF_MEMORY: std::sync::LazyLock<regex::Regex> = std::sync::LazyLock::new(|| {
    regex::Regex::new(
        r"(?i)\bout of memory\b|\boom-?kill|cannot allocate memory|memory allocation of \d+ bytes failed",
    )
    .unwrap()
});

/// Output that says a process got SIGKILL, without saying why. In a small
/// container VM that is usually the kernel reclaiming memory, but a step, a
/// test harness or a stopped container sends the same signal.
static KILLED: std::sync::LazyLock<regex::Regex> = std::sync::LazyLock::new(|| {
    regex::Regex::new(r"(?i)signal 9\b|\bSIGKILL\b|^\s*Killed\s*$").unwrap()
});

/// The exit status of a process ended by SIGKILL: 128 + 9.
const SIGKILL_EXIT: i32 = 137;

/// `line` without terminal escape sequences.
pub fn strip_ansi(line: &str) -> String {
    ANSI.replace_all(line, "").into_owned()
}

/// Whether `line` reports an error, rather than mentioning one in passing.
fn is_error_line(line: &str) -> bool {
    ERROR_WORD
        .find_iter(line)
        .any(|word| !ZERO_COUNT.is_match(&line[..word.start()]))
}

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
    /// What the output says about the cause, when it points away from the
    /// code, as a process killed for lack of memory does.
    pub cause: Option<FailureCause>,
}

/// A cause read from a failed step's output.
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct FailureCause {
    /// `out_of_memory` when the output says memory ran out, `killed` when a
    /// process got SIGKILL and the output doesn't say why, `setup_failed`
    /// when the job failed before any step ran.
    pub kind: &'static str,
    /// The line that shows it.
    pub evidence: String,
    /// What to do about it.
    pub hint: String,
}

/// A line that reports something passing, such as `test oom_is_handled ...
/// ok`: a word like `out of memory` in it names a test, not a failure.
static PASSING: std::sync::LazyLock<regex::Regex> = std::sync::LazyLock::new(|| {
    regex::Regex::new(r"(?i)\.\.\.\s*ok\s*$|\bPASS(ED)?\b|^\s*(ok|✓|✔)\b").unwrap()
});

/// The lines a cause may be read from: what the step printed, in the last
/// `tail_lines` or among the error lines above them, without lines that
/// report a pass. These are the lines the digest shows as the failure, so a
/// test named after memory that passed earlier in the log isn't evidence.
fn cause_lines(text: &str, tail_lines: usize) -> Vec<String> {
    let all: Vec<&str> = text.lines().collect();
    let start = output_start(&all);
    let tail_start = all.len().saturating_sub(tail_lines).max(start);
    all[start..]
        .iter()
        .enumerate()
        .map(|(i, line)| (start + i, strip_ansi(line)))
        .filter(|(at, line)| {
            !is_output_header(line)
                && !PASSING.is_match(line)
                && (*at >= tail_start || is_error_line(line))
        })
        .map(|(_, line)| line)
        .collect()
}

/// The cause `text` and `exit_code` point to, if it may not be the code.
///
/// Only output that names memory makes it `out_of_memory`. A SIGKILL alone,
/// from `signal 9`, `Killed` or exit code 137, is `killed`: memory is the
/// likely reason in a container VM, but not the only one, so the hint says
/// what to rule out before blaming the machine. Only [`cause_lines`] count.
fn failure_cause(text: &str, exit_code: Option<i32>, tail_lines: usize) -> Option<FailureCause> {
    let lines = cause_lines(text, tail_lines);
    if let Some(line) = lines.iter().find(|line| OUT_OF_MEMORY.is_match(line)) {
        return Some(FailureCause {
            kind: "out_of_memory",
            evidence: line.trim().to_string(),
            hint: "The output says memory ran out. This is the machine running the job, not \
                   a bug in the code: GitHub's runners have more memory than a default Docker \
                   or Podman VM. Give the VM more memory, or run fewer jobs at once, such as \
                   one with --job, then run it again"
                .to_string(),
        });
    }
    let evidence = lines
        .iter()
        .find(|line| KILLED.is_match(line))
        .map(|line| line.trim().to_string())
        .or_else(|| {
            (exit_code == Some(SIGKILL_EXIT)).then(|| format!("exit code {SIGKILL_EXIT}"))
        })?;
    Some(FailureCause {
        kind: "killed",
        evidence,
        hint: "A process was killed with SIGKILL, and the output doesn't say why. In a Docker \
               or Podman VM with less memory than GitHub's runners, that is usually the \
               kernel reclaiming memory, as when a linker runs out. If no step kills \
               processes or stops containers, give the VM more memory or run fewer jobs at \
               once, such as one with --job, before changing code"
            .to_string(),
    })
}

/// The longest `evidence` a setup failure quotes. Its first line can hold
/// a whole Dockerfile command.
const SETUP_EVIDENCE_CHARS: usize = 240;

/// The cause of a job that failed before any step ran: wrkflw couldn't set
/// it up, as when its image doesn't build or pull. `text` is the job's
/// log, whose first line is the error.
fn setup_failure(text: &str) -> Option<FailureCause> {
    let first = text.lines().map(str::trim).find(|line| !line.is_empty())?;
    let evidence = match first.char_indices().nth(SETUP_EVIDENCE_CHARS) {
        Some((cut, _)) => format!("{}…", &first[..cut]),
        None => first.to_string(),
    };
    Some(FailureCause {
        kind: "setup_failed",
        evidence,
        hint: "The job failed before any step ran, while wrkflw set it up: building or \
               pulling its image, or starting its services. No code under test ran, so this \
               says nothing about it. The job log has the full error. Pass --platform with \
               an image that has what the job needs, or fix what the log names"
            .to_string(),
    })
}

fn tail(text: &str, lines: usize) -> (Vec<String>, usize) {
    let all: Vec<&str> = text.lines().collect();
    let kept = all[all.len().saturating_sub(lines)..]
        .iter()
        .map(|line| strip_ansi(line))
        .collect();
    (kept, all.len())
}

fn is_output_header(line: &str) -> bool {
    line == STDOUT_HEADER || line == STDERR_HEADER
}

/// Index of the first line a step printed. wrkflw reports the exit code and
/// script ahead of the output, and a step that printed nothing has only
/// those. Lines from anywhere else, such as a job log, are all output.
fn output_start<S: AsRef<str>>(lines: &[S]) -> usize {
    match lines.iter().position(|l| is_output_header(l.as_ref())) {
        Some(header) => header + 1,
        None if lines
            .first()
            .is_some_and(|l| l.as_ref().starts_with(EXIT_CODE_LABEL)) =>
        {
            lines.len()
        }
        None => 0,
    }
}

/// Up to [`MAX_ERROR_LINES`] of the lines above the last `tail_lines` that
/// mention an error. A word in the script is not evidence, so only what the
/// step printed counts, and a line that reports a pass, such as a test whose
/// name says `Error:`, isn't one either.
fn error_lines(text: &str, tail_lines: usize) -> Vec<String> {
    let all: Vec<&str> = text.lines().collect();
    let tail_start = all.len().saturating_sub(tail_lines);
    let above_tail = all.get(output_start(&all)..tail_start).unwrap_or(&[]);
    let found: Vec<String> = above_tail
        .iter()
        .map(|line| strip_ansi(line))
        .filter(|line| !is_output_header(line) && is_error_line(line) && !PASSING.is_match(line))
        .collect();
    found[found.len().saturating_sub(MAX_ERROR_LINES)..].to_vec()
}

/// `line` with durations blanked. Two legs rarely take equally long, and
/// `Ran 2 tests in 0.001s` shouldn't tell their failures apart. Other numbers
/// stay, because in `5 != 6` they are the failure.
fn without_durations(line: &str) -> String {
    let is_duration = |word: &str| {
        let word = word.trim_matches(|c| matches!(c, '(' | ')' | '[' | ']' | ','));
        word.strip_suffix("ms")
            .or_else(|| word.strip_suffix('s'))
            .is_some_and(|number| number.parse::<f64>().is_ok_and(f64::is_finite))
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

/// Whether two failures of one job are one leg's failure repeated by another.
/// Scripts and step names are left out: they often differ by nothing but the
/// matrix value, and the same step printing the same thing is the same bug.
fn same_failure(a: &FailedStep, b: &FailedStep) -> bool {
    let printed = |failure: &FailedStep| -> Vec<String> {
        let lines = &failure.output_tail;
        lines[output_start(lines)..]
            .iter()
            .map(|line| without_durations(line))
            .collect()
    };
    (a.step_index, a.exit_code) == (b.step_index, b.exit_code) && printed(a) == printed(b)
}

/// The failures of `run`, with the legs of a matrix job that failed the same
/// way folded into one entry: a wide matrix would otherwise repeat one bug
/// until the digest is no longer compact.
fn failed_steps(store: &RunStore, run: &RunRecord, tail_lines: usize) -> Vec<FailedStep> {
    let mut failures: Vec<(&str, FailedStep)> = Vec::new();
    let failed_jobs: Vec<&JobRecord> = run
        .jobs
        .iter()
        .filter(|job| job.status == "failure" && !job.failure_allowed)
        .collect();
    // A leg that only repeated another ran nothing of its own, so it has no
    // output to read. It is named on the leg it repeated.
    let (folded, ran): (Vec<&JobRecord>, Vec<&JobRecord>) = failed_jobs
        .into_iter()
        .partition(|job| job.repeated_from.is_some());
    for job in ran {
        let failure = failed_step(store, run, job, tail_lines);
        let repeat_of = failures.iter_mut().find(|(canonical, earlier)| {
            *canonical == job.canonical_name && same_failure(earlier, &failure)
        });
        match repeat_of {
            Some((_, earlier)) => earlier.also_failed_in.push(failure.job),
            None => failures.push((&job.canonical_name, failure)),
        }
    }
    for job in folded {
        let original = job.repeated_from.as_deref().unwrap_or_default();
        let owner = failures.iter_mut().find(|(_, failure)| {
            failure.job == original || failure.also_failed_in.iter().any(|leg| leg == original)
        });
        match owner {
            Some((_, failure)) => failure.also_failed_in.push(job.name.clone()),
            // The leg it repeated isn't a failure here, so say what we know.
            None => failures.push((
                &job.canonical_name,
                failed_step(store, run, job, tail_lines),
            )),
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
            cause: if job.steps.is_empty() {
                setup_failure(&text)
            } else {
                failure_cause(&text, None, tail_lines)
            },
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
        cause: failure_cause(&text, step.exit_code, tail_lines),
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
        if let Some(cause) = &failure.cause {
            out.push_str(&format!(
                "Likely cause: {} ({})\n  {}\n",
                cause.kind, cause.evidence, cause.hint
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
            cause: None,
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
    fn failures_differing_only_in_the_script_are_the_same() {
        let leg = |version: &str| {
            let command = format!("pytest --python {version}");
            let tail = format!("Exit code: 1\nCommand: {command}\n\nStandard Output:\nboom");
            FailedStep {
                command: Some(command),
                ..failure(&tail.lines().collect::<Vec<_>>())
            }
        };
        assert!(same_failure(&leg("3.11"), &leg("3.12")));
    }

    #[test]
    fn silent_failures_differing_only_in_the_script_are_the_same() {
        assert!(same_failure(
            &failure(&["Exit code: 1", "Command: run 1", ""]),
            &failure(&["Exit code: 1", "Command: run 2", ""]),
        ));
    }

    #[test]
    fn failures_of_different_steps_are_not_the_same() {
        let later = FailedStep {
            step_index: Some(1),
            ..failure(&["boom"])
        };
        assert!(!same_failure(&failure(&["boom"]), &later));
    }

    #[test]
    fn output_headers_are_not_error_lines() {
        let text = "Exit code: 1\nCommand: x\n\nStandard Output:\nout\n\
                    Standard Error:\nerror: the cause\nnoise\nmore\n";
        assert_eq!(error_lines(text, 2), ["error: the cause"]);
    }

    #[test]
    fn a_passing_test_named_after_an_error_is_not_an_error_line() {
        let text = "Exit code: 1\nCommand: pytest\n\nStandard Output:\n\
                    tests/test_options.py::test_bad[Error: Invalid value] PASSED [ 35%]\n\
                    tests/test_echo.py::test_pager FAILED [ 80%]\n\
                    FAILED tests/test_echo.py::test_pager - AssertionError\nnoise\nmore\n";
        assert_eq!(
            error_lines(text, 2),
            [
                "tests/test_echo.py::test_pager FAILED [ 80%]",
                "FAILED tests/test_echo.py::test_pager - AssertionError"
            ]
        );
    }

    #[test]
    fn a_long_script_above_a_short_output_is_not_error_lines() {
        let text = "Exit code: 1\nCommand: echo error\necho fatal\necho failed\n\n\
                    Standard Output:\nok\n";
        assert!(error_lines(text, 3).is_empty());
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

    #[test]
    fn error_words_count_only_as_whole_words() {
        let text =
            "Standard Output:\n   Compiling thiserror v1.0.69\n  Downloaded thiserror-impl\n\
                    cc -Werror x.c\nerror[E0308]: mismatched types\ntail\n";
        assert_eq!(error_lines(text, 1), ["error[E0308]: mismatched types"]);
    }

    #[test]
    fn a_summary_of_zero_failures_is_not_an_error_line() {
        let text = "Standard Output:\ntest result: ok. 28 passed; 0 failed; 0 ignored\n\
                    no errors found: 0 errors\ntest a ... FAILED\ntail\n";
        assert_eq!(error_lines(text, 1), ["test a ... FAILED"]);
    }

    #[test]
    fn colours_and_links_are_stripped() {
        let text = "Standard Output:\n\x1b[1m\x1b[91merror\x1b[0m: boom\n\
                    \x1b]8;;https://doc.rust-lang.org\x1b\\docs\x1b]8;;\x1b\\ here\n";
        assert_eq!(error_lines(text, 1), ["error: boom"]);
        assert_eq!(tail(text, 1).0, ["docs here"]);
    }

    #[test]
    fn a_linker_killed_for_memory_is_named_as_the_cause() {
        let text = "Standard Error:\n  = note: collect2: fatal error: ld terminated with signal 9 [Killed]\n\
                    error: could not compile `wrkflw`\n";
        let cause = failure_cause(text, Some(101), 40).unwrap();
        assert_eq!(cause.kind, "killed");
        assert!(cause.evidence.contains("signal 9"), "{}", cause.evidence);
        assert!(cause.hint.contains("memory"), "{}", cause.hint);
        assert_eq!(
            failure_cause("Killed\n", Some(137), 40).unwrap().evidence,
            "Killed"
        );
        let oom = failure_cause(
            "memory allocation of 1048576 bytes failed\nKilled\n",
            Some(137),
            40,
        )
        .unwrap();
        assert_eq!(oom.kind, "out_of_memory");
        assert_eq!(oom.evidence, "memory allocation of 1048576 bytes failed");
        assert_eq!(
            failure_cause("test killed_by_timeout ... FAILED\n", Some(101), 40),
            None
        );
    }

    #[test]
    fn a_job_that_failed_before_its_steps_names_its_setup() {
        let text = format!(
            "Runtime error: Failed to build combined runtime image: {}\nLast build output:\n\
             error: externally-managed-environment\n",
            "x".repeat(400)
        );
        let cause = setup_failure(&text).unwrap();
        assert_eq!(cause.kind, "setup_failed");
        assert!(cause.evidence.starts_with("Runtime error: Failed to build"));
        assert!(cause.evidence.ends_with('…'));
        assert_eq!(cause.evidence.chars().count(), SETUP_EVIDENCE_CHARS + 1);
        assert!(cause.hint.contains("before any step ran"));
        assert_eq!(setup_failure("\n  \n"), None);
    }

    #[test]
    fn a_passing_test_named_after_memory_is_not_the_cause() {
        // The memory test passed long before the real failure, and a
        // passing line in the tail isn't evidence either.
        let mut text = String::from("Standard Output:\n");
        text.push_str("test alloc::handles_out_of_memory ... ok\n");
        text.push_str("test sigkill_child_is_reaped ... ok\n");
        for i in 0..50 {
            text.push_str(&format!("test case_{i} ... ok\n"));
        }
        text.push_str("test parse::rejects_bad_input ... FAILED\n");
        text.push_str("test oom_guard ... ok\n");
        text.push_str("error: test failed, to rerun pass `--lib`\n");
        assert_eq!(failure_cause(&text, Some(101), 40), None);

        // Evidence above the tail still counts when it is an error line.
        let mut linker = String::from("Standard Error:\n");
        linker.push_str("collect2: fatal error: ld terminated with signal 9 [Killed]\n");
        for i in 0..50 {
            linker.push_str(&format!("note: line {i}\n"));
        }
        assert_eq!(
            failure_cause(&linker, Some(101), 40).map(|c| c.kind),
            Some("killed")
        );
    }

    #[test]
    fn a_bare_exit_137_is_a_kill_not_a_memory_diagnosis() {
        let cause = failure_cause("Standard Output:\nrunning\n", Some(137), 40).unwrap();
        assert_eq!(cause.kind, "killed");
        assert_eq!(cause.evidence, "exit code 137");
        assert!(
            cause.hint.contains("If no step kills processes"),
            "{}",
            cause.hint
        );
        assert_eq!(failure_cause("", Some(1), 40), None);
    }

    fn job_json(name: &str, repeated_from: Option<&str>) -> serde_json::Value {
        serde_json::json!({
            "name": name,
            "canonical_name": "build",
            "status": "failure",
            "failure_allowed": false,
            "duration_ms": 1,
            "log": null,
            "repeated_from": repeated_from,
            "steps": [{
                "index": 0, "name": "test", "status": "failure", "outcome": "failure",
                "conclusion": "failure", "exit_code": 101, "duration_ms": 1,
                "command": "cargo test", "uses": null, "env_set": [],
                "log": format!("{name}.log"),
            }],
        })
    }

    #[test]
    fn a_folded_leg_is_named_on_the_leg_it_repeated() {
        let dir = tempfile::tempdir().unwrap();
        let store = RunStore::at(dir.path());
        let run: RunRecord = serde_json::from_value(serde_json::json!({
            "schema_version": SCHEMA_VERSION, "id": "r", "status": "failure",
            "workflow": "ci.yml", "cwd": "/repo", "repo_root": "/repo", "git_sha": null,
            "git_branch": null, "git_dirty": null, "runtime": "docker", "event": null,
            "source": "cli", "agent": null, "intent": null, "session": null,
            "replay_of": null, "argv": [], "variables": [], "pid": 1, "started_at": "",
            "finished_at": null, "duration_ms": null, "error": null, "skip_reason": null,
            "jobs": [
                job_json("build (os: macos-latest)", Some("build (os: ubuntu-latest)")),
                job_json("build (os: ubuntu-latest)", None),
                job_json("build (os: windows-latest)", Some("build (os: ubuntu-latest)")),
            ],
        }))
        .unwrap();
        for leg in ["ubuntu", "macos", "windows"] {
            store
                .write_log(
                    "r",
                    &format!("build (os: {leg}-latest).log"),
                    "Standard Output:\nboom\n",
                )
                .unwrap();
        }

        let digest = failure_digest(&store, &run, 10).unwrap();

        assert_eq!(digest.failures.len(), 1, "{:?}", digest.failures);
        assert_eq!(digest.failures[0].job, "build (os: ubuntu-latest)");
        assert_eq!(
            digest.failures[0].also_failed_in,
            ["build (os: macos-latest)", "build (os: windows-latest)"]
        );
    }
}
