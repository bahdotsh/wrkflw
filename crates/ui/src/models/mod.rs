// UI Models for wrkflw
use chrono::Local;
use std::path::PathBuf;
use std::sync::Arc;
use wrkflw_executor::{JobStatus, StepStatus};
use wrkflw_logging::symbols;
use wrkflw_parser::workflow::WorkflowDefinition;

/// Type alias for the complex execution result type
pub type ExecutionResultMsg = (usize, Result<(Vec<wrkflw_executor::JobResult>, ()), String>);

/// Result of trigger evaluation for TUI display
#[derive(Debug, Clone)]
pub enum TriggerMatchStatus {
    /// Workflow would trigger based on current diff
    Matched(String),
    /// Workflow would NOT trigger
    Skipped(String),
}

/// Represents an individual workflow file
pub struct Workflow {
    pub name: String,
    pub path: PathBuf,
    pub selected: bool,
    pub status: WorkflowStatus,
    pub execution_details: Option<WorkflowExecution>,
    pub job_names: Vec<String>,
    pub trigger_match: Option<TriggerMatchStatus>,
    /// Parsed workflow definition. Populated at load time so the Dashboard
    /// preview / mini-DAG don't have to reparse on every render. `None` when
    /// the file failed to parse (we still show the row so the user sees it).
    pub definition: Option<Arc<WorkflowDefinition>>,
}

/// A workflow queued for execution, with its own target job
/// Where one workflow job stands in the latest run, merged across
/// the matrix combinations it expanded into.
#[derive(Copy, Clone, Debug, PartialEq, Eq)]
pub enum JobState {
    Success,
    Failure,
    /// Failed under `continue-on-error`, so the run carried on.
    FailureAllowed,
    Skipped,
    Running,
    Pending,
}

impl JobExecution {
    /// Whether this result belongs to the workflow job keyed `key`.
    pub fn is_for(&self, key: &str) -> bool {
        self.canonical_name == key || self.name == key
    }
}

impl Workflow {
    /// State of the job keyed `key`. Results only arrive when a run
    /// ends, so while one is in flight every unreported job counts as
    /// running.
    pub fn job_state(&self, key: &str) -> JobState {
        let results: Vec<&JobExecution> = self
            .execution_details
            .as_ref()
            .map(|e| e.jobs.iter().filter(|j| j.is_for(key)).collect())
            .unwrap_or_default();
        if results.is_empty() {
            return if self.status == WorkflowStatus::Running {
                JobState::Running
            } else {
                JobState::Pending
            };
        }
        let failed = |allowed: bool| {
            results
                .iter()
                .any(|j| j.status == JobStatus::Failure && j.failure_allowed == allowed)
        };
        if failed(false) {
            JobState::Failure
        } else if failed(true) {
            JobState::FailureAllowed
        } else if results.iter().all(|j| j.status == JobStatus::Skipped) {
            JobState::Skipped
        } else {
            JobState::Success
        }
    }

    /// Workflow job keys that no reported result covers yet, in order.
    pub fn unreported_jobs(&self) -> Vec<&str> {
        let jobs = self
            .execution_details
            .as_ref()
            .map(|e| &e.jobs[..])
            .unwrap_or(&[]);
        self.job_names
            .iter()
            .filter(|n| !jobs.iter().any(|j| j.is_for(n)))
            .map(String::as_str)
            .collect()
    }
}

pub struct QueuedExecution {
    pub workflow_idx: usize,
    pub target_job: Option<String>,
}

/// Status of a workflow
#[derive(Debug, Clone, PartialEq)]
pub enum WorkflowStatus {
    NotStarted,
    Running,
    Success,
    Failed,
    Skipped,
}

/// Detailed execution information
pub struct WorkflowExecution {
    pub jobs: Vec<JobExecution>,
    pub start_time: chrono::DateTime<Local>,
    pub end_time: Option<chrono::DateTime<Local>>,
    pub progress: f64, // 0.0 - 1.0 for progress bar
    /// The run as it happened, from the executor's events.
    pub timeline: Vec<TimelineEntry>,
    /// What each job's steps are printing, as they print it.
    pub live: LiveTails,
}

/// Lines of live output kept per job.
pub const LIVE_TAIL_LINES: usize = 500;

