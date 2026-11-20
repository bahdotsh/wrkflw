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
use std::path::{Path, PathBuf};
use std::sync::Arc;
use wrkflw_parser::workflow::Job;
use wrkflw_runtime::container::{ContainerError, ContainerOutput, ContainerRuntime};

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
        HostJobs {
            runtime: Box::new(OffTask {
                runtime: Arc::from(runtime),
                one_at_a_time: Arc::new(tokio::sync::Semaphore::new(1)),
            }),
            overlay,
        }
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

/// A runtime whose steps run off the task that drives the run's jobs.
///
/// Emulation waits for a step's process in place, without yielding. A
/// run's jobs, and the legs of a matrix chunk, are all polled from one
/// task, so a host job's long `brew install` would stop the container jobs
/// beside it until it ended. Here each step waits on a blocking thread.
///
/// Host steps run one at a time. On GitHub each macOS job has a machine of
/// its own; here they share this Mac, its Homebrew lock and its caches, so
/// two at once can fail where neither would alone. They still run while
/// container jobs do.
struct OffTask {
    runtime: Arc<dyn ContainerRuntime + Send + Sync>,
    one_at_a_time: Arc<tokio::sync::Semaphore>,
}

/// Ends a step's processes when the step is given up on: a timeout or a
/// cancelled job drops the future waiting for it, which leaves its thread
/// running. A step that ended has nothing left to end.
struct EndOnDrop(wrkflw_runtime::emulation::StepHandle);

impl Drop for EndOnDrop {
    fn drop(&mut self) {
        self.0.kill();
    }
}

#[async_trait::async_trait]
impl ContainerRuntime for OffTask {
    async fn run_container(
        &self,
        image: &str,
        cmd: &[&str],
        env_vars: &[(&str, &str)],
        working_dir: &Path,
        volumes: &[(&Path, &Path)],
        entrypoint: Option<&str>,
    ) -> Result<ContainerOutput, ContainerError> {
        let runtime = Arc::clone(&self.runtime);
        let image = image.to_string();
        let cmd: Vec<String> = cmd.iter().map(|s| s.to_string()).collect();
        let env: Vec<(String, String)> = env_vars
            .iter()
            .map(|(k, v)| (k.to_string(), v.to_string()))
            .collect();
        let working_dir = working_dir.to_path_buf();
        let volumes: Vec<(PathBuf, PathBuf)> = volumes
            .iter()
            .map(|(host, inside)| (host.to_path_buf(), inside.to_path_buf()))
            .collect();
        let entrypoint = entrypoint.map(str::to_string);
        // The step's output goes to the tap of this task, which the thread
        // below doesn't have: it is carried there.
        let tap = wrkflw_runtime::live_output::current();
        let _turn = self.one_at_a_time.acquire().await.map_err(|e| {
            ContainerError::ContainerExecution(format!("Host steps can't run: {e}"))
        })?;
        let step = wrkflw_runtime::emulation::StepHandle::default();
        let _end = EndOnDrop(step.clone());
        tokio::task::spawn_blocking(move || {
            let cmd: Vec<&str> = cmd.iter().map(String::as_str).collect();
            let env: Vec<(&str, &str)> =
                env.iter().map(|(k, v)| (k.as_str(), v.as_str())).collect();
            let volumes: Vec<(&Path, &Path)> = volumes
                .iter()
                .map(|(host, inside)| (host.as_path(), inside.as_path()))
                .collect();
            let run = runtime.run_container(
                &image,
                &cmd,
                &env,
                &working_dir,
                &volumes,
                entrypoint.as_deref(),
            );
            step.run(|| match tap {
                Some(tap) => futures::executor::block_on(
                    wrkflw_runtime::live_output::with_output_tap(tap, run),
                ),
                None => futures::executor::block_on(run),
            })
        })
        .await
        .map_err(|e| ContainerError::ContainerExecution(format!("The step's thread failed: {e}")))?
    }

    async fn pull_image(&self, image: &str) -> Result<(), ContainerError> {
        self.runtime.pull_image(image).await
    }

    async fn build_image(
        &self,
        dockerfile: &Path,
        tag: &str,
        context_dir: &Path,
    ) -> Result<(), ContainerError> {
        self.runtime.build_image(dockerfile, tag, context_dir).await
    }

    async fn prepare_language_environment(
        &self,
        language: &str,
        version: Option<&str>,
        additional_packages: Option<Vec<String>>,
    ) -> Result<String, ContainerError> {
        self.runtime
            .prepare_language_environment(language, version, additional_packages)
            .await
    }

    async fn image_exists(&self, tag: &str) -> Result<bool, ContainerError> {
        self.runtime.image_exists(tag).await
    }

    async fn image_id(&self, image: &str) -> Option<String> {
        self.runtime.image_id(image).await
    }

    fn identifies_images(&self) -> bool {
        self.runtime.identifies_images()
    }

    fn mounts_are_copies(&self) -> bool {
        self.runtime.mounts_are_copies()
    }

