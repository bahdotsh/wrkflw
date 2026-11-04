//! `.wrkflw/affected.yml`: per-job hints a workflow file can't hold, as GitHub
//! rejects unknown keys there.

use super::SIDECAR_PATH;
use serde::Deserialize;
use std::collections::HashMap;
use std::path::Path;

/// One job's entry in `.wrkflw/affected.yml`.
#[derive(Debug, Clone, Deserialize, PartialEq)]
#[serde(deny_unknown_fields)]
pub struct JobHints {
    /// Globs of the files this job reads, relative to the repository root.
    #[serde(default)]
    pub paths: Option<Vec<String>>,
    /// False means this job always runs: never reused from a stored result
    /// and never skipped as unaffected. For a job with effects wrkflw can't
    /// see.
    #[serde(default = "default_true")]
    pub cache: bool,
    /// False leaves this job to CI: `wrkflw gate` skips it, as it would a
    /// job whose `if:` is false. For a deploy, a release, or anything else
    /// that must not run from a developer's machine on every push.
    #[serde(default = "default_true")]
    pub gate: bool,
}

impl Default for JobHints {
    fn default() -> Self {
        Self {
            paths: None,
            cache: true,
            gate: true,
        }
    }
}

fn default_true() -> bool {
    true
}

#[derive(Debug, Clone, Default, Deserialize, PartialEq)]
#[serde(deny_unknown_fields)]
pub struct Sidecar {
    #[serde(default)]
    pub jobs: HashMap<String, JobHints>,
}

impl Sidecar {
    /// A missing file means no hints. A broken one is an error: silently
    /// ignoring `cache: false` would reuse a job the author ruled out.
    pub fn load(root: &Path) -> Result<Self, String> {
        let path = root.join(SIDECAR_PATH);
        match std::fs::read_to_string(&path) {
            Ok(text) if text.trim().is_empty() => Ok(Self::default()),
            Ok(text) => {
                let sidecar: Self = serde_yaml::from_str(&text)
                    .map_err(|e| format!("invalid {}: {}", SIDECAR_PATH, e))?;
                for (job, path) in sidecar.root_only_paths() {
                    wrkflw_logging::warning(&format!(
                        "{}: '{}' for job '{}' only matches at the repository root, \
                         as '*' doesn't cross '/'. Use '**/{}' for any directory",
                        SIDECAR_PATH, path, job, path
                    ));
                }
                Ok(sidecar)
            }
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(Self::default()),
            Err(e) => Err(format!("cannot read {}: {}", SIDECAR_PATH, e)),
        }
    }

    /// `(job, path)` for each path with a wildcard but no `/`. `*` never
    /// crosses a separator, so `*.rs` misses `src/a.rs`, and a missed input
    /// is a job skipped that should have run.
    pub(super) fn root_only_paths(&self) -> Vec<(&str, &str)> {
        let mut found: Vec<(&str, &str)> = self
            .jobs
            .iter()
            .flat_map(|(job, hints)| {
                hints
                    .paths
                    .iter()
                    .flatten()
                    .map(move |path| (job.as_str(), path.as_str()))
            })
            .filter(|(_, path)| !path.contains('/') && path.contains(['*', '?', '[']))
            .collect();
        found.sort();
        found
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_job_is_cached_and_gated_unless_the_sidecar_says_otherwise() {
        let sidecar: Sidecar =
            serde_yaml::from_str("jobs:\n  a: {}\n  deploy: {gate: false}\n").unwrap();
        assert_eq!(sidecar.jobs["a"], JobHints::default());
        assert!(sidecar.jobs["deploy"].cache);
        assert!(!sidecar.jobs["deploy"].gate);
    }
}
