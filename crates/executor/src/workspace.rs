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

tokio::task_local! {
    /// The jobs that called the reusable workflow being run, outermost
    /// first, so two jobs that call the same workflow with different inputs
    /// don't share its jobs' workspaces.
    static CALLERS: String;
}

/// Run `future` as part of the reusable workflow `job` calls.
pub(crate) async fn with_caller_job<F: Future>(job: &str, future: F) -> F::Output {
    let callers = match CALLERS.try_with(|outer| outer.clone()) {
        Ok(outer) => format!("{outer}\0{job}"),
        Err(_) => job.to_string(),
    };
    CALLERS.scope(callers, future).await
}

fn callers() -> Option<String> {
    CALLERS.try_with(|callers| callers.clone()).ok()
}

/// What names `job` of `workflow`, run from `repo`, across runs: the same
/// parts a kept workspace is keyed by.
pub(crate) fn job_identity(repo: &Path, workflow: &str, job: &str) -> String {
    let repo = repo.canonicalize().unwrap_or_else(|_| repo.to_path_buf());
    let file = workflow_file();
    let file = file.as_deref().unwrap_or(Path::new(""));
    let file = file.strip_prefix(&repo).unwrap_or(file);
    format!(
        "{}\0{}\0{}\0{}",
        repo.display(),
        file.display(),
        workflow_key(workflow, callers().as_deref()),
        job
    )
}

/// The workflow part of a workspace key. Keys of jobs outside a reusable
/// workflow are the workflow name alone, as before, so their saved
/// workspaces stay valid.
fn workflow_key(workflow: &str, callers: Option<&str>) -> String {
    match callers {
        Some(callers) => format!("{workflow}\0called by\0{callers}"),
        None => workflow.to_string(),
    }
}

pub(crate) enum JobWorkspace {
    /// Kept between runs; the lock is held for as long as the job runs.
    Kept { path: PathBuf, _lock: File },
    /// Deleted when the job ends.
    Fresh(tempfile::TempDir),
}

