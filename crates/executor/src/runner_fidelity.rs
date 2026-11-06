//! Jobs that asked for one OS and ran on another.
//!
//! Locally `windows-latest` and `macos-latest` run in a Linux image, and
//! emulation runs every job on the host. Such a job can pass here and fail on
//! GitHub, since steps gated on `runner.os` and anything OS-specific were
//! never exercised. Each one is reported, so a pass isn't read as proof.

use crate::engine::RuntimeType;
use crate::events::{emit, ExecutionEvent};
use serde_yaml::Value;
use std::collections::HashMap;
use wrkflw_parser::workflow::Job;

/// The OS a job asked for, and the one it ran as.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RunnerSubstitution {
    /// The `runs-on` label that named the OS, with matrix values filled in.
    pub requested: String,
    /// `runner.os` as the job saw it.
    pub ran_as: String,
}

impl RunnerSubstitution {
    /// Why a pass of this job doesn't stand for GitHub's.
    pub fn reason(&self) -> String {
        format!(
            "runs-on asked for {}, but the job ran as {}: steps gated on runner.os \
             and OS-specific behaviour were not checked",
            self.requested, self.ran_as
        )
    }

    /// `job`'s entry in an `unverified` list, as `run_finished`, `gate` and
    /// `plan` all report it.
    pub fn unverified_entry(&self, job: &str) -> serde_json::Value {
        serde_json::json!({
            "job": job,
            "requested": self.requested,
            "ran_as": self.ran_as,
            "reason": self.reason(),
        })
    }
}

/// `runner.os` as a job sees it under `runtime`: Linux in a container,
/// the host's OS otherwise. `Auto` is resolved first, as a run does.
pub fn runner_os_for(runtime: RuntimeType) -> String {
    if crate::engine::detect_runtime(runtime).runs_in_container() {
        "Linux".to_string()
    } else {
        crate::environment::get_runner_os()
    }
}

/// The substitution for `job`, if its `runs-on` names an OS other than the
/// one in `env`'s `RUNNER_OS`.
///
/// A job with `container:` runs on Linux on GitHub too, and a label the user
/// mapped to an image with `--platform` is taken as their choice. Labels that
/// name no OS, such as a custom self-hosted label, can't be judged.
pub(crate) fn substitution(
    job: &Job,
    matrix: Option<&HashMap<String, Value>>,
    env: &HashMap<String, String>,
    platform_overrides: &HashMap<String, String>,
) -> Option<RunnerSubstitution> {
    substitution_as(job, matrix, env.get("RUNNER_OS")?, platform_overrides)
}

/// The substitution for `job` when it runs as `ran_as`, which is what
/// `plan` asks before anything runs.
pub fn substitution_as(
    job: &Job,
    matrix: Option<&HashMap<String, Value>>,
    ran_as: &str,
    platform_overrides: &HashMap<String, String>,
) -> Option<RunnerSubstitution> {
    if job.container.as_ref().is_some_and(|c| !c.image.is_empty()) {
        return None;
    }
    let labels: Vec<String> = job
        .runs_on
        .iter()
        .flatten()
        .map(|label| match matrix {
            Some(values) => crate::substitution::preprocess_command(label, values),
            None => label.clone(),
        })
        .collect();
    if labels
        .iter()
        .any(|label| platform_overrides.contains_key(&label.trim().to_ascii_lowercase()))
    {
        return None;
    }
    let (label, os) = labels
        .iter()
        .find_map(|label| label_os(label).map(|os| (label, os)))?;
    (!os.eq_ignore_ascii_case(ran_as)).then(|| RunnerSubstitution {
        requested: label.trim().to_string(),
        ran_as: ran_as.to_string(),
    })
}

/// The OS a `runs-on` label names, as `runner.os` spells it.
fn label_os(label: &str) -> Option<&'static str> {
    let label = label.trim().to_ascii_lowercase();
    if label.contains("${{") {
        return None;
    }
    if label.starts_with("windows") {
        Some("Windows")
    } else if label.starts_with("macos") {
        Some("macOS")
    } else if label.starts_with("ubuntu") || label == "linux" {
        Some("Linux")
    } else {
        None
    }
}

