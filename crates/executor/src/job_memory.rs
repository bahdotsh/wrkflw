//! How much memory each job used, so a run can tell what fits side by side.
//!
//! On a small Docker VM a run used to set aside a fixed 4 GB per job, which
//! serialised a matrix of light test jobs on an 8 GB VM. Instead, while a
//! job's step containers run their memory is sampled, and the job's peak is
//! kept between runs. The next run sets aside that peak with headroom, so
//! jobs measured as light run together, and a job never measured still gets
//! the room it always got.
//!
//! A step that ran out of memory forgets the job's measurements, so its
//! next run is given the old room again. That is a step container the
//! engine reports as OOM-killed, one that exited with SIGKILL's code, or
//! one whose output reports a process killed by SIGKILL, as cargo and
//! compilers do for the `rustc` or `ld` the kernel picked.
//!
//! Only step containers are measured. Building the job's image and the
//! helper that copies its workspace run outside any job's measurement, so
//! the first run after image caches are cleared can need more than
//! measured. Containers sampled never, which most that finish within a
//! second or two are, count as using nothing.

use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::{Arc, Mutex};

use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

const MIB: u64 = 1024 * 1024;

/// Where the measurements are kept; empty turns keeping them off.
pub const JOB_MEMORY_FILE_ENV: &str = "WRKFLW_JOB_MEMORY_FILE";

/// How many of a job's latest runs its estimate looks at. A run that failed
/// early, and so used little, must not shrink a heavy job's room.
const RUNS_KEPT: usize = 3;

/// The least set aside for a measured job: sampling is about once a second,
/// so a short spike can go unseen.
const MIN_RESERVED_BYTES: u64 = 512 * MIB;

/// Added to a measured peak, on top of half of it again.
const HEADROOM_BYTES: u64 = 256 * MIB;

/// Exit code of a process killed by SIGKILL, as the kernel's out-of-memory
/// killer does.
pub(crate) const SIGKILL_EXIT: i32 = 137;

/// What tools print when the kernel kills a process they started. The
/// step then exits with the tool's own code, not SIGKILL's.
const KILLED_CHILD_SIGNS: &[&str] = &[
    // cargo, for rustc or a build script
    "(signal: 9, SIGKILL",
    // gcc and collect2, for ld or cc1
    "terminated with signal 9",
    "Killed signal terminated program",
];

/// Whether a step container ended as the out-of-memory killer ends one:
/// the engine says so, it exited as SIGKILL leaves a shell, or a tool in it
/// reports a child killed that way.
pub(crate) fn ran_out_of_memory(exit_code: i32, oom_killed: bool, output: &[&str]) -> bool {
    exit_code == SIGKILL_EXIT
        || oom_killed
        || (exit_code != 0
            && output
                .iter()
                .any(|text| KILLED_CHILD_SIGNS.iter().any(|sign| text.contains(sign))))
}

/// What a job's containers used while it ran.
#[derive(Default)]
pub(crate) struct JobMeter {
    /// Latest working set of each running container.
    current: Mutex<HashMap<String, u64>>,
    /// Most the job's containers used at once.
    peak: AtomicU64,
    /// Some container sent at least one sample.
    sampled: AtomicBool,
    /// A container ran that couldn't be measured, so the peak is too low.
    incomplete: AtomicBool,
    /// A container was killed as if out of memory.
    killed: AtomicBool,
}

impl JobMeter {
    /// `container` uses `bytes` now.
    pub(crate) fn sample(&self, container: &str, bytes: u64) {
        let mut current = self.current.lock().unwrap_or_else(|e| e.into_inner());
        current.insert(container.to_string(), bytes);
        let total: u64 = current.values().sum();
        self.peak.fetch_max(total, Ordering::Relaxed);
        self.sampled.store(true, Ordering::Relaxed);
    }

    /// `container` stopped.
    pub(crate) fn stopped(&self, container: &str) {
        self.current
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .remove(container);
    }

    /// A container ran that this meter couldn't see.
    pub(crate) fn unmeasured(&self) {
        self.incomplete.store(true, Ordering::Relaxed);
    }

    /// A container ended as the out-of-memory killer ends one.
    pub(crate) fn killed(&self) {
        self.killed.store(true, Ordering::Relaxed);
    }

    #[cfg(test)]
    pub(crate) fn is_incomplete(&self) -> bool {
        self.incomplete.load(Ordering::Relaxed)
    }

    fn outcome(&self) -> Outcome {
        if self.killed.load(Ordering::Relaxed) {
            Outcome::Killed
        } else if self.incomplete.load(Ordering::Relaxed) || !self.sampled.load(Ordering::Relaxed) {
            Outcome::Unknown
        } else {
            Outcome::Peak(self.peak.load(Ordering::Relaxed))
        }
    }
}

#[derive(Debug, PartialEq)]
enum Outcome {
    Peak(u64),
    Killed,
    Unknown,
}

tokio::task_local! {
    static METER: Arc<JobMeter>;
}

