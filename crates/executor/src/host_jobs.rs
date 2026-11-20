//! `macos-*` jobs run on this Mac, when asked.
//!
//! On a Mac running jobs in containers, a job whose `runs-on` names macOS
//! runs as Linux, in the Ubuntu image, and its pass is only `unverified`.
//! With [`crate::run_options::MACOS_ON_HOST_ENV`] (`--macos-on-host`) such a
//! job, or matrix leg, runs under emulation instead: on the Mac itself, with
//! `runner.os` macOS and the host's paths, while the run's other jobs keep
//! their containers. Off by default, because emulation gives a step no
//! isolation: a macOS job's `brew install` changes this machine.

use crate::engine::RuntimeType;
use serde_yaml::Value;
use std::collections::HashMap;
use std::future::Future;
use std::sync::Arc;
use wrkflw_parser::workflow::Job;
use wrkflw_runtime::container::ContainerRuntime;

/// What a job run on the host runs with instead of the run's runtime.
pub(crate) struct HostJobs {
    pub runtime: Box<dyn ContainerRuntime + Send + Sync>,
    /// The run's env as emulation seeds it: host paths, `RUNNER_OS` macOS.
    pub env: HashMap<String, String>,
    pub user_env: HashMap<String, String>,
}

tokio::task_local! {
    static HOST_JOBS: Arc<HostJobs>;
}

/// Whether a run on `runtime` should run its `macos-*` jobs on the host.
pub(crate) fn wanted(runtime: &RuntimeType) -> bool {
    cfg!(target_os = "macos") && runtime.runs_in_container() && crate::run_options::macos_on_host()
}

/// Run `future`, a run's jobs, with `host` for its macOS jobs.
pub(crate) async fn scope<F: Future>(host: Option<Arc<HostJobs>>, future: F) -> F::Output {
    match host {
        Some(host) => HOST_JOBS.scope(host, future).await,
        None => future.await,
    }
}

/// What `job`, or its leg with `matrix`, runs with when it runs on the
/// host: when this run asked for that and the job asks for macOS. A job
/// with `container:` or a label mapped with `--platform` keeps the run's
/// runtime, as [`crate::runner_fidelity::requested_os`] doesn't judge it.
pub(crate) fn for_job(
    job: &Job,
    matrix: Option<&HashMap<String, Value>>,
    platform_overrides: &HashMap<String, String>,
) -> Option<Arc<HostJobs>> {
    let host = HOST_JOBS.try_with(Arc::clone).ok()?;
    asks_for_macos(job, matrix, platform_overrides).then_some(host)
}

fn asks_for_macos(
    job: &Job,
    matrix: Option<&HashMap<String, Value>>,
    platform_overrides: &HashMap<String, String>,
) -> bool {
    crate::runner_fidelity::requested_os(job, matrix, platform_overrides)
        .is_some_and(|requested| requested.os == "macOS")
}

#[cfg(test)]
mod tests {
    use super::*;

    fn job(yaml: &str) -> Job {
        serde_yaml::from_str(yaml).unwrap()
    }

    fn host() -> HostJobs {
        HostJobs {
            runtime: Box::new(wrkflw_runtime::emulation::EmulationRuntime::new()),
            env: HashMap::from([("RUNNER_OS".to_string(), "macOS".to_string())]),
            user_env: HashMap::new(),
        }
    }

    #[tokio::test]
    async fn only_macos_jobs_run_on_the_host_and_only_when_asked() {
        let mac = job("runs-on: macos-latest\nsteps: []\n");
        let linux = job("runs-on: ubuntu-latest\nsteps: []\n");
        let boxed = job("runs-on: macos-latest\ncontainer: node:22\nsteps: []\n");
        let none = HashMap::new();
        assert!(for_job(&mac, None, &none).is_none(), "not asked for");
        scope(Some(Arc::new(host())), async {
            assert!(for_job(&mac, None, &none).is_some());
            assert!(for_job(&linux, None, &none).is_none());
            assert!(for_job(&boxed, None, &none).is_none());
            let mapped = HashMap::from([("macos-latest".to_string(), "img".to_string())]);
            assert!(for_job(&mac, None, &mapped).is_none());
        })
        .await;
    }

    #[tokio::test]
    async fn a_matrix_leg_is_judged_by_its_own_runner() {
        let job = job("runs-on: ${{ matrix.os }}\nsteps: []\n");
        let leg = |os: &str| HashMap::from([("os".to_string(), Value::from(os))]);
        let none = HashMap::new();
        scope(Some(Arc::new(host())), async {
            assert!(for_job(&job, Some(&leg("macos-15")), &none).is_some());
            assert!(for_job(&job, Some(&leg("ubuntu-24.04")), &none).is_none());
        })
        .await;
    }
}
