//! Steps whose action wrkflw did not run.
//!
//! wrkflw implements some actions itself, such as `actions/checkout`,
//! `actions/cache` and the setup actions that install a language runtime
//! into the job's image. A JavaScript action outside that set has no
//! implementation here, and a setup action does nothing when its runtime
//! isn't in the job's image: in a job `container:`, inside a composite
//! action, or under emulation. Either way the step prints that it would
//! have run the action, reports success, and does nothing. The next steps
//! then run without whatever the action would have installed, configured
//! or checked. Such a step is reported, so a pass isn't read as proof of
//! what GitHub would do.

use serde::{Deserialize, Serialize};

/// A `uses:` step that reported success without running its action.
/// Run records store it as is.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct UnexecutedAction {
    /// The step's name, as `step_finished` reports it.
    pub step: String,
    /// The `uses:` reference, as written.
    pub uses: String,
}

impl UnexecutedAction {
    /// Why a pass of a job with this step doesn't stand for GitHub's.
    pub fn reason(&self) -> String {
        format!(
            "{} did not run here, so the step ran nothing and passed: anything it \
             would have installed, configured or checked was not",
            self.uses
        )
    }

    /// `job`'s entry in an `unverified` list, as `run_finished` and `gate`
    /// report it, next to the entries for jobs run as another OS.
    pub fn unverified_entry(&self, job: &str) -> serde_json::Value {
        serde_json::json!({
            "job": job,
            "step": self.step,
            "uses": self.uses,
            "reason": self.reason(),
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_unverified_entry_names_the_job_the_step_and_the_action() {
        let skipped = UnexecutedAction {
            step: "Install uv".to_string(),
            uses: "astral-sh/setup-uv@v9".to_string(),
        };
        let entry = skipped.unverified_entry("tests (python: 3.13)");
        assert_eq!(entry["job"], "tests (python: 3.13)");
        assert_eq!(entry["step"], "Install uv");
        assert_eq!(entry["uses"], "astral-sh/setup-uv@v9");
        let reason = entry["reason"].as_str().unwrap();
        assert!(reason.contains("astral-sh/setup-uv@v9"));
        assert!(reason.contains("ran nothing"));
    }
}
