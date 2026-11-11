//! How much memory the Docker or Podman VM has, and what that means for a run.
//!
//! GitHub's hosted Linux runners have 16 GB. Docker Desktop and a Podman
//! machine start with half that or less, and every job of a run shares it.
//! A job that fits on GitHub, such as cargo linking test binaries, can be
//! killed here for lack of memory and look like a broken build. So the run
//! says so up front, and runs fewer jobs at once while memory is short:
//! each job sets aside what it used on its last runs (see
//! [`crate::job_memory`]), or, never measured, a share of 4 GB or more.

use std::sync::{Arc, OnceLock};
use std::time::{Duration, Instant};
use tokio::sync::{OwnedSemaphorePermit, Semaphore};

const GIB: u64 = 1024 * 1024 * 1024;

/// Memory of a GitHub-hosted Linux runner for a public repository.
pub const GITHUB_RUNNER_BYTES: u64 = 16 * GIB;

/// Below this, the run warns: the VM has noticeably less than a runner.
const WARN_BELOW_BYTES: u64 = 14 * GIB;

/// Memory set aside for each job running at once while memory is short.
const BYTES_PER_JOB: u64 = 4 * GIB;

/// Env var that sets how many jobs run at once, overriding the default.
pub const MAX_PARALLEL_JOBS_ENV: &str = "WRKFLW_MAX_PARALLEL_JOBS";

/// How long `docker info` may take before it's given up on.
const QUERY_TIMEOUT: Duration = Duration::from_secs(10);

const MIB: u64 = 1024 * 1024;

/// How a run limits the jobs it runs at once.
#[derive(Debug, Clone, Copy, PartialEq)]
pub(crate) enum Limit {
    /// At most this many, as [`MAX_PARALLEL_JOBS_ENV`] asks.
    Jobs(usize),
    /// Jobs share `bytes`, less what the VM keeps for itself, by what each
    /// was measured to use. A job never measured gets an equal share of
    /// that among `slots`, so as many run at once as always did.
    Memory { bytes: u64, slots: usize },
}

/// What a VM of `bytes` keeps for itself: the engine, its file share, the
/// workspace helper and image builds, which no job's measurement counts.
/// A tenth of it, and at least 1 GiB.
fn vm_reserved(bytes: u64) -> u64 {
    (bytes / 10).max(1024 * MIB).min(bytes / 2)
}

/// The memory jobs share on a VM of `bytes`.
fn shared(bytes: u64) -> u64 {
    bytes - vm_reserved(bytes)
}

/// One run's pool: places for [`Limit::Jobs`], MiB for [`Limit::Memory`].
struct Pool {
    places: Arc<Semaphore>,
    limit: Limit,
    /// Told when a job of the run has kept its measurements.
    measured: tokio::sync::Notify,
}

impl Pool {
    fn new(limit: Limit) -> Pool {
        let places = match limit {
            Limit::Jobs(jobs) => jobs,
            Limit::Memory { bytes, .. } => mib(shared(bytes)),
        };
        Pool {
            places: Arc::new(Semaphore::new(places.clamp(1, Semaphore::MAX_PERMITS))),
            limit,
            measured: tokio::sync::Notify::new(),
        }
    }

    /// The places a job takes, given the room it was measured to need.
    fn weight(&self, measured: Option<u64>) -> u32 {
        let (total, wanted) = match self.limit {
            Limit::Jobs(_) => return 1,
            Limit::Memory { bytes, slots } => {
                let shared = shared(bytes);
                let unknown = shared / slots.max(1) as u64;
                (mib(shared), mib(measured.unwrap_or(unknown)))
            }
        };
        u32::try_from(wanted.clamp(1, total.max(1))).unwrap_or(u32::MAX)
    }
}

fn mib(bytes: u64) -> usize {
    usize::try_from(bytes / MIB).unwrap_or(usize::MAX)
}

