//! The on-disk shape of a run. Field names are a public contract: agents read
//! them through `--format json` and MCP. Add fields rather than rename them.

use serde::{Deserialize, Serialize};

/// Bumped only for incompatible changes to any JSON wrkflw emits.
pub const SCHEMA_VERSION: u32 = 1;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum RunStatus {
    Running,
    Success,
    Failure,
    /// wrkflw could not execute the workflow at all.
    Error,
    /// The trigger filter decided the workflow would not fire.
    Skipped,
    /// The process died before finishing.
    Interrupted,
}

impl RunStatus {
    pub fn as_str(self) -> &'static str {
        match self {
            RunStatus::Running => "running",
            RunStatus::Success => "success",
            RunStatus::Failure => "failure",
            RunStatus::Error => "error",
            RunStatus::Skipped => "skipped",
            RunStatus::Interrupted => "interrupted",
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RunRecord {
    pub schema_version: u32,
    pub id: String,
    pub status: RunStatus,
    pub workflow: String,
    pub cwd: String,
    /// Git toplevel of `cwd`, or `cwd` outside a repository. Scopes listings.
    pub repo_root: String,
    pub git_sha: Option<String>,
    pub git_branch: Option<String>,
    pub git_dirty: Option<bool>,
    pub runtime: String,
    pub event: Option<String>,
    /// Where the run came from: `cli`, `watch`, `tui` or `mcp`.
    pub source: String,
    pub agent: Option<String>,
    pub intent: Option<String>,
    pub replay_of: Option<String>,
    /// The step `--from-step` started at. Earlier steps did not run.
    #[serde(default)]
    pub from_step: Option<String>,
    /// Arguments after `wrkflw`, without `--variable` values.
    pub argv: Vec<String>,
    /// Names of the `--variable`s passed. Values are never stored.
    pub variables: Vec<String>,
    pub pid: u32,
    pub started_at: String,
    pub finished_at: Option<String>,
    pub duration_ms: Option<u64>,
    pub error: Option<String>,
    pub skip_reason: Option<String>,
    pub jobs: Vec<JobRecord>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct JobRecord {
    pub name: String,
    pub canonical_name: String,
    pub status: String,
    pub failure_allowed: bool,
    pub duration_ms: Option<u64>,
    /// Path of the job log, relative to the run directory.
    pub log: Option<String>,
    pub steps: Vec<StepRecord>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct StepRecord {
    pub index: usize,
    pub name: String,
    pub status: String,
    /// Result before `continue-on-error`.
    pub outcome: String,
    /// Result after `continue-on-error`.
    pub conclusion: String,
    pub exit_code: Option<i32>,
    pub duration_ms: Option<u64>,
    pub command: Option<String>,
    pub uses: Option<String>,
    /// Variables this step exported, with secrets masked.
    pub env_set: Vec<(String, String)>,
    /// Path of the step log, relative to the run directory.
    pub log: Option<String>,
}
