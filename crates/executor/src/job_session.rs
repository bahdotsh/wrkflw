//! One long-lived container per job, plus the job's service containers.
//!
//! By default each `run:` step gets its own container. A session starts one
//! container and `exec`s each step into it, which saves the container start
//! and matches GitHub, where steps share a machine: what a step installs is
//! there for the next one.
//!
//! Services share the job container's network namespace, like the containers
//! of a Kubernetes pod. So `localhost:5432` works, as it does for a job on a
//! GitHub runner, and so does the hostname `postgres`, as it does for a job
//! with a `container:`. Ports are the service's own: a `ports:` remap of the
//! host side has no meaning here.
//!
//! This talks to the CLI so Docker and Podman share one implementation.

use crate::acceleration;
use std::collections::HashMap;
use std::future::Future;
use std::path::{Path, PathBuf};
use std::process::Stdio;
use std::sync::Arc;
use std::time::Duration;
use tokio::process::Command;
use wrkflw_parser::workflow::{Job, Service};
use wrkflw_runtime::container::{ContainerError, ContainerOutput, ContainerRuntime};

const SERVICE_READY_TIMEOUT: Duration = Duration::from_secs(180);
const SERVICE_POLL: Duration = Duration::from_millis(500);
const KEEP_ALIVE: &str = "trap 'exit 0' TERM; while :; do sleep 3600 & wait $!; done";

/// A service as declared, before expressions are resolved.
#[derive(Clone)]
struct ServiceSpec {
    name: String,
    image: String,
    env: HashMap<String, String>,
    volumes: Vec<String>,
    options: Option<String>,
    ports: Vec<String>,
}

impl ServiceSpec {
    fn new(name: &str, service: &Service) -> Self {
        Self {
            name: name.to_string(),
            image: service.image.clone(),
            env: service.env.clone(),
            volumes: service.volumes.clone().unwrap_or_default(),
            options: service.options.clone(),
            ports: service.ports.clone().unwrap_or_default(),
        }
    }
}

struct Live {
    image: String,
    volumes: Vec<(PathBuf, PathBuf)>,
    id: String,
    /// Removes the containers when the session ends, however it ends.
    #[allow(dead_code)]
    containers: Containers,
}

/// Every container of a session, oldest first.
///
/// Removed with one command, unlike `ContainerGuard`: a service lives in the
/// job container's network namespace, so Podman refuses to remove the job
/// container until its services are gone. Dropping can't await, so this goes
/// through the CLI, and the removal finishes even if wrkflw exits first.
struct Containers {
    cli: &'static str,
    ids: Vec<String>,
}

impl Containers {
    fn add(&mut self, id: &str) {
        track(self.cli, id);
        self.ids.push(id.to_string());
    }

    /// Dependents before what they depend on.
    fn removal_args(&self) -> Vec<String> {
        let mut args = vec!["rm".to_string(), "-f".to_string()];
        args.extend(self.ids.iter().rev().cloned());
        args
    }
}

impl Drop for Containers {
    fn drop(&mut self) {
        if self.ids.is_empty() {
            return;
        }
        let untrack = untrack_for(self.cli);
        for id in &self.ids {
            untrack(id);
        }
        let spawned = std::process::Command::new(self.cli)
            .args(self.removal_args())
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .spawn();
        match spawned {
            Ok(mut child) => {
                std::thread::spawn(move || {
                    let _ = child.wait();
                });
            }
            Err(e) => wrkflw_logging::warning(&format!(
                "Failed to remove containers {}: {}",
                self.ids.join(", "),
                e
            )),
        }
    }
}

#[derive(Default)]
struct State {
    live: Option<Live>,
    /// A step is inside `exec`. Still true on entry means the last one was
    /// cancelled and may be running, so the container can't be trusted.
    busy: bool,
    /// Starting failed for a reason other than the services, so don't retry.
    unusable: bool,
}

pub(crate) struct JobSession {
    cli: &'static str,
    /// Steps exec into the job container even without `--job-container` when
    /// this isn't empty, as nothing else can reach the services.
    services: Vec<ServiceSpec>,
    state: tokio::sync::Mutex<State>,
}