    fn session_cli(&self) -> Option<&'static str> {
        self.runtime.session_cli()
    }

    fn reuses_build_layers(&self) -> bool {
        self.runtime.reuses_build_layers()
    }

    fn flattens_commands(&self) -> bool {
        self.runtime.flattens_commands()
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
///
/// `host` is task-local: whatever decides how a job runs, its reuse key and
/// where its result is kept must be polled inside this scope. Moving a job
/// to another task with `tokio::spawn` would quietly run its macOS legs in
/// containers again, and file a host pass under a container's key.
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

/// The runner substitution to report for `job`, or its leg with `matrix`,
/// in a run whose env is `env`. A job or leg that runs on the host is
/// judged by the env its steps get there, not the run's container env, so
/// a leg reused or folded into a twin isn't reported as run as Linux.
pub(crate) fn substitution(
    job: &Job,
    matrix: Option<&HashMap<String, Value>>,
    env: &HashMap<String, String>,
    platform_overrides: &HashMap<String, String>,
) -> Option<crate::runner_fidelity::RunnerSubstitution> {
    match for_job(job, matrix, platform_overrides) {
        Some(host) => {
            let (env, _) = host.env_for(env, &HashMap::new());
            crate::runner_fidelity::substitution(job, matrix, &env, platform_overrides)
        }
        None => crate::runner_fidelity::substitution(job, matrix, env, platform_overrides),
    }
}

/// How a leg folded into a twin says where both ran: the host, for a leg
/// run there, else the image.
pub(crate) fn place(
    job: &Job,
    matrix: Option<&HashMap<String, Value>>,
    platform_overrides: &HashMap<String, String>,
    image: String,
) -> String {
    match for_job(job, matrix, platform_overrides) {
        Some(_) => "this Mac's own environment (--macos-on-host)".to_string(),
        None => image,
    }
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

    /// A host step waits on a thread of its own: a job polled beside it,
    /// as every job of a batch is, goes on while it runs.
    #[cfg(unix)]
    #[tokio::test]
    async fn a_long_host_step_does_not_hold_up_the_jobs_beside_it() {
        let host = host();
        let dir = tempfile::tempdir().unwrap();
        let volumes = [(dir.path(), Path::new("/github/workspace"))];
        let started = std::time::Instant::now();
        let step = host
            .runtime
            .run_container("", &["sleep", "2"], &[], dir.path(), &volumes, None);
        let beside = async {
            tokio::time::sleep(std::time::Duration::from_millis(50)).await;
            started.elapsed()
        };
        let (step, beside) = tokio::join!(step, beside);
        assert_eq!(step.unwrap().exit_code, 0);
        assert!(
            beside < std::time::Duration::from_secs(1),
            "the job beside waited {beside:?} for the host step"
        );
    }

    /// A step given up on, as a timeout does, is ended with what it started,
    /// rather than left running on this machine.
    #[cfg(unix)]
    #[tokio::test]
    async fn a_host_step_given_up_on_is_ended_with_its_children() {
        let host = host();
        let dir = tempfile::tempdir().unwrap();
        let marker = dir.path().join("still-running");
        let volumes = [(dir.path(), Path::new("/github/workspace"))];
        let script = format!("sleep 1; touch {}", marker.display());
        let cmd = ["bash", "-c", script.as_str()];
        let step = host
            .runtime
            .run_container("", &cmd, &[], dir.path(), &volumes, None);
        let gave_up = tokio::time::timeout(std::time::Duration::from_millis(200), step).await;
        assert!(gave_up.is_err(), "the step should still have been running");
        std::thread::sleep(std::time::Duration::from_millis(1500));
        assert!(
            !marker.exists(),
            "the step went on after it was given up on"
        );
    }

    /// Two host steps don't run at once: they share this machine.
    #[cfg(unix)]
    #[tokio::test]
    async fn host_steps_take_turns() {
        let host = host();
        let dir = tempfile::tempdir().unwrap();
        let volumes = [(dir.path(), Path::new("/github/workspace"))];
        let started = std::time::Instant::now();
        let step = || {
            host.runtime
                .run_container("", &["sleep", "0.5"], &[], dir.path(), &volumes, None)
        };
        let (a, b) = tokio::join!(step(), step());
        assert_eq!(a.unwrap().exit_code, 0);
        assert_eq!(b.unwrap().exit_code, 0);
        assert!(
            started.elapsed() >= std::time::Duration::from_millis(1000),
            "two host steps overlapped: {:?}",
            started.elapsed()
        );
    }

    /// The step's output still reaches the tap of the job that ran it.
    #[cfg(unix)]
    #[tokio::test]
    async fn a_host_step_streams_to_its_jobs_tap() {
        let host = host();
        let dir = tempfile::tempdir().unwrap();
        let seen = Arc::new(std::sync::Mutex::new(String::new()));
        let tap = wrkflw_runtime::live_output::OutputTap::new({
            let seen = Arc::clone(&seen);
            move |_, chunk| seen.lock().unwrap().push_str(chunk)
        });
        let output = wrkflw_runtime::live_output::with_output_tap(
            tap,
            host.runtime.run_container(
                "",
                &["echo", "hello"],
                &[],
                dir.path(),
                &[(dir.path(), Path::new("/github/workspace"))],
                None,
            ),
        )
        .await
        .unwrap();
        assert!(output.stdout.contains("hello"), "{output:?}");
        let seen = seen.lock().unwrap();
        assert!(seen.contains("hello"), "{seen:?}");
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

    /// A macOS leg reused or folded into a twin under the flag ran on the
    /// Mac, so it isn't reported as run as Linux; without the flag it is.
    #[tokio::test]
    async fn a_host_leg_is_reported_by_the_os_it_ran_as() {
        let job = job("runs-on: ${{ matrix.os }}\nsteps: []\n");
        let leg = HashMap::from([("os".to_string(), Value::from("macos-15"))]);
        let run_env = map(&[("RUNNER_OS", "Linux")]);
        let none = HashMap::new();
        assert!(substitution(&job, Some(&leg), &run_env, &none).is_some());
        scope(Some(Arc::new(host())), async {
            assert!(substitution(&job, Some(&leg), &run_env, &none).is_none());
            let linux = HashMap::from([("os".to_string(), Value::from("windows-latest"))]);
            assert!(substitution(&job, Some(&linux), &run_env, &none).is_some());
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
