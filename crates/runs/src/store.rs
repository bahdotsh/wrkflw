//! Filesystem store: `<home>/runs/<id>/{run.json,logs/...}`.
//!
//! One directory per run, no database: a crash can only damage that run,
//! `grep` works on it, and pruning is a directory delete. Run ids sort by
//! time, so a listing is a reverse-sorted `read_dir`.

use crate::record::{RunRecord, RunStatus};
use std::fs;
use std::io::{self, Write};
use std::path::{Path, PathBuf};

/// Default number of runs kept, across all repositories.
pub const DEFAULT_KEEP: usize = 500;

/// `WRKFLW_HOME`, or the platform's local data directory.
pub fn home_dir() -> Option<PathBuf> {
    if let Some(home) = std::env::var_os("WRKFLW_HOME").filter(|h| !h.is_empty()) {
        return Some(PathBuf::from(home));
    }
    dirs::data_local_dir().map(|d| d.join("wrkflw"))
}

/// Whether history is switched off with `WRKFLW_NO_HISTORY`.
pub fn history_disabled() -> bool {
    std::env::var_os("WRKFLW_NO_HISTORY").is_some_and(|v| !v.is_empty() && v != "0")
}

#[derive(Debug, Clone)]
pub struct RunStore {
    root: PathBuf,
}

impl RunStore {
    pub fn open_default() -> Option<Self> {
        home_dir().map(|home| Self::at(home.join("runs")))
    }

    pub fn at(root: impl Into<PathBuf>) -> Self {
        Self { root: root.into() }
    }

    pub fn root(&self) -> &Path {
        &self.root
    }

    pub fn run_dir(&self, id: &str) -> PathBuf {
        self.root.join(id)
    }

    /// Runs newest first. Unreadable records are skipped: one corrupt
    /// directory must not hide the rest of the history.
    pub fn list(&self) -> Vec<RunRecord> {
        self.ids()
            .iter()
            .filter_map(|id| self.load(id).ok())
            .collect()
    }

    fn ids(&self) -> Vec<String> {
        let Ok(entries) = fs::read_dir(&self.root) else {
            return Vec::new();
        };
        let mut ids: Vec<String> = entries
            .filter_map(|entry| entry.ok())
            .filter(|entry| entry.path().join("run.json").is_file())
            .map(|entry| entry.file_name().to_string_lossy().into_owned())
            .collect();
        ids.sort_unstable_by(|a, b| b.cmp(a));
        ids
    }

    pub fn load(&self, id: &str) -> Result<RunRecord, String> {
        // Ids reach here from agents and URLs of a sort, so never let one
        // walk out of the store.
        if id.is_empty() || id.contains(['/', '\\']) || id.contains("..") {
            return Err(format!("invalid run id: {id}"));
        }
        let path = self.run_dir(id).join("run.json");
        let text = fs::read_to_string(&path).map_err(|e| format!("run {id}: {e}"))?;
        let mut record: RunRecord =
            serde_json::from_str(&text).map_err(|e| format!("run {id}: {e}"))?;
        if record.status == RunStatus::Running && !process_alive(record.pid) {
            record.status = RunStatus::Interrupted;
        }
        Ok(record)
    }

    /// Resolve `latest` within `repo_root`, a full id, or a unique id prefix.
    pub fn resolve(&self, reference: &str, repo_root: &str) -> Result<RunRecord, String> {
        if reference == "latest" {
            return self
                .list()
                .into_iter()
                .find(|run| run.repo_root == repo_root)
                .ok_or_else(|| format!("no recorded runs for {repo_root}"));
        }
        let matches: Vec<String> = self
            .ids()
            .into_iter()
            .filter(|id| id.starts_with(reference))
            .collect();
        match matches.as_slice() {
            [] => Err(format!("no run matches '{reference}'")),
            [only] => self.load(only),
            many if many.iter().any(|id| id == reference) => self.load(reference),
            many => Err(format!(
                "'{reference}' matches {} runs; use more of the id",
                many.len()
            )),
        }
    }

    /// Atomically replace `run.json`, so readers never see half a record.
    pub fn save(&self, record: &RunRecord) -> io::Result<()> {
        let dir = self.run_dir(&record.id);
        create_private_dir(&dir)?;
        let tmp = dir.join(format!("run.json.{}.tmp", std::process::id()));
        write_private(&tmp, serde_json::to_vec_pretty(record)?.as_slice())?;
        fs::rename(tmp, dir.join("run.json"))
    }

    pub fn write_log(&self, id: &str, relative: &str, text: &str) -> io::Result<()> {
        let path = self.run_dir(id).join(relative);
        if let Some(parent) = path.parent() {
            create_private_dir(parent)?;
        }
        write_private(&path, text.as_bytes())
    }

    pub fn read_log(&self, id: &str, relative: &str) -> io::Result<String> {
        fs::read_to_string(self.run_dir(id).join(relative))
    }

    /// Delete the oldest finished runs beyond `keep`. Returns how many went.
    pub fn prune(&self, keep: usize) -> usize {
        let mut removed = 0;
        for id in self.ids().into_iter().skip(keep) {
            let running = self
                .load(&id)
                .map(|run| run.status == RunStatus::Running)
                .unwrap_or(false);
            if !running && fs::remove_dir_all(self.run_dir(&id)).is_ok() {
                removed += 1;
            }
        }
        removed
    }
}