tokio::task_local! {
    static SESSION: Option<Arc<JobSession>>;
}

fn untrack_for(cli: &str) -> fn(&str) {
    if cli == "podman" {
        crate::podman::untrack_container
    } else {
        crate::docker::untrack_container
    }
}

fn track(cli: &str, id: &str) {
    if cli == "podman" {
        crate::podman::track_container(id);
    } else {
        crate::docker::track_container(id);
    }
}

/// Variables the CLI itself reads. Handing these over through the CLI's own
/// environment would point it at another daemon or binary, so they go in
/// `argv`. Everything else goes by name, which keeps secrets out of `ps`.
fn reaches_the_cli(name: &str) -> bool {
    matches!(name, "PATH" | "HOME" | "TMPDIR" | "USER" | "LOGNAME")
        || name.starts_with("DOCKER_")
        || name.starts_with("CONTAINER")
        || name.starts_with("PODMAN_")
        || name.starts_with("XDG_")
        || name.starts_with("SSL_")
}

/// The `-e` flags for `env`.
fn env_flags<'a>(env: impl Iterator<Item = (&'a str, &'a str)>) -> Vec<String> {
    let mut args = Vec::new();
    for (name, value) in env {
        args.push("-e".to_string());
        if reaches_the_cli(name) {
            args.push(format!("{}={}", name, value));
        } else {
            args.push(name.to_string());
        }
    }
    args
}

/// Give `command` the values `env_flags` passed by name.
fn set_named_env<'a>(command: &mut Command, env: impl Iterator<Item = (&'a str, &'a str)>) {
    for (name, value) in env {
        if !reaches_the_cli(name) {
            command.env(name, value);
        }
    }
}

fn exec_args(id: &str, working_dir: &Path, env_args: Vec<String>, cmd: &[&str]) -> Vec<String> {
    let mut args = vec![
        "exec".to_string(),
        "-w".to_string(),
        working_dir.display().to_string(),
    ];
    args.extend(env_args);
    args.push(id.to_string());
    args.extend(cmd.iter().map(|part| part.to_string()));
    args
}

fn start_args(
    name: &str,
    image: &str,
    volumes: &[(PathBuf, PathBuf)],
    service_names: &[&str],
) -> Vec<String> {
    let mut args: Vec<String> = ["run", "-d", "--name", name, "--entrypoint", "sh"]
        .iter()
        .map(|part| part.to_string())
        .collect();
    for (host, container) in volumes {
        args.push("-v".to_string());
        args.push(format!("{}:{}", host.display(), container.display()));
    }
    for service in service_names {
        args.push("--add-host".to_string());
        args.push(format!("{}:127.0.0.1", service));
    }
    args.push(image.to_string());
    args.push("-c".to_string());
    args.push(KEEP_ALIVE.to_string());
    args
}

fn service_args(
    name: &str,
    network_owner: &str,
    spec: &ServiceSpec,
    image: &str,
    env_args: Vec<String>,
) -> Result<Vec<String>, String> {
    let mut args: Vec<String> = ["run", "-d", "--name", name]
        .iter()
        .map(|part| part.to_string())
        .collect();
    args.push(format!("--network=container:{}", network_owner));
    args.extend(env_args);
    for volume in &spec.volumes {
        args.push("-v".to_string());
        args.push(volume.clone());
    }
    if let Some(options) = &spec.options {
        let parsed = shlex::split(options)
            .ok_or_else(|| format!("service '{}': can't parse options", spec.name))?;
        args.extend(parsed);
    }
    args.push(image.to_string());
    Ok(args)
}

/// A `ports:` entry whose host port differs from the service's own port.
fn remapped_port(port: &str) -> bool {
    let without_protocol = port.split('/').next().unwrap_or(port);
    let mut sides = without_protocol.rsplit(':');
    match (sides.next(), sides.next()) {
        (Some(container), Some(host)) => !host.is_empty() && host != container,
        _ => false,
    }
}

