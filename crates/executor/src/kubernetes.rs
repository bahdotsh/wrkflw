//! Kubernetes runtime backend.
//!
//! Every [`ContainerRuntime::run_container`] call becomes one short-lived pod
//! in the configured cluster. The pod carries one `emptyDir` per requested
//! volume and three containers:
//!
//! * `wrkflw-wait`, an init container that blocks until a ready marker
//!   appears. While it runs, every host volume is streamed into it as a tar
//!   archive, so the workspace, GitHub env files and action checkouts are in
//!   place before the step starts.
//! * `main`, the step itself: the requested image, command, env and
//!   working directory.
//! * `wrkflw-sync`, a sidecar that keeps the pod alive after `main` exits so
//!   the volumes can be streamed back to the host. That is how
//!   `GITHUB_OUTPUT`, artifacts and workspace changes reach the executor and
//!   the next step.
//!
//! Only `kubectl` is required on the host. Images the executor builds itself
//! (combined runtime images, Dockerfile actions) additionally need Docker or
//! Podman locally and a registry the cluster can pull from, given with
//! `--k8s-registry`.

use async_trait::async_trait;
use once_cell::sync::Lazy;
use serde_json::{json, Value};
use std::collections::{BTreeMap, HashSet};
use std::io::{self, Read};
use std::path::{Component, Path, PathBuf};
use std::process::Stdio;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Mutex;
use std::time::Duration;
use tokio::process::Command;
use wrkflw_runtime::container::{
    ContainerError, ContainerOutput, ContainerRuntime, COMBINED_IMAGE_PREFIX, LOCAL_IMAGE_PREFIX,
};

/// Image used for the wait and sync helper containers. Anything with `sh`,
/// `tar`, `mkdir` and `touch` works.
pub const DEFAULT_HELPER_IMAGE: &str = "busybox:1.36";

/// Upper bound for one step, from pod creation to volume sync-back.
pub const DEFAULT_TIMEOUT_SECS: u64 = 3600;

/// Added to the step timeout for `activeDeadlineSeconds`, so the client's own
/// timeout normally fires first and reports the failure properly.
const DEADLINE_MARGIN_SECS: u64 = 30;

const AVAILABILITY_TIMEOUT: Duration = Duration::from_secs(5);

/// How much of a sync's stderr is kept for its error message.
const MAX_STDERR_BYTES: usize = 64 * 1024;

const MAIN_CONTAINER: &str = "main";
const WAIT_CONTAINER: &str = "wrkflw-wait";
const SYNC_CONTAINER: &str = "wrkflw-sync";
const SYNC_VOLUME: &str = "wrkflw-sync";
const SYNC_DIR: &str = "/wrkflw-sync";
const READY_MARKER: &str = "/wrkflw-sync/ready";
const MANAGED_BY_LABEL: &str = "app.kubernetes.io/managed-by";
const SESSION_LABEL: &str = "wrkflw.dev/session";

/// Waiting reasons that never resolve on their own. Anything else (plain
/// `ContainerCreating`, `PodInitializing`) is waited out.
const FATAL_WAITING_REASONS: &[&str] = &[
    "ErrImagePull",
    "ImagePullBackOff",
    "ErrImageNeverPull",
    "InvalidImageName",
    "CreateContainerConfigError",
    "CreateContainerError",
];

/// One id per process, stamped on every pod as a label so a crashed run's
/// pods can be found with `kubectl get pods -l wrkflw.dev/session=<id>`.
static SESSION_ID: Lazy<String> = Lazy::new(|| uuid::Uuid::new_v4().simple().to_string());

/// Pods created by this process that have not been deleted yet.
static RUNNING_PODS: Lazy<Mutex<Vec<PodRef>>> = Lazy::new(|| Mutex::new(Vec::new()));

/// Registry references pushed by this process. Global so the TUI, which
/// creates a runtime per run, does not push the same content-hashed image again.
static PUSHED_IMAGES: Lazy<Mutex<HashSet<String>>> = Lazy::new(|| Mutex::new(HashSet::new()));

#[derive(Debug, Clone)]
struct PodRef {
    kubectl_args: Vec<String>,
    name: String,
}

/// Set once cleanup starts. Jobs still running at that point must not leave
/// pods behind that the cleanup sweep has already missed.
static SHUTTING_DOWN: AtomicBool = AtomicBool::new(false);

fn shutdown_error() -> ContainerError {
    ContainerError::ContainerStart("wrkflw is shutting down".to_string())
}

/// Cluster settings for [`KubernetesRuntime`], as given on the command line.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct KubernetesConfig {
    /// kubeconfig context. `None` uses kubectl's current context.
    pub context: Option<String>,
    /// Namespace pods are created in. `None` uses the context's namespace.
    pub namespace: Option<String>,
    /// `nodeSelector` entries applied to every pod.
    pub node_selector: BTreeMap<String, String>,
    /// Resource quantities applied as both `limits` and `requests` on the
    /// step container, e.g. `nvidia.com/gpu` → `1`.
    pub resources: BTreeMap<String, String>,
    /// `imagePullSecrets` names applied to every pod.
    pub image_pull_secrets: Vec<String>,
    /// Image for the helper containers; see [`DEFAULT_HELPER_IMAGE`].
    pub helper_image: Option<String>,
    /// Registry prefix locally built images are pushed to, e.g.
    /// `registry.example.com/ci`. Required for jobs that build images.
    pub registry: Option<String>,
    /// Per-step timeout; see [`DEFAULT_TIMEOUT_SECS`].
    pub timeout_secs: Option<u64>,
}

impl KubernetesConfig {
    pub fn helper_image(&self) -> &str {
        self.helper_image.as_deref().unwrap_or(DEFAULT_HELPER_IMAGE)
    }

    pub fn timeout(&self) -> Duration {
        Duration::from_secs(self.timeout_secs.unwrap_or(DEFAULT_TIMEOUT_SECS))
    }

    /// Global kubectl flags selecting the cluster and namespace.
    fn kubectl_args(&self) -> Vec<String> {
        let mut args = Vec::new();
        if let Some(ctx) = &self.context {
            args.push("--context".to_string());
            args.push(ctx.clone());
        }
        if let Some(ns) = &self.namespace {
            args.push("--namespace".to_string());
            args.push(ns.clone());
        }
        args
    }

    /// Human-readable target, for log and error messages.
    pub fn describe(&self) -> String {
        format!(
            "context '{}', namespace '{}'",
            self.context.as_deref().unwrap_or("current"),
            self.namespace.as_deref().unwrap_or("current")
        )
    }
}

/// Parse a `KEY=VALUE` pair as given to `--k8s-node-selector` and
/// `--k8s-resource`. Both sides are trimmed and must be non-empty; the value
/// may itself contain `=`.
pub fn parse_key_value(s: &str) -> Result<(String, String), String> {
    let (key, value) = s
        .split_once('=')
        .ok_or_else(|| format!("invalid KEY=VALUE: no `=` found in `{}`", s))?;
    let (key, value) = (key.trim(), value.trim());
    if key.is_empty() || value.is_empty() {
        return Err(format!(
            "invalid KEY=VALUE: key and value must both be set in `{}`",
            s
        ));
    }
    Ok((key.to_string(), value.to_string()))
}

/// True when `kubectl` is installed and the current context answers.
pub fn is_available() -> bool {
    is_available_with(&KubernetesConfig::default())
}

/// True when `kubectl` is installed and the configured context answers.
pub fn is_available_with(config: &KubernetesConfig) -> bool {
    if which::which("kubectl").is_err() {
        return false;
    }
    let child = std::process::Command::new("kubectl")
        .args(config.kubectl_args())
        .args(["--request-timeout=3s", "get", "--raw", "/version"])
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn();
    let Ok(mut child) = child else {
        return false;
    };
    // `--request-timeout` does not cover exec credential plugins, and the TUI
    // calls this from its event loop, so enforce the limit here as well.
    let deadline = std::time::Instant::now() + AVAILABILITY_TIMEOUT;
    loop {
        match child.try_wait() {
            Ok(Some(status)) => return status.success(),
            Ok(None) if std::time::Instant::now() < deadline => {
                std::thread::sleep(Duration::from_millis(50));
            }
            _ => {
                let _ = child.kill();
                let _ = child.wait();
                return false;
            }
        }
    }
}

/// Delete every pod this process still owns. Called from the Ctrl+C handler.
pub async fn cleanup_resources() {
    SHUTTING_DOWN.store(true, Ordering::SeqCst);
    let pods: Vec<PodRef> = match RUNNING_PODS.lock() {
        Ok(mut guard) => guard.drain(..).collect(),
        Err(_) => return,
    };
    futures::future::join_all(pods.iter().map(force_delete_pod)).await;
}

fn force_delete_args(name: &str) -> [&str; 7] {
    [
        "delete",
        "pod",
        name,
        "--ignore-not-found",
        "--wait=false",
        "--grace-period=0",
        "--force",
    ]
}

async fn force_delete_pod(pod: &PodRef) {
    wrkflw_logging::info(&format!("Deleting Kubernetes pod {}", pod.name));
    let _ = Command::new("kubectl")
        .args(&pod.kubectl_args)
        .args(force_delete_args(&pod.name))
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .status()
        .await;
}

fn track_pod(pod: &PodRef) {
    if let Ok(mut pods) = RUNNING_PODS.lock() {
        pods.push(pod.clone());
    }
}

fn untrack_pod(name: &str) {
    if let Ok(mut pods) = RUNNING_PODS.lock() {
        pods.retain(|p| p.name != name);
    }
}

/// Owns a pod for as long as a step runs.
///
/// The engine enforces `timeout-minutes` by dropping the step's future, so
/// cleanup after an `.await` is not enough: nothing past the await point runs.
/// `Drop` cannot await either, and the process may exit right after the job
/// fails, so it starts a `kubectl delete` that outlives both this future and
/// the tokio runtime.
struct PodGuard {
    pod: PodRef,
    kubectl: std::ffi::OsString,
    armed: bool,
}

impl PodGuard {
    fn new(pod: PodRef) -> Self {
        track_pod(&pod);
        PodGuard {
            pod,
            kubectl: "kubectl".into(),
            armed: true,
        }
    }

    /// The pod was deleted or deliberately kept; leave it alone.
    fn disarm(&mut self) {
        self.armed = false;
    }
}

