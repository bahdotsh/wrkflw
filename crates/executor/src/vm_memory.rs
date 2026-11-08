//! How much memory the Docker or Podman VM has, and what that means for a run.
//!
//! GitHub's hosted Linux runners have 16 GB. Docker Desktop and a Podman
//! machine start with half that or less, and every job of a run shares it.
//! A job that fits on GitHub, such as cargo linking test binaries, can be
//! killed here for lack of memory and look like a broken build. So the run
//! says so up front, and runs fewer jobs at once while memory is short.

use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};
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

/// Jobs that may run at once by the VM's memory, 0 for no limit.
static JOB_LIMIT: AtomicUsize = AtomicUsize::new(0);

/// The places running jobs share, and how many there are. Every job and
/// every matrix leg that runs steps holds one, so a run never has more of
/// them going at once than the limit, however jobs and legs are nested.
static POOL: Mutex<Option<(usize, Arc<Semaphore>)>> = Mutex::new(None);

/// How many jobs and matrix legs may run at once, or `None` for no limit:
/// `WRKFLW_MAX_PARALLEL_JOBS`, else what the VM's memory allows.
pub fn job_limit() -> Option<usize> {
    env_limit().or(match JOB_LIMIT.load(Ordering::Relaxed) {
        0 => None,
        limit => Some(limit),
    })
}

/// Wait for a place to run a job or matrix leg in, and hold it until the
/// returned permit drops. `None` when nothing limits how many run at once.
///
/// Take it only around a job's own steps: a job that calls a reusable
/// workflow runs that workflow's jobs, and holding a place while they wait
/// for one would deadlock a limit of one.
pub(crate) async fn job_slot() -> Option<OwnedSemaphorePermit> {
    let limit = job_limit()?;
    pool_of(&POOL, limit).acquire_owned().await.ok()
}

/// The pool in `slot` for `limit` places: the one already there when its
/// size is right, else a new one. A run that changes the limit gets a pool
/// of the new size; jobs still holding the old one finish in it.
fn pool_of(slot: &Mutex<Option<(usize, Arc<Semaphore>)>>, limit: usize) -> Arc<Semaphore> {
    let mut pool = slot.lock().unwrap_or_else(|e| e.into_inner());
    match &*pool {
        Some((size, semaphore)) if *size == limit => Arc::clone(semaphore),
        _ => {
            let semaphore = Arc::new(Semaphore::new(limit));
            *pool = Some((limit, Arc::clone(&semaphore)));
            semaphore
        }
    }
}

fn env_limit() -> Option<usize> {
    std::env::var(MAX_PARALLEL_JOBS_ENV)
        .ok()
        .and_then(|value| value.trim().parse::<usize>().ok())
        .filter(|limit| *limit > 0)
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
    pub fn limit(&self) -> Option<usize> {
        limit_for(self.bytes)
    }

    /// The warning for a runtime short of a runner's memory, or `None`.
    pub fn warning(&self) -> Option<String> {
        let own = self.limit()?;
        Some(warning(self, env_limit().unwrap_or(own)))
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

/// The warning for `memory`, running `limit` jobs at once.
fn warning(memory: &Memory, limit: usize) -> String {
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
    format!(
        "{subject} has {} of memory, and GitHub's Linux runners have {}. A job that needs more, \
         such as linking large test binaries, can be killed here and pass on GitHub. Running at \
         most {limit} job(s) at a time. {way_out}, or set {MAX_PARALLEL_JOBS_ENV}",
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

/// Set this run's job limit from the memory of the runtime it uses: `cli`
/// for Docker or Podman, `None` for a runtime without a VM to fill. Every
/// run sets it, so a limit never carries over from an earlier run in the
/// same process, such as the MCP server.
///
/// The probe runs on a blocking thread, so a slow `docker info` doesn't
/// stall the async runtime. A probe a parent already made, passed in
/// [`PROBED_ENV`], is used instead, and its warning isn't shown again.
pub(crate) async fn check(cli: Option<&str>) {
    let Some(cli) = cli else {
        JOB_LIMIT.store(0, Ordering::Relaxed);
        return;
    };
    let (memory, warn) = match std::env::var(PROBED_ENV) {
        Ok(value) => (
            Memory::from_env(&value).filter(|memory| memory.cli == cli),
            false,
        ),
        Err(_) => {
            let owned = cli.to_string();
            let memory = tokio::task::spawn_blocking(move || probe(&owned))
                .await
                .ok()
                .flatten();
            (memory, true)
        }
    };
    let limit = memory.as_ref().and_then(Memory::limit);
    JOB_LIMIT.store(limit.unwrap_or(0), Ordering::Relaxed);
    if let (true, Some(text)) = (warn, memory.as_ref().and_then(Memory::warning)) {
        wrkflw_logging::warning(&text);
    }
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
    fn every_job_and_leg_takes_its_place_from_one_pool() {
        let slot = Mutex::new(None);
        let pool = pool_of(&slot, 2);
        assert!(Arc::ptr_eq(&pool, &pool_of(&slot, 2)));
        let _a = Arc::clone(&pool).try_acquire_owned().unwrap();
        let _b = pool_of(&slot, 2).try_acquire_owned().unwrap();
        assert!(
            pool_of(&slot, 2).try_acquire_owned().is_err(),
            "a third place was handed out with a limit of two"
        );
        let resized = pool_of(&slot, 3);
        assert!(!Arc::ptr_eq(&pool, &resized));
        assert_eq!(resized.available_permits(), 3);
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
        let text = warning(&memory(8_217_751_552, true), 1);
        assert!(text.contains("7.7 GB"), "{text}");
        assert!(text.contains("16.0 GB"), "{text}");
        assert!(text.contains("Give the Docker VM more memory"), "{text}");
        assert!(text.contains(MAX_PARALLEL_JOBS_ENV), "{text}");
    }

    #[test]
    fn native_linux_is_not_told_to_resize_a_vm() {
        let native = parse_info("docker", "8217751552|Ubuntu 24.04 LTS", true).unwrap();
        assert!(!native.in_vm);
        let text = warning(&native, 1);
        assert!(text.starts_with("This machine has"), "{text}");
        assert!(!text.contains("VM"), "{text}");

        let desktop = parse_info("docker", "8217751552|Docker Desktop", true).unwrap();
        assert!(desktop.in_vm);
        let mac = parse_info("podman", "8217751552", false).unwrap();
        assert!(mac.in_vm);
        assert_eq!(parse_info("docker", "|Docker Desktop", false), None);
        assert_eq!(parse_info("docker", "0|x", false), None);
    }

    #[test]
    fn a_parent_probe_travels_to_children_intact() {
        let probed = memory(8_217_751_552, true);
        assert_eq!(Memory::from_env(&probed.to_env()), Some(probed));
        assert_eq!(Memory::from_env("none"), None);
        assert_eq!(memory(GITHUB_RUNNER_BYTES, true).warning(), None);
    }
}
