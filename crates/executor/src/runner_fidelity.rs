//! Jobs that asked for one OS and ran on another.
//!
//! Locally `windows-latest` and `macos-latest` run in a Linux image, and
//! emulation runs every job on the host. Such a job can pass here and fail on
//! GitHub, since steps gated on `runner.os` and anything OS-specific were
//! never exercised. Each one is reported, so a pass isn't read as proof.

use crate::engine::RuntimeType;
use crate::events::{emit, ExecutionEvent};
use serde::{Deserialize, Serialize};
use serde_yaml::Value;
use std::collections::HashMap;
use wrkflw_parser::workflow::Job;

/// The OS a job asked for, and the one it ran as. Run records store it as is.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
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
            "kind": "runner",
            "job": job,
            "requested": self.requested,
            "ran_as": self.ran_as,
            "reason": self.reason(),
        })
    }
}

/// `runner.os` as a job sees it under `runtime`: Linux in a container,
/// the host's OS otherwise. `Auto` is resolved first, as a run does.
///
/// Resolving `Auto` probes Docker and Podman, which can take seconds. The
/// answer is only needed to judge the OSes in `requested`: with none it
/// goes unused, and on a Linux host it is Linux either way, so then nothing
/// is probed.
pub fn runner_os_for(runtime: RuntimeType, requested: &[&str]) -> String {
    let host = crate::environment::get_runner_os();
    if runtime == RuntimeType::Auto && (requested.is_empty() || host == "Linux") {
        return host;
    }
    if crate::engine::detect_runtime(runtime).runs_in_container() {
        "Linux".to_string()
    } else {
        host
    }
}

/// `runs-on` labels with their matrix expressions evaluated. The engine maps
/// labels to images from these too, so the label judged here is the one
/// that picked the job's image.
///
/// A label is evaluated when its expressions read nothing but `matrix` and
/// literals, so `${{ matrix.os || 'ubuntu-latest' }}` resolves per leg. One
/// that reads another context, such as `needs`, is kept as written: its
/// value isn't known here, and a fallback after `||` would be a guess.
pub(crate) fn runs_on_labels(
    runs_on: &Option<Vec<String>>,
    matrix: Option<&HashMap<String, Value>>,
) -> Vec<String> {
    runs_on
        .iter()
        .flatten()
        .map(|label| matrix_label(label, matrix))
        .collect()
}

/// `label` with each `${{ }}` evaluated against `matrix`, or as written when
/// any of them can't be.
fn matrix_label(label: &str, matrix: Option<&HashMap<String, Value>>) -> String {
    crate::expression::evaluate_matrix_text(label, matrix)
        .unwrap_or_else(|| fallback_label(label, matrix))
}

/// What a label became before expressions were evaluated: plain
/// `${{ matrix.x }}` references filled in, the rest left alone.
fn fallback_label(label: &str, matrix: Option<&HashMap<String, Value>>) -> String {
    match matrix {
        Some(values) => crate::substitution::preprocess_command(label, values),
        None => label.to_string(),
    }
}

/// The image the user mapped one of `labels` to with `--platform`, trying
/// the labels in order.
pub(crate) fn mapped_image<'a>(
    labels: &[String],
    platform_overrides: &'a HashMap<String, String>,
) -> Option<&'a String> {
    labels
        .iter()
        .find_map(|label| platform_overrides.get(&label.trim().to_ascii_lowercase()))
}

/// The OS a job's `runs-on` names, before knowing what it runs as.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RequestedOs {
    /// The label that named it, with matrix values filled in.
    pub label: String,
    /// The OS, as `runner.os` spells it.
    pub os: &'static str,
}

impl RequestedOs {
    /// The substitution when the job runs as `ran_as`, if that's another OS.
    pub fn ran_as(&self, ran_as: &str) -> Option<RunnerSubstitution> {
        (!self.os.eq_ignore_ascii_case(ran_as)).then(|| RunnerSubstitution {
            requested: self.label.clone(),
            ran_as: ran_as.to_string(),
        })
    }
}