/// The end of what each job's current step printed, keyed by the job's
/// display name (a matrix leg has its own).
#[derive(Debug, Default)]
pub struct LiveTails {
    pub jobs: std::collections::HashMap<String, LiveTail>,
    /// Bumped on every update, so the newest tail can be found.
    pub seq: u64,
}

#[derive(Debug, Default)]
pub struct LiveTail {
    /// The step's index in its job; names can repeat, indexes can't.
    pub index: usize,
    pub step: String,
    pub lines: std::collections::VecDeque<String>,
    pub updated: u64,
}

impl LiveTails {
    pub fn push(&mut self, job: &str, index: usize, step: &str, lines: Vec<String>) {
        self.seq += 1;
        let tail = self.jobs.entry(job.to_string()).or_default();
        // A fresh tail has an empty name, which no step has, so its first
        // push always starts over too.
        if tail.index != index || tail.step != step {
            tail.index = index;
            tail.step = step.to_string();
            tail.lines.clear();
        }
        tail.lines.extend(lines);
        let excess = tail.lines.len().saturating_sub(LIVE_TAIL_LINES);
        tail.lines.drain(..excess);
        tail.updated = self.seq;
    }

    /// The tail to show: `job`'s if it has one, else the newest.
    pub fn pick(&self, job: Option<&str>) -> Option<(&str, &LiveTail)> {
        if let Some((name, tail)) = job.and_then(|j| self.jobs.get_key_value(j)) {
            return Some((name.as_str(), tail));
        }
        self.jobs
            .iter()
            .max_by_key(|(_, tail)| tail.updated)
            .map(|(name, tail)| (name.as_str(), tail))
    }
}

impl WorkflowExecution {
    /// Wall time of each finished job, keyed by its workflow key. A matrix
    /// job's entries ran side by side, so the key keeps the slowest.
    pub fn job_durations_ms(&self) -> std::collections::HashMap<String, u64> {
        let mut canonical = std::collections::HashMap::new();
        let mut out: std::collections::HashMap<String, u64> = std::collections::HashMap::new();
        for entry in &self.timeline {
            match entry {
                TimelineEntry::JobStarted {
                    job,
                    canonical_name,
                } => {
                    canonical.insert(job.as_str(), canonical_name.as_str());
                }
                TimelineEntry::JobFinished {
                    job, duration_ms, ..
                } => {
                    let key = canonical.get(job.as_str()).copied().unwrap_or(job.as_str());
                    let slot = out.entry(key.to_string()).or_default();
                    *slot = (*slot).max(*duration_ms);
                }
                TimelineEntry::StepFinished { .. } => {}
            }
        }
        out
    }
}

/// Lines of a failed step's output kept in the timeline.
pub const TIMELINE_FAILURE_TAIL: usize = 6;

/// One line of a run's story, built from the executor's events. Every text
/// field is already masked by the executor.
#[derive(Clone, Debug, PartialEq)]
pub enum TimelineEntry {
    JobStarted {
        job: String,
        canonical_name: String,
    },
    StepFinished {
        job: String,
        step: String,
        status: StepStatus,
        exit_code: Option<i32>,
        duration_ms: u64,
        /// The end of the output, kept only when the step failed.
        tail: Vec<String>,
    },
    JobFinished {
        job: String,
        status: JobStatus,
        failure_allowed: bool,
        duration_ms: u64,
        /// The matrix leg this one did not run in favour of.
        repeated_from: Option<String>,
    },
}

impl TimelineEntry {
    /// The entry for `event`, or `None` for events the story leaves out.
    pub fn from_event(event: &wrkflw_executor::ExecutionEvent) -> Option<Self> {
        use wrkflw_executor::ExecutionEvent as E;
        match event {
            E::JobStarted {
                job,
                canonical_name,
            } => Some(TimelineEntry::JobStarted {
                job: job.clone(),
                canonical_name: canonical_name.clone(),
            }),
            E::StepStarted { .. } | E::StepOutput { .. } => None,
            E::StepFinished {
                job,
                step,
                status,
                exit_code,
                duration_ms,
                output,
                ..
            } => Some(TimelineEntry::StepFinished {
                job: job.clone(),
                step: step.clone(),
                status: *status,
                exit_code: *exit_code,
                duration_ms: *duration_ms,
                tail: if *status == StepStatus::Failure {
                    failure_tail(output)
                } else {
                    Vec::new()
                },
            }),
            E::JobFinished {
                job,
                status,
                failure_allowed,
                duration_ms,
                repeated_from,
                ..
            } => Some(TimelineEntry::JobFinished {
                job: job.clone(),
                status: status.clone(),
                failure_allowed: *failure_allowed,
                duration_ms: *duration_ms,
                repeated_from: repeated_from.clone(),
            }),
        }
    }
}