impl Drop for PodGuard {
    fn drop(&mut self) {
        if !self.armed {
            return;
        }
        untrack_pod(&self.pod.name);
        wrkflw_logging::info(&format!(
            "Step cancelled, deleting Kubernetes pod {}",
            self.pod.name
        ));
        let child = std::process::Command::new(&self.kubectl)
            .args(&self.pod.kubectl_args)
            .args(force_delete_args(&self.pod.name))
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .spawn();
        match child {
            // Only reaps it. If wrkflw exits first, the delete still finishes.
            Ok(mut child) => {
                std::thread::spawn(move || {
                    let _ = child.wait();
                });
            }
            Err(e) => wrkflw_logging::warning(&format!(
                "Failed to delete pod {}: {}; activeDeadlineSeconds will stop it",
                self.pod.name, e
            )),
        }
    }
}

/// One host directory and the container path it is exposed at.
#[derive(Debug, Clone, PartialEq, Eq)]
struct Mount {
    host: PathBuf,
    container: PathBuf,
}

/// Turn the caller's `(host, container)` pairs into an ordered mount plan.
///
/// Mounts are sorted by container path so a parent mount is populated before
/// a mount nested inside it, and a container path requested twice keeps its
/// first host directory, as a `docker run` with duplicate `-v` flags would.
fn plan_mounts(volumes: &[(&Path, &Path)]) -> Vec<Mount> {
    let mut mounts: Vec<Mount> = Vec::new();
    for (host, container) in volumes {
        // An emptyDir cannot stand in for a file or a host socket such as
        // /var/run/docker.sock, and a bare name is a Docker named volume.
        if host.exists() && !host.is_dir() {
            wrkflw_logging::warning(&format!(
                "Kubernetes: skipping mount of {}: only directories can be synced into a pod",
                host.display()
            ));
            continue;
        }
        if !host.is_absolute() {
            wrkflw_logging::warning(&format!(
                "Kubernetes: mounting {} empty: host path '{}' is not absolute, \
                 and named volumes do not exist on a cluster",
                container.display(),
                host.display()
            ));
        }
        if mounts.iter().any(|m| m.container == *container) {
            wrkflw_logging::warning(&format!(
                "Kubernetes: container path {} is mounted twice; keeping the first mapping",
                container.display()
            ));
            continue;
        }
        mounts.push(Mount {
            host: host.to_path_buf(),
            container: container.to_path_buf(),
        });
    }
    mounts.sort_by(|a, b| a.container.cmp(&b.container));
    mounts
}

/// Paths nested inside `mount` that belong to other mounts, relative to
/// `mount.container` and prefixed with `./` the way `tar -C dir .` names
/// them. They are excluded when the parent is streamed back out so a nested
/// emptyDir is not copied into the parent's host directory.
fn nested_excludes(mount: &Mount, all: &[Mount]) -> Vec<String> {
    all.iter()
        .filter(|other| other.container != mount.container)
        .filter_map(|other| other.container.strip_prefix(&mount.container).ok())
        .map(|rel| {
            let parts: Vec<_> = rel
                .components()
                .map(|c| c.as_os_str().to_string_lossy())
                .collect();
            format!("./{}", parts.join("/"))
        })
        .collect()
}

/// Everything that goes into one pod, independent of the cluster.
struct PodInput<'a> {
    name: &'a str,
    image: &'a str,
    cmd: &'a [&'a str],
    env_vars: &'a [(&'a str, &'a str)],
    working_dir: &'a Path,
    mounts: &'a [Mount],
    entrypoint: Option<&'a str>,
}

/// Build the pod manifest. Pure, so it is unit-tested without a cluster.
fn build_pod_manifest(config: &KubernetesConfig, input: &PodInput<'_>) -> Value {
    let volume_name = |i: usize| format!("vol-{}", i);

    let step_mounts: Vec<Value> = input
        .mounts
        .iter()
        .enumerate()
        .map(|(i, m)| json!({ "name": volume_name(i), "mountPath": m.container.to_string_lossy() }))
        .collect();
    let mut helper_mounts = step_mounts.clone();
    helper_mounts.push(json!({ "name": SYNC_VOLUME, "mountPath": SYNC_DIR }));

    let mut volumes: Vec<Value> = (0..input.mounts.len())
        .map(|i| json!({ "name": volume_name(i), "emptyDir": {} }))
        .collect();
    volumes.push(json!({ "name": SYNC_VOLUME, "emptyDir": {} }));

    let env: Vec<Value> = input
        .env_vars
        .iter()
        .map(|(k, v)| json!({ "name": k, "value": escape_k8s_expansion(v) }))
        .collect();

    let mut main = json!({
        "name": MAIN_CONTAINER,
        "image": input.image,
        "env": env,
        "workingDir": input.working_dir.to_string_lossy(),
        "volumeMounts": step_mounts,
    });
    // Docker semantics: `--entrypoint` replaces ENTRYPOINT, trailing args
    // replace CMD. Kubernetes spells those `command` and `args`.
    if let Some(ep) = input.entrypoint.filter(|s| !s.is_empty()) {
        main["command"] = json!([escape_k8s_expansion(ep)]);
    }
    if !input.cmd.is_empty() {
        main["args"] = json!(input
            .cmd
            .iter()
            .map(|arg| escape_k8s_expansion(arg))
            .collect::<Vec<_>>());
    }
    if !config.resources.is_empty() {
        main["resources"] = json!({
            "limits": config.resources,
            "requests": config.resources,
        });
    }

    let wait = json!({
        "name": WAIT_CONTAINER,
        "image": config.helper_image(),
        "command": ["sh", "-c", format!("until [ -f {} ]; do sleep 0.2; done", READY_MARKER)],
        "volumeMounts": helper_mounts,
        "resources": helper_resources(),
    });
    let sync = json!({
        "name": SYNC_CONTAINER,
        "image": config.helper_image(),
        "command": ["sh", "-c", "while true; do sleep 3600; done"],
        "volumeMounts": helper_mounts,
        "resources": helper_resources(),
    });

    let mut spec = json!({
        "restartPolicy": "Never",
        "automountServiceAccountToken": false,
        "terminationGracePeriodSeconds": 2,
        // The sidecar never exits on its own. If wrkflw dies, this is what
        // stops an orphaned pod from holding its node's GPUs forever.
        "activeDeadlineSeconds": config.timeout().as_secs() + DEADLINE_MARGIN_SECS,
        "initContainers": [wait],
        "containers": [main, sync],
        "volumes": volumes,
    });
    if !config.node_selector.is_empty() {
        spec["nodeSelector"] = json!(config.node_selector);
    }
    if !config.image_pull_secrets.is_empty() {
        spec["imagePullSecrets"] = json!(config
            .image_pull_secrets
            .iter()
            .map(|name| json!({ "name": name }))
            .collect::<Vec<_>>());
    }

    json!({
        "apiVersion": "v1",
        "kind": "Pod",
        "metadata": {
            "name": input.name,
            "labels": {
                MANAGED_BY_LABEL: "wrkflw",
                SESSION_LABEL: *SESSION_ID,
            },
        },
        "spec": spec,
    })
}

/// Requests and limits for the helper containers. They only run `sh` and
/// `tar`, but a namespace with a `ResourceQuota` rejects any container that
/// does not declare these, and there would be no flag to fix it with.
fn helper_resources() -> Value {
    json!({
        "requests": { "cpu": "10m", "memory": "32Mi" },
        "limits": { "cpu": "500m", "memory": "256Mi" },
    })
}

/// Protect a string from Kubernetes' own variable expansion.
///
/// The API server treats `$(NAME)` as a reference and `$$` as an escaped `$`
/// in `command`, `args` and `env[].value`. Left alone, a script's `echo $$`
/// would reach the shell as `echo $`. Doubling every `$` makes the kubelet
/// hand the original string to the container.
fn escape_k8s_expansion(s: &str) -> String {
    s.replace('$', "$$")
}

/// Registry reference a local `tag` is pushed under.
fn registry_ref(registry: &str, tag: &str) -> String {
    format!("{}/{}", registry.trim_end_matches('/'), tag)
}

fn is_locally_built(image: &str) -> bool {
    image.starts_with(LOCAL_IMAGE_PREFIX) || image.starts_with(COMBINED_IMAGE_PREFIX)
}

fn pod_name() -> String {
    format!("wrkflw-{}", uuid::Uuid::new_v4().simple())
}

/// How long to wait before asking about a pod again.
///
/// Every poll is a new `kubectl` process, which re-runs the kubeconfig's exec
/// credential plugin: on EKS that is an STS call. Short steps stay snappy and
/// a long training run does not cost one of those per second.
fn poll_interval(elapsed: Duration) -> Duration {
    match elapsed.as_secs() {
        0..=9 => Duration::from_millis(500),
        10..=59 => Duration::from_secs(1),
        _ => Duration::from_secs(5),
    }
}

/// `status.<list>[name == container].state`, if reported yet.
fn container_state<'a>(pod: &'a Value, list: &str, container: &str) -> Option<&'a Value> {
    pod.pointer(&format!("/status/{}", list))?
        .as_array()?
        .iter()
        .find(|c| c["name"] == container)
        .map(|c| &c["state"])
}

fn is_running(state: Option<&Value>) -> bool {
    state.map(|s| s["running"].is_object()).unwrap_or(false)
}

fn terminated_exit_code(state: Option<&Value>) -> Option<i32> {
    state?["terminated"]["exitCode"]
        .as_i64()
        .map(|code| code as i32)
}

/// A container stuck in a waiting state that will never clear, as
/// `"<container>: <reason>: <message>"`.
fn fatal_waiting_reason(pod: &Value) -> Option<String> {
    for list in ["initContainerStatuses", "containerStatuses"] {
        // A list that is not reported yet must not hide the other one.
        let Some(statuses) = pod
            .pointer(&format!("/status/{}", list))
            .and_then(Value::as_array)
        else {
            continue;
        };
        for status in statuses {
            let waiting = &status["state"]["waiting"];
            if let Some(reason) = waiting["reason"].as_str() {
                if FATAL_WAITING_REASONS.contains(&reason) {
                    return Some(format!(
                        "container '{}': {}: {}",
                        status["name"].as_str().unwrap_or("?"),
                        reason,
                        waiting["message"].as_str().unwrap_or("")
                    ));
                }
            }
        }
    }
    None
}

/// The scheduler's message when the pod cannot be placed yet. Not fatal: an
/// autoscaler may still bring up a matching node.
fn unschedulable_message(pod: &Value) -> Option<String> {
    pod.pointer("/status/conditions")?
        .as_array()?
        .iter()
        .find(|c| c["type"] == "PodScheduled" && c["reason"] == "Unschedulable")
        .map(|c| c["message"].as_str().unwrap_or("Unschedulable").to_string())
}

fn pod_phase(pod: &Value) -> &str {
    pod.pointer("/status/phase")
        .and_then(Value::as_str)
        .unwrap_or("Unknown")
}