/// One run's pool: set once by [`check`] when the run knows its runtime,
/// `None` inside it for no limit.
type RunPool = Arc<OnceLock<Option<Arc<Pool>>>>;

tokio::task_local! {
    /// The pool of the run being executed. Every job and every matrix leg
    /// that runs steps holds a place from it, so a run never has more of
    /// them going at once than the limit, however jobs and legs are nested.
    /// It is the run's own: two runs in one process, as in the MCP server,
    /// never share or resize each other's.
    static POOL: RunPool;
}

/// Run `future` as one run, with a job pool of its own for [`check`] to
/// size.
pub(crate) async fn with_run_pool<F: std::future::Future>(future: F) -> F::Output {
    POOL.scope(Arc::new(OnceLock::new()), future).await
}

/// Wait for a place to run a job or matrix leg in, and hold it until the
/// returned permit drops. `None` when nothing limits how many run at once,
/// or outside a run.
///
/// Take it only around a job's own steps: a job that calls a reusable
/// workflow runs that workflow's jobs, and holding a place while they wait
/// for one would deadlock a limit of one.
///
/// `job` is the job's [`crate::job_memory::job_id`], whose measurements
/// size its place when jobs share memory. `family` is the id the legs of
/// its matrix share, for a leg: one never measured takes what another leg
/// was, and while it waits for a place it looks again whenever a job of
/// the run keeps its measurements, so only the first leg of a new matrix
/// runs alone.
pub(crate) async fn job_slot(
    job: Option<&str>,
    family: Option<&str>,
) -> Option<OwnedSemaphorePermit> {
    let pool = POOL.try_with(|pool| pool.get().cloned().flatten()).ok()??;
    let (Limit::Memory { .. }, Some(job)) = (pool.limit, job) else {
        let weight = pool.weight(None);
        return pool.places.clone().acquire_many_owned(weight).await.ok();
    };
    loop {
        // Listening before the store is read, so a measurement kept in
        // between isn't missed.
        let changed = pool.measured.notified();
        tokio::pin!(changed);
        changed.as_mut().enable();
        let measured = {
            let job = job.to_string();
            let family = family.map(str::to_string);
            // The store is a file under a lock: not for the async runtime.
            tokio::task::spawn_blocking(move || crate::job_memory::reserve(&job, family.as_deref()))
                .await
                .ok()
                .flatten()
        };
        let place = pool
            .places
            .clone()
            .acquire_many_owned(pool.weight(measured));
        if measured.is_some() || family.is_none() {
            return place.await.ok();
        }
        tokio::select! {
            // A place freed by the leg that was just measured comes with
            // its measurement: read it before taking the place.
            biased;
            _ = &mut changed => {}
            place = place => return place.ok(),
        }
    }
}

/// A job of this run kept its measurements: legs waiting unmeasured look
/// again.
pub(crate) fn measurements_changed() {
    if let Ok(Some(pool)) = POOL.try_with(|pool| pool.get().cloned().flatten()) {
        pool.measured.notify_waiters();
    }
}

/// Whether this run sizes jobs by the memory they were measured to use, so
/// what they use now is worth keeping for the next run.
pub(crate) fn sizes_by_memory() -> bool {
    POOL.try_with(|pool| pool.get().cloned().flatten())
        .ok()
        .flatten()
        .is_some_and(|pool| matches!(pool.limit, Limit::Memory { .. }))
}

fn env_limit() -> Option<usize> {
    parse_limit(&std::env::var(MAX_PARALLEL_JOBS_ENV).ok()?)
}

/// A job limit as [`MAX_PARALLEL_JOBS_ENV`] gives it: a positive number,
/// clamped to what a semaphore can hold, since asking for more panics.
fn parse_limit(value: &str) -> Option<usize> {
    value
        .trim()
        .parse::<usize>()
        .ok()
        .filter(|limit| *limit > 0)
        .map(|limit| limit.min(Semaphore::MAX_PERMITS))
}