/// The last few non-blank lines the step itself printed, which is where a
/// failure explains itself. wrkflw reports the exit code and the script
/// ahead of the output, under headers, and that preamble is left out.
fn failure_tail(output: &str) -> Vec<String> {
    use wrkflw_executor::{EXIT_CODE_LABEL, STDERR_HEADER, STDOUT_HEADER};
    let all: Vec<&str> = output.lines().map(str::trim_end).collect();
    let is_header = |l: &str| l == STDOUT_HEADER || l == STDERR_HEADER;
    let start = match all.iter().position(|l| is_header(l)) {
        Some(header) => header + 1,
        // Only the preamble: the step printed nothing.
        None if all.first().is_some_and(|l| l.starts_with(EXIT_CODE_LABEL)) => all.len(),
        None => 0,
    };
    let lines: Vec<&str> = all[start..]
        .iter()
        .copied()
        .filter(|l| !l.trim().is_empty() && !is_header(l))
        .collect();
    lines[lines.len().saturating_sub(TIMELINE_FAILURE_TAIL)..]
        .iter()
        .map(|l| l.to_string())
        .collect()
}

/// What a run reports to the TUI while it runs.
#[derive(Debug, Clone, PartialEq)]
pub enum RunUpdate {
    Entry(TimelineEntry),
    /// Lines a running step printed.
    Output {
        job: String,
        index: usize,
        step: String,
        lines: Vec<String>,
    },
}

impl RunUpdate {
    pub fn from_event(event: &wrkflw_executor::ExecutionEvent) -> Option<Self> {
        match event {
            wrkflw_executor::ExecutionEvent::StepOutput {
                job,
                step,
                index,
                lines,
            } => Some(RunUpdate::Output {
                job: job.clone(),
                index: *index,
                step: step.clone(),
                lines: lines.clone(),
            }),
            other => TimelineEntry::from_event(other).map(RunUpdate::Entry),
        }
    }
}

/// An update for the workflow at the index.
pub type TimelineMsg = (usize, RunUpdate);

/// Job execution details
pub struct JobExecution {
    pub name: String,
    /// The job's key in the workflow. Differs from `name` for matrix
    /// jobs, whose name carries the combination, e.g. `build (os: x)`.
    pub canonical_name: String,
    pub status: JobStatus,
    pub steps: Vec<StepExecution>,
    pub logs: Vec<String>,
    /// The job failed, but `continue-on-error` keeps the run green.
    pub failure_allowed: bool,
}

/// Step execution details
pub struct StepExecution {
    pub name: String,
    pub status: StepStatus,
    pub output: String,
    pub exit_code: Option<i32>,
}

/// Severity level for status bar toast messages
#[derive(Debug, Clone, Copy, PartialEq, Default)]
pub enum StatusSeverity {
    Success,
    Info,
    Warning,
    #[default]
    Error,
}

/// The badge a log line is rendered with in the log panes.
///
/// This is the single source of truth for "what kind of line is this".
/// Both the renderer (which draws the badge) and [`LogFilterLevel`]
/// (which decides whether the line survives a filter) classify through
/// here, so a line can never be badged `WARN` and then hidden by the
/// Warning filter — which is exactly what happened while the two carried
/// independent keyword lists.
///
/// Classification is first-match-wins in declaration order, so a line
/// mentioning both an error and a success is an error.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LogBadge {
    Error,
    Warn,
    Success,
    /// Active work. Rendered `INFO` with the info accent.
    Running,
    Trigger,
    /// Logged at debug level: executor diagnostics, hidden unless the TUI
    /// was started with `--verbose`.
    Debug,
    /// Everything else. Also rendered `INFO`, but dimmed.
    Plain,
}