fn pod_failure_message(pod: &Value) -> String {
    let reason = pod
        .pointer("/status/reason")
        .and_then(Value::as_str)
        .unwrap_or("");
    let message = pod
        .pointer("/status/message")
        .and_then(Value::as_str)
        .unwrap_or("");
    match (reason.is_empty(), message.is_empty()) {
        (true, true) => format!("pod phase is {}", pod_phase(pod)),
        (false, true) => reason.to_string(),
        (true, false) => message.to_string(),
        (false, false) => format!("{}: {}", reason, message),
    }
}

/// Runs steps as pods in a Kubernetes cluster through `kubectl`.
pub struct KubernetesRuntime {
    config: KubernetesConfig,
    preserve_pods_on_failure: bool,
}

impl KubernetesRuntime {
    /// Construct without probing the cluster. Callers should have checked
    /// [`is_available_with`] first.
    pub fn new(config: KubernetesConfig, preserve_pods_on_failure: bool) -> Self {
        KubernetesRuntime {
            config,
            preserve_pods_on_failure,
        }
    }

    fn kubectl_args(&self) -> Vec<String> {
        self.config.kubectl_args()
    }

    /// Run `kubectl <args>` to completion, feeding `stdin` if given.
    async fn kubectl(
        &self,
        args: &[&str],
        stdin: Option<Vec<u8>>,
    ) -> Result<std::process::Output, ContainerError> {
        let mut cmd = Command::new("kubectl");
        cmd.args(self.kubectl_args()).args(args);
        cmd.stdin(if stdin.is_some() {
            Stdio::piped()
        } else {
            Stdio::null()
        });
        cmd.stdout(Stdio::piped()).stderr(Stdio::piped());
        // A cancelled step must not leave a `kubectl create` racing its delete.
        cmd.kill_on_drop(true);
        wrkflw_logging::debug(&format!("Running: kubectl {}", args.join(" ")));

        let mut child = cmd.spawn().map_err(|e| {
            ContainerError::ContainerStart(format!("Failed to spawn kubectl: {}", e))
        })?;
        if let Some(data) = stdin {
            if let Some(mut pipe) = child.stdin.take() {
                use tokio::io::AsyncWriteExt;
                pipe.write_all(&data).await.map_err(|e| {
                    ContainerError::ContainerExecution(format!(
                        "Failed to write to kubectl stdin: {}",
                        e
                    ))
                })?;
                pipe.shutdown().await.ok();
            }
        }
        child.wait_with_output().await.map_err(|e| {
            ContainerError::ContainerExecution(format!("kubectl did not complete: {}", e))
        })
    }

    /// Run `kubectl <args>` and require success, returning stdout.
    async fn kubectl_ok(
        &self,
        args: &[&str],
        stdin: Option<Vec<u8>>,
        what: &str,
    ) -> Result<String, ContainerError> {
        let output = self.kubectl(args, stdin).await?;
        if !output.status.success() {
            return Err(ContainerError::ContainerExecution(format!(
                "{} failed ({}): {}",
                what,
                self.config.describe(),
                String::from_utf8_lossy(&output.stderr).trim()
            )));
        }
        Ok(String::from_utf8_lossy(&output.stdout).to_string())
    }

    async fn get_pod(&self, name: &str) -> Result<Value, ContainerError> {
        let raw = self
            .kubectl_ok(&["get", "pod", name, "-o", "json"], None, "kubectl get pod")
            .await?;
        serde_json::from_str(&raw).map_err(|e| {
            ContainerError::ContainerExecution(format!(
                "Unexpected `kubectl get pod` output: {}",
                e
            ))
        })
    }

    async fn delete_pod(&self, name: &str) {
        let output = self
            .kubectl(
                &["delete", "pod", name, "--ignore-not-found", "--wait=false"],
                None,
            )
            .await;
        match output {
            Ok(o) if o.status.success() => wrkflw_logging::debug(&format!("Deleted pod {}", name)),
            Ok(o) => wrkflw_logging::warning(&format!(
                "Failed to delete pod {}: {}",
                name,
                String::from_utf8_lossy(&o.stderr).trim()
            )),
            Err(e) => wrkflw_logging::warning(&format!("Failed to delete pod {}: {}", name, e)),
        }
        untrack_pod(name);
    }

    /// Where a locally built `tag` lives once [`Self::build_image`] pushed it.
    fn pushed_ref(&self, tag: &str) -> Option<String> {
        let remote = registry_ref(self.config.registry.as_deref()?, tag);
        let pushed = PUSHED_IMAGES.lock().ok()?;
        pushed.contains(&remote).then_some(remote)
    }

    /// Image reference the cluster can pull for `image`.
    fn cluster_image(&self, image: &str) -> Result<String, ContainerError> {
        if !is_locally_built(image) {
            return Ok(image.to_string());
        }
        self.pushed_ref(image).ok_or_else(|| {
            ContainerError::ContainerStart(format!(
                "image '{}' was built locally and is not reachable from the cluster; \
                 pass --k8s-registry so wrkflw can push it",
                image
            ))
        })
    }

    /// Block until the wait container runs, so volumes can be streamed into it.
    async fn wait_for_init_running(&self, name: &str) -> Result<(), ContainerError> {
        let mut warned_unschedulable = false;
        let started = std::time::Instant::now();
        loop {
            let pod = self.get_pod(name).await?;
            if let Some(reason) = fatal_waiting_reason(&pod) {
                return Err(ContainerError::ContainerStart(format!(
                    "pod {} cannot start: {}",
                    name, reason
                )));
            }
            if is_running(container_state(
                &pod,
                "initContainerStatuses",
                WAIT_CONTAINER,
            )) {
                return Ok(());
            }
            match pod_phase(&pod) {
                "Failed" | "Succeeded" => {
                    return Err(ContainerError::ContainerStart(format!(
                        "pod {} ended before its workspace was synced: {}",
                        name,
                        pod_failure_message(&pod)
                    )));
                }
                _ => {}
            }
            if !warned_unschedulable {
                if let Some(msg) = unschedulable_message(&pod) {
                    wrkflw_logging::warning(&format!(
                        "Pod {} is not schedulable yet, waiting: {}",
                        name, msg
                    ));
                    warned_unschedulable = true;
                }
            }
            tokio::time::sleep(poll_interval(started.elapsed())).await;
        }
    }

    /// Block until the step container has exited, returning its exit code.
    async fn wait_for_main_terminated(&self, name: &str) -> Result<i32, ContainerError> {
        let started = std::time::Instant::now();
        loop {
            let pod = self.get_pod(name).await?;
            if let Some(code) =
                terminated_exit_code(container_state(&pod, "containerStatuses", MAIN_CONTAINER))
            {
                return Ok(code);
            }
            if let Some(reason) = fatal_waiting_reason(&pod) {
                return Err(ContainerError::ContainerStart(format!(
                    "pod {} cannot start: {}",
                    name, reason
                )));
            }
            if pod_phase(&pod) == "Failed" {
                return Err(ContainerError::ContainerExecution(format!(
                    "pod {} failed: {}",
                    name,
                    pod_failure_message(&pod)
                )));
            }
            tokio::time::sleep(poll_interval(started.elapsed())).await;
        }
    }

    /// Stream `host` into `mount.container` inside the wait container.
    async fn copy_in(&self, name: &str, mount: &Mount) -> Result<(), ContainerError> {
        if !mount.host.is_absolute() || !mount.host.exists() {
            wrkflw_logging::debug(&format!(
                "Kubernetes: host path {} does not exist, mounting {} empty",
                mount.host.display(),
                mount.container.display()
            ));
            return Ok(());
        }
        let base = self.kubectl_args();
        let pod = name.to_string();
        let mount = mount.clone();
        tokio::task::spawn_blocking(move || copy_dir_into_pod(&base, &pod, &mount))
            .await
            .map_err(|e| ContainerError::ContainerExecution(format!("copy task failed: {}", e)))?
    }

    /// Stream `mount.container` out of the sync container onto `host`.
    async fn copy_out(
        &self,
        name: &str,
        mount: &Mount,
        excludes: Vec<String>,
    ) -> Result<(), ContainerError> {
        if !mount.host.is_absolute() {
            return Ok(());
        }
        let base = self.kubectl_args();
        let pod = name.to_string();
        let mount = mount.clone();
        tokio::task::spawn_blocking(move || copy_dir_out_of_pod(&base, &pod, &mount, &excludes))
            .await
            .map_err(|e| ContainerError::ContainerExecution(format!("copy task failed: {}", e)))?
    }

    #[allow(clippy::too_many_arguments)]
    async fn run_container_inner(
        &self,
        name: &str,
        image: &str,
        cmd: &[&str],
        env_vars: &[(&str, &str)],
        working_dir: &Path,
        volumes: &[(&Path, &Path)],
        entrypoint: Option<&str>,
    ) -> Result<ContainerOutput, ContainerError> {
        let cluster_image = self.cluster_image(image)?;
        let mounts = plan_mounts(volumes);
        let manifest = build_pod_manifest(
            &self.config,
            &PodInput {
                name,
                image: &cluster_image,
                cmd,
                env_vars,
                working_dir,
                mounts: &mounts,
                entrypoint,
            },
        );

        wrkflw_logging::debug(&format!("Command: {:?}", cmd));
        wrkflw_logging::debug(&format!("Working directory: {}", working_dir.display()));

        // Armed before the pod exists: cancellation can land inside `create`.
        let mut guard = PodGuard::new(PodRef {
            kubectl_args: self.kubectl_args(),
            name: name.to_string(),
        });
        let result = self.run_pod(name, &manifest, working_dir, &mounts).await;

        match &result {
            Ok(output) if output.exit_code == 0 => self.delete_pod(name).await,
            Ok(output) if self.preserve_pods_on_failure => {
                wrkflw_logging::info(&format!(
                    "Preserving failed pod {} for debugging (exit code: {}). \
                     Inspect with 'kubectl logs {} -c {}' or 'kubectl exec -it {} -c {} -- sh', \
                     then delete it with 'kubectl delete pod {}'.",
                    name, output.exit_code, name, MAIN_CONTAINER, name, SYNC_CONTAINER, name
                ));
                untrack_pod(name);
            }
            Ok(output) => {
                wrkflw_logging::info(&format!(
                    "Kubernetes step failed with exit code: {}",
                    output.exit_code
                ));
                self.delete_pod(name).await;
            }
            Err(e) => {
                wrkflw_logging::error(&format!("Kubernetes execution error: {}", e));
                self.delete_pod(name).await;
            }
        }
        guard.disarm();
        result
    }

