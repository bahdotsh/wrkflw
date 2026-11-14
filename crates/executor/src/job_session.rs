//! One long-lived container per job, plus the job's service containers.
//!
//! A session starts one container and `exec`s each step into it, which
//! matches GitHub, where steps share a machine: what a step installs is
//! there for the next one. It also saves a container start per step.
//! `--step-containers` gives each `run:` step a container of its own
//! instead, as does keeping a failed step's container for debugging.
//!
//! The job container and its services are sampled into the job's memory
//! meter, as a step's own container is, so a matrix of light jobs still
//! runs side by side.
//!
//! With `--docker-in-jobs`, a job whose steps run `docker` gets a Docker
//! engine of its own inside its container, which then runs privileged.
//! The machine's engine is never handed to a step: a `-v $(pwd):/src`
//! there would name a path on this machine, not in the job.
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
use wrkflw_runtime::container::{
    ContainerError, ContainerOutput, ContainerRuntime, PATH_ADDITIONS_ENV,
};

const SERVICE_READY_TIMEOUT: Duration = Duration::from_secs(180);
const SERVICE_POLL: Duration = Duration::from_millis(500);
pub(crate) const KEEP_ALIVE: &str = "trap 'exit 0' TERM; while :; do sleep 3600 & wait $!; done";

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

/// Where a composite action's own directory appears in a step's container,
/// as `GITHUB_ACTION_PATH`.
pub(crate) const CONTAINER_ACTION_DIR: &str = "/github/action";

struct Live {
    image: String,
    volumes: Vec<(PathBuf, PathBuf)>,
    id: String,
    /// The action directory last copied to [`CONTAINER_ACTION_DIR`].
    action_dir: Option<PathBuf>,
    /// Samples the containers' memory. Before `_cleanup`, so it stops
    /// before the containers go and their going isn't taken for a failure.
    _memory: Option<MemoryWatch>,
    /// Removes the containers when the session ends, however it ends.
    _cleanup: Containers,
}

/// A session's containers that ran this long and sent no sample went
/// unmeasured.
const UNSAMPLED_AFTER: Duration = Duration::from_secs(3);

/// What `stats` prints for each container, each time it samples them.
const STATS_FORMAT: &str = "{{.ID}} {{.MemUsage}}";

/// Samples a session's containers into the job's meter while they live,
/// through the CLI's `stats`, which reports the same working set the
/// engine's API gives for a step's own container.
struct MemoryWatch {
    ids: Vec<String>,
    meter: Arc<crate::job_memory::JobMeter>,
    sampled: Arc<std::sync::atomic::AtomicBool>,
    /// `stats` ended while the containers were still there.
    broken: Arc<std::sync::atomic::AtomicBool>,
    started: std::time::Instant,
    task: tokio::task::JoinHandle<()>,
}

impl MemoryWatch {
    fn start(cli: &'static str, ids: Vec<String>, meter: Arc<crate::job_memory::JobMeter>) -> Self {
        use std::sync::atomic::{AtomicBool, Ordering};
        use tokio::io::AsyncBufReadExt;
        let sampled = Arc::new(AtomicBool::new(false));
        let broken = Arc::new(AtomicBool::new(false));
        let task = {
            let (ids, meter, sampled, broken) =
                (ids.clone(), meter.clone(), sampled.clone(), broken.clone());
            tokio::spawn(async move {
                let child = Command::new(cli)
                    .args(["stats", "--no-trunc", "--format", STATS_FORMAT])
                    .args(&ids)
                    .stdin(Stdio::null())
                    .stdout(Stdio::piped())
                    .stderr(Stdio::null())
                    .kill_on_drop(true)
                    .spawn();
                let Ok(mut child) = child else {
                    broken.store(true, Ordering::Relaxed);
                    return;
                };
                if let Some(stdout) = child.stdout.take() {
                    let mut lines = tokio::io::BufReader::new(stdout).lines();
                    while let Ok(Some(line)) = lines.next_line().await {
                        if let Some((id, bytes)) = stats_sample(&line, &ids) {
                            meter.sample(id, bytes);
                            sampled.store(true, Ordering::Relaxed);
                        }
                    }
                }
                // Aborted when the session ends, so getting here means
                // `stats` gave up first: what was used since is unseen.
                broken.store(true, Ordering::Relaxed);
            })
        };
        MemoryWatch {
            ids,
            meter,
            sampled,
            broken,
            started: std::time::Instant::now(),
            task,
        }
    }
}

impl Drop for MemoryWatch {
    fn drop(&mut self) {
        use std::sync::atomic::Ordering;
        self.task.abort();
        for id in &self.ids {
            self.meter.stopped(id);
        }
        if self.broken.load(Ordering::Relaxed)
            || (!self.sampled.load(Ordering::Relaxed) && self.started.elapsed() >= UNSAMPLED_AFTER)
        {
            self.meter.unmeasured();
        }
    }
}

/// `text` without the escape sequences `stats` clears the screen with
/// between samples, even when its output is a pipe.
fn without_escapes(text: &str) -> String {
    let mut plain = String::with_capacity(text.len());
    let mut chars = text.chars();
    while let Some(c) = chars.next() {
        if c != '\x1b' {
            plain.push(c);
            continue;
        }
        // `ESC [ ... letter`; anything else after ESC is one character.
        if chars.next() == Some('[') {
            for c in chars.by_ref() {
                if c.is_ascii_alphabetic() {
                    break;
                }
            }
        }
    }
    plain
}

/// A size as `stats` prints it, such as `276.3MiB` or `1.2GB`, in bytes.
fn stats_bytes(size: &str) -> Option<u64> {
    let split = size.find(|c: char| !(c.is_ascii_digit() || c == '.'))?;
    let (number, unit) = size.split_at(split);
    let number: f64 = number.parse().ok()?;
    let scale: f64 = match unit {
        "B" => 1.0,
        "kB" | "KB" => 1e3,
        "MB" => 1e6,
        "GB" => 1e9,
        "TB" => 1e12,
        "KiB" => 1024.0,
        "MiB" => 1024.0 * 1024.0,
        "GiB" => 1024.0 * 1024.0 * 1024.0,
        "TiB" => 1024.0 * 1024.0 * 1024.0 * 1024.0,
        _ => return None,
    };
    Some((number * scale) as u64)
}

/// The container a line of `stats` output is about, one of `ids`, and the
/// memory it uses. `None` for a container that isn't running, which prints
/// dashes, and for anything else that isn't a sample.
fn stats_sample<'a>(line: &str, ids: &'a [String]) -> Option<(&'a str, u64)> {
    let line = without_escapes(line);
    let mut words = line.split_whitespace();
    let printed = words.next()?;
    let id = ids
        .iter()
        .find(|id| !printed.is_empty() && id.starts_with(printed))?;
    Some((id.as_str(), stats_bytes(words.next()?)?))
}

/// Marks the job unmeasured unless the step it watches ran to its end: one
/// cancelled part way may have gone on to use more than was seen.
struct StepWatch(Option<Arc<crate::job_memory::JobMeter>>);

