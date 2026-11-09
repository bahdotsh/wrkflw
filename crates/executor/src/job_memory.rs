//! How much memory each job used, so a run can tell what fits side by side.
//!
//! On a small Docker VM a run used to set aside a fixed 4 GB per job, which
//! serialised a matrix of light test jobs on an 8 GB VM. Instead, while a
//! job's step containers run their memory is sampled, and the job's peak is
//! kept between runs. The next run sets aside that peak with headroom, so
//! jobs measured as light run together, and a job never measured still gets
//! the room it always got.
//!
//! A step killed with SIGKILL, which is how the kernel ends a process that
//! ran out of memory, forgets the job's measurements, so its next run is
//! given the old room again.

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

#[derive(Default, Serialize, Deserialize)]
struct Store {
    /// Latest peaks of each job, newest last.
    jobs: HashMap<String, Vec<u64>>,
}

static STORE_LOCK: Mutex<()> = Mutex::new(());

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
        .and_then(|bytes| serde_json::from_slice(&bytes).ok())
        .unwrap_or_default()
}

/// The room to set aside for the job `id`, if it was measured before.
pub(crate) fn reserve(id: &str) -> Option<u64> {
    let path = store_path()?;
    let _guard = STORE_LOCK.lock().unwrap_or_else(|e| e.into_inner());
    reserve_for(load(&path).jobs.get(id)?)
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
    let mut store = load(&path);
    apply(&mut store, id, outcome);
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

fn apply(store: &mut Store, id: &str, outcome: Outcome) {
    match outcome {
        Outcome::Peak(bytes) => {
            let peaks = store.jobs.entry(id.to_string()).or_default();
            peaks.push(bytes);
            let excess = peaks.len().saturating_sub(RUNS_KEPT);
            peaks.drain(..excess);
        }
        Outcome::Killed => {
            store.jobs.remove(id);
        }
        Outcome::Unknown => {}
    }
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
            apply(&mut store, "job", Outcome::Peak(peak * MIB));
        }
        assert_eq!(store.jobs["job"], vec![100 * MIB, 200 * MIB, 300 * MIB]);
        apply(&mut store, "job", Outcome::Unknown);
        assert_eq!(store.jobs["job"].len(), 3);
        apply(&mut store, "job", Outcome::Killed);
        assert!(!store.jobs.contains_key("job"));
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