    /// Create the pod, sync volumes in, run the step, sync volumes out.
    async fn run_pod(
        &self,
        name: &str,
        manifest: &Value,
        working_dir: &Path,
        mounts: &[Mount],
    ) -> Result<ContainerOutput, ContainerError> {
        if SHUTTING_DOWN.load(Ordering::SeqCst) {
            return Err(shutdown_error());
        }
        let body = serde_json::to_vec(manifest).map_err(|e| {
            ContainerError::ContainerStart(format!("Failed to serialize pod manifest: {}", e))
        })?;
        let output = self.kubectl(&["create", "-f", "-"], Some(body)).await?;
        if !output.status.success() {
            return Err(ContainerError::ContainerStart(format!(
                "kubectl create pod failed ({}): {}",
                self.config.describe(),
                String::from_utf8_lossy(&output.stderr).trim()
            )));
        }
        // The pod may have been created just after the sweep looked for it.
        // `run_container_inner` deletes it on this error.
        if SHUTTING_DOWN.load(Ordering::SeqCst) {
            return Err(shutdown_error());
        }
        wrkflw_logging::info(&format!(
            "Kubernetes: created pod {} ({})",
            name,
            self.config.describe()
        ));

        self.wait_for_init_running(name).await?;
        for mount in mounts {
            self.copy_in(name, mount).await?;
        }
        // Create the working directory if it is inside a fresh volume, then
        // release the init container so the step starts.
        let wd = working_dir.to_string_lossy().to_string();
        self.kubectl_ok(
            &[
                "exec",
                name,
                "-c",
                WAIT_CONTAINER,
                "--",
                "sh",
                "-c",
                &format!("mkdir -p \"$1\" && touch {}", READY_MARKER),
                "sh",
                &wd,
            ],
            None,
            "releasing the step",
        )
        .await?;

        let exit_code = self.wait_for_main_terminated(name).await?;

        let logs = match self
            .kubectl(&["logs", name, "-c", MAIN_CONTAINER], None)
            .await
        {
            Ok(o) if o.status.success() => String::from_utf8_lossy(&o.stdout).to_string(),
            Ok(o) => {
                wrkflw_logging::warning(&format!(
                    "Could not read logs of pod {}: {}",
                    name,
                    String::from_utf8_lossy(&o.stderr).trim()
                ));
                String::new()
            }
            Err(e) => {
                wrkflw_logging::warning(&format!("Could not read logs of pod {}: {}", name, e));
                String::new()
            }
        };

        for mount in mounts {
            self.copy_out(name, mount, nested_excludes(mount, mounts))
                .await?;
        }

        // Kubernetes interleaves stdout and stderr in one log stream.
        Ok(ContainerOutput {
            stdout: logs,
            stderr: String::new(),
            exit_code,
        })
    }

    /// Docker or Podman, whichever can build images on this host.
    fn local_builder() -> Option<&'static str> {
        if crate::docker::is_available() {
            Some("docker")
        } else if crate::podman::is_available() {
            Some("podman")
        } else {
            None
        }
    }

    fn registry(&self) -> Result<&str, ContainerError> {
        self.config.registry.as_deref().ok_or_else(|| {
            ContainerError::ImageBuild(
                "this job needs an image built locally, which the cluster cannot pull; \
                 pass --k8s-registry REGISTRY so wrkflw can push it"
                    .to_string(),
            )
        })
    }

    async fn builder_command(
        &self,
        builder: &str,
        args: &[&str],
        timeout: Duration,
        what: &str,
    ) -> Result<(), ContainerError> {
        wrkflw_logging::debug(&format!("Running: {} {}", builder, args.join(" ")));
        let output = tokio::time::timeout(
            timeout,
            Command::new(builder)
                .args(args)
                .stdin(Stdio::null())
                .stdout(Stdio::piped())
                .stderr(Stdio::piped())
                .output(),
        )
        .await
        .map_err(|_| {
            ContainerError::ImageBuild(format!(
                "{} timed out after {} seconds",
                what,
                timeout.as_secs()
            ))
        })?
        .map_err(|e| ContainerError::ImageBuild(format!("{} failed to start: {}", what, e)))?;
        if !output.status.success() {
            return Err(ContainerError::ImageBuild(format!(
                "{} failed: {}",
                what,
                String::from_utf8_lossy(&output.stderr).trim()
            )));
        }
        Ok(())
    }

    /// Tag a local image for the registry, push it and remember that it is there.
    async fn push_local_image(&self, builder: &str, tag: &str) -> Result<(), ContainerError> {
        let remote = registry_ref(self.registry()?, tag);
        self.builder_command(
            builder,
            &["tag", tag, &remote],
            Duration::from_secs(30),
            "tagging image",
        )
        .await?;
        wrkflw_logging::info(&format!("Pushing image {} to {}", tag, remote));
        self.builder_command(
            builder,
            &["push", &remote],
            Duration::from_secs(600),
            "pushing image",
        )
        .await?;
        if let Ok(mut pushed) = PUSHED_IMAGES.lock() {
            pushed.insert(remote);
        }
        Ok(())
    }
}

/// Read `child`'s stderr to the end on its own thread, keeping the first
/// [`MAX_STDERR_BYTES`].
///
/// Whoever is busy with the child's stdin or stdout cannot also read stderr.
/// A child that fills that pipe would block with us blocked on it in turn,
/// until `--k8s-timeout` ended the step.
fn drain_stderr(child: &mut std::process::Child) -> std::thread::JoinHandle<String> {
    let stderr = child.stderr.take();
    std::thread::spawn(move || {
        let Some(mut stderr) = stderr else {
            return String::new();
        };
        let mut kept = Vec::new();
        let mut buf = [0u8; 8192];
        while let Ok(n) = stderr.read(&mut buf) {
            if n == 0 {
                break;
            }
            let room = MAX_STDERR_BYTES - kept.len();
            kept.extend_from_slice(&buf[..n.min(room)]);
        }
        String::from_utf8_lossy(&kept).trim().to_string()
    })
}

/// Blocking: tar `mount.host` and pipe it into `tar -x` in the wait container.
fn copy_dir_into_pod(
    kubectl_args: &[String],
    pod: &str,
    mount: &Mount,
) -> Result<(), ContainerError> {
    let dest = mount.container.to_string_lossy().to_string();
    let mut command = std::process::Command::new("kubectl");
    command.args(kubectl_args).args([
        "exec",
        "-i",
        pod,
        "-c",
        WAIT_CONTAINER,
        "--",
        "tar",
        "-C",
        &dest,
        "-xof",
        "-",
    ]);
    stream_dir_into(command, mount)?;
    wrkflw_logging::debug(&format!(
        "Kubernetes: synced {} -> {}:{}",
        mount.host.display(),
        pod,
        dest
    ));
    Ok(())
}

/// Blocking: write `mount.host` as a tar archive to `command`'s stdin.
fn stream_dir_into(
    mut command: std::process::Command,
    mount: &Mount,
) -> Result<(), ContainerError> {
    let mut child = command
        .stdin(Stdio::piped())
        .stdout(Stdio::null())
        .stderr(Stdio::piped())
        .spawn()
        .map_err(|e| {
            ContainerError::ContainerStart(format!("Failed to spawn kubectl exec: {}", e))
        })?;
    let stderr = drain_stderr(&mut child);

    let stdin = child.stdin.take().ok_or_else(|| {
        ContainerError::ContainerExecution("kubectl exec stdin was not captured".to_string())
    })?;
    let mut builder = tar::Builder::new(stdin);
    // Keep mtimes and modes so tools comparing timestamps behave as they would
    // on a bind mount. The host's uid means nothing in the pod, so `tar -xo`
    // discards it. Symlinks are shipped as symlinks, not followed.
    builder.mode(tar::HeaderMode::Complete);
    builder.follow_symlinks(false);
    let archived = builder
        .append_dir_all(".", &mount.host)
        .and_then(|_| builder.into_inner())
        .map(drop);

    let status = child.wait().map_err(|e| {
        ContainerError::ContainerExecution(format!("kubectl exec did not complete: {}", e))
    })?;
    let stderr = stderr.join().unwrap_or_default();
    if !status.success() {
        return Err(ContainerError::ContainerExecution(format!(
            "syncing {} into the pod failed: {}",
            mount.host.display(),
            stderr
        )));
    }
    archived.map_err(|e| {
        ContainerError::ContainerExecution(format!(
            "failed to archive {}: {}",
            mount.host.display(),
            e
        ))
    })
}

/// Write the tar archive in `reader` onto `host`, returning every path it held.
///
/// The archive comes from a workflow-controlled pod, so nothing here follows a
/// symlink or leaves `host`. A clean return does not prove the archive is
/// whole: tar has no length header, so a stream that stops between two entries
/// reads as a normal end of file. Only the sender's exit status can tell.
fn unpack_archive<R: io::Read>(reader: R, host: &Path) -> io::Result<HashSet<PathBuf>> {
    let mut archive = tar::Archive::new(reader);
    let mut seen: HashSet<PathBuf> = HashSet::new();
    let mut directories = Vec::new();

    for entry in archive.entries()? {
        let mut entry = entry?;
        let Some(rel) = safe_relative_path(&entry.path()?) else {
            continue;
        };
        if rel.as_os_str().is_empty() {
            continue;
        }
        let is_dir = entry.header().entry_type().is_dir();
        clear_type_conflict(host, &rel, is_dir)?;
        // Not every tar writes an entry for each parent, and whatever is
        // missing from `seen` is deleted by the prune.
        seen.extend(
            rel.ancestors()
                .skip(1)
                .filter(|parent| !parent.as_os_str().is_empty())
                .map(Path::to_path_buf),
        );
        seen.insert(rel);
        if is_dir {
            directories.push(entry);
        } else {
            entry.unpack_in(host)?;
        }
    }

    // Deepest first and after the files, so a read-only directory's mode is
    // only applied once its contents are in place.
    directories.sort_by_key(|dir| {
        std::cmp::Reverse(dir.path().map(|p| p.components().count()).unwrap_or(0))
    });
    for mut dir in directories {
        dir.unpack_in(host)?;
    }
    Ok(seen)
}

/// Remove what the pod no longer holds, as if its directory had been bind
/// mounted, so a step's `rm` or `mv` is seen by the next step.
///
/// `keep` lists paths in `tar --exclude` form that belong to nested mounts;
/// they are absent from the archive on purpose and must survive. Call this
/// only once the whole archive is known to have arrived.
fn prune_to_archive(host: &Path, seen: &HashSet<PathBuf>, keep: &[String]) -> io::Result<()> {
    let keep: Vec<PathBuf> = keep
        .iter()
        .filter_map(|p| safe_relative_path(Path::new(p)))
        .collect();
    prune_unlisted(host, Path::new(""), seen, &keep)
}

