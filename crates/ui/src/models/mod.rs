// UI Models for wrkflw
use chrono::Local;
use std::path::PathBuf;
use wrkflw_executor::{events::LogStream, JobStatus, StepStatus};
use wrkflw_logging::symbols;

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
    /// Set when the executor emits `JobStarted`. `None` for jobs seeded as
    /// `Pending` from the YAML plan before execution reaches them.
    pub start_time: Option<chrono::DateTime<Local>>,
    /// Set when the executor emits `JobCompleted`.
    pub end_time: Option<chrono::DateTime<Local>>,
    /// Per-run job id from the executor's event stream. `None` for YAML-seeded
    /// `Pending` jobs that haven't emitted `JobStarted` yet.
    pub event_job_id: Option<u64>,
}

/// A single streamed chunk from a step's stdout/stderr, as captured by the
/// live execution event stream.
#[derive(Debug, Clone)]
pub struct LogLine {
    pub stream: LogStream,
    pub text: String,
    pub at: chrono::DateTime<Local>,
}

/// Step execution details
pub struct StepExecution {
    pub name: String,
    pub status: StepStatus,
    /// Legacy final-output blob kept for the no-event-sink (CLI-style) path.
    /// When live streaming is wired (event_sink is Some), `log_buffer` is the
    /// source of truth and this is only populated post-mortem for archival.
    pub output: String,
    /// Incremental log lines streamed by the executor. Appended in order via
    /// `StepLogChunk` events. Empty when the CLI path is used.
    pub log_buffer: Vec<LogLine>,
    /// Set when the executor emits `StepStarted`.
    pub start_time: Option<chrono::DateTime<Local>>,
    /// Set when the executor emits `StepCompleted`.
    pub end_time: Option<chrono::DateTime<Local>>,
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
    pub fn matches(&self, log: &str) -> bool {
        match self {
            LogFilterLevel::Info => {
                log.contains(symbols::INFO) || (log.contains("INFO") && !log.contains("SUCCESS"))
            }
            LogFilterLevel::Warning => log.contains(symbols::WARNING) || log.contains("WARN"),
            LogFilterLevel::Error => log.contains(symbols::FAILURE) || log.contains("ERROR"),
            LogFilterLevel::Success => {
                log.contains(symbols::SUCCESS) || log.contains("SUCCESS") || log.contains("success")
            }
            LogFilterLevel::Trigger => {
                log.contains("Triggering") || log.contains("triggered") || log.contains("TRIG")
            }
            LogFilterLevel::All => true,
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