impl LogBadge {
    /// Classify a raw log line, timestamp prefix and all.
    pub fn classify(log: &str) -> Self {
        // The level glyph wrkflw_logging writes after the timestamp wins
        // over keywords: a diagnostic that mentions an error is still a
        // diagnostic.
        let level_glyph = log
            .strip_prefix('[')
            .and_then(|rest| rest.split_once("] "))
            .map(|(_, body)| body);
        if level_glyph.is_some_and(|body| body.starts_with(symbols::DEBUG)) {
            LogBadge::Debug
        } else if log.contains("Error") || log.contains("error") || log.contains(symbols::FAILURE) {
            LogBadge::Error
        } else if log.contains("Warning")
            || log.contains("warning")
            || log.contains(symbols::WARNING)
        {
            LogBadge::Warn
        } else if log.contains("Success")
            || log.contains("success")
            || log.contains(symbols::SUCCESS)
        {
            LogBadge::Success
        } else if log.contains("Running")
            || log.contains("running")
            || log.contains(symbols::RUNNING)
        {
            LogBadge::Running
        } else if log.contains("Triggering") || log.contains("triggered") {
            LogBadge::Trigger
        } else {
            LogBadge::Plain
        }
    }

    /// The text drawn in the badge column.
    pub fn as_str(&self) -> &'static str {
        match self {
            LogBadge::Error => "ERROR",
            LogBadge::Warn => "WARN",
            LogBadge::Success => "SUCCESS",
            LogBadge::Running | LogBadge::Plain => "INFO",
            LogBadge::Trigger => "TRIG",
            LogBadge::Debug => "DEBUG",
        }
    }

    /// The key `theme::log_badge` styles by. Distinct from [`Self::as_str`]
    /// only for [`LogBadge::Plain`], which shares the `INFO` label but is
    /// drawn dim so that active work stands out against routine chatter.
    pub fn style_key(&self) -> &'static str {
        match self {
            LogBadge::Plain => "",
            other => other.as_str(),
        }
    }
}

/// Log filter levels
#[derive(Debug, Clone, PartialEq)]
pub enum LogFilterLevel {
    Info,
    Warning,
    Error,
    Success,
    Trigger,
    All,
}

impl LogFilterLevel {
    /// Whether `log` survives this filter.
    ///
    /// Delegates to [`LogBadge::classify`] rather than matching keywords
    /// itself: the filter must agree with the badge the user can see, or
    /// lines disappear under the filter that names them.
    pub fn matches(&self, log: &str) -> bool {
        let badge = LogBadge::classify(log);
        match self {
            LogFilterLevel::All => true,
            // Two badges are drawn `INFO`; the filter named INFO shows both.
            LogFilterLevel::Info => matches!(badge, LogBadge::Running | LogBadge::Plain),
            LogFilterLevel::Warning => badge == LogBadge::Warn,
            LogFilterLevel::Error => badge == LogBadge::Error,
            LogFilterLevel::Success => badge == LogBadge::Success,
            LogFilterLevel::Trigger => badge == LogBadge::Trigger,
        }
    }

    pub fn next(&self) -> Self {
        match self {
            LogFilterLevel::All => LogFilterLevel::Info,
            LogFilterLevel::Info => LogFilterLevel::Warning,
            LogFilterLevel::Warning => LogFilterLevel::Error,
            LogFilterLevel::Error => LogFilterLevel::Success,
            LogFilterLevel::Success => LogFilterLevel::Trigger,
            LogFilterLevel::Trigger => LogFilterLevel::All,
        }
    }