/// `path` without `.` components, or `None` if it could leave its root.
fn safe_relative_path(path: &Path) -> Option<PathBuf> {
    let mut clean = PathBuf::new();
    for component in path.components() {
        match component {
            Component::CurDir => {}
            Component::Normal(part) => clean.push(part),
            _ => return None,
        }
    }
    Some(clean)
}

/// Remove what is at `host/rel` when it is the wrong kind for the incoming
/// entry, because a step replaced a directory with a file or the reverse.
/// Does nothing if any parent is a symlink, which could point outside `host`.
fn clear_type_conflict(host: &Path, rel: &Path, incoming_is_dir: bool) -> io::Result<()> {
    let mut current = host.to_path_buf();
    let parents = rel.parent().into_iter().flat_map(Path::components);
    for part in parents {
        current.push(part);
        match std::fs::symlink_metadata(&current) {
            Ok(meta) if meta.file_type().is_symlink() => return Ok(()),
            Ok(_) => {}
            Err(_) => return Ok(()),
        }
    }
    let target = host.join(rel);
    let Ok(meta) = std::fs::symlink_metadata(&target) else {
        return Ok(());
    };
    if meta.is_dir() && !incoming_is_dir {
        std::fs::remove_dir_all(&target)
    } else if !meta.is_dir() && incoming_is_dir {
        std::fs::remove_file(&target)
    } else {
        Ok(())
    }
}

/// Delete everything under `host/rel` that is neither in `seen` nor kept.
fn prune_unlisted(
    host: &Path,
    rel: &Path,
    seen: &HashSet<PathBuf>,
    keep: &[PathBuf],
) -> io::Result<()> {
    for entry in std::fs::read_dir(host.join(rel))? {
        let entry = entry?;
        let child = rel.join(entry.file_name());
        if keep.iter().any(|k| child.starts_with(k)) {
            continue;
        }
        // `file_type` does not follow symlinks, so a link is removed itself
        // and never descended into.
        let file_type = entry.file_type()?;
        let removed = if seen.contains(&child) {
            if file_type.is_dir() {
                prune_unlisted(host, &child, seen, keep)?;
            }
            continue;
        } else if file_type.is_dir() {
            std::fs::remove_dir_all(entry.path())
        } else if file_type.is_file() || file_type.is_symlink() {
            std::fs::remove_file(entry.path())
        } else {
            // tar cannot carry a socket, so one that appeared on the host
            // during the step says nothing about what the step deleted.
            continue;
        };
        // A leftover file is not worth failing a step that already succeeded.
        if let Err(e) = removed {
            wrkflw_logging::warning(&format!(
                "Kubernetes: could not remove {}: {}",
                entry.path().display(),
                e
            ));
        }
    }
    Ok(())
}

/// Blocking: run `tar -c` in the sync container and unpack it onto `mount.host`.
fn copy_dir_out_of_pod(
    kubectl_args: &[String],
    pod: &str,
    mount: &Mount,
    excludes: &[String],
) -> Result<(), ContainerError> {
    std::fs::create_dir_all(&mount.host).map_err(|e| {
        ContainerError::ContainerExecution(format!(
            "failed to create {}: {}",
            mount.host.display(),
            e
        ))
    })?;
    let src = mount.container.to_string_lossy().to_string();
    let mut tar_args: Vec<String> = vec!["tar".into(), "-C".into(), src.clone()];
    for ex in excludes {
        tar_args.push(format!("--exclude={}", ex));
    }
    tar_args.extend(["-cf".into(), "-".into(), ".".into()]);

    let mut command = std::process::Command::new("kubectl");
    command
        .args(kubectl_args)
        .args(["exec", pod, "-c", SYNC_CONTAINER, "--"])
        .args(&tar_args);
    mirror_from(command, &src, &mount.host, excludes)?;
    wrkflw_logging::debug(&format!(
        "Kubernetes: synced {}:{} -> {}",
        pod,
        src,
        mount.host.display()
    ));
    Ok(())
}

/// Blocking: unpack the tar archive `command` prints onto `host`, then prune.
/// `src` only names the source in errors.
fn mirror_from(
    mut command: std::process::Command,
    src: &str,
    host: &Path,
    excludes: &[String],
) -> Result<(), ContainerError> {
    let mut child = command
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .map_err(|e| {
            ContainerError::ContainerStart(format!("Failed to spawn kubectl exec: {}", e))
        })?;
    let stderr = drain_stderr(&mut child);

    let stdout = child.stdout.take().ok_or_else(|| {
        ContainerError::ContainerExecution("kubectl exec stdout was not captured".to_string())
    })?;
    let unpacked = unpack_archive(stdout, host);

    let status = child.wait().map_err(|e| {
        ContainerError::ContainerExecution(format!("kubectl exec did not complete: {}", e))
    })?;
    let stderr = stderr.join().unwrap_or_default();
    if !status.success() {
        return Err(ContainerError::ContainerExecution(format!(
            "syncing {} back from the pod failed: {}",
            src, stderr
        )));
    }
    let unpack_error = |e: io::Error| {
        ContainerError::ContainerExecution(format!(
            "failed to unpack {} into {}: {}",
            src,
            host.display(),
            e
        ))
    };
    let seen = unpacked.map_err(unpack_error)?;
    prune_to_archive(host, &seen, excludes).map_err(unpack_error)
}

#[async_trait]
impl ContainerRuntime for KubernetesRuntime {
    async fn run_container(
        &self,
        image: &str,
        cmd: &[&str],
        env_vars: &[(&str, &str)],
        working_dir: &Path,
        volumes: &[(&Path, &Path)],
        entrypoint: Option<&str>,
    ) -> Result<ContainerOutput, ContainerError> {
        wrkflw_logging::info(&format!(
            "Kubernetes: running step with image {} ({})",
            image,
            self.config.describe()
        ));
        let timeout = self.config.timeout();
        let name = pod_name();
        match tokio::time::timeout(
            timeout,
            self.run_container_inner(
                &name,
                image,
                cmd,
                env_vars,
                working_dir,
                volumes,
                entrypoint,
            ),
        )
        .await
        {
            Ok(result) => result,
            Err(_) => {
                wrkflw_logging::error(&format!(
                    "Kubernetes step timed out after {} seconds",
                    timeout.as_secs()
                ));
                // Dropping the step's future already deleted its pod.
                Err(ContainerError::ContainerExecution(format!(
                    "Operation timed out after {} seconds (--k8s-timeout)",
                    timeout.as_secs()
                )))
            }
        }
    }

    /// The kubelet pulls images itself; nothing to do on the host.
    async fn pull_image(&self, image: &str) -> Result<(), ContainerError> {
        wrkflw_logging::debug(&format!(
            "Kubernetes: image {} is pulled by the cluster when the pod starts",
            image
        ));
        Ok(())
    }

    async fn build_image(
        &self,
        dockerfile: &Path,
        tag: &str,
        context_dir: &Path,
    ) -> Result<(), ContainerError> {
        self.registry()?;
        let builder = Self::local_builder().ok_or_else(|| {
            ContainerError::ImageBuild(
                "this job needs an image built locally, but neither Docker nor Podman is available"
                    .to_string(),
            )
        })?;
        let dockerfile_str = dockerfile.to_string_lossy().to_string();
        let context_str = context_dir.to_string_lossy().to_string();
        let timeout = if tag.starts_with(COMBINED_IMAGE_PREFIX) {
            Duration::from_secs(600)
        } else {
            Duration::from_secs(120)
        };
        wrkflw_logging::info(&format!("Building image {} with {}", tag, builder));
        self.builder_command(
            builder,
            &["build", "-f", &dockerfile_str, "-t", tag, &context_str],
            timeout,
            "building image",
        )
        .await?;
        self.push_local_image(builder, tag).await
    }

    /// Steps run in the image as given; there is no host-side image to
    /// customize. The executor treats this error as "use the image as-is".
    async fn prepare_language_environment(
        &self,
        language: &str,
        _version: Option<&str>,
        _additional_packages: Option<Vec<String>>,
    ) -> Result<String, ContainerError> {
        Err(ContainerError::ContainerStart(format!(
            "language environment customization for {} is not supported on Kubernetes",
            language
        )))
    }