/// The job limit for a VM with `bytes` of memory, or `None` when it has
/// enough for jobs to run side by side as they would on GitHub.
fn limit_for(bytes: u64) -> Option<usize> {
    (bytes < WARN_BELOW_BYTES).then(|| usize::try_from(bytes / BYTES_PER_JOB).unwrap_or(1).max(1))
}

/// `bytes` as `7.7 GB`.
fn gigabytes(bytes: u64) -> String {
    format!("{:.1} GB", bytes as f64 / GIB as f64)
}

/// What a probe of a container runtime found.
#[derive(Debug, Clone, PartialEq)]
pub struct Memory {
    /// `docker` or `podman`.
    pub cli: String,
    /// Total memory the runtime's containers share.
    pub bytes: u64,
    /// Whether that memory is a VM's, as with Docker Desktop or a Podman
    /// machine, rather than this Linux machine's own.
    pub in_vm: bool,
}

impl Memory {
    /// Jobs that may run at once, or `None` for no limit.
    ///
    /// Only a VM is limited. Its memory is a hard cap without swap, so a
    /// linker that outgrows it is killed. A Linux host's own memory usually
    /// has swap and other tenants, and `bytes / 4 GB` would serialise an
    /// 8 GB machine's runs for a risk it may not have. It is warned about,
    /// and [`MAX_PARALLEL_JOBS_ENV`] limits it on request.
    pub fn limit(&self) -> Option<usize> {
        if self.in_vm {
            limit_for(self.bytes)
        } else {
            None
        }
    }

    /// How a run on this runtime shares its memory, or `None` for no limit.
    fn pool_limit(&self) -> Option<Limit> {
        self.limit().map(|slots| Limit::Memory {
            bytes: self.bytes,
            slots,
        })
    }

    /// The warning for a runtime short of a runner's memory, or `None`.
    pub fn warning(&self) -> Option<String> {
        if self.bytes >= WARN_BELOW_BYTES {
            return None;
        }
        Some(warning(self, env_limit().or(self.limit())))
    }

    /// As [`PROBED_ENV`] carries it: `docker:8217751552:vm`.
    pub fn to_env(&self) -> String {
        let place = if self.in_vm { "vm" } else { "host" };
        format!("{}:{}:{place}", self.cli, self.bytes)
    }

    fn from_env(value: &str) -> Option<Memory> {
        let mut parts = value.split(':');
        let cli = parts.next()?.to_string();
        let bytes = parts.next()?.parse().ok()?;
        let in_vm = parts.next()? == "vm";
        Some(Memory { cli, bytes, in_vm })
    }
}

/// Set by a parent, such as `wrkflw gate`, that already probed the runtime
/// and showed the warning, so each child run uses its result instead of
/// asking the runtime again. `none` when the probe found nothing.
pub const PROBED_ENV: &str = "WRKFLW_PROBED_MEMORY";

/// The warning for `memory`, running at most `limit` jobs at once, or as
/// many as the run has when `None`.
fn warning(memory: &Memory, limit: Option<usize>) -> String {
    let runtime = if memory.cli == "podman" {
        "Podman"
    } else {
        "Docker"
    };
    let (subject, way_out) = if memory.in_vm {
        (
            format!("The {runtime} VM"),
            format!("Give the {runtime} VM more memory"),
        )
    } else {
        (
            "This machine".to_string(),
            "Free memory, or run on a machine with more".to_string(),
        )
    };
    let running = match limit {
        Some(limit) => format!(
            "Jobs not measured yet run at most {limit} at a time; a job measured on an \
             earlier run sets aside what it used then, so light jobs run side by side. \
             {way_out}, or set {MAX_PARALLEL_JOBS_ENV}"
        ),
        None => format!(
            "Jobs run side by side as on GitHub. {way_out}, or set {MAX_PARALLEL_JOBS_ENV} to \
             run fewer at once"
        ),
    };
    format!(
        "{subject} has {} of memory, and GitHub's Linux runners have {}. A job that needs more, \
         such as linking large test binaries, can be killed here and pass on GitHub. {running}",
        gigabytes(memory.bytes),
        gigabytes(GITHUB_RUNNER_BYTES),
    )
}

