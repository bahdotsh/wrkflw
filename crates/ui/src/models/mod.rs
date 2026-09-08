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
    pub logs: Vec<String>,
    pub progress: f64, // 0.0 - 1.0 for progress bar
}

/// Job execution details
pub struct JobExecution {
    pub name: String,
    pub status: JobStatus,
    pub steps: Vec<StepExecution>,
    pub logs: Vec<String>,
}

/// Step execution details
pub struct StepExecution {
    pub name: String,
    pub status: StepStatus,
    pub output: String,
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
    /// Everything else. Also rendered `INFO`, but dimmed.
    Plain,
}

impl LogBadge {
    /// Classify a raw log line, timestamp prefix and all.
    pub fn classify(log: &str) -> Self {
        if log.contains("Error") || log.contains("error") || log.contains(symbols::FAILURE) {
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