    /// For locally built tags, "exists" means "this process pushed it". A tag
    /// still in the local Docker/Podman store reports false, and the rebuild
    /// that follows is a cache hit that ends in the push.
    async fn image_exists(&self, tag: &str) -> Result<bool, ContainerError> {
        // A registry image: the kubelet resolves it when the pod starts.
        Ok(!is_locally_built(tag) || self.pushed_ref(tag).is_some())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn cfg() -> KubernetesConfig {
        KubernetesConfig::default()
    }

    fn sample_mounts() -> Vec<Mount> {
        vec![
            Mount {
                host: PathBuf::from("/tmp/ws"),
                container: PathBuf::from("/github/workspace"),
            },
            Mount {
                host: PathBuf::from("/tmp/gh"),
                container: PathBuf::from("/github/workflow"),
            },
        ]
    }

    fn manifest_for(config: &KubernetesConfig, cmd: &[&str], entrypoint: Option<&str>) -> Value {
        let mounts = sample_mounts();
        build_pod_manifest(
            config,
            &PodInput {
                name: "wrkflw-test",
                image: "ubuntu:22.04",
                cmd,
                env_vars: &[("FOO", "bar")],
                working_dir: Path::new("/github/workspace/sub"),
                mounts: &mounts,
                entrypoint,
            },
        )
    }

    #[test]
    fn parse_key_value_accepts_pairs_and_trims() {
        assert_eq!(
            parse_key_value(" accelerator = a100 ").unwrap(),
            ("accelerator".into(), "a100".into())
        );
        assert_eq!(
            parse_key_value("nvidia.com/gpu=1").unwrap(),
            ("nvidia.com/gpu".into(), "1".into())
        );
        // The value may itself contain `=`.
        assert_eq!(
            parse_key_value("a=b=c").unwrap(),
            ("a".into(), "b=c".into())
        );
    }

    #[test]
    fn parse_key_value_rejects_malformed_pairs() {
        assert!(parse_key_value("novalue").is_err());
        assert!(parse_key_value("=v").is_err());
        assert!(parse_key_value("k=").is_err());
        assert!(parse_key_value("").is_err());
    }

    #[test]
    fn config_defaults() {
        let c = cfg();
        assert_eq!(c.helper_image(), DEFAULT_HELPER_IMAGE);
        assert_eq!(c.timeout(), Duration::from_secs(DEFAULT_TIMEOUT_SECS));
        assert!(c.kubectl_args().is_empty());
        assert_eq!(c.describe(), "context 'current', namespace 'current'");
    }

    #[test]
    fn config_kubectl_args_select_context_and_namespace() {
        let c = KubernetesConfig {
            context: Some("gpu-cluster".into()),
            namespace: Some("ci".into()),
            timeout_secs: Some(10),
            helper_image: Some("alpine:3".into()),
            ..Default::default()
        };
        assert_eq!(
            c.kubectl_args(),
            vec!["--context", "gpu-cluster", "--namespace", "ci"]
        );
        assert_eq!(c.timeout(), Duration::from_secs(10));
        assert_eq!(c.helper_image(), "alpine:3");
        assert_eq!(c.describe(), "context 'gpu-cluster', namespace 'ci'");
    }

    #[test]
    fn plan_mounts_sorts_parents_first_and_dedupes_container_paths() {
        let volumes = [
            (Path::new("/h/nested"), Path::new("/github/workspace/sub")),
            (Path::new("/h/ws"), Path::new("/github/workspace")),
            (Path::new("/h/other"), Path::new("/github/workspace")),
        ];
        let mounts = plan_mounts(&volumes);
        assert_eq!(mounts.len(), 2);
        assert_eq!(mounts[0].container, Path::new("/github/workspace"));
        assert_eq!(mounts[0].host, Path::new("/h/ws"), "first mapping wins");
        assert_eq!(mounts[1].container, Path::new("/github/workspace/sub"));
    }

    #[test]
    fn nested_excludes_lists_child_mounts_relative_to_parent() {
        let mounts = plan_mounts(&[
            (Path::new("/h/ws"), Path::new("/github/workspace")),
            (
                Path::new("/h/nested"),
                Path::new("/github/workspace/sub/deep"),
            ),
            (Path::new("/h/gh"), Path::new("/github/workflow")),
        ]);
        let by_path = |p: &str| {
            mounts
                .iter()
                .find(|m| m.container == Path::new(p))
                .expect("mount present")
        };
        assert_eq!(
            nested_excludes(by_path("/github/workspace"), &mounts),
            vec!["./sub/deep"]
        );
        assert!(nested_excludes(by_path("/github/workspace/sub/deep"), &mounts).is_empty());
        assert!(nested_excludes(by_path("/github/workflow"), &mounts).is_empty());
    }

    #[test]
    fn manifest_has_wait_init_container_step_and_sync_sidecar() {
        let m = manifest_for(&cfg(), &["sh", "-c", "echo hi"], None);
        assert_eq!(m["kind"], "Pod");
        assert_eq!(m["metadata"]["name"], "wrkflw-test");
        assert_eq!(m["metadata"]["labels"][MANAGED_BY_LABEL], "wrkflw");
        assert_eq!(m["metadata"]["labels"][SESSION_LABEL], *SESSION_ID);

        let spec = &m["spec"];
        assert_eq!(spec["restartPolicy"], "Never");
        assert_eq!(
            spec["activeDeadlineSeconds"],
            DEFAULT_TIMEOUT_SECS + DEADLINE_MARGIN_SECS
        );
        assert_eq!(spec["automountServiceAccountToken"], false);
        assert_eq!(spec["initContainers"][0]["name"], WAIT_CONTAINER);
        assert_eq!(spec["initContainers"][0]["image"], DEFAULT_HELPER_IMAGE);
        assert!(spec["initContainers"][0]["command"][2]
            .as_str()
            .unwrap()
            .contains(READY_MARKER));

        let containers = spec["containers"].as_array().unwrap();
        assert_eq!(containers.len(), 2);
        assert_eq!(containers[0]["name"], MAIN_CONTAINER);
        assert_eq!(containers[1]["name"], SYNC_CONTAINER);
        assert!(spec["nodeSelector"].is_null());
        assert!(spec["imagePullSecrets"].is_null());
    }

    #[test]
    fn manifest_step_container_carries_image_cmd_env_and_mounts() {
        let m = manifest_for(&cfg(), &["sh", "-c", "echo hi"], None);
        let main = &m["spec"]["containers"][0];
        assert_eq!(main["image"], "ubuntu:22.04");
        assert_eq!(main["args"], json!(["sh", "-c", "echo hi"]));
        assert!(
            main["command"].is_null(),
            "ENTRYPOINT is kept unless overridden"
        );
        assert_eq!(main["workingDir"], "/github/workspace/sub");
        assert_eq!(main["env"], json!([{ "name": "FOO", "value": "bar" }]));
        assert!(main["resources"].is_null());

        let mounts = main["volumeMounts"].as_array().unwrap();
        assert_eq!(mounts.len(), 2);
        assert_eq!(mounts[0]["mountPath"], "/github/workspace");
        assert_eq!(mounts[1]["mountPath"], "/github/workflow");
        // The step must not see the sync marker volume.
        assert!(mounts.iter().all(|v| v["mountPath"] != SYNC_DIR));

        // Helpers see every step mount plus the marker volume.
        let helper = m["spec"]["containers"][1]["volumeMounts"]
            .as_array()
            .unwrap();
        assert_eq!(helper.len(), 3);
        assert_eq!(helper[2]["mountPath"], SYNC_DIR);
        let wait = m["spec"]["initContainers"][0]["volumeMounts"]
            .as_array()
            .unwrap();
        assert_eq!(wait.len(), 3);

        let volumes = m["spec"]["volumes"].as_array().unwrap();
        assert_eq!(volumes.len(), 3);
        assert!(volumes.iter().all(|v| v["emptyDir"].is_object()));
    }

    #[test]
    fn manifest_maps_entrypoint_to_command_and_empty_cmd_to_image_defaults() {
        let m = manifest_for(&cfg(), &[], Some("/entrypoint.sh"));
        let main = &m["spec"]["containers"][0];
        assert_eq!(main["command"], json!(["/entrypoint.sh"]));
        assert!(main["args"].is_null());

        let m = manifest_for(&cfg(), &[], None);
        let main = &m["spec"]["containers"][0];
        assert!(main["command"].is_null());
        assert!(main["args"].is_null());

        // An empty entrypoint string is "not set", as in the Docker backend.
        let m = manifest_for(&cfg(), &["ls"], Some(""));
        assert!(m["spec"]["containers"][0]["command"].is_null());
    }

    #[test]
    fn manifest_applies_node_selector_resources_pull_secrets_and_helper_image() {
        let mut config = cfg();
        config
            .node_selector
            .insert("accelerator".into(), "a100".into());
        config.resources.insert("nvidia.com/gpu".into(), "1".into());
        config.resources.insert("memory".into(), "8Gi".into());
        config.image_pull_secrets = vec!["regcred".into()];
        config.helper_image = Some("alpine:3.19".into());

        let m = manifest_for(&config, &["nvidia-smi"], None);
        let spec = &m["spec"];
        assert_eq!(spec["nodeSelector"], json!({ "accelerator": "a100" }));
        assert_eq!(spec["imagePullSecrets"], json!([{ "name": "regcred" }]));
        assert_eq!(spec["initContainers"][0]["image"], "alpine:3.19");
        assert_eq!(spec["containers"][1]["image"], "alpine:3.19");

        let resources = &spec["containers"][0]["resources"];
        assert_eq!(resources["limits"]["nvidia.com/gpu"], "1");
        assert_eq!(resources["requests"]["nvidia.com/gpu"], "1");
        assert_eq!(resources["limits"]["memory"], "8Gi");
        // Helpers never request accelerators, but do satisfy resource quotas.
        for helper in [&spec["initContainers"][0], &spec["containers"][1]] {
            assert_eq!(helper["resources"], helper_resources());
        }
    }

    #[test]
    fn manifest_serializes_as_valid_json_with_no_volumes() {
        let m = build_pod_manifest(
            &cfg(),
            &PodInput {
                name: "wrkflw-empty",
                image: "alpine",
                cmd: &["true"],
                env_vars: &[],
                working_dir: Path::new("/"),
                mounts: &[],
                entrypoint: None,
            },
        );
        let text = serde_json::to_string(&m).unwrap();
        assert!(text.contains("\"wrkflw-empty\""));
        assert_eq!(m["spec"]["volumes"].as_array().unwrap().len(), 1);
        assert!(m["spec"]["containers"][0]["volumeMounts"]
            .as_array()
            .unwrap()
            .is_empty());
    }

    #[test]
    fn manifest_escapes_dollars_from_kubernetes_expansion() {
        assert_eq!(escape_k8s_expansion("no dollars"), "no dollars");
        assert_eq!(escape_k8s_expansion("$"), "$$");
        assert_eq!(escape_k8s_expansion("$$ $(HOME)"), "$$$$ $$(HOME)");

        let mounts = sample_mounts();
        let m = build_pod_manifest(
            &cfg(),
            &PodInput {
                name: "wrkflw-test",
                image: "ubuntu:22.04",
                cmd: &["bash", "-c", "echo $$ $(date)"],
                env_vars: &[("LIT", "a$(HOME)b")],
                working_dir: Path::new("/github/workspace"),
                mounts: &mounts,
                entrypoint: Some("/bin/$x"),
            },
        );
        let main = &m["spec"]["containers"][0];
        assert_eq!(main["args"][2], "echo $$$$ $$(date)");
        assert_eq!(main["env"][0]["value"], "a$$(HOME)b");
        assert_eq!(main["command"][0], "/bin/$$x");
    }

    #[test]
    fn plan_mounts_skips_files_but_keeps_missing_directories() {
        let dir = tempfile::tempdir().unwrap();
        let file = dir.path().join("docker.sock");
        std::fs::write(&file, "").unwrap();
        let missing = dir.path().join("not-yet");
        let mounts = plan_mounts(&[
            (file.as_path(), Path::new("/var/run/docker.sock")),
            (dir.path(), Path::new("/data")),
            (missing.as_path(), Path::new("/later")),
        ]);
        let paths: Vec<_> = mounts.iter().map(|m| m.container.clone()).collect();
        assert_eq!(paths, vec![PathBuf::from("/data"), PathBuf::from("/later")]);
    }

    #[tokio::test]
    async fn relative_host_paths_are_never_written_back() {
        let rt = KubernetesRuntime::new(cfg(), false);
        let mount = Mount {
            host: PathBuf::from("named-volume"),
            container: PathBuf::from("/data"),
        };
        rt.copy_in("no-such-pod", &mount).await.unwrap();
        rt.copy_out("no-such-pod", &mount, Vec::new())
            .await
            .unwrap();
        assert!(!Path::new("named-volume").exists());
    }

    /// What [`mirror_from`] does once the stream is known to be complete.
    fn unpack_mirroring<R: io::Read>(reader: R, host: &Path, keep: &[String]) -> io::Result<()> {
        let seen = unpack_archive(reader, host)?;
        prune_to_archive(host, &seen, keep)
    }

    /// Tar builder for the mirroring tests.
    fn archive(build: impl FnOnce(&mut tar::Builder<Vec<u8>>)) -> Vec<u8> {
        let mut b = tar::Builder::new(Vec::new());
        build(&mut b);
        b.into_inner().unwrap()
    }

    fn add_file(b: &mut tar::Builder<Vec<u8>>, path: &str, body: &[u8]) {
        let mut h = tar::Header::new_gnu();
        h.set_size(body.len() as u64);
        h.set_mode(0o644);
        h.set_cksum();
        b.append_data(&mut h, path, body).unwrap();
    }

    fn add_dir(b: &mut tar::Builder<Vec<u8>>, path: &str, mode: u32) {
        let mut h = tar::Header::new_gnu();
        h.set_entry_type(tar::EntryType::Directory);
        h.set_size(0);
        h.set_mode(mode);
        h.set_cksum();
        b.append_data(&mut h, path, io::empty()).unwrap();
    }

    #[test]
    fn mirroring_writes_updates_and_removes_deleted_entries() {
        let host = tempfile::tempdir().unwrap();
        std::fs::create_dir_all(host.path().join("gone/deep")).unwrap();
        std::fs::write(host.path().join("gone/deep/f"), "x").unwrap();
        std::fs::write(host.path().join("deleted.txt"), "x").unwrap();
        std::fs::write(host.path().join("kept.txt"), "old").unwrap();

        let tar = archive(|b| {
            add_dir(b, "./", 0o755);
            add_file(b, "./kept.txt", b"new");
            add_dir(b, "./fresh/", 0o755);
            add_file(b, "./fresh/f.txt", b"made in pod");
        });
        unpack_mirroring(tar.as_slice(), host.path(), &[]).unwrap();

        let read = |p: &str| std::fs::read_to_string(host.path().join(p)).unwrap();
        assert_eq!(read("kept.txt"), "new");
        assert_eq!(read("fresh/f.txt"), "made in pod");
        assert!(!host.path().join("deleted.txt").exists());
        assert!(!host.path().join("gone").exists());
    }

    /// `tar` is free to leave parents implicit, and `--k8s-helper-image` lets
    /// users pick the tar.
    #[test]
    fn mirroring_keeps_directories_the_archive_only_implies() {
        let host = tempfile::tempdir().unwrap();
        std::fs::create_dir_all(host.path().join("a/b")).unwrap();
        std::fs::write(host.path().join("a/b/stale.txt"), "x").unwrap();

        let tar = archive(|b| add_file(b, "./a/b/f.txt", b"from the pod"));
        unpack_mirroring(tar.as_slice(), host.path(), &[]).unwrap();

        assert_eq!(
            std::fs::read_to_string(host.path().join("a/b/f.txt")).unwrap(),
            "from the pod"
        );
        assert!(!host.path().join("a/b/stale.txt").exists());
    }

    #[test]
    fn mirroring_keeps_nested_mounts_out_of_the_prune() {
        let host = tempfile::tempdir().unwrap();
        std::fs::create_dir_all(host.path().join("sub/deep")).unwrap();
        std::fs::write(host.path().join("sub/deep/nested.txt"), "x").unwrap();
        std::fs::write(host.path().join("sub/other.txt"), "x").unwrap();

        let tar = archive(|b| add_dir(b, "./sub/", 0o755));
        unpack_mirroring(tar.as_slice(), host.path(), &["./sub/deep".to_string()]).unwrap();

        assert!(host.path().join("sub/deep/nested.txt").exists());
        assert!(!host.path().join("sub/other.txt").exists());
    }

    #[test]
    fn mirroring_replaces_entries_that_changed_type() {
        let host = tempfile::tempdir().unwrap();
        std::fs::create_dir_all(host.path().join("was-dir/child")).unwrap();
        std::fs::write(host.path().join("was-file"), "x").unwrap();

        let tar = archive(|b| {
            add_file(b, "./was-dir", b"now a file");
            add_dir(b, "./was-file/", 0o755);
            add_file(b, "./was-file/child", b"now a dir");
        });
        unpack_mirroring(tar.as_slice(), host.path(), &[]).unwrap();

        assert!(host.path().join("was-dir").is_file());
        assert!(host.path().join("was-file/child").is_file());
    }

    #[test]
    fn mirroring_applies_read_only_directory_modes_after_their_files() {
        let host = tempfile::tempdir().unwrap();
        let tar = archive(|b| {
            add_dir(b, "./locked/", 0o555);
            add_file(b, "./locked/f.txt", b"inside");
        });
        unpack_mirroring(tar.as_slice(), host.path(), &[]).unwrap();
        assert!(host.path().join("locked/f.txt").is_file());

        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            let dir = host.path().join("locked");
            let mode = std::fs::metadata(&dir).unwrap().permissions().mode();
            assert_eq!(mode & 0o777, 0o555);
            // Let tempdir clean up.
            std::fs::set_permissions(&dir, std::fs::Permissions::from_mode(0o755)).unwrap();
        }
    }