/// Run `future`, one job or matrix leg, with a meter of its own.
pub(crate) async fn metered<F: std::future::Future>(future: F) -> F::Output {
    METER.scope(Arc::new(JobMeter::default()), future).await
}

/// The meter of the job being run, if any.
pub(crate) fn meter() -> Option<Arc<JobMeter>> {
    METER.try_with(Arc::clone).ok()
}

/// Mark the current job's measurement incomplete, for a container this
/// module can't sample.
pub(crate) fn unmeasured() {
    if let Some(meter) = meter() {
        meter.unmeasured();
    }
}

/// Keeps what the current job's meter saw once the job is over.
pub(crate) struct Recorder {
    id: String,
    meter: Option<Arc<JobMeter>>,
}

impl Recorder {
    pub(crate) fn new(id: String) -> Recorder {
        Recorder { id, meter: meter() }
    }
}

impl Drop for Recorder {
    fn drop(&mut self) {
        if let Some(meter) = &self.meter {
            record(&self.id, meter);
        }
    }
}

/// What identifies a job across runs, without saying which repository it is.
pub(crate) fn job_id(identity: &str) -> String {
    let digest = Sha256::digest(identity.as_bytes());
    digest[..12].iter().map(|b| format!("{:02x}", b)).collect()
}

/// The room to set aside for a job whose recent peaks are `peaks`, or
/// `None` when it has none.
fn reserve_for(peaks: &[u64]) -> Option<u64> {
    let peak = peaks.iter().copied().max()?;
    Some((peak + peak / 2 + HEADROOM_BYTES).max(MIN_RESERVED_BYTES))
}

/// The store's format. A file in any other is read as empty.
const STORE_VERSION: u32 = 1;

/// A job not run for this long is forgotten, so renamed jobs, workflows
/// and repositories don't pile up.
const FORGET_AFTER_SECS: i64 = 90 * 24 * 60 * 60;

#[derive(Serialize, Deserialize)]
struct Store {
    #[serde(default)]
    version: u32,
    jobs: HashMap<String, Measured>,
}

impl Default for Store {
    fn default() -> Self {
        Store {
            version: STORE_VERSION,
            jobs: HashMap::new(),
        }
    }
}

#[derive(Debug, Default, Clone, PartialEq, Serialize, Deserialize)]
struct Measured {
    /// Latest peaks, newest last.
    peaks: Vec<u64>,
    /// When the last of them was taken, in seconds since the epoch.
    at: i64,
}

/// Serialises this process's use of the store; the lock file does the same
/// for other processes, such as two runs in two terminals.
static STORE_LOCK: Mutex<()> = Mutex::new(());

/// Hold the store's lock file, shared for reading or exclusive for writing.
fn lock_file(path: &std::path::Path, exclusive: bool) -> Option<std::fs::File> {
    let dir = path.parent()?;
    std::fs::create_dir_all(dir).ok()?;
    let file = std::fs::File::options()
        .create(true)
        .truncate(false)
        .write(true)
        .open(path.with_extension("lock"))
        .ok()?;
    let locked = if exclusive {
        file.lock()
    } else {
        file.lock_shared()
    };
    locked.ok().map(|()| file)
}

fn store_path() -> Option<PathBuf> {
    match std::env::var_os(JOB_MEMORY_FILE_ENV) {
        Some(path) if path.is_empty() => None,
        Some(path) => Some(PathBuf::from(path)),
        None => Some(dirs::cache_dir()?.join("wrkflw").join("job-memory.json")),
    }
}

fn load(path: &std::path::Path) -> Store {
    std::fs::read(path)
        .ok()
        .and_then(|bytes| serde_json::from_slice::<Store>(&bytes).ok())
        .filter(|store| store.version == STORE_VERSION)
        .unwrap_or_default()
}

/// The room to set aside for the job `id`, if it was measured before.
pub(crate) fn reserve(id: &str) -> Option<u64> {
    let path = store_path()?;
    let _guard = STORE_LOCK.lock().unwrap_or_else(|e| e.into_inner());
    let _file = lock_file(&path, false);
    reserve_for(&load(&path).jobs.get(id)?.peaks)
}

/// Keep what `meter` saw of the job `id`.
pub(crate) fn record(id: &str, meter: &JobMeter) {
    let Some(path) = store_path() else {
        return;
    };
    let outcome = meter.outcome();
    if outcome == Outcome::Unknown {
        return;
    }
    let _guard = STORE_LOCK.lock().unwrap_or_else(|e| e.into_inner());
    // Held from load to write, so another process's update isn't lost,
    // least of all one that forgets a job that ran out of memory.
    let _file = lock_file(&path, true);
    let mut store = load(&path);
    apply(&mut store, id, outcome, chrono::Utc::now().timestamp());
    let written = (|| -> std::io::Result<()> {
        let dir = path.parent().unwrap_or(std::path::Path::new("."));
        std::fs::create_dir_all(dir)?;
        let temp = tempfile::NamedTempFile::new_in(dir)?;
        serde_json::to_writer(&temp, &store)?;
        temp.persist(&path).map_err(|e| e.error)?;
        Ok(())
    })();
    if let Err(e) = written {
        wrkflw_logging::debug(&format!(
            "Could not keep job memory in {}: {}",
            path.display(),
            e
        ));
    }
}