async fn cli_output(
    cli: &str,
    args: &[String],
    prepare: impl FnOnce(&mut Command),
) -> Result<ContainerOutput, ContainerError> {
    let mut command = Command::new(cli);
    command
        .args(args)
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .kill_on_drop(true);
    prepare(&mut command);
    let output = command
        .output()
        .await
        .map_err(|e| ContainerError::ContainerStart(format!("Failed to run {}: {}", cli, e)))?;
    Ok(ContainerOutput {
        stdout: String::from_utf8_lossy(&output.stdout).to_string(),
        stderr: String::from_utf8_lossy(&output.stderr).to_string(),
        exit_code: output.status.code().unwrap_or(-1),
    })
}

fn last_chars(text: &str, count: usize) -> String {
    let skip = text.chars().count().saturating_sub(count);
    text.chars().skip(skip).collect()
}

/// What `inspect` says about readiness, read from `<status>`.
enum Readiness {
    Ready,
    Waiting,
    Failed,
}

fn readiness(status: &str) -> Readiness {
    match status.trim() {
        "healthy" | "running" => Readiness::Ready,
        "unhealthy" | "exited" | "dead" => Readiness::Failed,
        _ => Readiness::Waiting,
    }
}

impl JobSession {
    /// Wait for a service's health check, or for it to run if it has none.
    async fn wait_until_ready(&self, name: &str, label: &str) -> Result<(), String> {
        let format = "{{if .State.Health}}{{.State.Health.Status}}{{else}}{{.State.Status}}{{end}}";
        let args: Vec<String> = ["inspect", "--format", format, name]
            .iter()
            .map(|part| part.to_string())
            .collect();
        let deadline = tokio::time::Instant::now() + SERVICE_READY_TIMEOUT;
        loop {
            let output = cli_output(self.cli, &args, |_| {})
                .await
                .map_err(|e| e.to_string())?;
            match readiness(&output.stdout) {
                Readiness::Ready => return Ok(()),
                Readiness::Failed => {
                    let log_args = vec!["logs".to_string(), name.to_string()];
                    let logs = cli_output(self.cli, &log_args, |_| {}).await.ok();
                    let tail = logs.map(|l| l.stderr + &l.stdout).unwrap_or_default();
                    return Err(format!(
                        "service '{}' failed ({}): {}",
                        label,
                        output.stdout.trim(),
                        last_chars(&tail, 2000)
                    ));
                }
                Readiness::Waiting => {}
            }
            if tokio::time::Instant::now() >= deadline {
                return Err(format!(
                    "service '{}' wasn't ready after {}s",
                    label,
                    SERVICE_READY_TIMEOUT.as_secs()
                ));
            }
            tokio::time::sleep(SERVICE_POLL).await;
        }
    }

    async fn start_services(
        &self,
        job_id: &str,
        containers: &mut Containers,
        resolve: &dyn Fn(&str) -> String,
    ) -> Result<(), String> {
        let suffix = uuid::Uuid::new_v4().simple().to_string();
        for spec in &self.services {
            if spec.ports.iter().any(|port| remapped_port(port)) {
                wrkflw_logging::warning(&format!(
                    "Service '{}' remaps a port, which doesn't apply locally: \
                     steps reach it on the port the service listens on",
                    spec.name
                ));
            }
            let name = format!("wrkflw-svc-{}-{}", spec.name, suffix);
            let image = resolve(&spec.image);
            let env: Vec<(String, String)> = spec
                .env
                .iter()
                .map(|(k, v)| (k.clone(), resolve(v)))
                .collect();
            let pairs = || env.iter().map(|(k, v)| (k.as_str(), v.as_str()));
            let args = service_args(&name, job_id, spec, &image, env_flags(pairs()))?;
            let output = cli_output(self.cli, &args, |command| set_named_env(command, pairs()))
                .await
                .map_err(|e| e.to_string())?;
            if output.exit_code != 0 {
                return Err(format!(
                    "service '{}' didn't start: {}",
                    spec.name,
                    output.stderr.trim()
                ));
            }
            containers.add(output.stdout.trim());
            self.wait_until_ready(&name, &spec.name).await?;
        }
        Ok(())
    }