    pub fn as_str(&self) -> &str {
        match self {
            LogFilterLevel::All => "ALL",
            LogFilterLevel::Info => "INFO",
            LogFilterLevel::Warning => "WARNING",
            LogFilterLevel::Error => "ERROR",
            LogFilterLevel::Success => "SUCCESS",
            LogFilterLevel::Trigger => "TRIGGER",
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn live_tails_keep_the_current_step_and_the_newest_job() {
        let mut live = LiveTails::default();
        live.push("lint", 0, "clippy", vec!["Checking a".into()]);
        live.push("build", 0, "compile", vec!["Compiling a".into()]);
        assert_eq!(live.pick(None).map(|(job, _)| job), Some("build"));
        assert_eq!(live.pick(Some("lint")).map(|(job, _)| job), Some("lint"));

        // A new step starts its tail over; the tail stays bounded.
        live.push("build", 1, "test", vec!["running 3 tests".into()]);
        let lines: Vec<String> = (0..LIVE_TAIL_LINES + 10).map(|i| i.to_string()).collect();
        live.push("build", 1, "test", lines);
        let tail = &live.jobs["build"];
        assert_eq!(tail.step, "test");
        assert_eq!(tail.lines.len(), LIVE_TAIL_LINES);
        assert_eq!(tail.lines.back().map(String::as_str), Some("509"));
    }

    #[test]
    fn a_step_sharing_its_name_with_the_last_one_starts_its_own_tail() {
        let mut live = LiveTails::default();
        live.push("build", 0, "Test", vec!["unit".into()]);
        live.push("build", 1, "Test", vec!["integration".into()]);
        let tail = &live.jobs["build"];
        assert_eq!(tail.index, 1);
        assert_eq!(tail.lines, ["integration"]);
    }

    #[test]
    fn debug_lines_are_classified_by_their_level_glyph() {
        let line = format!("[12:00:00] {} Failed command: error", symbols::DEBUG);
        assert_eq!(LogBadge::classify(&line), LogBadge::Debug);
        assert!(!LogFilterLevel::Info.matches(&line));
        assert!(!LogFilterLevel::Error.matches(&line));
        let info = format!("[12:00:00] {} build failed with an error", symbols::INFO);
        assert_eq!(LogBadge::classify(&info), LogBadge::Error);
    }

    fn step_event(status: StepStatus, output: &str) -> wrkflw_executor::ExecutionEvent {
        wrkflw_executor::ExecutionEvent::StepFinished {
            job: "build (os: a)".into(),
            step: "test".into(),
            index: 0,
            status,
            outcome: status,
            conclusion: status,
            exit_code: Some(1),
            duration_ms: 1500,
            output: output.into(),
            command: None,
            uses: None,
            env_set: Vec::new(),
        }
    }

    #[test]
    fn timeline_keeps_the_tail_of_failed_steps_only() {
        let output = (1..=10)
            .map(|i| format!("line {i}"))
            .collect::<Vec<_>>()
            .join("\n\n");
        let Some(TimelineEntry::StepFinished { tail, .. }) =
            TimelineEntry::from_event(&step_event(StepStatus::Failure, &output))
        else {
            panic!("a step event becomes a step entry");
        };
        assert_eq!(tail.len(), TIMELINE_FAILURE_TAIL);
        assert_eq!(tail.last().map(String::as_str), Some("line 10"));
        let Some(TimelineEntry::StepFinished { tail, .. }) =
            TimelineEntry::from_event(&step_event(StepStatus::Success, &output))
        else {
            panic!("a step event becomes a step entry");
        };
        assert!(tail.is_empty());
    }

    #[test]
    fn failure_tail_keeps_only_what_the_step_printed() {
        let output = "Exit code: 1\nCommand: make test\nmake test\nStandard Output:\nok 1\nStandard Error:\nboom\n";
        let Some(TimelineEntry::StepFinished { tail, .. }) =
            TimelineEntry::from_event(&step_event(StepStatus::Failure, output))
        else {
            panic!("a step event becomes a step entry");
        };
        assert_eq!(tail, vec!["ok 1".to_string(), "boom".to_string()]);
        let silent = "Exit code: 1\nCommand: false\nfalse\n";
        let Some(TimelineEntry::StepFinished { tail, .. }) =
            TimelineEntry::from_event(&step_event(StepStatus::Failure, silent))
        else {
            panic!("a step event becomes a step entry");
        };
        assert!(tail.is_empty());
    }

    #[test]
    fn job_durations_take_the_slowest_matrix_entry_per_job_key() {
        let started = |job: &str| TimelineEntry::JobStarted {
            job: job.into(),
            canonical_name: "build".into(),
        };
        let finished = |job: &str, ms| TimelineEntry::JobFinished {
            job: job.into(),
            status: JobStatus::Success,
            failure_allowed: false,
            duration_ms: ms,
            repeated_from: None,
        };
        let exec = WorkflowExecution {
            jobs: Vec::new(),
            start_time: Local::now(),
            end_time: None,
            progress: 0.0,
            timeline: vec![
                started("build (os: a)"),
                started("build (os: b)"),
                finished("build (os: a)", 900),
                finished("build (os: b)", 2_500),
            ],
            live: Default::default(),
        };
        assert_eq!(exec.job_durations_ms().get("build"), Some(&2_500));
    }

    /// The invariant this type exists to hold: whatever badge the log pane
    /// draws, the filter named after that badge must show the line.
    /// Before `LogFilterLevel` classified through `LogBadge`, the two kept
    /// independent keyword lists — the badge matched lowercase `warning`,
    /// the filter only uppercase `WARN` — so the diff-filter warning lines
    /// were badged `WARN` and then hidden by the Warning filter.
    #[test]
    fn every_filter_shows_the_lines_it_badges() {
        let cases = [
            "[12:00:00] ↳ warning: git ls-files --others failed (exit 128)",
            "[12:00:00] ↳ parse error: broken.yml: Invalid glob pattern",
            "[12:00:00] Diff filter: 1 warning(s)",
            "[12:00:00] Diff filter: evaluation failed",
            "[12:00:00] Diff filter ON: 1/2 workflows would trigger",
            "[12:00:00] Running job 'build'",
            "[12:00:00] Triggering workflow: ci.yml",
            "[12:00:00] Workflow completed successfully",
            "[12:00:00] Diff filter OFF",
            // Lines arriving from the `wrkflw_logging` store, which
            // stamps a level glyph after the timestamp. Execution events
            // are logged there and nowhere else, so these shapes are what
            // the panes actually draw for them. The glyph is load-bearing:
            // it is checked in the same tier as the level keyword, so it
            // badges lines whose wording alone would badge them wrong.
            // "failed" is not a keyword, and this line would be INFO
            // without the failure glyph:
            "[12:00:00] \u{2716} Workflow 'ci' failed: exit status 1",
            // The Warn tier is tested before the Success tier, so the
            // glyph wins over the "Success" in the state name:
            "[12:00:00] \u{26A0} Cannot trigger workflow 'ci' in Success state",
            "[12:00:00] \u{26A0} No workflow selected to trigger",
            "[12:00:00] \u{25CF} Executing workflow: ci",
            "[12:00:00] \u{25CF} Workflow 'ci' completed successfully!",
        ];

        let levels = [
            LogFilterLevel::Info,
            LogFilterLevel::Warning,
            LogFilterLevel::Error,
            LogFilterLevel::Success,
            LogFilterLevel::Trigger,
        ];

        for line in cases {
            let badge = LogBadge::classify(line);
            let mut shown_by = levels.iter().filter(|l| l.matches(line));
            let level = shown_by.next().unwrap_or_else(|| {
                panic!("no filter shows {:?} (badged {})", line, badge.as_str())
            });
            assert!(
                shown_by.next().is_none(),
                "{:?} must be shown by exactly one filter, it is badged {} once",
                line,
                badge.as_str()
            );
            assert_eq!(
                level.as_str(),
                match badge {
                    LogBadge::Error => "ERROR",
                    LogBadge::Warn => "WARNING",
                    LogBadge::Success => "SUCCESS",
                    LogBadge::Trigger => "TRIGGER",
                    LogBadge::Running | LogBadge::Plain => "INFO",
                    // Debug lines are shown only by `All`, never one of
                    // these, and the cases above carry no debug glyph.
                    LogBadge::Debug => unreachable!("{line:?} is a debug line"),
                },
                "{:?} is badged {} but shown by the {} filter",
                line,
                badge.as_str(),
                level.as_str()
            );
        }
    }

    /// A warning sub-item is badged WARN, so the Warning filter must keep it.
    /// This is the line the glyph fix made visible in the first place.
    #[test]
    fn warning_sub_item_survives_the_warning_filter() {
        let line = format!(
            "[12:00:00] {} warning: git ls-files --others failed",
            symbols::NESTED
        );
        assert_eq!(LogBadge::classify(&line), LogBadge::Warn);
        assert!(LogFilterLevel::Warning.matches(&line));
        assert!(!LogFilterLevel::Info.matches(&line));
    }

    /// `Plain` shares the `INFO` label with `Running` but not its style —
    /// routine chatter stays dim so active work stands out.
    #[test]
    fn plain_is_labelled_info_but_styled_separately() {
        assert_eq!(LogBadge::Plain.as_str(), LogBadge::Running.as_str());
        assert_ne!(LogBadge::Plain.style_key(), LogBadge::Running.style_key());
        assert_eq!(LogBadge::Plain.style_key(), "");
    }

    #[test]
    fn all_shows_everything() {
        assert!(LogFilterLevel::All.matches("[12:00:00] anything at all"));
        assert!(LogFilterLevel::All.matches(""));
    }
}