/// The `info` command of `cli`, formatted with `format`, as one line.
///
/// This blocks for up to [`QUERY_TIMEOUT`], so async code calls it through
/// [`check`], which runs it on a blocking thread.
fn info(cli: &str, format: &str) -> Option<String> {
    let mut child = std::process::Command::new(cli)
        .args(["info", "--format", format])
        .stdin(std::process::Stdio::null())
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::null())
        .spawn()
        .ok()?;
    let deadline = Instant::now() + QUERY_TIMEOUT;
    loop {
        match child.try_wait() {
            Ok(Some(status)) if status.success() => break,
            Ok(Some(_)) | Err(_) => return None,
            Ok(None) if Instant::now() >= deadline => {
                let _ = child.kill();
                let _ = child.wait();
                return None;
            }
            Ok(None) => std::thread::sleep(Duration::from_millis(50)),
        }
    }
    let mut out = String::new();
    std::io::Read::read_to_string(&mut child.stdout.take()?, &mut out).ok()?;
    Some(out.trim().to_string())
}

/// Read `docker info` or `podman info` output for [`probe`].
fn parse_info(cli: &str, text: &str, host_is_linux: bool) -> Option<Memory> {
    let (bytes, os) = text.split_once('|').unwrap_or((text, ""));
    let bytes = bytes
        .trim()
        .parse::<u64>()
        .ok()
        .filter(|bytes| *bytes > 0)?;
    // Off Linux, containers always run in a VM. On Linux, Docker Desktop
    // runs one too; a native daemon or Podman shares the host's memory.
    let in_vm = !host_is_linux || os.contains("Docker Desktop");
    Some(Memory {
        cli: cli.to_string(),
        bytes,
        in_vm,
    })
}

/// Ask `cli` (`docker` or `podman`) how much memory its containers share.
/// Blocks for up to [`QUERY_TIMEOUT`].
pub fn probe(cli: &str) -> Option<Memory> {
    let format = if cli == "podman" {
        "{{.Host.MemTotal}}"
    } else {
        "{{.MemTotal}}|{{.OperatingSystem}}"
    };
    parse_info(cli, &info(cli, format)?, cfg!(target_os = "linux"))
}

/// The memory a parent's [`PROBED_ENV`] gives for `cli`: `None` when the
/// parent found nothing (`none`) or probed another runtime, as when it
/// resolved `auto` to Docker and this run uses Podman.
fn parent_probe(value: &str, cli: &str) -> Option<Memory> {
    Memory::from_env(value).filter(|memory| memory.cli == cli)
}

/// Set this run's job limit from the memory of the runtime it uses: `cli`
/// for Docker or Podman, `None` for a runtime without a VM to fill.
/// [`MAX_PARALLEL_JOBS_ENV`] overrides it for any runtime. The limit is
/// this run's alone, in the pool [`with_run_pool`] gave it.
///
/// The probe runs on a blocking thread, so a slow `docker info` doesn't
/// stall the async runtime. A probe a parent already made, passed in
/// [`PROBED_ENV`], is used instead, and its warning isn't shown again.
pub(crate) async fn check(cli: Option<&str>) {
    let limit = match cli {
        Some(cli) => limit_from_memory(cli).await,
        None => None,
    };
    set_limit(env_limit().map(Limit::Jobs).or(limit));
}

/// Size this run's pool for `limit` jobs at once, `None` for no limit. Only
/// the first call of a run counts: it checks its runtime once, before any
/// job runs.
fn set_limit(limit: Option<Limit>) {
    let _ = POOL.try_with(|pool| pool.set(limit.map(|limit| Arc::new(Pool::new(limit)))));
}