/// The OS `job` asks for, if it can be judged.
///
/// A job with `container:` runs on Linux on GitHub too, and a label the user
/// mapped to an image with `--platform` is taken as their choice. Labels that
/// name no OS, such as a custom self-hosted label, can't be judged.
pub fn requested_os(
    job: &Job,
    matrix: Option<&HashMap<String, Value>>,
    platform_overrides: &HashMap<String, String>,
) -> Option<RequestedOs> {
    if job.container.as_ref().is_some_and(|c| !c.image.is_empty()) {
        return None;
    }
    let labels = runs_on_labels(&job.runs_on, matrix);
    if mapped_image(&labels, platform_overrides).is_some() {
        return None;
    }
    labels.iter().find_map(|label| {
        label_os(label).map(|os| RequestedOs {
            label: label.trim().to_string(),
            os,
        })
    })
}

/// The substitution for `job`, if its `runs-on` names an OS other than the
/// one in `env`'s `RUNNER_OS`.
pub(crate) fn substitution(
    job: &Job,
    matrix: Option<&HashMap<String, Value>>,
    env: &HashMap<String, String>,
    platform_overrides: &HashMap<String, String>,
) -> Option<RunnerSubstitution> {
    requested_os(job, matrix, platform_overrides)?.ran_as(env.get("RUNNER_OS")?)
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
    fn larger_runner_and_versioned_labels_name_their_os() {
        for (label, os) in [
            ("windows-latest-8-cores", "Windows"),
            ("windows-2022", "Windows"),
            ("macos-14-xlarge", "macOS"),
            ("ubuntu-24.04-arm", "Linux"),
            ("Linux", "Linux"),
        ] {
            assert_eq!(label_os(label), Some(os), "{label}");
        }
        assert_eq!(label_os("self-hosted"), None);
        assert_eq!(label_os("gpu-linux"), None);
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
        assert_eq!(entry["kind"], "runner");
        assert_eq!(entry["reason"], found.reason());
    }

    #[test]
    fn auto_answers_without_probing_when_nothing_is_judged() {
        let host = crate::environment::get_runner_os();
        assert_eq!(runner_os_for(RuntimeType::Auto, &[]), host);
        assert_eq!(runner_os_for(RuntimeType::Docker, &["Windows"]), "Linux");
        assert_eq!(runner_os_for(RuntimeType::Emulation, &["Windows"]), host);
    }

    #[test]
    fn the_engine_and_the_check_read_the_same_mapped_label() {
        let job = job("runs-on: [self-hosted, \"${{ matrix.os }}\"]\nsteps: []");
        let overrides = HashMap::from([("macos-14".to_string(), "my/mac:1".to_string())]);
        let labels = runs_on_labels(&job.runs_on, Some(&matrix("macos-14")));
        assert_eq!(labels, ["self-hosted", "macos-14"]);
        assert_eq!(
            mapped_image(&labels, &overrides).map(String::as_str),
            Some("my/mac:1")
        );
        assert_eq!(
            requested_os(&job, Some(&matrix("macos-14")), &overrides),
            None
        );
    }

    #[test]
    fn a_matrix_label_with_a_fallback_is_judged_per_leg() {
        let job = job("runs-on: ${{ matrix.os || 'ubuntu-latest' }}\nsteps: []");
        let windows = substitution(
            &job,
            Some(&matrix("windows-latest")),
            &env("Linux"),
            &HashMap::new(),
        )
        .unwrap();
        assert_eq!(windows.requested, "windows-latest");
        let no_os = HashMap::from([("python".to_string(), Value::String("3.13".into()))]);
        assert_eq!(
            substitution(&job, Some(&no_os), &env("Linux"), &HashMap::new()),
            None
        );
        let on_mac = substitution(&job, Some(&no_os), &env("macOS"), &HashMap::new()).unwrap();
        assert_eq!(on_mac.requested, "ubuntu-latest");
    }

    #[test]
    fn a_fallback_after_another_context_is_not_guessed() {
        let job = job("runs-on: ${{ needs.setup.outputs.os || 'ubuntu-latest' }}\nsteps: []");
        assert_eq!(
            substitution(&job, None, &env("macOS"), &HashMap::new()),
            None
        );
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
