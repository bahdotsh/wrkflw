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

mod manifest;
mod sync;

use async_trait::async_trait;
use manifest::{build_pod_manifest, is_nested, nested_excludes, plan_mounts, Mount, PodInput};
use once_cell::sync::Lazy;
use serde_json::Value;
use std::collections::{BTreeMap, HashSet};
use std::ffi::{OsStr, OsString};
use std::path::Path;
use std::process::Stdio;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Mutex;
use std::time::Duration;
use sync::{copy_dir_into_pod, copy_dir_out_of_pod};
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
    "ErrImageNeverPull",
    "InvalidImageName",
    "CreateContainerConfigError",
    "CreateContainerError",
];

/// Waiting reasons of a pull the kubelet is still retrying. A bad image and a
/// registry blip look the same here: both go from `ErrImagePull` to
/// `ImagePullBackOff` within seconds. Only time tells them apart.
const PULL_RETRY_REASONS: &[&str] = &["ErrImagePull", "ImagePullBackOff"];

/// How long a pull may keep failing before the step gives up. The kubelet
/// retries after 10, 20 and 40 seconds, so this covers two or three attempts.
const PULL_RETRY_GRACE: Duration = Duration::from_secs(60);

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
    check_available(config).is_ok()
}

/// Check that `kubectl` is installed and the configured context answers,
/// saying which of the two failed.
pub fn check_available(config: &KubernetesConfig) -> Result<(), String> {
    check_available_using("kubectl".as_ref(), config)
}

fn check_available_using(kubectl: &OsStr, config: &KubernetesConfig) -> Result<(), String> {
    if which::which(kubectl).is_err() {
        return Err(format!(
            "{} was not found on PATH; the Kubernetes runtime needs it installed",
            kubectl.to_string_lossy()
        ));
    }
    if cluster_answers(kubectl, config) {
        Ok(())
    } else {
        Err(format!(
            "Kubernetes cluster is not reachable ({}); check KUBECONFIG and --k8s-context",
            config.describe()
        ))
    }
}

fn cluster_answers(kubectl: &OsStr, config: &KubernetesConfig) -> bool {
    let child = std::process::Command::new(kubectl)
        .args(config.kubectl_args())
        .args(["--request-timeout=3s", "get", "--raw", "/version"])
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn();
    let Ok(mut child) = child else {
        return false;
    };
    // `--request-timeout` does not cover exec credential plugins, and a run
    // or a TUI probe must not wait on one forever, so enforce the limit here.
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
    futures::future::join_all(pods.iter().map(delete_tracked_pod)).await;
}

/// Never `--force`: that frees the pod's GPUs in the scheduler while the
/// kubelet may still be killing the process using them, and the next pod
/// placed there fails to initialise the device. `--wait=false` returns at once
/// and `terminationGracePeriodSeconds` bounds the rest.
fn delete_args(name: &str) -> [&str; 5] {
    ["delete", "pod", name, "--ignore-not-found", "--wait=false"]
}

async fn delete_tracked_pod(pod: &PodRef) {
    wrkflw_logging::info(&format!("Deleting Kubernetes pod {}", pod.name));
    let _ = Command::new("kubectl")
        .args(&pod.kubectl_args)
        .args(delete_args(&pod.name))
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
    kubectl: OsString,
    armed: bool,
}