/// The job limit the memory of `cli`'s runtime allows, warning when it is
/// short.
async fn limit_from_memory(cli: &str) -> Option<Limit> {
    let (memory, warn) = match std::env::var(PROBED_ENV) {
        Ok(value) => (parent_probe(&value, cli), false),
        Err(_) => {
            let owned = cli.to_string();
            let memory = tokio::task::spawn_blocking(move || probe(&owned))
                .await
                .ok()
                .flatten();
            (memory, true)
        }
    };
    if let (true, Some(text)) = (warn, memory.as_ref().and_then(Memory::warning)) {
        wrkflw_logging::warning(&text);
    }
    memory.as_ref().and_then(Memory::pool_limit)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_small_vm_runs_fewer_jobs_at_once() {
        assert_eq!(limit_for(8_217_751_552), Some(1));
        assert_eq!(limit_for(2 * GIB), Some(1));
        assert_eq!(limit_for(12 * GIB), Some(3));
        assert_eq!(limit_for(GITHUB_RUNNER_BYTES), None);
    }

    #[test]
    fn a_huge_limit_is_clamped_to_what_a_pool_can_hold() {
        let huge = parse_limit(&usize::MAX.to_string()).unwrap();
        assert_eq!(huge, Semaphore::MAX_PERMITS);
        assert_eq!(Semaphore::new(huge).available_permits(), huge);
        assert_eq!(parse_limit(" 3 "), Some(3));
        assert_eq!(parse_limit("0"), None);
        assert_eq!(parse_limit("many"), None);
    }

    #[tokio::test]
    async fn every_job_and_leg_takes_its_place_from_the_runs_pool() {
        with_run_pool(async {
            set_limit(Some(Limit::Jobs(2)));
            let _a = job_slot(None, None).await.unwrap();
            let _b = job_slot(None, None).await.unwrap();
            let third = tokio::time::timeout(Duration::from_millis(50), job_slot(None, None)).await;
            assert!(
                third.is_err(),
                "a third place was handed out with a limit of two"
            );
            // A second check in the same run doesn't resize its pool.
            set_limit(Some(Limit::Jobs(3)));
            let again = tokio::time::timeout(Duration::from_millis(50), job_slot(None, None)).await;
            assert!(again.is_err(), "the run's pool was resized");
        })
        .await;
    }

    #[tokio::test]
    async fn runs_never_share_or_resize_each_others_pool() {
        // One run with a limit of one holds its only place while another
        // run, in the same process, sets no limit and runs freely.
        with_run_pool(async {
            set_limit(Some(Limit::Jobs(1)));
            let _held = job_slot(None, None).await.unwrap();
            with_run_pool(async {
                set_limit(None);
                assert!(job_slot(None, None).await.is_none());
            })
            .await;
            let blocked =
                tokio::time::timeout(Duration::from_millis(50), job_slot(None, None)).await;
            assert!(blocked.is_err(), "the other run changed this run's limit");
        })
        .await;
    }

    #[tokio::test]
    async fn measured_jobs_share_the_vm_and_unmeasured_ones_keep_their_room() {
        const GIB: u64 = 1024 * MIB;
        // An 8 GB VM used to run one job at a time.
        let pool = Pool::new(Limit::Memory {
            bytes: 8 * GIB,
            slots: 1,
        });
        // 1 GiB of it stays with the VM.
        assert_eq!(pool.weight(None), 7 * 1024, "an unmeasured job runs alone");
        assert_eq!(pool.weight(Some(GIB)), 1024);
        assert_eq!(
            pool.weight(Some(64 * GIB)),
            7 * 1024,
            "never more than the VM shares"
        );
        // A 12 GB VM ran three jobs at once, and still does.
        let pool = Pool::new(Limit::Memory {
            bytes: 12 * GIB,
            slots: 3,
        });
        let unmeasured = pool.weight(None) as usize;
        assert_eq!(pool.places.available_permits() / unmeasured, 3);
        assert_eq!(vm_reserved(12 * GIB), 12 * GIB / 10);
        assert_eq!(vm_reserved(GIB), GIB / 2, "a tiny VM still shares half");
        assert_eq!(Pool::new(Limit::Jobs(2)).weight(Some(GIB)), 1);

        // Seven jobs measured at 1 GB fit together, with 1 GiB left to the
        // VM, where one unmeasured job would have run alone.
        with_run_pool(async {
            set_limit(Some(Limit::Memory {
                bytes: 8 * GIB,
                slots: 1,
            }));
            let pool = POOL.with(|pool| pool.get().cloned().flatten()).unwrap();
            let held: Vec<_> = (0..7)
                .map(|_| {
                    pool.places
                        .clone()
                        .try_acquire_many_owned(pool.weight(Some(GIB)))
                        .unwrap()
                })
                .collect();
            assert!(pool.places.clone().try_acquire_many_owned(1).is_err());
            drop(held);
            let alone = pool
                .places
                .clone()
                .try_acquire_many_owned(pool.weight(None));
            assert!(alone.is_ok());
        })
        .await;
    }

    #[tokio::test]
    async fn outside_a_run_nothing_is_limited() {
        set_limit(Some(Limit::Jobs(1)));
        assert!(job_slot(None, None).await.is_none());
        assert!(job_slot(None, None).await.is_none());
    }

    fn memory(bytes: u64, in_vm: bool) -> Memory {
        Memory {
            cli: "docker".to_string(),
            bytes,
            in_vm,
        }
    }

    #[test]
    fn the_warning_names_both_sizes_and_the_way_out() {
        let text = warning(&memory(8_217_751_552, true), Some(1));
        assert!(text.contains("7.7 GB"), "{text}");
        assert!(text.contains("16.0 GB"), "{text}");
        assert!(text.contains("Give the Docker VM more memory"), "{text}");
        assert!(text.contains(MAX_PARALLEL_JOBS_ENV), "{text}");
    }

    #[test]
    fn native_linux_is_not_told_to_resize_a_vm() {
        let native = parse_info("docker", "8217751552|Ubuntu 24.04 LTS", true).unwrap();
        assert!(!native.in_vm);
        let text = warning(&native, None);
        assert!(text.starts_with("This machine has"), "{text}");
        assert!(!text.contains("VM"), "{text}");
        assert!(text.contains("side by side"), "{text}");

        let desktop = parse_info("docker", "8217751552|Docker Desktop", true).unwrap();
        assert!(desktop.in_vm);
        let mac = parse_info("podman", "8217751552", false).unwrap();
        assert!(mac.in_vm);
        assert_eq!(parse_info("docker", "|Docker Desktop", false), None);
        assert_eq!(parse_info("docker", "0|x", false), None);
    }

    #[test]
    fn only_a_vm_is_limited_but_both_are_warned_about() {
        // Native Linux keeps swap and shares memory with nothing else that
        // GitHub's runner lacks, so its runs keep their parallelism.
        assert_eq!(memory(8_217_751_552, false).limit(), None);
        assert_eq!(memory(8_217_751_552, true).limit(), Some(1));
        assert!(memory(8_217_751_552, false).warning().is_some());
        assert_eq!(memory(GITHUB_RUNNER_BYTES, false).warning(), None);
    }

    #[test]
    fn a_parent_probe_counts_only_for_the_runtime_it_probed() {
        let docker = memory(8_217_751_552, true).to_env();
        assert_eq!(
            parent_probe(&docker, "docker"),
            Some(memory(8_217_751_552, true))
        );
        assert_eq!(parent_probe(&docker, "podman"), None);
        assert_eq!(parent_probe("none", "docker"), None);
        assert_eq!(parent_probe("docker:lots:vm", "docker"), None);
    }

    #[test]
    fn a_parent_probe_travels_to_children_intact() {
        let probed = memory(8_217_751_552, true);
        assert_eq!(Memory::from_env(&probed.to_env()), Some(probed));
        assert_eq!(Memory::from_env("none"), None);
        assert_eq!(memory(GITHUB_RUNNER_BYTES, true).warning(), None);
    }
}