    /// Start the job container, then its services. On error `containers` still
    /// removes whatever did start.
    async fn start(
        &self,
        image: &str,
        volumes: &[(PathBuf, PathBuf)],
        resolve: &dyn Fn(&str) -> String,
    ) -> Result<Live, String> {
        let name = format!("wrkflw-job-{}", uuid::Uuid::new_v4());
        let service_names: Vec<&str> = self.services.iter().map(|s| s.name.as_str()).collect();
        let args = start_args(&name, image, volumes, &service_names);
        let output = cli_output(self.cli, &args, |_| {})
            .await
            .map_err(|e| e.to_string())?;
        if output.exit_code != 0 {
            return Err(format!(
                "job container didn't start: {}",
                output.stderr.trim()
            ));
        }
        let id = output.stdout.trim().to_string();
        let mut containers = Containers {
            cli: self.cli,
            ids: Vec::new(),
        };
        containers.add(&id);
        self.start_services(&id, &mut containers, resolve).await?;
        Ok(Live {
            image: image.to_string(),
            volumes: volumes.to_vec(),
            id,
            containers,
        })
    }

    /// Run a step in the session.
    ///
    /// `Ok(None)` means the caller should run the step the ordinary way: the
    /// step wants another image or other mounts, or the image can't idle.
    /// `Err` is a service failure, which fails the step. Falling back there
    /// would run tests against a database that isn't there.
    pub(crate) async fn run(
        &self,
        image: &str,
        cmd: &[&str],
        env: &[(&str, &str)],
        working_dir: &Path,
        volumes: &[(&Path, &Path)],
        resolve: &dyn Fn(&str) -> String,
    ) -> Result<Option<ContainerOutput>, String> {
        let wanted: Vec<(PathBuf, PathBuf)> = volumes
            .iter()
            .map(|(host, container)| (host.to_path_buf(), container.to_path_buf()))
            .collect();
        let mut state = self.state.lock().await;
        if state.unusable {
            return Ok(None);
        }
        if state.busy {
            state.live = None;
            state.busy = false;
        }
        if state.live.is_none() {
            match self.start(image, &wanted, resolve).await {
                Ok(live) => state.live = Some(live),
                Err(e) if self.services.is_empty() => {
                    wrkflw_logging::warning(&format!(
                        "Falling back to one container per step: {}",
                        e
                    ));
                    state.unusable = true;
                    return Ok(None);
                }
                Err(e) => return Err(e),
            }
        }
        let Some(live) = &state.live else {
            return Ok(None);
        };
        let same_mounts = live.volumes.len() == wanted.len()
            && wanted.iter().all(|volume| live.volumes.contains(volume));
        if live.image != image || !same_mounts {
            return Ok(None);
        }
        let id = live.id.clone();
        state.busy = true;

        let args = exec_args(&id, working_dir, env_flags(env.iter().copied()), cmd);
        let output = cli_output(self.cli, &args, |command| {
            set_named_env(command, env.iter().copied())
        })
        .await
        .map_err(|e| e.to_string())?;
        state.busy = false;
        Ok(Some(output))
    }
}

/// The session of the job this task is running, if it has one.
pub(crate) fn current() -> Option<Arc<JobSession>> {
    SESSION.try_with(|session| session.clone()).ok().flatten()
}

/// True when `runtime` can give `job` what a session gives it.
pub(crate) fn wanted_for(job: &Job, runtime: &dyn ContainerRuntime) -> Option<&'static str> {
    let cli = runtime.session_cli()?;
    let run = acceleration::current()?;
    // A kept-for-debugging container is one per failed step, which a shared
    // container can't provide, so that flag wins over the speed-up.
    let reuse = run.accel.job_container && !run.preserve_containers;
    (reuse || !job.services.is_empty()).then_some(cli)
}

