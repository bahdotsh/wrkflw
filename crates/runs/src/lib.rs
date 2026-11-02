//! Run history for wrkflw: what ran, what it printed, and why it failed.
//!
//! This is what lets an agent answer "why did the last run fail?" with one
//! read instead of a rerun. See [`execute_recorded`].

mod digest;
mod record;
mod recorder;
mod store;

pub use digest::{failure_digest, render_digest, FailedStep, FailureDigest, DEFAULT_TAIL_LINES};
pub use record::{JobRecord, RunRecord, RunStatus, StepRecord, SCHEMA_VERSION};
pub use recorder::{
    detect_agent, emit_line, execute_recorded, record_skipped, repo_root_of, RecordedRun, RunMeta,
    RunOptions,
};
pub use store::{history_disabled, home_dir, RunStore, DEFAULT_KEEP};