    #[cfg(unix)]
    #[test]
    fn mirroring_never_follows_symlinks_out_of_the_host_directory() {
        let host = tempfile::tempdir().unwrap();
        let outside = tempfile::tempdir().unwrap();
        std::fs::write(outside.path().join("precious.txt"), "x").unwrap();
        std::fs::create_dir(outside.path().join("target")).unwrap();
        std::os::unix::fs::symlink(outside.path(), host.path().join("link")).unwrap();

        // `link` stays in the archive, so it is not pruned, and an entry
        // beneath it must not reach the directory it points at.
        let tar = archive(|b| {
            let mut h = tar::Header::new_gnu();
            h.set_entry_type(tar::EntryType::Symlink);
            h.set_size(0);
            h.set_mode(0o777);
            b.append_link(&mut h, "./link", outside.path()).unwrap();
            add_file(b, "./link/target", b"replaced");
        });
        let _ = unpack_mirroring(tar.as_slice(), host.path(), &[]);
        assert!(outside.path().join("target").is_dir());
        assert!(outside.path().join("precious.txt").exists());

        // A pruned symlink goes away without touching its target.
        let tar = archive(|b| add_dir(b, "./", 0o755));
        unpack_mirroring(tar.as_slice(), host.path(), &[]).unwrap();
        assert!(!host.path().join("link").exists());
        assert!(outside.path().join("precious.txt").exists());
    }

    #[test]
    fn mirroring_does_not_delete_on_a_broken_stream() {
        let host = tempfile::tempdir().unwrap();
        std::fs::write(host.path().join("existing.txt"), "x").unwrap();
        let mut tar = archive(|b| add_file(b, "./big.bin", &[7u8; 4096]));
        // Cut the stream off inside the file body.
        tar.truncate(1024);
        assert!(unpack_mirroring(tar.as_slice(), host.path(), &[]).is_err());
        assert!(host.path().join("existing.txt").exists());
    }

    /// `sh -c script` with the archive in `$1`, standing in for `kubectl exec`.
    #[cfg(unix)]
    fn sender(script: &str, tar: &Path) -> std::process::Command {
        let mut command = std::process::Command::new("sh");
        command.args(["-c", script, "sh"]).arg(tar);
        command
    }

    #[cfg(unix)]
    fn write_tar(dir: &Path, build: impl FnOnce(&mut tar::Builder<Vec<u8>>)) -> PathBuf {
        let path = dir.join("stream.tar");
        std::fs::write(&path, archive(build)).unwrap();
        path
    }

    /// A connection lost between two entries is a clean EOF to the tar reader,
    /// and everything not yet received would otherwise look deleted.
    #[cfg(unix)]
    #[test]
    fn a_failed_exec_never_prunes_even_if_the_stream_looks_complete() {
        let work = tempfile::tempdir().unwrap();
        let host = tempfile::tempdir().unwrap();
        std::fs::write(host.path().join("not-yet-sent.txt"), "precious").unwrap();

        let tar = write_tar(work.path(), |b| add_file(b, "./first.txt", b"x"));
        let err = mirror_from(
            sender("head -c 1024 \"$1\"; echo lost >&2; exit 1", &tar),
            "/github/workspace",
            host.path(),
            &[],
        )
        .expect_err("a failed exec must fail the sync");

        assert!(err.to_string().contains("lost"), "{}", err);
        assert!(host.path().join("not-yet-sent.txt").exists());
    }

    #[cfg(unix)]
    #[test]
    fn a_successful_exec_mirrors_and_prunes() {
        let work = tempfile::tempdir().unwrap();
        let host = tempfile::tempdir().unwrap();
        std::fs::write(host.path().join("deleted.txt"), "x").unwrap();

        let tar = write_tar(work.path(), |b| add_file(b, "./kept.txt", b"x"));
        mirror_from(sender("cat \"$1\"", &tar), "src", host.path(), &[]).unwrap();

        assert!(host.path().join("kept.txt").exists());
        assert!(!host.path().join("deleted.txt").exists());
    }

    /// More stderr than a pipe holds, before any stdout. Unread, this hangs.
    #[cfg(unix)]
    #[test]
    fn a_noisy_stderr_does_not_stall_the_sync_out() {
        let work = tempfile::tempdir().unwrap();
        let host = tempfile::tempdir().unwrap();
        let tar = write_tar(work.path(), |b| add_file(b, "./f.txt", b"x"));
        mirror_from(
            sender("head -c 1000000 /dev/zero >&2; cat \"$1\"", &tar),
            "src",
            host.path(),
            &[],
        )
        .unwrap();
        assert!(host.path().join("f.txt").exists());
    }

    #[cfg(unix)]
    #[test]
    fn a_noisy_stderr_does_not_stall_the_sync_in() {
        let host = tempfile::tempdir().unwrap();
        std::fs::write(host.path().join("f.txt"), "x").unwrap();
        let mut command = std::process::Command::new("sh");
        command.args([
            "-c",
            "head -c 1000000 /dev/zero >&2; cat >/dev/null; echo refused >&2; exit 2",
        ]);
        let mount = Mount {
            host: host.path().to_path_buf(),
            container: PathBuf::from("/data"),
        };
        let err = stream_dir_into(command, &mount).expect_err("must report the failure");
        // Only the first bytes of a flood are kept, and it was all NULs.
        assert!(err.to_string().contains("into the pod failed"), "{}", err);
    }

    #[cfg(unix)]
    #[test]
    fn pruning_leaves_sockets_alone() {
        let host = tempfile::tempdir().unwrap();
        let socket = host.path().join("s");
        let _listener = std::os::unix::net::UnixListener::bind(&socket).unwrap();
        let tar = archive(|b| add_dir(b, "./", 0o755));
        unpack_mirroring(tar.as_slice(), host.path(), &[]).unwrap();
        assert!(socket.exists());
    }

