//! How much memory the Docker or Podman VM has, and what that means for a run.
//!
//! GitHub's hosted Linux runners have 16 GB. Docker Desktop and a Podman
//! machine start with half that or less, and every job of a run shares it.
//! A job that fits on GitHub, such as cargo linking test binaries, can be
//! killed here for lack of memory and look like a broken build. So the run
//! says so up front, and runs fewer jobs at once while memory is short.

use std::sync::atomic::{AtomicUsize, Ordering};
use std::time::{Duration, Instant};

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

/// Jobs that may run at once, 0 for no limit.
static JOB_LIMIT: AtomicUsize = AtomicUsize::new(0);

/// How many jobs of one batch run at once.
pub fn parallel_jobs() -> usize {
    if let Some(limit) = env_limit() {
        return limit;
    }
    match JOB_LIMIT.load(Ordering::Relaxed) {
        0 => usize::MAX,
        limit => limit,
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

/// The warning for a VM with `bytes` of memory, running `limit` jobs at once.
fn warning(runtime: &str, bytes: u64, limit: usize) -> String {
    format!(
        "{runtime} has {} of memory, and GitHub's Linux runners have {}. A job that needs more, \
         such as linking large test binaries, can be killed here and pass on GitHub. Running at \
         most {limit} job(s) at a time. Give {runtime} more memory, or set {MAX_PARALLEL_JOBS_ENV}",
        gigabytes(bytes),
        gigabytes(GITHUB_RUNNER_BYTES),
    )
}

/// Total memory as the `info` command of `cli` reports it, in bytes.
fn query(cli: &str, format: &str) -> Option<u64> {
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
    out.trim().parse::<u64>().ok().filter(|bytes| *bytes > 0)
}

/// Check the memory of the VM behind `cli` (`docker` or `podman`): warn
/// when it's short of a runner's, and limit how many jobs run at once.
pub(crate) fn check(cli: &str) {
    let format = if cli == "podman" {
        "{{.Host.MemTotal}}"
    } else {
        "{{.MemTotal}}"
    };
    let Some(bytes) = query(cli, format) else {
        return;
    };
    let runtime = if cli == "podman" { "Podman" } else { "Docker" };
    match limit_for(bytes) {
        Some(limit) => {
            JOB_LIMIT.store(limit, Ordering::Relaxed);
            let shown = env_limit().unwrap_or(limit);
            wrkflw_logging::warning(&warning(runtime, bytes, shown));
        }
        None => JOB_LIMIT.store(0, Ordering::Relaxed),
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
    fn the_warning_names_both_sizes_and_the_way_out() {
        let text = warning("Docker", 8_217_751_552, 1);
        assert!(text.contains("7.7 GB"), "{text}");
        assert!(text.contains("16.0 GB"), "{text}");
        assert!(text.contains(MAX_PARALLEL_JOBS_ENV), "{text}");
    }
}
