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
    /// The env values emulation seeds where they differ from the run's:
    /// host paths, `RUNNER_OS` macOS, and workflow `env:` resolved with
    /// them. Laid over the env a job is handed, so what its caller gave it,
    /// such as a called workflow's inputs and secrets, stays.
    overlay: Vec<(String, String)>,
}

impl HostJobs {
    /// `run_env` is the run's env as its container runtime seeds it, and
    /// `host_env` the same run's env as emulation seeds it.
    pub fn new(
        runtime: Box<dyn ContainerRuntime + Send + Sync>,
        run_env: &HashMap<String, String>,
        host_env: HashMap<String, String>,
    ) -> Self {
        let overlay = host_env
            .into_iter()
            .filter(|(key, value)| run_env.get(key) != Some(value))
            .collect();
        HostJobs { runtime, overlay }
    }

    /// The env and user env a job handed `env` and `user_env` runs with on
    /// the host. A user env key is changed only where the job has it, so
    /// a called workflow's empty user env stays empty.
    pub fn env_for(
        &self,
        env: &HashMap<String, String>,
        user_env: &HashMap<String, String>,
    ) -> (HashMap<String, String>, HashMap<String, String>) {
        let mut env = env.clone();
        let mut user_env = user_env.clone();
        for (key, value) in &self.overlay {
            env.insert(key.clone(), value.clone());
            if let Some(slot) = user_env.get_mut(key) {
                slot.clone_from(value);
            }
        }
        (env, user_env)
    }
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

/// Whether some leg of `job` may run on the host: it has a matrix and this
/// run runs `macos-*` jobs there. Its result then can't stand in for, or
/// be stood in for by, a run with every leg in a container. Judged before
/// the legs are known, so a matrix with no macOS leg counts too; that
/// costs only reuse, and only under `--macos-on-host`.
pub(crate) fn legs_may_run_on_host(job: &Job) -> bool {
    job.matrix_config().is_some() && HOST_JOBS.try_with(|_| ()).is_ok()
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

    fn map(pairs: &[(&str, &str)]) -> HashMap<String, String> {
        pairs
            .iter()
            .map(|(k, v)| (k.to_string(), v.to_string()))
            .collect()
    }

    fn host() -> HostJobs {
        HostJobs::new(
            Box::new(wrkflw_runtime::emulation::EmulationRuntime::new()),
            &map(&[("RUNNER_OS", "Linux")]),
            map(&[("RUNNER_OS", "macOS")]),
        )
    }

    /// A called workflow's job keeps its inputs and secrets on the host,
    /// and its empty user env gets none of the caller's workflow env.
    #[test]
    fn a_host_job_keeps_the_env_it_was_handed_with_host_values_laid_over_it() {
        let run = map(&[
            ("RUNNER_OS", "Linux"),
            ("GITHUB_WORKSPACE", "/github/workspace"),
            ("OUT", "/github/workspace/out"),
            ("GITHUB_REPOSITORY", "o/r"),
        ]);
        let host_env = map(&[
            ("RUNNER_OS", "macOS"),
            ("GITHUB_WORKSPACE", "/tmp/ws"),
            ("OUT", "/tmp/ws/out"),
            ("GITHUB_REPOSITORY", "o/r"),
        ]);
        let host = HostJobs::new(
            Box::new(wrkflw_runtime::emulation::EmulationRuntime::new()),
            &run,
            host_env,
        );
        let mut called = run.clone();
        called.insert("INPUT_TARGET".into(), "release".into());
        called.insert("SECRET_TOKEN".into(), "s3cret".into());
        let (env, user_env) = host.env_for(&called, &HashMap::new());
        assert_eq!(env["INPUT_TARGET"], "release");
        assert_eq!(env["SECRET_TOKEN"], "s3cret");
        assert_eq!(env["RUNNER_OS"], "macOS");
        assert_eq!(env["GITHUB_WORKSPACE"], "/tmp/ws");
        assert!(user_env.is_empty(), "{user_env:?}");

        let (_, user_env) = host.env_for(&run, &map(&[("OUT", "/github/workspace/out")]));
        assert_eq!(user_env["OUT"], "/tmp/ws/out");
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
    async fn only_a_matrix_job_under_the_flag_may_have_legs_on_the_host() {
        let matrix = job("runs-on: ${{ matrix.os }}\nstrategy:\n  matrix:\n    os: [ubuntu-latest, macos-15]\nsteps: []\n");
        let plain = job("runs-on: macos-latest\nsteps: []\n");
        assert!(!legs_may_run_on_host(&matrix), "not asked for");
        scope(Some(Arc::new(host())), async {
            assert!(legs_may_run_on_host(&matrix));
            assert!(!legs_may_run_on_host(&plain), "judged by its own env");
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