/// Logs can carry anything a step printed, so nobody else gets to read them.
#[cfg(unix)]
fn create_private_dir(path: &Path) -> io::Result<()> {
    use std::os::unix::fs::{DirBuilderExt, PermissionsExt};
    fs::DirBuilder::new()
        .recursive(true)
        .mode(0o700)
        .create(path)?;
    fs::set_permissions(path, fs::Permissions::from_mode(0o700))
}

/// On Windows the per-user data directory's ACL already restricts access.
#[cfg(not(unix))]
fn create_private_dir(path: &Path) -> io::Result<()> {
    fs::create_dir_all(path)
}

#[cfg(unix)]
fn write_private(path: &Path, bytes: &[u8]) -> io::Result<()> {
    use std::os::unix::fs::OpenOptionsExt;
    let mut file = fs::OpenOptions::new()
        .create(true)
        .write(true)
        .truncate(true)
        .mode(0o600)
        .open(path)?;
    file.write_all(bytes)
}

#[cfg(not(unix))]
fn write_private(path: &Path, bytes: &[u8]) -> io::Result<()> {
    fs::File::create(path)?.write_all(bytes)
}

#[cfg(unix)]
fn process_alive(pid: u32) -> bool {
    use nix::errno::Errno;
    use nix::sys::signal::kill;
    use nix::unistd::Pid;
    !matches!(kill(Pid::from_raw(pid as i32), None), Err(Errno::ESRCH))
}

/// No cheap liveness check here. Reporting a dead run as running is safer
/// than the reverse, because pruning would delete a live run's directory.
#[cfg(not(unix))]
fn process_alive(_pid: u32) -> bool {
    true
}

#[cfg(test)]
mod tests {
    use super::*;

    fn record(id: &str, repo: &str) -> RunRecord {
        RunRecord {
            schema_version: 1,
            id: id.to_string(),
            status: RunStatus::Success,
            workflow: "ci.yml".to_string(),
            cwd: repo.to_string(),
            repo_root: repo.to_string(),
            git_sha: None,
            git_branch: None,
            git_dirty: None,
            runtime: "emulation".to_string(),
            event: None,
            source: "cli".to_string(),
            agent: None,
            intent: None,
            replay_of: None,
            argv: Vec::new(),
            variables: Vec::new(),
            pid: std::process::id(),
            started_at: String::new(),
            finished_at: None,
            duration_ms: None,
            error: None,
            skip_reason: None,
            jobs: Vec::new(),
        }
    }

    #[test]
    fn resolve_handles_latest_prefixes_and_ambiguity() {
        let dir = tempfile::tempdir().unwrap();
        let store = RunStore::at(dir.path());
        store.save(&record("20260101-a", "/one")).unwrap();
        store.save(&record("20260102-b", "/one")).unwrap();
        store.save(&record("20260103-c", "/two")).unwrap();
        assert_eq!(store.resolve("latest", "/one").unwrap().id, "20260102-b");
        assert_eq!(store.resolve("20260101", "/two").unwrap().id, "20260101-a");
        assert!(store
            .resolve("2026", "/one")
            .unwrap_err()
            .contains("matches 3"));
        assert!(store.resolve("nope", "/one").is_err());
    }

    #[test]
    fn ids_cannot_escape_the_store() {
        let dir = tempfile::tempdir().unwrap();
        let store = RunStore::at(dir.path());
        assert!(store.load("../secrets").is_err());
        assert!(store.load("a/b").is_err());
    }

    #[cfg(unix)]
    #[test]
    fn a_dead_running_process_reads_as_interrupted() {
        let dir = tempfile::tempdir().unwrap();
        let store = RunStore::at(dir.path());
        let mut run = record("r", "/one");
        run.status = RunStatus::Running;
        run.pid = i32::MAX as u32;
        store.save(&run).unwrap();
        assert_eq!(store.load("r").unwrap().status, RunStatus::Interrupted);
    }

    #[test]
    fn a_corrupt_record_does_not_hide_the_others() {
        let dir = tempfile::tempdir().unwrap();
        let store = RunStore::at(dir.path());
        store.save(&record("good", "/one")).unwrap();
        fs::create_dir(dir.path().join("zbad")).unwrap();
        fs::write(dir.path().join("zbad/run.json"), "{").unwrap();
        assert_eq!(store.list().len(), 1);
    }

    #[cfg(unix)]
    #[test]
    fn run_data_is_private_to_the_owner() {
        use std::os::unix::fs::PermissionsExt;
        let dir = tempfile::tempdir().unwrap();
        let store = RunStore::at(dir.path());
        store.save(&record("r", "/one")).unwrap();
        store.write_log("r", "logs/a.log", "text").unwrap();
        let mode = |p: PathBuf| fs::metadata(p).unwrap().permissions().mode() & 0o777;
        assert_eq!(mode(dir.path().join("r")), 0o700);
        assert_eq!(mode(dir.path().join("r/run.json")), 0o600);
        assert_eq!(mode(dir.path().join("r/logs/a.log")), 0o600);
    }
}
