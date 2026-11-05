//! Job workspaces that outlive the run.
//!
//! A hosted runner starts every job in an empty directory, so a build
//! starts cold each time. Locally that is the slowest part of a run, so
//! each job keeps its workspace between runs instead: checkout brings the
//! source files up to date, and whatever the job left behind that the
//! repository ignores (build output, installed dependencies, caches) is
//! still there next time. That makes reruns incremental for any toolchain,
//! without wrkflw knowing which one a job uses.
//!
//! The workspace is keyed by repository, workflow and job (matrix leg
//! included). A run that finds it in use by another run gets a fresh
//! directory, as does every job when `run_options::fresh_workspaces` is on.

use std::fs::File;
use std::path::{Path, PathBuf};

pub(crate) enum JobWorkspace {
    /// Kept between runs; the lock is held for as long as the job runs.
    Kept { path: PathBuf, _lock: File },
    /// Deleted when the job ends.
    Fresh(tempfile::TempDir),
}

impl JobWorkspace {
    pub(crate) fn path(&self) -> &Path {
        match self {
            JobWorkspace::Kept { path, .. } => path,
            JobWorkspace::Fresh(dir) => dir.path(),
        }
    }

    /// The workspace for `job` of `workflow`, run from `repo`.
    pub(crate) fn acquire(repo: &Path, workflow: &str, job: &str) -> std::io::Result<Self> {
        if crate::run_options::fresh_workspaces() {
            return Self::fresh();
        }
        let Some(root) = workspaces_root() else {
            return Self::fresh();
        };
        match Self::keep(&root, repo, workflow, job) {
            Ok(Some(workspace)) => Ok(workspace),
            Ok(None) => {
                wrkflw_logging::info(&format!(
                    "Job '{}': its saved workspace is in use by another run, so this run \
                     starts from an empty one",
                    job
                ));
                Self::fresh()
            }
            Err(e) => {
                wrkflw_logging::warning(&format!(
                    "Job '{}': can't use a saved workspace under {} ({}), so this run starts \
                     from an empty one",
                    job,
                    root.display(),
                    e
                ));
                Self::fresh()
            }
        }
    }

    fn fresh() -> std::io::Result<Self> {
        Ok(JobWorkspace::Fresh(tempfile::tempdir()?))
    }

    /// `None` when another run holds the workspace.
    fn keep(root: &Path, repo: &Path, workflow: &str, job: &str) -> std::io::Result<Option<Self>> {
        let repo = repo.canonicalize().unwrap_or_else(|_| repo.to_path_buf());
        let key = format!("{}\0{}\0{}", repo.display(), workflow, job);
        let repo_name = repo
            .file_name()
            .map(|n| n.to_string_lossy().into_owned())
            .unwrap_or_default();
        let dir = root
            .join(format!(
                "{}-{:012x}",
                slug(&repo_name),
                short_hash(&repo.to_string_lossy())
            ))
            .join(format!("{}-{:012x}", slug(job), short_hash(&key)));
        std::fs::create_dir_all(&dir)?;

        // The lock file sits beside the workspace, not in it, so the job
        // can't see or remove it.
        let lock = File::options()
            .create(true)
            .truncate(false)
            .write(true)
            .open(dir.with_extension("lock"))?;
        match lock.try_lock() {
            Ok(()) => {}
            Err(std::fs::TryLockError::WouldBlock) => return Ok(None),
            Err(std::fs::TryLockError::Error(e)) => return Err(e),
        }
        wrkflw_logging::debug(&format!(
            "Job '{}' reuses its workspace at {}",
            job,
            dir.display()
        ));
        Ok(Some(JobWorkspace::Kept {
            path: dir,
            _lock: lock,
        }))
    }
}

/// Where kept workspaces live: `<cache dir>/wrkflw/workspaces`. Deleting it
/// is always safe; jobs then start cold once.
pub fn workspaces_root() -> Option<PathBuf> {
    Some(dirs::cache_dir()?.join("wrkflw").join("workspaces"))
}

fn slug(text: &str) -> String {
    let slug: String = text
        .chars()
        .map(|c| {
            if c.is_ascii_alphanumeric() {
                c.to_ascii_lowercase()
            } else {
                '-'
            }
        })
        .collect::<String>()
        .split('-')
        .filter(|part| !part.is_empty())
        .collect::<Vec<_>>()
        .join("-");
    slug.chars().take(40).collect()
}

/// FNV-1a, truncated: stable across toolchains, unlike `DefaultHasher`.
fn short_hash(text: &str) -> u64 {
    let mut hash: u64 = 14695981039346656037;
    for byte in text.bytes() {
        hash ^= u64::from(byte);
        hash = hash.wrapping_mul(1099511628211);
    }
    hash & 0xffff_ffff_ffff
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_job_gets_the_same_workspace_on_every_run() {
        let root = tempfile::tempdir().unwrap();
        let repo = tempfile::tempdir().unwrap();
        let first = JobWorkspace::keep(root.path(), repo.path(), "CI", "build (os: a)")
            .unwrap()
            .unwrap();
        let path = first.path().to_path_buf();
        std::fs::write(path.join("out.o"), "built").unwrap();
        drop(first);

        let second = JobWorkspace::keep(root.path(), repo.path(), "CI", "build (os: a)")
            .unwrap()
            .unwrap();
        assert_eq!(second.path(), path);
        assert!(second.path().join("out.o").exists());
    }

    #[test]
    fn jobs_and_legs_get_their_own_workspace() {
        let root = tempfile::tempdir().unwrap();
        let repo = tempfile::tempdir().unwrap();
        let a = JobWorkspace::keep(root.path(), repo.path(), "CI", "build (os: a)")
            .unwrap()
            .unwrap();
        let b = JobWorkspace::keep(root.path(), repo.path(), "CI", "build (os: b)")
            .unwrap()
            .unwrap();
        let c = JobWorkspace::keep(root.path(), repo.path(), "Release", "build (os: a)")
            .unwrap()
            .unwrap();
        assert_ne!(a.path(), b.path());
        assert_ne!(a.path(), c.path());
    }

    #[test]
    fn a_workspace_in_use_is_not_shared() {
        let root = tempfile::tempdir().unwrap();
        let repo = tempfile::tempdir().unwrap();
        let _held = JobWorkspace::keep(root.path(), repo.path(), "CI", "lint")
            .unwrap()
            .unwrap();
        assert!(JobWorkspace::keep(root.path(), repo.path(), "CI", "lint")
            .unwrap()
            .is_none());
    }
}