fn apply(store: &mut Store, id: &str, outcome: Outcome, now: i64) {
    match outcome {
        Outcome::Peak(bytes) => {
            let measured = store.jobs.entry(id.to_string()).or_default();
            measured.peaks.push(bytes);
            let excess = measured.peaks.len().saturating_sub(RUNS_KEPT);
            measured.peaks.drain(..excess);
            measured.at = now;
        }
        Outcome::Killed => {
            store.jobs.remove(id);
        }
        Outcome::Unknown => {}
    }
    store
        .jobs
        .retain(|_, measured| now - measured.at < FORGET_AFTER_SECS);
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_job_gets_its_peak_with_headroom_and_never_less_than_the_floor() {
        assert_eq!(reserve_for(&[]), None);
        assert_eq!(reserve_for(&[100 * MIB]), Some(MIN_RESERVED_BYTES));
        assert_eq!(
            reserve_for(&[2048 * MIB, 1024 * MIB]),
            Some(3072 * MIB + HEADROOM_BYTES)
        );
    }

    #[test]
    fn the_estimate_follows_the_latest_runs_and_a_kill_forgets_it() {
        let mut store = Store::default();
        for peak in [900, 100, 200, 300] {
            apply(&mut store, "job", Outcome::Peak(peak * MIB), 1000);
        }
        assert_eq!(
            store.jobs["job"].peaks,
            vec![100 * MIB, 200 * MIB, 300 * MIB]
        );
        apply(&mut store, "job", Outcome::Unknown, 1000);
        assert_eq!(store.jobs["job"].peaks.len(), 3);
        apply(&mut store, "job", Outcome::Killed, 1000);
        assert!(!store.jobs.contains_key("job"));
    }

    #[test]
    fn a_job_not_run_for_a_long_time_is_forgotten() {
        let mut store = Store::default();
        apply(&mut store, "old", Outcome::Peak(MIB), 0);
        apply(&mut store, "new", Outcome::Peak(MIB), FORGET_AFTER_SECS);
        assert!(!store.jobs.contains_key("old"));
        assert!(store.jobs.contains_key("new"));
    }

    #[test]
    fn measurements_survive_a_round_trip_and_another_format_reads_as_empty() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("job-memory.json");
        let mut store = Store::default();
        apply(&mut store, "job", Outcome::Peak(MIB), 1);
        std::fs::write(&path, serde_json::to_vec(&store).unwrap()).unwrap();
        assert_eq!(load(&path).jobs["job"].peaks, vec![MIB]);
        std::fs::write(&path, br#"{"jobs": {"job": [1048576]}}"#).unwrap();
        assert!(load(&path).jobs.is_empty());
        std::fs::write(&path, b"not json").unwrap();
        assert!(load(&path).jobs.is_empty());
        // The lock file sits beside the store and can be taken twice in turn.
        drop(lock_file(&path, true).unwrap());
        assert!(lock_file(&path, false).is_some());
    }

    #[test]
    fn running_out_of_memory_is_seen_through_the_tool_that_reports_it() {
        assert!(ran_out_of_memory(SIGKILL_EXIT, false, &[]));
        assert!(ran_out_of_memory(1, true, &[]));
        let cargo = "error: could not compile `big` (lib)\n\nCaused by:\n  process didn't \
                     exit successfully: `rustc ...` (signal: 9, SIGKILL: kill)";
        assert!(ran_out_of_memory(101, false, &["", cargo]));
        let gcc = "collect2: fatal error: ld terminated with signal 9 [Killed]";
        assert!(ran_out_of_memory(1, false, &[gcc]));
        assert!(!ran_out_of_memory(1, false, &["test failed"]));
        // A step that passed wasn't killed, whatever it printed.
        assert!(!ran_out_of_memory(0, false, &[cargo]));
    }

    #[test]
    fn concurrent_containers_add_up_and_a_gap_means_unknown() {
        let meter = JobMeter::default();
        assert_eq!(meter.outcome(), Outcome::Unknown);
        meter.sample("job", 300);
        meter.sample("service", 200);
        meter.sample("job", 100);
        meter.stopped("service");
        meter.sample("job", 250);
        assert_eq!(meter.outcome(), Outcome::Peak(500));
        meter.unmeasured();
        assert_eq!(meter.outcome(), Outcome::Unknown);
        meter.killed();
        assert_eq!(meter.outcome(), Outcome::Killed);
    }

    #[tokio::test]
    async fn each_job_has_its_own_meter() {
        assert!(meter().is_none());
        // Both meters are held to the end, so neither address can be reused.
        let (a, b) = tokio::join!(
            metered(async { meter().unwrap() }),
            metered(async { meter().unwrap() })
        );
        assert!(!Arc::ptr_eq(&a, &b));
        a.sample("step", 1);
        assert_eq!(b.outcome(), Outcome::Unknown);
    }
}