impl StepWatch {
    /// The step ended, with `output` if it could be run at all.
    fn ended(mut self, output: Option<&ContainerOutput>) {
        let Some(meter) = self.0.take() else {
            return;
        };
        if output.is_some_and(|output| {
            crate::job_memory::ran_out_of_memory(
                output.exit_code,
                false,
                &[&output.stdout, &output.stderr],
            )
        }) {
            meter.killed();
        }
    }
}

impl Drop for StepWatch {
    fn drop(&mut self) {
        if let Some(meter) = self.0.take() {
            meter.unmeasured();
        }
    }
}

/// Why a removal failed, or `None` if it worked.
fn removal_failure(output: std::io::Result<std::process::Output>) -> Option<String> {
    match output {
        Ok(output) if output.status.success() => None,
        Ok(output) => {
            let stderr = String::from_utf8_lossy(&output.stderr).trim().to_string();
            Some(if stderr.is_empty() {
                format!("exited with {}", output.status)
            } else {
                stderr
            })
        }
        Err(e) => Some(e.to_string()),
    }
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
        // Cleanup on exit removes tracked containers in order, and Podman
        // won't remove the job container while a service uses its network.
        // So the job container, the first one, moves behind each service.
        if let Some(job) = self.ids.first() {
            untrack_for(self.cli)(job);
            track(self.cli, job);
        }
        self.ids.push(id.to_string());
    }

    /// Dependents before what they depend on. `-v` takes the anonymous
    /// volumes with them: database images declare one, and without it every
    /// run would leave one behind per service.
    fn removal_args(&self) -> Vec<String> {
        let mut args = vec!["rm".to_string(), "-f".to_string(), "-v".to_string()];
        args.extend(self.ids.iter().rev().cloned());
        args
    }
}