impl Drop for JobWorkspace {
    fn drop(&mut self) {
        // Before the directory goes: a fresh workspace's volume goes with it.
        let kept = matches!(self, JobWorkspace::Kept { .. });
        crate::vm_workspace::release(self.path(), kept);
    }
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
        let workflow = workflow_key(workflow, callers().as_deref());
        match Self::keep(&root, repo, file, &workflow, job) {
            Ok(Some(workspace)) => Ok(workspace),
            Ok(None) => {
                wrkflw_logging::info(&format!(
                    "Job '{}': its saved workspace is in use by another run or by `wrkflw \
                     clean`, so this run starts from an empty one",
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
        if !lock_soon(&lock)? {
            return Ok(None);
        }
        // `wrkflw clean` may have removed the directory between its creation
        // above and the lock; now that this run holds the lock it can't again.
        std::fs::create_dir_all(&dir)?;
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

/// Where checkout records what git tracked at the last sync of the kept
/// workspace `workspace`, or `None` when it isn't a kept one. The record
/// sits beside the workspace, as its lock does, so the job can't change it.
pub(crate) fn tracked_record(workspace: &Path) -> Option<PathBuf> {
    workspace
        .with_extension("lock")
        .is_file()
        .then(|| workspace.with_extension("tracked"))
}

/// Env var that moves kept workspaces out of the cache directory.
pub const WORKSPACES_DIR_ENV: &str = "WRKFLW_WORKSPACES_DIR";

/// Take `lock`, waiting briefly if it is held. A lock released a moment ago
/// can still look held: a process another thread is starting inherits the
/// lock file's handle until it execs. Only a lock held for longer than
/// that belongs to another run. `false` when it does.
fn lock_soon(lock: &File) -> std::io::Result<bool> {
    const ATTEMPTS: u32 = 10;
    for attempt in 1..=ATTEMPTS {
        match lock.try_lock() {
            Ok(()) => return Ok(true),
            Err(std::fs::TryLockError::WouldBlock) if attempt < ATTEMPTS => {
                std::thread::sleep(std::time::Duration::from_millis(20));
            }
            Err(std::fs::TryLockError::WouldBlock) => return Ok(false),
            Err(std::fs::TryLockError::Error(e)) => return Err(e),
        }
    }
    Ok(false)
}

/// Where kept workspaces live: [`WORKSPACES_DIR_ENV`] if set, else
/// `<cache dir>/wrkflw/workspaces`. Deleting it is always safe; jobs then
/// start cold once.
pub fn workspaces_root() -> Option<PathBuf> {
    root_from(std::env::var_os(WORKSPACES_DIR_ENV), dirs::cache_dir())
}

fn root_from(configured: Option<std::ffi::OsString>, cache: Option<PathBuf>) -> Option<PathBuf> {
    match configured {
        // Made absolute once, so a relative value names one directory
        // whatever the current directory, and works as a bind mount source.
        Some(dir) if !dir.is_empty() => {
            let dir = PathBuf::from(dir);
            Some(std::path::absolute(&dir).unwrap_or(dir))
        }
        _ => Some(cache?.join("wrkflw").join("workspaces")),
    }
}

/// What [`clean`] did.
#[derive(Debug, Default, PartialEq, Eq)]
pub struct CleanReport {
    pub removed: usize,
    /// Bytes the removed workspaces held. Hard-linked git objects are
    /// counted although removing a link frees nothing the repository uses.
    pub bytes: u64,
    /// Workspaces left alone because a run holds them.
    pub in_use: usize,
    /// Workspaces that could not be removed, with why. The rest are still
    /// cleaned, so one stubborn directory doesn't hide what was freed.
    pub failed: Vec<(PathBuf, String)>,
}

/// Remove every kept workspace no run is using.
pub fn clean() -> std::io::Result<CleanReport> {
    let report = match workspaces_root() {
        Some(root) => clean_under(&root),
        None => Ok(CleanReport::default()),
    };
    // A workspace's container volume is no use once its directory is gone.
    crate::vm_workspace::remove_orphaned_volumes();
    report
}

fn clean_under(root: &Path) -> std::io::Result<CleanReport> {
    let mut report = CleanReport::default();
    let repos = match std::fs::read_dir(root) {
        Ok(repos) => repos,
        // Nothing was ever kept.
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(report),
        Err(e) => return Err(e),
    };
    for repo in repos.flatten() {
        let repo = repo.path();
        if !repo.is_dir() {
            continue;
        }
        let entries = match std::fs::read_dir(&repo) {
            Ok(entries) => entries,
            Err(e) => {
                report.failed.push((repo.clone(), e.to_string()));
                continue;
            }
        };
        for entry in entries.flatten() {
            let dir = entry.path();
            if !dir.is_dir() || dir.is_symlink() {
                continue;
            }
            // Take the job's lock first, as a run would: a held lock means
            // a run is using the workspace right now.
            let lock_path = dir.with_extension("lock");
            let lock = match File::options()
                .create(true)
                .truncate(false)
                .write(true)
                .open(&lock_path)
            {
                Ok(lock) => lock,
                Err(e) => {
                    report.failed.push((dir, e.to_string()));
                    continue;
                }
            };
            match lock.try_lock() {
                Ok(()) => {}
                Err(std::fs::TryLockError::WouldBlock) => {
                    report.in_use += 1;
                    continue;
                }
                Err(std::fs::TryLockError::Error(e)) => {
                    report.failed.push((dir, e.to_string()));
                    continue;
                }
            }
            // The lock file stays. A run may already have it open, waiting
            // to lock it; removing it would let a later run create a second
            // lock file for the same workspace, and both would get it.
            let bytes = size_of(&dir);
            match std::fs::remove_dir_all(&dir) {
                Ok(()) => {
                    report.removed += 1;
                    report.bytes += bytes;
                    // The record describes the files just removed. Held
                    // under the lock, so no run is about to read it.
                    let _ = std::fs::remove_file(dir.with_extension("tracked"));
                }
                // Part of it may be gone; what is left is counted next time.
                Err(e) => report.failed.push((dir, e.to_string())),
            }
        }
    }
    Ok(report)
}

/// Total size of the files under `dir`, not following symlinks. Entries
/// that can't be read count as empty.
fn size_of(dir: &Path) -> u64 {
    let Ok(entries) = std::fs::read_dir(dir) else {
        return 0;
    };
    entries
        .flatten()
        .map(|entry| match entry.metadata() {
            Ok(meta) if meta.is_dir() => size_of(&entry.path()),
            Ok(meta) if meta.is_file() => meta.len(),
            _ => 0,
        })
        .sum()
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
    fn the_workspace_root_can_be_moved() {
        let cache = Some(PathBuf::from("/cache"));
        assert_eq!(
            root_from(None, cache.clone()),
            Some(PathBuf::from("/cache/wrkflw/workspaces"))
        );
        assert_eq!(
            root_from(Some("".into()), cache.clone()),
            Some(PathBuf::from("/cache/wrkflw/workspaces"))
        );
        // `/big/disk` is absolute on Unix; on Windows it gains the current
        // drive, as any path without one does.
        let big = std::path::absolute("/big/disk").unwrap();
        assert_eq!(
            root_from(Some("/big/disk".into()), cache),
            Some(big.clone())
        );
        assert_eq!(root_from(Some("/big/disk".into()), None), Some(big));
        let relative = root_from(Some("ws".into()), None).unwrap();
        assert!(relative.is_absolute());
        assert_eq!(relative, std::env::current_dir().unwrap().join("ws"));
    }

    #[cfg(unix)]
    #[test]
    fn clean_reports_a_root_it_cannot_read() {
        use std::os::unix::fs::PermissionsExt;
        let root = tempfile::tempdir().unwrap();
        let locked = root.path().join("locked");
        std::fs::create_dir(&locked).unwrap();
        std::fs::set_permissions(&locked, std::fs::Permissions::from_mode(0o000)).unwrap();
        let readable = std::fs::read_dir(&locked).is_ok();
        let result = clean_under(&locked);
        std::fs::set_permissions(&locked, std::fs::Permissions::from_mode(0o755)).unwrap();
        // Root can read anything; the check only means something otherwise.
        if !readable {
            assert!(result.is_err());
        }
        assert_eq!(
            clean_under(&root.path().join("missing")).unwrap(),
            CleanReport::default()
        );
    }

    #[tokio::test]
    async fn callers_of_one_reusable_workflow_get_their_own_workspaces() {
        let key = |callers: Option<&str>| workflow_key("Build", callers);
        let seen_a = with_caller_job("build-a", async { callers() }).await;
        let seen_b = with_caller_job("build-b", async { callers() }).await;
        let nested = with_caller_job("outer", with_caller_job("inner", async { callers() })).await;
        assert_eq!(seen_a.as_deref(), Some("build-a"));
        assert_eq!(nested.as_deref(), Some("outer\0inner"));
        assert_eq!(callers(), None);

        let root = tempfile::tempdir().unwrap();
        let repo = tempfile::tempdir().unwrap();
        let ws = |workflow: String| {
            JobWorkspace::keep(
                root.path(),
                repo.path(),
                Path::new(CI),
                &workflow,
                "compile",
            )
            .unwrap()
            .unwrap()
        };
        let a = ws(key(seen_a.as_deref()));
        let b = ws(key(seen_b.as_deref()));
        assert_ne!(a.path(), b.path());
        drop((a, b));
        assert_eq!(key(None), "Build");
    }

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
        assert_eq!(
            tracked_record(&free_path),
            Some(free_path.with_extension("tracked"))
        );
        std::fs::write(free_path.with_extension("tracked"), "out.o\0").unwrap();
        drop(free);
        let held = JobWorkspace::keep(root.path(), repo.path(), Path::new(CI), "CI", "test")
            .unwrap()
            .unwrap();

        let report = clean_under(root.path()).unwrap();
        assert_eq!(
            report,
            CleanReport {
                removed: 1,
                bytes: 5,
                in_use: 1,
                failed: Vec::new(),
            }
        );
        assert!(!free_path.exists());
        assert!(!free_path.with_extension("tracked").exists());
        assert!(held.path().is_dir());

        // The cleaned workspace comes back, empty, on the next run.
        let again = JobWorkspace::keep(root.path(), repo.path(), Path::new(CI), "CI", "lint")
            .unwrap()
            .unwrap();
        assert_eq!(again.path(), free_path);
        assert!(again.path().is_dir());
        assert!(!again.path().join("out.o").exists());
    }

    #[cfg(unix)]
    #[test]
    fn clean_reports_a_workspace_it_cannot_remove_and_cleans_the_rest() {
        use std::os::unix::fs::PermissionsExt;
        let root = tempfile::tempdir().unwrap();
        let repo = tempfile::tempdir().unwrap();
        let stuck = JobWorkspace::keep(root.path(), repo.path(), Path::new(CI), "CI", "a")
            .unwrap()
            .unwrap();
        let stuck_path = stuck.path().to_path_buf();
        std::fs::create_dir_all(stuck_path.join("locked")).unwrap();
        std::fs::write(stuck_path.join("locked/file"), "x").unwrap();
        drop(stuck);
        let free = JobWorkspace::keep(root.path(), repo.path(), Path::new(CI), "CI", "b")
            .unwrap()
            .unwrap();
        let free_path = free.path().to_path_buf();
        drop(free);
        // A directory whose entries can't be removed, as a container that
        // ran as another user leaves behind.
        let locked = stuck_path.join("locked");
        std::fs::set_permissions(&locked, std::fs::Permissions::from_mode(0o555)).unwrap();

        let report = clean_under(root.path()).unwrap();

        std::fs::set_permissions(&locked, std::fs::Permissions::from_mode(0o755)).unwrap();
        // Running as root removes it anyway; then there is nothing to report.
        if stuck_path.exists() {
            assert_eq!(report.failed.len(), 1, "{report:?}");
            assert_eq!(report.failed[0].0, stuck_path);
        }
        assert!(
            !free_path.exists(),
            "a failure stopped the rest of the clean"
        );
    }

    #[test]
    fn a_lock_file_survives_clean_so_a_waiting_run_still_excludes_others() {
        let root = tempfile::tempdir().unwrap();
        let repo = tempfile::tempdir().unwrap();
        let first = JobWorkspace::keep(root.path(), repo.path(), Path::new(CI), "CI", "lint")
            .unwrap()
            .unwrap();
        let lock = first.path().with_extension("lock");
        drop(first);
        clean_under(root.path()).unwrap();
        assert!(lock.is_file());
    }
}