impl PodGuard {
    fn new(pod: PodRef, kubectl: OsString) -> Self {
        track_pod(&pod);
        PodGuard {
            pod,
            kubectl,
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
            .args(delete_args(&self.pod.name))
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
    waiting_reason(pod, FATAL_WAITING_REASONS)
}

/// Why a pod cannot make progress, if it cannot: a fatal waiting reason, or an
/// image pull that has outlasted [`PULL_RETRY_GRACE`].
fn stuck_reason(pod: &Value, name: &str, watch: &mut PullWatch) -> Option<String> {
    fatal_waiting_reason(pod)
        .or_else(|| watch.check(pod, name, std::time::Instant::now(), PULL_RETRY_GRACE))
}

/// Tracks how long a pod has been failing to pull an image.
#[derive(Default)]
struct PullWatch {
    first_failure: Option<std::time::Instant>,
}

impl PullWatch {
    /// The failure to report once a pull has been failing for `grace`.
    ///
    /// A retry in flight can report `ContainerCreating` between two failures,
    /// so the clock never resets: a pull that later works moves the pod on,
    /// and one that does not must not wait out `--k8s-timeout`.
    fn check(
        &mut self,
        pod: &Value,
        name: &str,
        now: std::time::Instant,
        grace: Duration,
    ) -> Option<String> {
        let reason = waiting_reason(pod, PULL_RETRY_REASONS)?;
        let first_failure = *self.first_failure.get_or_insert_with(|| {
            wrkflw_logging::warning(&format!(
                "Pod {} failed to pull an image, retrying for up to {} seconds: {}",
                name,
                grace.as_secs(),
                reason
            ));
            now
        });
        (now.duration_since(first_failure) >= grace).then_some(reason)
    }
}

/// The first container waiting for one of `reasons`.
fn waiting_reason(pod: &Value, reasons: &[&str]) -> Option<String> {
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
                if reasons.contains(&reason) {
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
    kubectl: OsString,
    /// Arguments placed before every kubectl command line. Lets tests run a
    /// script through `sh`, which unlike a freshly written executable cannot
    /// fail with `ETXTBSY` when another test forks at the wrong moment.
    kubectl_prefix: Vec<String>,
}

impl KubernetesRuntime {
    /// Construct without probing the cluster. Callers should have checked
    /// [`is_available_with`] first.
    pub fn new(config: KubernetesConfig, preserve_pods_on_failure: bool) -> Self {
        KubernetesRuntime {
            config,
            preserve_pods_on_failure,
            kubectl: "kubectl".into(),
            kubectl_prefix: Vec::new(),
        }
    }

    fn kubectl_args(&self) -> Vec<String> {
        let mut args = self.kubectl_prefix.clone();
        args.extend(self.config.kubectl_args());
        args
    }

    /// Run `kubectl <args>` to completion, feeding `stdin` if given.
    async fn kubectl(
        &self,
        args: &[&str],
        stdin: Option<Vec<u8>>,
    ) -> Result<std::process::Output, ContainerError> {
        let mut cmd = Command::new(&self.kubectl);
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
        let output = self.kubectl(&delete_args(name), None).await;
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
        let mut pull_watch = PullWatch::default();
        let started = std::time::Instant::now();
        loop {
            let pod = self.get_pod(name).await?;
            if let Some(reason) = stuck_reason(&pod, name, &mut pull_watch) {
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
        let mut pull_watch = PullWatch::default();
        let started = std::time::Instant::now();
        loop {
            let pod = self.get_pod(name).await?;
            if let Some(code) =
                terminated_exit_code(container_state(&pod, "containerStatuses", MAIN_CONTAINER))
            {
                return Ok(code);
            }
            if let Some(reason) = stuck_reason(&pod, name, &mut pull_watch) {
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

    /// Empty `mount.container`, keeping the mount itself.
    async fn clear_mount(&self, name: &str, mount: &Mount) -> Result<(), ContainerError> {
        let path = mount.container.to_string_lossy().to_string();
        self.kubectl_ok(
            &[
                "exec",
                name,
                "-c",
                WAIT_CONTAINER,
                "--",
                "sh",
                "-c",
                "cd \"$1\" && rm -rf ./* ./.[!.]* ./..?*",
                "sh",
                &path,
            ],
            None,
            "clearing a nested mount",
        )
        .await
        .map(drop)
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
        let kubectl = self.kubectl.clone();
        let base = self.kubectl_args();
        let pod = name.to_string();
        let mount = mount.clone();
        tokio::task::spawn_blocking(move || copy_dir_into_pod(&kubectl, &base, &pod, &mount))
            .await
            .map_err(|e| ContainerError::ContainerExecution(format!("copy task failed: {}", e)))?
    }

    /// Stream `mount.container` out of the sync container onto `host`.
    ///
    /// `spawn_blocking` work cannot be cancelled, so a step timeout that lands
    /// here leaves this thread running until the pod's delete ends its stream.
    /// That stream then fails, which skips the prune, so at worst a few files
    /// land in a job directory that is on its way out.
    async fn copy_out(
        &self,
        name: &str,
        mount: &Mount,
        excludes: Vec<String>,
    ) -> Result<(), ContainerError> {
        if !mount.host.is_absolute() {
            return Ok(());
        }
        let kubectl = self.kubectl.clone();
        let base = self.kubectl_args();
        let pod = name.to_string();
        let mount = mount.clone();
        tokio::task::spawn_blocking(move || {
            copy_dir_out_of_pod(&kubectl, &base, &pod, &mount, &excludes)
        })
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
        let mut guard = PodGuard::new(
            PodRef {
                kubectl_args: self.kubectl_args(),
                name: name.to_string(),
            },
            self.kubectl.clone(),
        );
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
            if is_nested(mount, mounts) {
                self.clear_mount(name, mount).await?;
            }
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
        // Each probe can block for seconds.
        let builder = tokio::task::spawn_blocking(Self::local_builder)
            .await
            .map_err(|e| ContainerError::ImageBuild(format!("builder probe failed: {}", e)))?
            .ok_or_else(|| {
                ContainerError::ImageBuild(
                    "this job needs an image built locally, but neither Docker nor Podman \
                     is available"
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
    use serde_json::json;
    use std::path::PathBuf;

    fn cfg() -> KubernetesConfig {
        KubernetesConfig::default()
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
    fn missing_kubectl_is_not_reported_as_an_unreachable_cluster() {
        let err = check_available_using("wrkflw-no-such-kubectl".as_ref(), &cfg()).unwrap_err();
        assert!(err.contains("not found on PATH"), "{}", err);
        assert!(!err.contains("not reachable"), "{}", err);
    }

    #[cfg(unix)]
    #[test]
    fn an_unreachable_cluster_is_reported_as_one() {
        let err = check_available_using("false".as_ref(), &cfg()).unwrap_err();
        assert!(err.contains("not reachable"), "{}", err);
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
    fn fatal_waiting_reason_flags_unrecoverable_states_only() {
        let pod = pod_status(json!({
            "containerStatuses": [{
                "name": MAIN_CONTAINER,
                "state": { "waiting": { "reason": "InvalidImageName", "message": "bad ref" } }
            }]
        }));
        let reason = fatal_waiting_reason(&pod).unwrap();
        assert!(reason.contains("container 'main'"));
        assert!(reason.contains("InvalidImageName"));
        assert!(reason.contains("bad ref"));

        for retried in PULL_RETRY_REASONS {
            let pod = pod_status(json!({
                "containerStatuses": [{
                    "name": MAIN_CONTAINER,
                    "state": { "waiting": { "reason": retried } }
                }]
            }));
            assert!(fatal_waiting_reason(&pod).is_none(), "{}", retried);
        }

        let pod = pod_status(json!({
            "initContainerStatuses": [{
                "name": WAIT_CONTAINER,
                "state": { "waiting": { "reason": "ContainerCreating" } }
            }]
        }));
        assert!(fatal_waiting_reason(&pod).is_none());
    }

    fn pod_pulling(reason: &str) -> Value {
        pod_status(json!({
            "containerStatuses": [{
                "name": MAIN_CONTAINER,
                "state": { "waiting": { "reason": reason, "message": "registry said no" } }
            }]
        }))
    }

    #[test]
    fn a_failing_pull_gets_a_grace_period_that_a_retry_does_not_reset() {
        let grace = Duration::from_secs(60);
        let start = std::time::Instant::now();
        let at = |secs| start + Duration::from_secs(secs);
        let mut watch = PullWatch::default();

        let pulling = pod_pulling("ErrImagePull");
        let backing_off = pod_pulling("ImagePullBackOff");
        let retrying = pod_pulling("ContainerCreating");
        assert!(watch.check(&pulling, "p", at(0), grace).is_none());
        assert!(watch.check(&backing_off, "p", at(30), grace).is_none());
        assert!(watch.check(&retrying, "p", at(59), grace).is_none());
        // The retry above did not restart the clock.
        let reason = watch.check(&backing_off, "p", at(60), grace).unwrap();
        assert!(reason.contains("ImagePullBackOff"));
        assert!(reason.contains("registry said no"));
        // Only a failure being reported right now can end the step.
        assert!(watch.check(&retrying, "p", at(90), grace).is_none());
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

    /// A runtime whose "kubectl" is a script playing a cluster: pods are
    /// directories under `work/pod`, and one step exits 3 after deleting
    /// `deleted.txt` and writing `new.txt` in the workspace mount.
    #[cfg(unix)]
    fn runtime_with_fake_cluster(work: &Path) -> KubernetesRuntime {
        let script = work.join("kubectl.sh");
        let body = r#"
work="$(dirname "$0")"
echo "$*" >> "$work/log"
case "$1" in
create) cat > "$work/manifest.json" ;;
get)
    polls=$(($(cat "$work/polls" 2>/dev/null || echo 0) + 1))
    echo "$polls" > "$work/polls"
    case "$polls" in
    1) echo '{"status":{"phase":"Pending"}}' ;;
    2) echo '{"status":{"phase":"Pending","initContainerStatuses":[{"name":"wrkflw-wait","state":{"running":{}}}]}}' ;;
    3) echo '{"status":{"phase":"Running","containerStatuses":[{"name":"main","state":{"running":{}}}]}}' ;;
    *) echo '{"status":{"phase":"Running","containerStatuses":[{"name":"main","state":{"terminated":{"exitCode":3}}}]}}' ;;
    esac
    ;;
logs)
    rm "$work/pod/github/workspace/deleted.txt"
    echo made in the pod > "$work/pod/github/workspace/new.txt"
    echo step output
    ;;
exec)
    while [ "$1" != "--" ]; do shift; done
    shift
    if [ "$1" = tar ]; then
        dir="$work/pod$3"
        mkdir -p "$dir"
        shift 3
        case "$*" in
        *-xof*) tar -C "$dir" -xf - ;;
        *) tar -C "$dir" "$@" ;;
        esac
    fi
    ;;
esac
"#;
        std::fs::write(&script, body).unwrap();
        let mut rt = KubernetesRuntime::new(cfg(), false);
        rt.kubectl = "sh".into();
        rt.kubectl_prefix = vec![script.to_string_lossy().to_string()];
        rt
    }

    #[cfg(unix)]
    #[tokio::test]
    async fn a_step_runs_from_pod_creation_to_sync_back_and_delete() {
        let work = tempfile::tempdir().unwrap();
        let ws = tempfile::tempdir().unwrap();
        let nested = tempfile::tempdir().unwrap();
        std::fs::write(ws.path().join("deleted.txt"), "x").unwrap();
        std::fs::create_dir(ws.path().join("sub")).unwrap();
        std::fs::write(ws.path().join("sub/kept-on-host.txt"), "x").unwrap();
        std::fs::write(nested.path().join("nested.txt"), "x").unwrap();

        let rt = runtime_with_fake_cluster(work.path());
        let output = rt
            .run_container(
                "ubuntu:22.04",
                &["sh", "-c", "exit 3"],
                &[],
                Path::new("/github/workspace"),
                &[
                    (ws.path(), Path::new("/github/workspace")),
                    (nested.path(), Path::new("/github/workspace/sub")),
                ],
                None,
            )
            .await
            .unwrap();

        assert_eq!(output.exit_code, 3);
        assert_eq!(output.stdout.trim(), "step output");
        assert!(ws.path().join("new.txt").exists());
        assert!(!ws.path().join("deleted.txt").exists());
        assert!(
            ws.path().join("sub/kept-on-host.txt").exists(),
            "a nested mount's path is not the parent's to prune"
        );

        let log = std::fs::read_to_string(work.path().join("log")).unwrap();
        let line_of = |needle: &str| {
            log.lines()
                .position(|line| line.contains(needle))
                .unwrap_or_else(|| panic!("no `{}` in:\n{}", needle, log))
        };
        let order = [
            "create -f -",
            "rm -rf",
            "/github/workspace/sub -xof",
            "touch /wrkflw-sync/ready",
            "logs ",
            "-cf - .",
            "delete pod",
        ];
        let lines: Vec<_> = order.iter().map(|needle| line_of(needle)).collect();
        assert!(lines.windows(2).all(|w| w[0] < w[1]), "{}", log);
        assert!(line_of("--exclude=./sub") > line_of("logs "), "{}", log);
        assert!(!RUNNING_PODS
            .lock()
            .unwrap()
            .iter()
            .any(|p| log.contains(&p.name)));
    }

    /// A guard whose "kubectl" records its arguments in `log` instead.
    #[cfg(unix)]
    fn recording_guard(name: &str, log: &Path) -> PodGuard {
        PodGuard::new(
            PodRef {
                kubectl_args: vec![
                    "-c".into(),
                    format!("echo \"$@\" > {}.tmp && mv {0}.tmp {0}", log.display()),
                    "sh".into(),
                ],
                name: name.into(),
            },
            "sh".into(),
        )
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
        assert!(!args.contains("--force"), "{}", args);
        assert!(!args.contains("--grace-period"), "{}", args);
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