impl Drop for Containers {
    fn drop(&mut self) {
        if self.ids.is_empty() {
            return;
        }
        let spawned = std::process::Command::new(self.cli)
            .args(self.removal_args())
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::piped())
            .spawn();
        match spawned {
            Ok(child) => {
                // Only now: if removal couldn't start, the cleanup on exit
                // still has to know about these containers.
                let untrack = untrack_for(self.cli);
                for id in &self.ids {
                    untrack(id);
                }
                let ids = self.ids.join(" ");
                let cli = self.cli;
                std::thread::spawn(move || {
                    if let Some(message) = removal_failure(child.wait_with_output()) {
                        wrkflw_logging::warning(&format!(
                            "Failed to remove containers {}: {}. Remove them with `{} rm -f -v {}`",
                            ids, message, cli, ids
                        ));
                    }
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
    /// A service failed to start. Every later step fails with this, so an
    /// `if: always()` step doesn't wait for the same service all over again.
    failed: Option<String>,
    /// Set once a step has been sent to its own container, so the warning
    /// that says so comes once per job.
    warned_outside: bool,
}

pub(crate) struct JobSession {
    cli: &'static str,
    /// Steps exec into the job container even with `--step-containers` when
    /// this isn't empty, as nothing else can reach the services.
    services: Vec<ServiceSpec>,
    /// Start a Docker engine in the job container, for steps that run
    /// `docker`.
    docker_engine: bool,
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

/// Runs a step with the variables it reads on stdin.
///
/// The step's variables reach the container this way, not through `-e`. A
/// value in `argv` shows in `ps`, and a value in the CLI's own environment
/// would let a workflow set `LD_PRELOAD` or `DOCKER_HOST` for a program that
/// runs on this machine. stdin carries `set -- 'NAME=value' ... "$@"`, and
/// `env` takes names that `export` can't, such as `INPUT_MY-INPUT`.
///
/// `env` hands such a name to the step's shell, but dash, the `sh` of Debian
/// and Ubuntu images, drops names that aren't identifiers. So a `shell: sh`
/// step there can't pass `INPUT_MY-INPUT` on to its children, as on GitHub.
const ENV_LOADER: &str = "eval \"$(cat)\" && exec env \"$@\"";

/// `value` as one word of a POSIX shell script.
fn shell_quote(value: &str) -> String {
    format!("'{}'", value.replace('\'', "'\\''"))
}

/// Why `name` can't be handed to a container, or `None` if it can.
fn bad_env_name(name: &str) -> Option<&'static str> {
    if name.is_empty() {
        Some("it's empty")
    } else if name.starts_with('-') {
        Some("it starts with '-'")
    } else if name.contains(['=', '\0', '\n', '\r']) {
        Some("it contains '=' or a control character")
    } else {
        None
    }
}

/// The script `ENV_LOADER` reads, which puts `env` ahead of the command.
///
/// Directories steps added through `$GITHUB_PATH` go in front of the
/// container's own PATH, which `exec` started this shell with, instead of the
/// job's `PATH`, which was built on the host's.
fn env_script<'a>(env: impl Iterator<Item = (&'a str, &'a str)>) -> Result<String, String> {
    let env: Vec<(&str, &str)> = env.collect();
    let additions = env
        .iter()
        .rev()
        .find(|(name, _)| *name == PATH_ADDITIONS_ENV)
        .map(|(_, value)| *value)
        .filter(|value| !value.is_empty());
    let mut script = String::from("set --");
    for (name, value) in env {
        if name == PATH_ADDITIONS_ENV || (additions.is_some() && name == "PATH") {
            continue;
        }
        if let Some(reason) = bad_env_name(name) {
            return Err(format!("can't pass variable '{}': {}", name, reason));
        }
        if value.contains('\0') {
            return Err(format!(
                "can't pass variable '{}': it holds a NUL byte",
                name
            ));
        }
        script.push(' ');
        script.push_str(&shell_quote(&format!("{}={}", name, value)));
    }
    if let Some(additions) = additions {
        if additions.contains('\0') {
            return Err("can't pass PATH: it holds a NUL byte".to_string());
        }
        script.push(' ');
        script.push_str(&shell_quote(&format!("PATH={}:", additions)));
        script.push_str("\"$PATH\"");
    }
    script.push_str(" \"$@\"\n");
    Ok(script)
}

fn exec_args(id: &str, working_dir: &Path, cmd: &[&str]) -> Vec<String> {
    let mut args: Vec<String> = ["exec", "-i", "-w"]
        .iter()
        .map(|part| part.to_string())
        .collect();
    args.push(working_dir.display().to_string());
    args.push(id.to_string());
    args.extend(["sh", "-c", ENV_LOADER, "wrkflw-env"].map(str::to_string));
    args.extend(cmd.iter().map(|part| part.to_string()));
    args
}

/// Whether an env file holds `value` as written. It can't hold a line break,
/// and Podman 5 reads a leading quote as the start of a quoted value.
fn fits_env_file(value: &str) -> bool {
    !value.contains(['\n', '\r', '\0']) && !value.starts_with(['"', '\''])
}

/// A service's environment, for `docker run`: an `--env-file` that only this
/// user can read, plus `-e NAME=value` for a value an env file can't hold.
/// The file has to outlive the command.
fn service_env<'a>(
    service: &str,
    env: impl Iterator<Item = (&'a str, &'a str)>,
) -> Result<(Option<tempfile::NamedTempFile>, Vec<String>), String> {
    use std::io::Write;
    let mut lines = String::new();
    let mut args = Vec::new();
    for (name, value) in env {
        if let Some(reason) = bad_env_name(name) {
            return Err(format!(
                "service '{}': can't pass variable '{}': {}",
                service, name, reason
            ));
        }
        if fits_env_file(value) {
            lines.push_str(&format!("{}={}\n", name, value));
        } else {
            wrkflw_logging::warning(&format!(
                "Service '{}': {} spans lines, so it goes on the command line, \
                 where other users of this machine can see it",
                service, name
            ));
            args.push("-e".to_string());
            args.push(format!("{}={}", name, value));
        }
    }
    if lines.is_empty() {
        return Ok((None, args));
    }
    // `NamedTempFile` creates the file readable by this user only.
    let mut file = tempfile::NamedTempFile::new()
        .map_err(|e| format!("service '{}': can't write its env file: {}", service, e))?;
    file.write_all(lines.as_bytes())
        .and_then(|_| file.flush())
        .map_err(|e| format!("service '{}': can't write its env file: {}", service, e))?;
    let mut all = vec!["--env-file".to_string(), file.path().display().to_string()];
    all.extend(args);
    Ok((Some(file), all))
}

/// Flags a service's `options:` may take with a value. They shape the
/// service container and nothing outside it. Anything that mounts, shares a
/// namespace or adds privileges would let workflow YAML reach this machine.
const SERVICE_FLAGS_WITH_VALUE: &[&str] = &[
    "--health-cmd",
    "--health-interval",
    "--health-timeout",
    "--health-retries",
    "--health-start-period",
    "--health-start-interval",
    "--user",
    "-u",
    "--entrypoint",
    "--tmpfs",
    "--shm-size",
    "--ulimit",
    "--env",
    "-e",
    "--label",
    "-l",
    "--workdir",
    "-w",
    "--memory",
    "-m",
    "--memory-swap",
    "--cpus",
    "--stop-signal",
    "--stop-timeout",
    "--platform",
];

/// Flags a service's `options:` may take on their own.
const SERVICE_FLAGS_BARE: &[&str] = &["--no-healthcheck", "--init", "--read-only"];

/// A service's `options:`, checked against the flags above.
fn service_options(service: &str, options: &str) -> Result<Vec<String>, String> {
    let tokens = shlex::split(options)
        .ok_or_else(|| format!("service '{}': can't parse options", service))?;
    let mut tokens = tokens.into_iter();
    let mut args = Vec::new();
    while let Some(token) = tokens.next() {
        let (flag, inline) = if token.starts_with("--") {
            match token.split_once('=') {
                Some((flag, value)) => (flag.to_string(), Some(value.to_string())),
                None => (token.clone(), None),
            }
        } else if token.starts_with('-') && token.len() > 2 {
            let value = token[2..].strip_prefix('=').unwrap_or(&token[2..]);
            (token[..2].to_string(), Some(value.to_string()))
        } else {
            (token.clone(), None)
        };
        if SERVICE_FLAGS_BARE.contains(&flag.as_str()) {
            args.push(token);
            continue;
        }
        if !SERVICE_FLAGS_WITH_VALUE.contains(&flag.as_str()) {
            return Err(format!(
                "service '{}': option '{}' isn't allowed locally, as it could reach \
                 this machine. Allowed: {}, {}",
                service,
                flag,
                SERVICE_FLAGS_WITH_VALUE.join(" "),
                SERVICE_FLAGS_BARE.join(" ")
            ));
        }
        let value = inline
            .or_else(|| tokens.next())
            .ok_or_else(|| format!("service '{}': option '{}' needs a value", service, flag))?;
        if matches!(flag.as_str(), "--env" | "-e") && !value.contains('=') {
            return Err(format!(
                "service '{}': '{} {}' would copy a variable from this machine. \
                 Give it a value, or use the service's env",
                service, flag, value
            ));
        }
        args.push(flag);
        args.push(value);
    }
    Ok(args)
}

/// A service's `volumes:` entry, if it stays off this machine's filesystem: a
/// named volume, or a bare container path for an anonymous one.
fn service_volume(service: &str, volume: &str) -> Result<String, String> {
    let parts: Vec<&str> = volume.split(':').collect();
    let source = parts[0];
    let named = source.len() > 1
        && source.starts_with(|c: char| c.is_ascii_alphanumeric())
        && source
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || matches!(c, '_' | '.' | '-'));
    let anonymous = parts.len() == 1 && source.starts_with('/');
    if (named || anonymous) && !volume.contains('\\') {
        return Ok(volume.to_string());
    }
    Err(format!(
        "service '{}': volume '{}' mounts a path from this machine, which wrkflw \
         doesn't allow for services. Use a named volume",
        service, volume
    ))
}

/// Where the job's own Docker engine keeps its images and containers: a
/// volume, as it can't keep them on the container's layered filesystem.
const DOCKER_ENGINE_DIR: &str = "/var/lib/docker";

/// Where the job's own Docker engine logs.
const DOCKER_ENGINE_LOG: &str = "/var/log/wrkflw-dockerd.log";

/// How long the job's own Docker engine gets to answer.
const DOCKER_ENGINE_TIMEOUT: Duration = Duration::from_secs(60);

/// Starts the job's own Docker engine ahead of [`KEEP_ALIVE`], logging to
/// [`DOCKER_ENGINE_LOG`].
///
/// On cgroup v2 the engine can only give its containers controllers once
/// no process sits in the job container's own group, so the shell moves
/// there itself first, as the `docker:dind` image does. `exec` puts a
/// step beside the container's first process, which is that shell.
fn docker_engine_start() -> String {
    format!(
        "if [ -f /sys/fs/cgroup/cgroup.controllers ]; then \
           mkdir -p /sys/fs/cgroup/init && \
           xargs -rn1 < /sys/fs/cgroup/cgroup.procs > /sys/fs/cgroup/init/cgroup.procs 2>/dev/null; \
           sed -e 's/ / +/g' -e 's/^/+/' < /sys/fs/cgroup/cgroup.controllers \
             > /sys/fs/cgroup/cgroup.subtree_control 2>/dev/null; \
         fi; \
         dockerd > {DOCKER_ENGINE_LOG} 2>&1 & "
    )
}

fn start_args(
    name: &str,
    image: &str,
    volumes: &[(PathBuf, PathBuf)],
    service_names: &[&str],
    docker_engine: bool,
) -> Vec<String> {
    let mut args: Vec<String> = ["run", "-d", "--name", name, "--entrypoint", "sh"]
        .iter()
        .map(|part| part.to_string())
        .collect();
    if docker_engine {
        args.push("--privileged".to_string());
        args.push("-v".to_string());
        args.push(DOCKER_ENGINE_DIR.to_string());
    }
    for (host, container) in volumes {
        args.push("-v".to_string());
        args.push(format!("{}:{}", host.display(), container.display()));
    }
    // Services join this container's network, and Docker and Podman then hand
    // them this container's /etc/hosts, so these names resolve in every
    // service too. Both refuse `--add-host` on a container that joins
    // another's network, so services must not be given entries of their own.
    for service in service_names {
        args.push("--add-host".to_string());
        args.push(format!("{}:127.0.0.1", service));
    }
    args.push(image.to_string());
    args.push("-c".to_string());
    if docker_engine {
        args.push(format!("{}{}", docker_engine_start(), KEEP_ALIVE));
    } else {
        args.push(KEEP_ALIVE.to_string());
    }
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
        args.push(service_volume(&spec.name, volume)?);
    }
    if let Some(options) = &spec.options {
        args.extend(service_options(&spec.name, options)?);
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

/// The port inside the container that a `ports:` entry names.
fn container_port(port: &str) -> &str {
    let without_protocol = port.split('/').next().unwrap_or(port);
    without_protocol
        .rsplit(':')
        .next()
        .unwrap_or(without_protocol)
}

/// The repository part of an image reference, without tag or digest.
fn image_repository(image: &str) -> &str {
    let without_digest = image.split('@').next().unwrap_or(image);
    match without_digest.rsplit_once(':') {
        // A colon after the last '/' starts a tag. One before it is a
        // registry port, as in `localhost:5000/postgres`.
        Some((repository, tag)) if !tag.contains('/') => repository,
        _ => without_digest,
    }
}

/// Services share the job container's network namespace, so two that listen
/// on one port can't both start. An error for ports the workflow declares,
/// which would otherwise surface as a health check timing out, and a warning
/// for two services of one image, which most likely listen on the same one.
fn check_ports(services: &[ServiceSpec]) -> Result<(), String> {
    let mut ports: HashMap<&str, &str> = HashMap::new();
    let mut images: HashMap<&str, &str> = HashMap::new();
    for spec in services {
        let declared: std::collections::BTreeSet<&str> =
            spec.ports.iter().map(|port| container_port(port)).collect();
        for port in declared {
            if let Some(other) = ports.insert(port, &spec.name) {
                return Err(format!(
                    "services '{}' and '{}' both listen on port {}, and locally \
                     services share one network, so only one of them can",
                    other, spec.name, port
                ));
            }
        }
        if let Some(other) = images.insert(image_repository(&spec.image), &spec.name) {
            wrkflw_logging::warning(&format!(
                "Services '{}' and '{}' use the same image, and locally services share \
                 one network, so they may both try to listen on the same port",
                other, spec.name
            ));
        }
    }
    Ok(())
}

/// Run `cli` with `args`, writing `stdin` to it if given.
async fn cli_output(
    cli: &str,
    args: &[String],
    stdin: Option<&str>,
) -> Result<ContainerOutput, ContainerError> {
    run_cli(cli, args, stdin, false).await
}

/// `child`'s output, passed to the live tap as it is printed when `live`.
async fn wait_with_tap(
    child: tokio::process::Child,
    live: bool,
) -> std::io::Result<std::process::Output> {
    if live {
        wrkflw_runtime::live_output::wait_with_live_output(child).await
    } else {
        child.wait_with_output().await
    }
}

/// [`cli_output`], optionally streaming what the command prints to the live
/// tap; only a step's own command should.
async fn run_cli(
    cli: &str,
    args: &[String],
    stdin: Option<&str>,
    live: bool,
) -> Result<ContainerOutput, ContainerError> {
    use tokio::io::AsyncWriteExt;
    let start_error =
        |e: std::io::Error| ContainerError::ContainerStart(format!("Failed to run {}: {}", cli, e));
    let mut child = Command::new(cli)
        .args(args)
        .stdin(if stdin.is_some() {
            Stdio::piped()
        } else {
            Stdio::null()
        })
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .kill_on_drop(true)
        .spawn()
        .map_err(start_error)?;
    let pipe = child.stdin.take();
    // Written while the output is read, so neither side waits on a full pipe.
    // A failed write shows as the command's own failure, so it's ignored.
    let write = async move {
        if let (Some(mut pipe), Some(input)) = (pipe, stdin) {
            let _ = pipe.write_all(input.as_bytes()).await;
        }
    };
    let ((), output) = tokio::join!(write, wait_with_tap(child, live));
    let output = output.map_err(start_error)?;
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
            let output = cli_output(self.cli, &args, None)
                .await
                .map_err(|e| e.to_string())?;
            match readiness(&output.stdout) {
                Readiness::Ready => return Ok(()),
                Readiness::Failed => {
                    let log_args = vec!["logs".to_string(), name.to_string()];
                    let logs = cli_output(self.cli, &log_args, None).await.ok();
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
        check_ports(&self.services)?;
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
            let pairs = env.iter().map(|(k, v)| (k.as_str(), v.as_str()));
            // `_env_file` is read by the CLI, so it lives until the command ends.
            let (_env_file, env_args) = service_env(&spec.name, pairs)?;
            let args = service_args(&name, job_id, spec, &image, env_args)?;
            let output = cli_output(self.cli, &args, None)
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
        if self.docker_engine {
            // Said for each job, as the switch may come from the environment
            // and a run shouldn't grant this unseen.
            wrkflw_logging::warning(&format!(
                "Starting job container {} privileged, with a Docker engine of its own \
                 (--docker-in-jobs)",
                name
            ));
        }
        let service_names: Vec<&str> = self.services.iter().map(|s| s.name.as_str()).collect();
        let args = start_args(&name, image, volumes, &service_names, self.docker_engine);
        let output = cli_output(self.cli, &args, None)
            .await
            .map_err(|e| e.to_string())?;
        if output.exit_code != 0 {
            return Err(format!(
                "{}: {}",
                crate::setup_errors::JOB_CONTAINER,
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
        if self.docker_engine {
            self.wait_for_docker_engine(&id).await?;
        }
        let memory = match crate::job_memory::meter() {
            // Podman's containers aren't sampled anywhere, so the job keeps
            // the room an unmeasured job gets.
            Some(meter) if self.cli == "podman" => {
                meter.unmeasured();
                None
            }
            Some(meter) => Some(MemoryWatch::start(self.cli, containers.ids.clone(), meter)),
            None => None,
        };
        Ok(Live {
            image: image.to_string(),
            volumes: volumes.to_vec(),
            id,
            action_dir: None,
            _memory: memory,
            _cleanup: containers,
        })
    }

    /// Wait until the Docker engine started in job container `id` answers.
    async fn wait_for_docker_engine(&self, id: &str) -> Result<(), String> {
        let exec = |command: &[&str]| -> Vec<String> {
            ["exec", id]
                .iter()
                .chain(command)
                .map(|part| part.to_string())
                .collect()
        };
        let deadline = tokio::time::Instant::now() + DOCKER_ENGINE_TIMEOUT;
        loop {
            let answer = cli_output(self.cli, &exec(&["docker", "info"]), None).await;
            if answer.is_ok_and(|output| output.exit_code == 0) {
                return Ok(());
            }
            if tokio::time::Instant::now() >= deadline {
                let log = cli_output(self.cli, &exec(&["cat", DOCKER_ENGINE_LOG]), None)
                    .await
                    .map(|output| output.stdout)
                    .unwrap_or_default();
                return Err(format!(
                    "the job's own Docker engine didn't start within {}s: {}",
                    DOCKER_ENGINE_TIMEOUT.as_secs(),
                    last_chars(log.trim(), 2000)
                ));
            }
            tokio::time::sleep(SERVICE_POLL).await;
        }
    }

    /// Replace [`CONTAINER_ACTION_DIR`] in container `id` with a copy of `dir`.
    async fn copy_action_dir(&self, id: &str, dir: &Path) -> Result<(), String> {
        let clear: Vec<String> = ["exec", id, "rm", "-rf", CONTAINER_ACTION_DIR]
            .map(str::to_string)
            .to_vec();
        let copy = vec![
            "cp".to_string(),
            dir.display().to_string(),
            format!("{}:{}", id, CONTAINER_ACTION_DIR),
        ];
        for args in [clear, copy] {
            let output = cli_output(self.cli, &args, None)
                .await
                .map_err(|e| e.to_string())?;
            if output.exit_code != 0 {
                return Err(output.stderr.trim().to_string());
            }
        }
        Ok(())
    }

    /// Run a step in the session.
    ///
    /// `Ok(None)` means the caller should run the step the ordinary way: the
    /// step wants another image or other mounts, or the image can't idle.
    /// Never with services, as a step outside the job container can't reach
    /// them: that is an `Err`, which fails the step rather than running tests
    /// against a database that isn't there.
    pub(crate) async fn run(
        &self,
        image: &str,
        cmd: &[&str],
        env: &[(&str, &str)],
        working_dir: &Path,
        volumes: &[(&Path, &Path)],
        resolve: &dyn Fn(&str) -> String,
    ) -> Result<Option<ContainerOutput>, String> {
        // A composite action's steps mount its directory, which a running
        // container can't gain. They only read it, so it is copied in below
        // instead, and the step keeps the toolchain and files earlier steps
        // left, as on GitHub.
        let mut action_dir: Option<PathBuf> = None;
        let wanted: Vec<(PathBuf, PathBuf)> = volumes
            .iter()
            .filter(|(host, container)| {
                if *container == Path::new(CONTAINER_ACTION_DIR) && host.is_dir() {
                    action_dir = Some(host.to_path_buf());
                    false
                } else {
                    true
                }
            })
            .map(|(host, container)| (host.to_path_buf(), container.to_path_buf()))
            .collect();
        let mut state = self.state.lock().await;
        if let Some(message) = &state.failed {
            return Err(message.clone());
        }
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
                Err(e) => {
                    state.failed = Some(e.clone());
                    return Err(e);
                }
            }
        }
        let Some(live) = &state.live else {
            return Ok(None);
        };
        let same_mounts = live.volumes.len() == wanted.len()
            && wanted.iter().all(|volume| live.volumes.contains(volume));
        if live.image != image || !same_mounts {
            let difference = if live.image != image {
                format!(
                    "uses image {}, not the job container's {}",
                    image, live.image
                )
            } else {
                "needs other mounts than the job container".to_string()
            };
            if !self.services.is_empty() {
                return Err(format!(
                    "this step {}, so it can't reach the job's services",
                    difference
                ));
            }
            if !state.warned_outside {
                state.warned_outside = true;
                wrkflw_logging::warning(&format!(
                    "A step {}, so it runs in a container of its own: files it leaves \
                     outside the workspace won't be there for later steps",
                    difference
                ));
            }
            return Ok(None);
        }
        let id = live.id.clone();
        if let Some(dir) = action_dir {
            if live.action_dir.as_ref() != Some(&dir) {
                if let Err(e) = self.copy_action_dir(&id, &dir).await {
                    if !self.services.is_empty() {
                        return Err(format!(
                            "this step's action directory couldn't be copied to the job \
                             container, so it can't reach the job's services: {}",
                            e
                        ));
                    }
                    wrkflw_logging::warning(&format!(
                        "A step runs in a container of its own, as its action directory \
                         couldn't be copied to the job container: {}",
                        e
                    ));
                    return Ok(None);
                }
                if let Some(live) = state.live.as_mut() {
                    live.action_dir = Some(dir);
                }
            }
        }
        let script = env_script(env.iter().copied())?;
        state.busy = true;

        let args = exec_args(&id, working_dir, cmd);
        let watch = StepWatch(crate::job_memory::meter());
        let output = run_cli(self.cli, &args, Some(&script), true).await;
        watch.ended(output.as_ref().ok());
        state.busy = false;
        output.map(Some).map_err(|e| e.to_string())
    }
}

/// The session of the job this task is running, if it has one.
pub(crate) fn current() -> Option<Arc<JobSession>> {
    SESSION.try_with(|session| session.clone()).ok().flatten()
}

/// True when `runtime` can give `job` what a session gives it.
pub(crate) fn wanted_for(job: &Job, runtime: &dyn ContainerRuntime) -> Option<&'static str> {
    let cli = runtime.session_cli()?;
    let policy = acceleration::container_policy()?;
    // A kept-for-debugging container is one per failed step, which a shared
    // container can't provide, so that flag wins.
    (!policy.one_container_per_step() || !job.services.is_empty()).then_some(cli)
}

/// True when `job` gets a Docker engine of its own: its steps run
/// `docker`, the run allows it, and the job has a container on an image
/// wrkflw builds, which the engine is installed in. Only on Docker: a
/// rootless Podman container can't run an engine even when privileged.
///
/// Both the image and the session ask this, so it reads the job alone: a
/// matrix leg's steps differ from the job's only in `with:`, which says
/// nothing about running `docker`, and asking of the same steps keeps the
/// image and the session from ever disagreeing.
pub(crate) fn docker_engine_for(job: &Job, runtime: &dyn ContainerRuntime) -> bool {
    job.container.is_none()
        && acceleration::container_policy().is_some_and(|policy| policy.docker_in_jobs)
        && wanted_for(job, runtime) == Some("docker")
        && crate::script_commands::steps_run(&job.steps, &["docker"])
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
        if !services.is_empty()
            && acceleration::container_policy().is_some_and(|policy| policy.preserve_containers)
        {
            wrkflw_logging::warning(
                "--preserve-containers-on-failure doesn't apply to a job with services: \
                 its steps share one container with them, which is always removed",
            );
        }
        Arc::new(JobSession {
            cli,
            services,
            docker_engine: docker_engine_for(job, runtime),
            state: tokio::sync::Mutex::new(State::default()),
        })
    });
    SESSION.scope(session, future).await
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn exec_puts_no_variable_in_argv() {
        let args = exec_args("id", Path::new("/w"), &["true"]);
        assert!(args.contains(&"-i".to_string()));
        assert!(args.windows(2).any(|w| w == ["-c", ENV_LOADER]));
        assert!(!args.iter().any(|arg| arg == "-e"));
    }

    /// Run `cmd` the way a job container would, with `env` on stdin. The step
    /// itself runs in bash, the default `run:` shell: dash, which is `sh` on
    /// Ubuntu, drops variable names that aren't identifiers.
    #[cfg(unix)]
    fn through_loader(env: &[(&str, &str)], cmd: &str) -> String {
        use std::io::Write;
        let script = env_script(env.iter().copied()).unwrap();
        let mut child = std::process::Command::new("sh")
            .args(["-c", ENV_LOADER, "wrkflw-env", "bash", "-c", cmd])
            .env_clear()
            .env("PATH", std::env::var("PATH").unwrap())
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .spawn()
            .unwrap();
        child
            .stdin
            .take()
            .unwrap()
            .write_all(script.as_bytes())
            .unwrap();
        let output = child.wait_with_output().unwrap();
        assert!(output.status.success());
        String::from_utf8(output.stdout).unwrap()
    }

    #[cfg(unix)]
    #[test]
    fn path_additions_extend_the_containers_own_path() {
        let container_path = std::env::var("PATH").unwrap();
        let printed = through_loader(
            &[
                ("PATH", "/host/only/bin"),
                (PATH_ADDITIONS_ENV, "/opt/new:/opt/o'ld"),
            ],
            r#"printf '%s|%s' "$PATH" "${WRKFLW_PATH_ADDITIONS-unset}""#,
        );
        assert_eq!(
            printed,
            format!("/opt/new:/opt/o'ld:{}|unset", container_path)
        );
    }

    #[cfg(unix)]
    #[test]
    fn path_passes_through_when_no_step_added_to_it() {
        let printed = through_loader(&[("PATH", "/usr/bin:/bin")], r#"printf '%s' "$PATH""#);
        assert_eq!(printed, "/usr/bin:/bin");
    }

    #[cfg(unix)]
    #[test]
    fn variables_reach_the_step_exactly() {
        let key = "-----BEGIN KEY-----\nab'c\"d $(x) `y`\n-----END KEY-----\n\n";
        let printed = through_loader(
            &[("KEY", key), ("EMPTY", ""), ("INPUT_MY-INPUT", "v")],
            r#"printf '%s|%s|' "$KEY" "${EMPTY-unset}"; env | grep '^INPUT_MY-INPUT='"#,
        );
        assert_eq!(printed, format!("{}||INPUT_MY-INPUT=v\n", key));
    }

    #[test]
    fn a_name_the_container_cant_take_is_an_error() {
        for name in ["", "-x", "A=B", "A\nB"] {
            assert!(env_script([(name, "v")].into_iter()).is_err(), "{:?}", name);
        }
    }

    #[test]
    fn service_env_goes_in_a_file_and_only_odd_values_in_argv() {
        let env = [("PASSWORD", "hunter2"), ("CERT", "a\nb"), ("Q", "\"x\"")];
        let (file, args) = service_env("db", env.into_iter()).unwrap();
        let file = file.unwrap();
        assert_eq!(
            args[..2],
            ["--env-file".to_string(), file.path().display().to_string()]
        );
        assert_eq!(
            std::fs::read_to_string(file.path()).unwrap(),
            "PASSWORD=hunter2\n"
        );
        assert!(!args.iter().any(|arg| arg.contains("hunter2")));
        assert!(args.contains(&"CERT=a\nb".to_string()));
        assert!(args.contains(&"Q=\"x\"".to_string()));
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            let mode = std::fs::metadata(file.path()).unwrap().permissions().mode();
            assert_eq!(mode & 0o077, 0);
        }
    }

    #[test]
    fn service_options_that_reach_the_host_are_refused() {
        for options in [
            "--privileged",
            "-v /:/host",
            "--volume=/:/host",
            "--mount type=bind,src=/,dst=/h",
            "--pid=host",
            "--network host",
            "--cap-add SYS_ADMIN",
            "--security-opt seccomp=unconfined",
            "--device /dev/sda",
            "--env-file /etc/passwd",
            "-e AWS_SECRET_ACCESS_KEY",
            "-it",
            "postgres:16",
        ] {
            assert!(service_options("db", options).is_err(), "{}", options);
        }
    }

    #[test]
    fn service_options_for_the_service_itself_are_kept() {
        let args = service_options(
            "db",
            "--health-cmd 'pg_isready -U x' --health-interval=1s -e A=1 -eB=2 --init --tmpfs /run",
        )
        .unwrap();
        assert_eq!(
            args,
            [
                "--health-cmd",
                "pg_isready -U x",
                "--health-interval",
                "1s",
                "-e",
                "A=1",
                "-e",
                "B=2",
                "--init",
                "--tmpfs",
                "/run"
            ]
        );
        assert!(service_options("db", "--health-cmd").is_err());
    }

    #[test]
    fn service_volumes_must_stay_off_this_machine() {
        for volume in ["pgdata:/var/lib/postgresql/data", "cache:/c:ro", "/scratch"] {
            assert!(service_volume("db", volume).is_ok(), "{}", volume);
        }
        for volume in [
            "/home/me/.ssh:/x",
            "./data:/x",
            "~/data:/x",
            "C:/data:/x",
            "C:\\data:/x",
            "../x:/x",
        ] {
            assert!(service_volume("db", volume).is_err(), "{}", volume);
        }
    }

    /// A stand-in CLI that logs its arguments and answers like docker, apart
    /// from anything `fails` matches.
    #[cfg(unix)]
    fn fake_cli(dir: &Path, fails: &str) -> (&'static str, PathBuf) {
        use std::os::unix::fs::PermissionsExt;
        let log = dir.join("calls");
        let script = dir.join("cli");
        std::fs::write(
            &script,
            format!(
                "#!/bin/sh\necho \"$1 $*\" >> '{}'\ncase \"$*\" in\n{}) exit 1;;\nrm*) exit 0;;\ninspect*) echo running; exit 0;;\nexec*) echo step; exit 0;;\nesac\necho id-$$\n",
                log.display(),
                fails
            ),
        )
        .unwrap();
        std::fs::set_permissions(&script, std::fs::Permissions::from_mode(0o755)).unwrap();
        // A process another test forked while the script was open for writing
        // holds it until that process execs, and running the script meanwhile
        // fails with ETXTBSY. Once one run succeeds no writer is left, so wait
        // for that here instead of in the code under test.
        for _ in 0..100 {
            match std::process::Command::new(&script).arg("probe").output() {
                Err(e) if e.raw_os_error() == Some(26) => {
                    std::thread::sleep(std::time::Duration::from_millis(10))
                }
                Err(e) => panic!("can't run the fake CLI: {e}"),
                Ok(_) => break,
            }
        }
        let _ = std::fs::remove_file(&log);
        let cli: &'static str = Box::leak(script.display().to_string().into_boxed_str());
        (cli, log)
    }

    #[cfg(unix)]
    fn session(cli: &'static str, services: Vec<ServiceSpec>) -> JobSession {
        JobSession {
            cli,
            services,
            docker_engine: false,
            state: tokio::sync::Mutex::new(State::default()),
        }
    }

    #[cfg(unix)]
    fn db_service() -> ServiceSpec {
        ServiceSpec {
            name: "db".to_string(),
            image: "postgres:16".to_string(),
            env: HashMap::new(),
            volumes: vec![],
            options: None,
            ports: vec![],
        }
    }

    #[cfg(unix)]
    async fn step(session: &JobSession, image: &str) -> Result<Option<ContainerOutput>, String> {
        session
            .run(image, &["true"], &[], Path::new("/w"), &[], &|v| {
                v.to_string()
            })
            .await
    }

    #[cfg(unix)]
    fn calls(log: &Path, verb: &str) -> usize {
        std::fs::read_to_string(log)
            .unwrap_or_default()
            .lines()
            .filter(|line| line.starts_with(&format!("{} ", verb)))
            .count()
    }

    #[cfg(unix)]
    #[tokio::test]
    async fn steps_share_one_container() {
        let dir = tempfile::tempdir().unwrap();
        let (cli, log) = fake_cli(dir.path(), "nothing-fails");
        let session = session(cli, vec![]);
        assert!(step(&session, "ubuntu").await.unwrap().is_some());
        assert!(step(&session, "ubuntu").await.unwrap().is_some());
        assert_eq!(calls(&log, "run"), 1);
        assert_eq!(calls(&log, "exec"), 2);
    }

    #[cfg(unix)]
    #[tokio::test]
    async fn another_image_falls_back_to_its_own_container() {
        let dir = tempfile::tempdir().unwrap();
        let (cli, log) = fake_cli(dir.path(), "nothing-fails");
        let session = session(cli, vec![]);
        step(&session, "ubuntu").await.unwrap();
        assert!(step(&session, "alpine").await.unwrap().is_none());
        assert!(session.state.lock().await.warned_outside);
        assert!(step(&session, "alpine").await.unwrap().is_none());
        assert_eq!(calls(&log, "exec"), 1);
    }

    #[cfg(unix)]
    #[tokio::test]
    async fn a_start_failure_without_services_falls_back_once() {
        let dir = tempfile::tempdir().unwrap();
        let (cli, log) = fake_cli(dir.path(), "run*");
        let session = session(cli, vec![]);
        assert!(step(&session, "ubuntu").await.unwrap().is_none());
        assert!(step(&session, "ubuntu").await.unwrap().is_none());
        assert_eq!(calls(&log, "run"), 1);
    }

    #[cfg(unix)]
    #[tokio::test]
    async fn a_service_failure_fails_every_step_without_retrying() {
        let dir = tempfile::tempdir().unwrap();
        let (cli, log) = fake_cli(dir.path(), "run*postgres*");
        let session = session(cli, vec![db_service()]);
        let first = step(&session, "ubuntu").await.unwrap_err();
        assert!(first.contains("didn't start"), "{}", first);
        assert_eq!(step(&session, "ubuntu").await.unwrap_err(), first);
        assert_eq!(calls(&log, "run"), 2);
    }

    #[cfg(unix)]
    #[tokio::test]
    async fn another_image_fails_a_step_that_needs_services() {
        let dir = tempfile::tempdir().unwrap();
        let (cli, log) = fake_cli(dir.path(), "nothing-fails");
        let session = session(cli, vec![db_service()]);
        assert!(step(&session, "ubuntu").await.unwrap().is_some());
        let error = step(&session, "alpine").await.unwrap_err();
        assert!(error.contains("services"), "{}", error);
        assert_eq!(calls(&log, "exec"), 1);
    }

    #[cfg(unix)]
    #[tokio::test]
    async fn a_cancelled_step_retires_its_container() {
        let dir = tempfile::tempdir().unwrap();
        let (cli, log) = fake_cli(dir.path(), "nothing-fails");
        let session = session(cli, vec![]);
        step(&session, "ubuntu").await.unwrap();
        session.state.lock().await.busy = true;
        step(&session, "ubuntu").await.unwrap();
        assert_eq!(calls(&log, "run"), 2);
    }

    #[test]
    fn cleanup_on_exit_reaches_the_job_container_after_its_services() {
        let job = format!("job-{}", uuid::Uuid::new_v4());
        let db = format!("db-{}", uuid::Uuid::new_v4());
        let cache = format!("cache-{}", uuid::Uuid::new_v4());
        let mut containers = Containers {
            cli: "docker",
            ids: Vec::new(),
        };
        for id in [&job, &db, &cache] {
            containers.add(id);
        }
        let ours: Vec<String> = crate::docker::tracked_containers()
            .into_iter()
            .filter(|id| [&job, &db, &cache].contains(&id))
            .collect();
        assert_eq!(ours, [db.clone(), cache.clone(), job.clone()]);
        for id in &containers.ids {
            crate::docker::untrack_container(id);
        }
        // Nothing to remove: these containers never existed.
        containers.ids.clear();
    }

    #[cfg(unix)]
    #[test]
    fn a_failed_removal_says_why() {
        let run = |script: &str| {
            std::process::Command::new("sh")
                .args(["-c", script])
                .stderr(Stdio::piped())
                .output()
        };
        assert_eq!(removal_failure(run("exit 0")), None);
        assert_eq!(
            removal_failure(run("echo 'no such container' >&2; exit 1")).as_deref(),
            Some("no such container")
        );
        assert!(removal_failure(run("exit 3")).unwrap().contains('3'));
    }

    #[test]
    fn services_are_removed_before_the_job_container_they_depend_on() {
        let mut containers = Containers {
            cli: "true",
            ids: Vec::new(),
        };
        containers.ids.push("job".to_string());
        containers.ids.push("db".to_string());
        assert_eq!(containers.removal_args(), ["rm", "-f", "-v", "db", "job"]);
        containers.ids.clear();
    }

    #[test]
    fn exec_keeps_a_multi_line_script_as_one_argument() {
        let args = exec_args(
            "id",
            Path::new("/github/workspace/sub"),
            &["bash", "-c", "echo one\necho two"],
        );
        assert_eq!(
            args,
            [
                "exec",
                "-i",
                "-w",
                "/github/workspace/sub",
                "id",
                "sh",
                "-c",
                ENV_LOADER,
                "wrkflw-env",
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
            false,
        );
        assert!(!args.iter().any(|arg| arg == "--privileged"));
        assert!(args.windows(2).any(|w| w == ["-v", "/h:/c"]));
        assert!(args
            .windows(2)
            .any(|w| w == ["--add-host", "postgres:127.0.0.1"]));
    }

    #[test]
    fn a_job_with_its_own_docker_engine_runs_privileged_and_starts_it() {
        let args = start_args("n", "ubuntu:24.04", &[], &[], true);
        assert!(args.iter().any(|arg| arg == "--privileged"));
        // The engine's images go to a volume, removed with the container.
        assert!(args.windows(2).any(|w| w == ["-v", DOCKER_ENGINE_DIR]));
        let script = args.last().unwrap();
        assert!(script.contains("dockerd > "), "{script}");
        assert!(script.contains(DOCKER_ENGINE_LOG), "{script}");
        assert!(script.ends_with(KEEP_ALIVE), "{script}");
    }

    #[test]
    fn stats_lines_become_samples_of_the_sessions_containers() {
        let ids = [
            "aaaa1111bbbb2222".to_string(),
            "cccc3333dddd4444".to_string(),
        ];
        assert_eq!(
            stats_sample("aaaa1111bbbb2222 276.3MiB / 7.653GiB", &ids),
            Some(("aaaa1111bbbb2222", (276.3 * 1024.0 * 1024.0) as u64))
        );
        // A truncated id, and the screen cleared ahead of each round.
        assert_eq!(
            stats_sample("\x1b[2J\x1b[Hcccc3333dddd 1.5GiB / 7.653GiB", &ids),
            Some(("cccc3333dddd4444", (1.5 * 1024.0 * 1024.0 * 1024.0) as u64))
        );
        assert_eq!(
            stats_sample("aaaa1111bbbb2222 12.5MB / 2GB", &ids),
            Some(("aaaa1111bbbb2222", 12_500_000))
        );
        // Not running, not one of ours, and not a sample at all.
        assert_eq!(stats_sample("aaaa1111bbbb2222 -- / --", &ids), None);
        assert_eq!(stats_sample("eeee5555 10MiB / 1GiB", &ids), None);
        assert_eq!(stats_sample("", &ids), None);
    }

    #[test]
    fn a_step_cancelled_part_way_leaves_the_job_unmeasured() {
        let meter = Arc::new(crate::job_memory::JobMeter::default());
        let passed = ContainerOutput {
            stdout: String::new(),
            stderr: String::new(),
            exit_code: 0,
        };
        StepWatch(Some(meter.clone())).ended(Some(&passed));
        assert!(!meter.is_incomplete());
        drop(StepWatch(Some(meter.clone())));
        assert!(meter.is_incomplete());
    }

    /// A runtime that can keep a container running, and nothing else.
    struct SessionRuntime(&'static str);

    #[async_trait::async_trait]
    impl ContainerRuntime for SessionRuntime {
        async fn run_container(
            &self,
            _image: &str,
            _cmd: &[&str],
            _env_vars: &[(&str, &str)],
            _working_dir: &Path,
            _volumes: &[(&Path, &Path)],
            _entrypoint: Option<&str>,
        ) -> Result<ContainerOutput, ContainerError> {
            unreachable!("no step runs")
        }
        async fn pull_image(&self, _image: &str) -> Result<(), ContainerError> {
            Ok(())
        }
        async fn build_image(
            &self,
            _dockerfile: &Path,
            _tag: &str,
            _context_dir: &Path,
        ) -> Result<(), ContainerError> {
            Ok(())
        }
        async fn prepare_language_environment(
            &self,
            _language: &str,
            _version: Option<&str>,
            _additional_packages: Option<Vec<String>>,
        ) -> Result<String, ContainerError> {
            unreachable!("no environment is prepared")
        }
        async fn image_exists(&self, _tag: &str) -> Result<bool, ContainerError> {
            Ok(true)
        }
        fn session_cli(&self) -> Option<&'static str> {
            Some(self.0)
        }
    }

    fn job_running(run: &str) -> Job {
        serde_yaml::from_str(&format!("runs-on: ubuntu-latest\nsteps:\n  - run: {run}\n")).unwrap()
    }

    async fn in_run<T>(
        accel: acceleration::Acceleration,
        preserve_containers: bool,
        check: impl FnOnce() -> T,
    ) -> T {
        let state =
            acceleration::RunState::new(accel, preserve_containers, PathBuf::from(".")).unwrap();
        acceleration::with_run_state(Arc::new(state), async { check() }).await
    }

    #[tokio::test]
    async fn a_jobs_steps_share_a_container_unless_the_run_says_otherwise() {
        let job = job_running("echo hello");
        let wanted = || wanted_for(&job, &SessionRuntime("docker"));
        assert_eq!(
            in_run(acceleration::Acceleration::default(), false, wanted).await,
            Some("docker")
        );
        let step_containers = acceleration::Acceleration {
            step_containers: true,
            ..Default::default()
        };
        assert_eq!(in_run(step_containers, false, wanted).await, None);
        // A kept container is one per failed step.
        assert_eq!(
            in_run(acceleration::Acceleration::default(), true, wanted).await,
            None
        );
        // Outside a run, as in a GitLab pipeline.
        assert_eq!(wanted(), None);
    }

    #[tokio::test]
    async fn a_called_workflows_jobs_share_a_container_too() {
        let job = job_running("echo hello");
        let state =
            acceleration::RunState::new(Default::default(), false, PathBuf::from(".")).unwrap();
        let wanted = acceleration::with_run_state(
            Arc::new(state),
            acceleration::without_run_state(async { wanted_for(&job, &SessionRuntime("docker")) }),
        )
        .await;
        assert_eq!(wanted, Some("docker"));
    }

    #[tokio::test]
    async fn a_run_says_once_that_steps_lack_docker_called_workflows_included() {
        let state =
            acceleration::RunState::new(Default::default(), false, PathBuf::from(".")).unwrap();
        let (caller, called) = acceleration::with_run_state(Arc::new(state), async {
            let caller = acceleration::first_missing_docker_warning();
            let called = acceleration::without_run_state(async {
                acceleration::first_missing_docker_warning()
            })
            .await;
            (caller, called)
        })
        .await;
        assert!(caller);
        assert!(!called, "a called workflow said it again");
    }

    #[tokio::test]
    async fn only_a_run_that_allows_it_gives_a_job_a_docker_engine() {
        let docker = job_running("docker run alpine true");
        let plain = job_running("echo docker");
        let allowed = || acceleration::Acceleration {
            docker_in_jobs: true,
            ..Default::default()
        };
        let engine = |job: &Job| docker_engine_for(job, &SessionRuntime("docker"));
        assert!(in_run(allowed(), false, || engine(&docker)).await);
        assert!(!in_run(allowed(), false, || engine(&plain)).await);
        assert!(!in_run(Default::default(), false, || engine(&docker)).await);
        // The engine lives in the job's one container.
        let step_containers = acceleration::Acceleration {
            step_containers: true,
            ..allowed()
        };
        assert!(!in_run(step_containers, false, || engine(&docker)).await);
        // Rootless Podman can't run an engine inside a container.
        let on_podman = |job: &Job| docker_engine_for(job, &SessionRuntime("podman"));
        assert!(!in_run(allowed(), false, || on_podman(&docker)).await);
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
    fn two_services_on_one_port_are_refused() {
        let service = |name: &str, image: &str, ports: &[&str]| ServiceSpec {
            name: name.to_string(),
            image: image.to_string(),
            env: HashMap::new(),
            volumes: vec![],
            options: None,
            ports: ports.iter().map(|p| p.to_string()).collect(),
        };
        let clash = check_ports(&[
            service("primary", "postgres:16", &["5432:5432"]),
            service("replica", "postgres:16", &["5433:5432/tcp"]),
        ])
        .unwrap_err();
        assert!(clash.contains("5432"), "{}", clash);
        assert!(check_ports(&[
            service("db", "postgres:16", &["5432"]),
            service("cache", "redis:7", &["6379", "6379/tcp"]),
        ])
        .is_ok());
    }

    #[cfg(unix)]
    #[tokio::test]
    async fn a_port_clash_fails_before_any_service_starts() {
        let dir = tempfile::tempdir().unwrap();
        let (cli, log) = fake_cli(dir.path(), "nothing-fails");
        let mut primary = db_service();
        primary.ports = vec!["5432".to_string()];
        let mut replica = primary.clone();
        replica.name = "replica".to_string();
        let session = session(cli, vec![primary, replica]);
        let error = step(&session, "ubuntu").await.unwrap_err();
        assert!(error.contains("port 5432"), "{}", error);
        // Only the job container started, and it's removed with the session.
        assert_eq!(calls(&log, "run"), 1);
    }

    #[test]
    fn an_image_repository_drops_tag_and_digest_but_not_a_registry_port() {
        assert_eq!(image_repository("postgres:16"), "postgres");
        assert_eq!(image_repository("postgres"), "postgres");
        assert_eq!(
            image_repository("localhost:5000/pg:16"),
            "localhost:5000/pg"
        );
        assert_eq!(image_repository("localhost:5000/pg"), "localhost:5000/pg");
        assert_eq!(image_repository("pg@sha256:abc"), "pg");
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