    #[test]
    fn mirroring_unpacks_hard_links_inside_the_host_directory() {
        let host = tempfile::tempdir().unwrap();
        let tar = archive(|b| {
            add_file(b, "./original", b"same");
            let mut h = tar::Header::new_gnu();
            h.set_entry_type(tar::EntryType::Link);
            h.set_size(0);
            h.set_mode(0o644);
            b.append_link(&mut h, "./linked", "./original").unwrap();
        });
        unpack_mirroring(tar.as_slice(), host.path(), &[]).unwrap();
        assert_eq!(
            std::fs::read_to_string(host.path().join("linked")).unwrap(),
            "same"
        );
    }

    #[cfg(unix)]
    #[test]
    fn a_file_that_cannot_be_pruned_does_not_fail_the_step() {
        use std::os::unix::fs::PermissionsExt;
        let host = tempfile::tempdir().unwrap();
        let locked = host.path().join("locked");
        std::fs::create_dir(&locked).unwrap();
        std::fs::write(locked.join("stuck.txt"), "x").unwrap();
        std::fs::set_permissions(&locked, std::fs::Permissions::from_mode(0o555)).unwrap();

        let tar = archive(|b| add_dir(b, "./locked/", 0o555));
        let result = unpack_mirroring(tar.as_slice(), host.path(), &[]);
        std::fs::set_permissions(&locked, std::fs::Permissions::from_mode(0o755)).unwrap();
        result.unwrap();
    }

    #[test]
    fn safe_relative_path_rejects_escapes() {
        assert_eq!(
            safe_relative_path(Path::new("./a/./b")),
            Some(PathBuf::from("a/b"))
        );
        assert_eq!(safe_relative_path(Path::new("./")), Some(PathBuf::new()));
        assert_eq!(safe_relative_path(Path::new("a/../../b")), None);
        assert_eq!(safe_relative_path(Path::new("/etc/passwd")), None);
    }

    #[test]
    fn registry_ref_joins_without_double_slash() {
        assert_eq!(
            registry_ref("registry.example.com/ci/", "wrkflw-combined:abc"),
            "registry.example.com/ci/wrkflw-combined:abc"
        );
        assert_eq!(
            registry_ref("localhost:5000", "wrkflw-python-3.11"),
            "localhost:5000/wrkflw-python-3.11"
        );
    }

    #[test]
    fn pod_name_is_a_valid_dns_label() {
        let name = pod_name();
        assert!(name.starts_with("wrkflw-"));
        assert!(name.len() <= 63);
        assert!(name
            .chars()
            .all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '-'));
    }

    #[test]
    fn polling_backs_off_as_a_step_runs() {
        assert_eq!(poll_interval(Duration::ZERO), Duration::from_millis(500));
        assert_eq!(
            poll_interval(Duration::from_secs(30)),
            Duration::from_secs(1)
        );
        assert_eq!(
            poll_interval(Duration::from_secs(3600)),
            Duration::from_secs(5)
        );
    }

    fn pod_status(status: Value) -> Value {
        json!({ "status": status })
    }

    #[test]
    fn status_helpers_read_running_terminated_and_phase() {
        let pod = pod_status(json!({
            "phase": "Running",
            "initContainerStatuses": [
                { "name": WAIT_CONTAINER, "state": { "running": { "startedAt": "now" } } }
            ],
            "containerStatuses": [
                { "name": MAIN_CONTAINER, "state": { "terminated": { "exitCode": 3 } } },
                { "name": SYNC_CONTAINER, "state": { "running": {} } }
            ]
        }));
        assert!(is_running(container_state(
            &pod,
            "initContainerStatuses",
            WAIT_CONTAINER
        )));
        assert!(!is_running(container_state(
            &pod,
            "containerStatuses",
            MAIN_CONTAINER
        )));
        assert_eq!(
            terminated_exit_code(container_state(&pod, "containerStatuses", MAIN_CONTAINER)),
            Some(3)
        );
        assert_eq!(
            terminated_exit_code(container_state(&pod, "containerStatuses", SYNC_CONTAINER)),
            None
        );
        assert_eq!(pod_phase(&pod), "Running");
        assert!(fatal_waiting_reason(&pod).is_none());
        assert!(unschedulable_message(&pod).is_none());

        let empty = json!({});
        assert!(container_state(&empty, "containerStatuses", MAIN_CONTAINER).is_none());
        assert_eq!(pod_phase(&empty), "Unknown");
    }

    #[test]
    fn fatal_waiting_reason_flags_image_pull_failures_only() {
        let pod = pod_status(json!({
            "containerStatuses": [{
                "name": MAIN_CONTAINER,
                "state": { "waiting": { "reason": "ImagePullBackOff", "message": "no such image" } }
            }]
        }));
        let reason = fatal_waiting_reason(&pod).unwrap();
        assert!(reason.contains("container 'main'"));
        assert!(reason.contains("ImagePullBackOff"));
        assert!(reason.contains("no such image"));

        let pod = pod_status(json!({
            "initContainerStatuses": [{
                "name": WAIT_CONTAINER,
                "state": { "waiting": { "reason": "ContainerCreating" } }
            }]
        }));
        assert!(fatal_waiting_reason(&pod).is_none());
    }

    #[test]
    fn unschedulable_and_failure_messages() {
        let pod = pod_status(json!({
            "phase": "Pending",
            "conditions": [{
                "type": "PodScheduled",
                "status": "False",
                "reason": "Unschedulable",
                "message": "0/3 nodes are available: insufficient nvidia.com/gpu"
            }]
        }));
        assert_eq!(
            unschedulable_message(&pod).as_deref(),
            Some("0/3 nodes are available: insufficient nvidia.com/gpu")
        );

        let evicted = pod_status(json!({
            "phase": "Failed",
            "reason": "Evicted",
            "message": "node was low on resource: memory"
        }));
        assert_eq!(
            pod_failure_message(&evicted),
            "Evicted: node was low on resource: memory"
        );
        let bare = pod_status(json!({ "phase": "Failed" }));
        assert_eq!(pod_failure_message(&bare), "pod phase is Failed");
    }

    #[tokio::test]
    async fn pushed_images_are_found_without_touching_a_builder() {
        let config = KubernetesConfig {
            registry: Some("reg.test".into()),
            ..Default::default()
        };
        let rt = KubernetesRuntime::new(config.clone(), false);
        let tag = "wrkflw-combined:pushed-test";

        assert_eq!(rt.cluster_image("ubuntu:22.04").unwrap(), "ubuntu:22.04");
        let err = rt.cluster_image(tag).expect_err("not pushed yet");
        assert!(err.to_string().contains("--k8s-registry"), "{}", err);
        // Reports false rather than pushing as a side effect of a check.
        assert!(!rt.image_exists(tag).await.unwrap());

        PUSHED_IMAGES
            .lock()
            .unwrap()
            .insert(registry_ref("reg.test", tag));
        assert_eq!(
            rt.cluster_image(tag).unwrap(),
            "reg.test/wrkflw-combined:pushed-test"
        );
        // A later runtime in the same process, as the TUI creates per run.
        let later = KubernetesRuntime::new(config, false);
        assert!(later.image_exists(tag).await.unwrap());
    }

    #[tokio::test]
    async fn image_exists_without_registry_reports_local_tags_missing() {
        let rt = KubernetesRuntime::new(cfg(), false);
        assert!(rt.image_exists("python:3.11").await.unwrap());
        assert!(!rt.image_exists("wrkflw-combined:abc").await.unwrap());
        assert!(!rt.image_exists("wrkflw-python-3.11").await.unwrap());
    }

    #[tokio::test]
    async fn build_image_without_registry_explains_the_flag() {
        let rt = KubernetesRuntime::new(cfg(), false);
        let err = rt
            .build_image(
                Path::new("Dockerfile"),
                "wrkflw-combined:abc",
                Path::new("."),
            )
            .await
            .expect_err("must need a registry");
        assert!(err.to_string().contains("--k8s-registry"), "{}", err);
    }

    #[tokio::test]
    async fn language_environment_is_declined() {
        let rt = KubernetesRuntime::new(cfg(), false);
        assert!(rt
            .prepare_language_environment("python", Some("3.11"), None)
            .await
            .is_err());
        assert!(rt.pull_image("anything").await.is_ok());
    }

    /// A guard whose "kubectl" records its arguments in `log` instead.
    #[cfg(unix)]
    fn recording_guard(name: &str, log: &Path) -> PodGuard {
        let mut guard = PodGuard::new(PodRef {
            kubectl_args: vec![
                "-c".into(),
                format!("echo \"$@\" > {}.tmp && mv {0}.tmp {0}", log.display()),
                "sh".into(),
            ],
            name: name.into(),
        });
        guard.kubectl = "sh".into();
        guard
    }

    #[cfg(unix)]
    #[tokio::test]
    async fn cancelling_a_step_deletes_its_pod() {
        let dir = tempfile::tempdir().unwrap();
        let log = dir.path().join("kubectl.log");
        let guard = recording_guard("wrkflw-cancelled", &log);

        // Exactly what the engine's `timeout-minutes` does to a running step.
        let step = async move {
            let _guard = guard;
            std::future::pending::<()>().await;
        };
        assert!(tokio::time::timeout(Duration::from_millis(10), step)
            .await
            .is_err());

        assert!(!RUNNING_PODS
            .lock()
            .unwrap()
            .iter()
            .any(|p| p.name == "wrkflw-cancelled"));
        let deadline = std::time::Instant::now() + Duration::from_secs(10);
        while !log.exists() {
            assert!(std::time::Instant::now() < deadline, "delete never ran");
            std::thread::sleep(Duration::from_millis(10));
        }
        let args = std::fs::read_to_string(&log).unwrap();
        assert!(args.starts_with("delete pod wrkflw-cancelled "), "{}", args);
    }

    #[cfg(unix)]
    #[test]
    fn a_disarmed_guard_leaves_the_pod_alone() {
        let dir = tempfile::tempdir().unwrap();
        let log = dir.path().join("kubectl.log");
        let mut guard = recording_guard("wrkflw-preserved", &log);
        guard.disarm();
        drop(guard);
        untrack_pod("wrkflw-preserved");
        std::thread::sleep(Duration::from_millis(100));
        assert!(!log.exists());
    }

    #[test]
    fn pod_tracking_adds_and_removes() {
        let pod = PodRef {
            kubectl_args: vec![],
            name: "wrkflw-tracked".into(),
        };
        track_pod(&pod);
        assert!(RUNNING_PODS
            .lock()
            .unwrap()
            .iter()
            .any(|p| p.name == "wrkflw-tracked"));
        untrack_pod("wrkflw-tracked");
        assert!(!RUNNING_PODS
            .lock()
            .unwrap()
            .iter()
            .any(|p| p.name == "wrkflw-tracked"));
    }
}