/// Tell the run's observers, and the log, that `job` ran as another OS.
/// Call it after the job's `JobStarted`.
pub(crate) fn announce(job: &str, substitution: Option<&RunnerSubstitution>) {
    let Some(substitution) = substitution else {
        return;
    };
    wrkflw_logging::warning(&format!(
        "Job '{}' asked for {} but ran as {}: steps that depend on the OS were not checked",
        job, substitution.requested, substitution.ran_as
    ));
    emit(|| ExecutionEvent::RunnerSubstituted {
        job: job.to_string(),
        requested: substitution.requested.clone(),
        ran_as: substitution.ran_as.clone(),
    });
}

#[cfg(test)]
mod tests {
    use super::*;

    fn job(yaml: &str) -> Job {
        serde_yaml::from_str(yaml).unwrap()
    }

    fn env(os: &str) -> HashMap<String, String> {
        HashMap::from([("RUNNER_OS".to_string(), os.to_string())])
    }

    fn matrix(os: &str) -> HashMap<String, Value> {
        HashMap::from([("os".to_string(), Value::String(os.to_string()))])
    }

    #[test]
    fn a_windows_matrix_leg_run_as_linux_is_reported() {
        let job = job("runs-on: ${{ matrix.os }}\nsteps: []");
        let found = substitution(
            &job,
            Some(&matrix("windows-latest")),
            &env("Linux"),
            &HashMap::new(),
        );
        assert_eq!(
            found,
            Some(RunnerSubstitution {
                requested: "windows-latest".to_string(),
                ran_as: "Linux".to_string(),
            })
        );
        let linux = substitution(
            &job,
            Some(&matrix("ubuntu-latest")),
            &env("Linux"),
            &HashMap::new(),
        );
        assert_eq!(linux, None);
    }

    #[test]
    fn emulation_on_a_mac_reports_a_linux_job() {
        let job = job("runs-on: ubuntu-22.04\nsteps: []");
        let found = substitution(&job, None, &env("macOS"), &HashMap::new()).unwrap();
        assert_eq!(found.requested, "ubuntu-22.04");
        assert_eq!(found.ran_as, "macOS");
    }

    #[test]
    fn self_hosted_labels_are_judged_by_the_label_that_names_an_os() {
        let job = job("runs-on: [self-hosted, macOS, ARM64]\nsteps: []");
        let found = substitution(&job, None, &env("Linux"), &HashMap::new()).unwrap();
        assert_eq!(found.requested, "macOS");
        let custom = self::job("runs-on: [self-hosted, gpu]\nsteps: []");
        assert_eq!(
            substitution(&custom, None, &env("Linux"), &HashMap::new()),
            None
        );
    }

    #[test]
    fn a_container_job_or_a_mapped_label_is_not_reported() {
        let container = job("runs-on: macos-latest\ncontainer: node:20\nsteps: []");
        assert_eq!(
            substitution(&container, None, &env("Linux"), &HashMap::new()),
            None
        );
        let mapped = job("runs-on: macos-latest\nsteps: []");
        let overrides = HashMap::from([("macos-latest".to_string(), "my/mac:1".to_string())]);
        assert_eq!(substitution(&mapped, None, &env("Linux"), &overrides), None);
    }

    #[test]
    fn the_unverified_entry_names_the_job_and_why() {
        let found = RunnerSubstitution {
            requested: "macos-latest".to_string(),
            ran_as: "Linux".to_string(),
        };
        let entry = found.unverified_entry("build (os: macos-latest)");
        assert_eq!(entry["job"], "build (os: macos-latest)");
        assert_eq!(entry["requested"], "macos-latest");
        assert_eq!(entry["ran_as"], "Linux");
        assert_eq!(entry["reason"], found.reason());
    }

    #[test]
    fn an_unresolved_label_is_not_judged() {
        let job = job("runs-on: ${{ fromJSON(needs.setup.outputs.os) }}\nsteps: []");
        assert_eq!(
            substitution(&job, None, &env("Linux"), &HashMap::new()),
            None
        );
    }
}
