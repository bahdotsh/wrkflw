//! The on-disk shape of a run. Field names are a public contract: agents read
//! them through `--format json` and MCP. Add fields rather than rename them.

use serde::{Deserialize, Serialize};
use wrkflw_executor::{RunnerSubstitution, UnexecutedAction};

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
    /// The agent session that started the run, such as a Claude Code session id.
    #[serde(default)]
    pub session: Option<String>,
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
    /// The matrix leg whose result this one reports, when it was not run
    /// because it would only have repeated that leg.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub repeated_from: Option<String>,
    /// Why an earlier result stands in for running the job: `stored` or
    /// `unaffected`. Its one step says the same.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub reused: Option<String>,
    /// Set when `runs-on` asked for another OS than the job ran as. A pass
    /// on such a job says nothing about its OS-specific steps.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub runner: Option<RunnerSubstitution>,
    /// `uses:` steps that passed without running their action, because
    /// wrkflw has no implementation of it. A pass on such a job says
    /// nothing about what the action would have done.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub actions_not_run: Vec<UnexecutedAction>,
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_job_recorded_before_repeated_from_existed_still_reads() {
        let job: JobRecord = serde_json::from_str(
            r#"{"name":"build","canonical_name":"build","status":"success",
                "failure_allowed":false,"duration_ms":12,"log":null,"steps":[]}"#,
        )
        .unwrap();
        assert_eq!(job.repeated_from, None);
        // And a job without one writes no field, as before.
        assert!(!serde_json::to_string(&job)
            .unwrap()
            .contains("repeated_from"));
    }
}
