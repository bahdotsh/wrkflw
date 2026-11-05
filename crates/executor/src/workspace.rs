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
//! The workspace is keyed by repository, workflow file and job (matrix leg
//! included). A run that finds it in use by another run gets a fresh
//! directory, as does every job when `run_options::fresh_workspaces` is on.
//! `wrkflw clean` removes every workspace no run is using.

use std::fs::File;
use std::future::Future;
use std::path::{Path, PathBuf};

tokio::task_local! {
    /// The workflow file being run, so two files that share a workflow
    /// name don't share workspaces.
    static WORKFLOW_FILE: PathBuf;
}

/// Run `future` with `path` as the workflow file its jobs belong to.
pub(crate) async fn with_workflow_file<F: Future>(path: &Path, future: F) -> F::Output {
    let path = path.canonicalize().unwrap_or_else(|_| path.to_path_buf());
    WORKFLOW_FILE.scope(path, future).await
}

fn workflow_file() -> Option<PathBuf> {
    WORKFLOW_FILE.try_with(|path| path.clone()).ok()
}

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
        let file = workflow_file();
        let file = file.as_deref().unwrap_or(Path::new(""));
        match Self::keep(&root, repo, file, workflow, job) {
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

    /// `None` when another run holds the workspace. `file` is the workflow
    /// file; `workflow` its name, which tells apart the workflows a file
    /// calls.
    fn keep(
        root: &Path,
        repo: &Path,
        file: &Path,
        workflow: &str,
        job: &str,
    ) -> std::io::Result<Option<Self>> {
        let repo = repo.canonicalize().unwrap_or_else(|_| repo.to_path_buf());
        let file = file.strip_prefix(&repo).unwrap_or(file);
        let key = format!(
            "{}\0{}\0{}\0{}",
            repo.display(),
            file.display(),
            workflow,
            job
        );
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
        let reused = dir.is_dir();
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
        // Said at info level: a job that skips checkout now finds the last
        // run's files, which is worth knowing when a result surprises.
        if reused {
            wrkflw_logging::info(&format!(
                "Job '{}' reuses its workspace from an earlier run (--fresh-workspace for an \
                 empty one)",
                job
            ));
        }
        wrkflw_logging::debug(&format!("Job '{}' workspace: {}", job, dir.display()));
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

/// What [`clean`] did.
#[derive(Debug, Default, PartialEq, Eq)]
pub struct CleanReport {
    pub removed: usize,
    /// Workspaces left alone because a run holds them.
    pub in_use: usize,
}

/// Remove every kept workspace no run is using.
pub fn clean() -> std::io::Result<CleanReport> {
    match workspaces_root() {
        Some(root) => clean_under(&root),
        None => Ok(CleanReport::default()),
    }
}

fn clean_under(root: &Path) -> std::io::Result<CleanReport> {
    let mut report = CleanReport::default();
    let Ok(repos) = std::fs::read_dir(root) else {
        return Ok(report);
    };
    for repo in repos.flatten() {
        let repo = repo.path();
        if !repo.is_dir() {
            continue;
        }
        for entry in std::fs::read_dir(&repo)?.flatten() {
            let dir = entry.path();
            if !dir.is_dir() || dir.is_symlink() {
                continue;
            }
            // Take the job's lock first, as a run would: a held lock means
            // a run is using the workspace right now.
            let lock_path = dir.with_extension("lock");
            let lock = File::options()
                .create(true)
                .truncate(false)
                .write(true)
                .open(&lock_path)?;
            match lock.try_lock() {
                Ok(()) => {}
                Err(std::fs::TryLockError::WouldBlock) => {
                    report.in_use += 1;
                    continue;
                }
                Err(std::fs::TryLockError::Error(e)) => return Err(e),
            }
            std::fs::remove_dir_all(&dir)?;
            let _ = std::fs::remove_file(&lock_path);
            report.removed += 1;
        }
        // Gone once every workspace in it is.
        let _ = std::fs::remove_dir(&repo);
    }
    Ok(report)
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

    const CI: &str = ".github/workflows/ci.yml";

    #[test]
    fn a_job_gets_the_same_workspace_on_every_run() {
        let root = tempfile::tempdir().unwrap();
        let repo = tempfile::tempdir().unwrap();
        let first = JobWorkspace::keep(
            root.path(),
            repo.path(),
            Path::new(CI),
            "CI",
            "build (os: a)",
        )
        .unwrap()
        .unwrap();
        let path = first.path().to_path_buf();
        std::fs::write(path.join("out.o"), "built").unwrap();
        drop(first);

        let second = JobWorkspace::keep(
            root.path(),
            repo.path(),
            Path::new(CI),
            "CI",
            "build (os: a)",
        )
        .unwrap()
        .unwrap();
        assert_eq!(second.path(), path);
        assert!(second.path().join("out.o").exists());
    }

    #[test]
    fn jobs_and_legs_get_their_own_workspace() {
        let root = tempfile::tempdir().unwrap();
        let repo = tempfile::tempdir().unwrap();
        let a = JobWorkspace::keep(
            root.path(),
            repo.path(),
            Path::new(CI),
            "CI",
            "build (os: a)",
        )
        .unwrap()
        .unwrap();
        let b = JobWorkspace::keep(
            root.path(),
            repo.path(),
            Path::new(CI),
            "CI",
            "build (os: b)",
        )
        .unwrap()
        .unwrap();
        let c = JobWorkspace::keep(
            root.path(),
            repo.path(),
            Path::new(CI),
            "Release",
            "build (os: a)",
        )
        .unwrap()
        .unwrap();
        assert_ne!(a.path(), b.path());
        assert_ne!(a.path(), c.path());
    }

    #[test]
    fn a_workspace_in_use_is_not_shared() {
        let root = tempfile::tempdir().unwrap();
        let repo = tempfile::tempdir().unwrap();
        let _held = JobWorkspace::keep(root.path(), repo.path(), Path::new(CI), "CI", "lint")
            .unwrap()
            .unwrap();
        assert!(
            JobWorkspace::keep(root.path(), repo.path(), Path::new(CI), "CI", "lint")
                .unwrap()
                .is_none()
        );
    }

    #[test]
    fn workflow_files_sharing_a_name_get_their_own_workspace() {
        let root = tempfile::tempdir().unwrap();
        let repo = tempfile::tempdir().unwrap();
        let a = JobWorkspace::keep(root.path(), repo.path(), Path::new(CI), "CI", "build")
            .unwrap()
            .unwrap();
        let b = JobWorkspace::keep(
            root.path(),
            repo.path(),
            Path::new(".github/workflows/nightly.yml"),
            "CI",
            "build",
        )
        .unwrap()
        .unwrap();
        assert_ne!(a.path(), b.path());
    }

    #[test]
    fn clean_removes_free_workspaces_and_keeps_ones_in_use() {
        let root = tempfile::tempdir().unwrap();
        let repo = tempfile::tempdir().unwrap();
        let free = JobWorkspace::keep(root.path(), repo.path(), Path::new(CI), "CI", "lint")
            .unwrap()
            .unwrap();
        let free_path = free.path().to_path_buf();
        std::fs::write(free_path.join("out.o"), "built").unwrap();
        drop(free);
        let held = JobWorkspace::keep(root.path(), repo.path(), Path::new(CI), "CI", "test")
            .unwrap()
            .unwrap();

        let report = clean_under(root.path()).unwrap();
        assert_eq!(
            report,
            CleanReport {
                removed: 1,
                in_use: 1
            }
        );
        assert!(!free_path.exists());
        assert!(held.path().is_dir());
    }
}