/// Run `future`, one job's steps, with a session if `job` calls for one. The
/// session's containers go when this returns.
pub(crate) async fn with_job_session<F: Future>(
    job: &Job,
    runtime: &dyn ContainerRuntime,
    future: F,
) -> F::Output {
    let session = wanted_for(job, runtime).map(|cli| {
        let mut services: Vec<ServiceSpec> = job
            .services
            .iter()
            .map(|(name, service)| ServiceSpec::new(name, service))
            .collect();
        services.sort_by(|a, b| a.name.cmp(&b.name));
        Arc::new(JobSession {
            cli,
            services,
            state: tokio::sync::Mutex::new(State::default()),
        })
    });
    SESSION.scope(session, future).await
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_secret_goes_by_name_and_cli_variables_go_by_value() {
        let env = [("TOKEN", "hunter2"), ("DOCKER_HOST", "tcp://x")];
        let mut command = Command::new("true");
        set_named_env(&mut command, env.into_iter());
        assert_eq!(
            env_flags(env.into_iter()),
            ["-e", "TOKEN", "-e", "DOCKER_HOST=tcp://x"]
        );
        let passed: Vec<_> = command.as_std().get_envs().collect();
        assert_eq!(passed.len(), 1);
        assert_eq!(passed[0].0, "TOKEN");
    }

    #[test]
    fn services_are_removed_before_the_job_container_they_depend_on() {
        let mut containers = Containers {
            cli: "true",
            ids: Vec::new(),
        };
        containers.ids.push("job".to_string());
        containers.ids.push("db".to_string());
        assert_eq!(containers.removal_args(), ["rm", "-f", "db", "job"]);
        containers.ids.clear();
    }

    #[test]
    fn exec_keeps_a_multi_line_script_as_one_argument() {
        let args = exec_args(
            "id",
            Path::new("/github/workspace/sub"),
            vec!["-e".to_string(), "A".to_string()],
            &["bash", "-c", "echo one\necho two"],
        );
        assert_eq!(
            args,
            [
                "exec",
                "-w",
                "/github/workspace/sub",
                "-e",
                "A",
                "id",
                "bash",
                "-c",
                "echo one\necho two"
            ]
        );
    }

    #[test]
    fn services_are_reachable_by_name_from_the_job_container() {
        let args = start_args(
            "n",
            "ubuntu:24.04",
            &[(PathBuf::from("/h"), PathBuf::from("/c"))],
            &["postgres"],
        );
        assert!(args.windows(2).any(|w| w == ["-v", "/h:/c"]));
        assert!(args
            .windows(2)
            .any(|w| w == ["--add-host", "postgres:127.0.0.1"]));
    }

    #[test]
    fn a_service_joins_the_job_network_and_keeps_its_options() {
        let spec = ServiceSpec {
            name: "db".to_string(),
            image: "postgres:16".to_string(),
            env: HashMap::new(),
            volumes: vec![],
            options: Some("--health-cmd 'pg_isready -U x' --health-interval 1s".to_string()),
            ports: vec![],
        };
        let args = service_args("n", "job", &spec, "postgres:16", vec![]).unwrap();
        assert!(args.contains(&"--network=container:job".to_string()));
        assert!(args.contains(&"pg_isready -U x".to_string()));
        assert_eq!(args.last().unwrap(), "postgres:16");
    }

    #[test]
    fn only_a_changed_host_port_counts_as_a_remap() {
        assert!(remapped_port("5433:5432"));
        assert!(remapped_port("5433:5432/tcp"));
        assert!(!remapped_port("5432:5432"));
        assert!(!remapped_port("5432"));
        assert!(!remapped_port("5432/tcp"));
    }

    #[test]
    fn readiness_follows_health_then_state() {
        assert!(matches!(readiness("healthy\n"), Readiness::Ready));
        assert!(matches!(readiness("running"), Readiness::Ready));
        assert!(matches!(readiness("starting"), Readiness::Waiting));
        assert!(matches!(readiness("created"), Readiness::Waiting));
        assert!(matches!(readiness("unhealthy"), Readiness::Failed));
        assert!(matches!(readiness("exited"), Readiness::Failed));
    }
}
