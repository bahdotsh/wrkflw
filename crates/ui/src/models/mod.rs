// UI Models for wrkflw
use chrono::Local;
use std::path::PathBuf;
use wrkflw_executor::{JobStatus, StepStatus};
use wrkflw_logging::symbols;

/// Type alias for the complex execution result type
pub type ExecutionResultMsg = (usize, Result<(Vec<wrkflw_executor::JobResult>, ()), String>);

/// Represents an individual workflow file
pub struct Workflow {
    pub name: String,
    pub path: PathBuf,
    pub selected: bool,
    pub status: WorkflowStatus,
    pub execution_details: Option<WorkflowExecution>,
    pub job_names: Vec<String>,
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

    pub fn to_string(&self) -> &str {
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
