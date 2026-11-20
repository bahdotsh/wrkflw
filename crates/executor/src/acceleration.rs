//! Doing less work on a rerun: job result reuse, `--affected` skipping,
//! dependency cache mounts, and per-run `concurrency:` groups.
//!
//! Everything here is opt-in except concurrency groups, and everything fails
//! towards running the job: an unreadable tree, a cache miss or a bad entry
//! means the job runs as it would have without this module.
//!
//! The state lives in a task-local rather than in the step and job context
//! structs, the same way `events::HOOKS` does. A `tokio::spawn` inside the
//! executor would lose it, so job execution has to stay on the run's task.

mod concurrency;
mod sidecar;
mod store;
mod tree;

pub(crate) use concurrency::acquire_concurrency;
use serde_json::Value;
use sha2::{Digest, Sha256};
pub use sidecar::{JobHints, Sidecar};
use std::collections::{BTreeMap, HashMap, HashSet};
use std::future::Future;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex, OnceLock};
use store::ResultStore;
pub(crate) use store::StoredJob;
use tokio::sync::{OnceCell, Semaphore};
pub(crate) use tree::hash_tree;
use tree::Tree;
#[cfg(test)]
use tree::TreeHashes;
use wrkflw_parser::workflow::{Job, WorkflowDefinition};

/// The sidecar that holds what a workflow file can't: GitHub rejects unknown
/// keys in workflow YAML, so job hints can't live next to the job.
pub const SIDECAR_PATH: &str = ".wrkflw/affected.yml";

/// Bump when the fingerprint's inputs change, so old entries stop matching.
/// Version 5: steps of the built-in image run as the runner user, so a pass
/// stored when they ran as root says nothing of a run as that user.
/// Version 6: JavaScript actions run, so a pass stored when they passed
/// without running says nothing of a run of them.
const KEY_VERSION: u32 = 6;

/// Host directories mounted into containers under `--dep-cache`, and where
/// in a user's home directory each tool keeps that cache.
const DEP_CACHE_DIRS: &[(&str, &str)] = &[
    ("cargo-registry", ".cargo/registry"),
    ("cargo-git", ".cargo/git"),
    ("npm", ".npm"),
    ("pip", ".cache/pip"),
    ("go-mod", "go/pkg/mod"),
];

/// The home directories a step's caches may be in, in the image wrkflw
/// builds: root's, and the runner user's. Which user a step runs as isn't
/// known until the job's container is up, after its mounts are fixed, and
/// cargo keeps its caches in `CARGO_HOME`, the runner user's home there for
/// root too, so in that image each cache is mounted in both. Any other
/// image only gets root's: its own user's home, even one named `runner`,
/// isn't wrkflw's to mount over.
const DEP_CACHE_HOMES: &[&str] = &["/root", crate::runner_user::HOME];

/// The directory, beside a repository's caches, that holds a second set for
/// steps that always run as root: a job `container:`, an image mapped with
/// `-P`, a step in a container of its own, or a run with
/// `WRKFLW_STEPS_AS_ROOT`. Sharing one set with steps that run as the
/// runner user would leave that user files it can't write, from a root job
/// running beside it.
const ROOT_DEP_CACHE_DIR: &str = "as-root";

/// The `--dep-cache` directories of a run: each one's host directory and
/// where in a home directory it goes, one set for steps that may run as the
/// runner user and one for steps that run as root.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
struct DepCacheMounts {
    runner: Vec<(PathBuf, &'static str)>,
    root: Vec<(PathBuf, &'static str)>,
}

/// Who a step's `--dep-cache` mounts are for.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct DepCacheFor {
    /// The step may run as the runner user: it gets the set that user's
    /// steps share. Otherwise it runs as root, and gets root's own set.
    pub(crate) runner_user: bool,
    /// The step's image is one wrkflw built with the runner user, whose home
    /// the caches go in as well as root's: see [`DEP_CACHE_HOMES`].
    pub(crate) runner_home: bool,
}

/// Files that always count as input, because steps load them without naming
/// them. Path hints would otherwise let a composite action change go unseen.
const ALWAYS_INPUT: &[&str] = &[".github/actions/**", ".gitea/actions/**", SIDECAR_PATH];

/// Covered by the job definition in the key. Hashing them too would make any
/// edit to a workflow invalidate every job in the repository.
const NEVER_INPUT: &[&str] = &[".github/workflows/**", ".gitea/workflows/**"];

/// Text that reads a credential. Plain "secrets" won't do: every serialized
/// job has a `secrets` key, so that would rule out every job.
const SECRET_REFERENCES: &[&str] = &[
    "secrets.",
    "secrets[",
    "secrets)",
    "secrets }",
    "secrets}",
    "github.token",
    "github_token",
    "github-token",
];

/// Actions that change something on GitHub or a registry, and take
/// `github.token` by default, so a job that uses them reads a credential
/// without its YAML saying so. Matched as a prefix of `uses:`, lowercased.
const MUTATING_ACTIONS: &[&str] = &[
    "actions/github-script",
    "actions/create-release",
    "actions/upload-release-asset",
    "actions/labeler",
    "actions/stale",
    "actions/first-interaction",
    "actions/deploy-pages",
    "actions/attest",
    "softprops/action-gh-release",
    "ncipollo/release-action",
    "peter-evans/create-pull-request",
    "peter-evans/create-or-update-comment",
    "peter-evans/repository-dispatch",
    "marocchino/sticky-pull-request-comment",
    "thollander/actions-comment-pull-request",
    "stefanzweifel/git-auto-commit-action",
    "ad-m/github-push-action",
    "googleapis/release-please-action",
    "changesets/action",
    "docker/login-action",
    "docker/build-push-action",
    "pypa/gh-action-pypi-publish",
    "js-devtools/npm-publish",
    "jamesives/github-pages-deploy-action",
    "peaceiris/actions-gh-pages",
];

/// Actions that hand files to another job. A reused job hands over nothing,
/// so the job that needs them would fail.
const ARTIFACT_ACTIONS: &[&str] = &[
    "actions/upload-artifact",
    "actions/download-artifact",
    "actions/upload-pages-artifact",
];

/// The `uses:` of the first step in `job` that runs one of `actions`, at any
/// version or subpath. Lowercased, as GitHub matches action names.
fn uses_one_of<'a>(job: &'a Job, actions: &[&str]) -> Option<&'a str> {
    job.steps.iter().find_map(|step| {
        let uses = step.uses.as_deref()?;
        let lower = uses.to_ascii_lowercase();
        actions
            .iter()
            .any(|action| {
                lower
                    .strip_prefix(action)
                    .is_some_and(|rest| rest.is_empty() || rest.starts_with(['@', '/']))
            })
            .then_some(uses)
    })
}

/// A credential that must not reach logs through `Debug`.
#[derive(Clone, Default, PartialEq)]
pub struct Secret(pub String);

impl std::fmt::Debug for Secret {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("Secret([REDACTED])")
    }
}

/// What a run may do to avoid work. The default does nothing.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct Acceleration {
    /// Return the stored result of a job whose fingerprint already passed.
    pub reuse_results: bool,
    /// Files changed since the base. `Some` turns on `--affected`.
    pub changed_files: Option<Vec<String>>,
    /// Run each `run:` step in a container of its own instead of the job's
    /// one, so what a step leaves outside the workspace is gone in the next.
    pub step_containers: bool,
    /// Start a Docker engine inside the container of a job whose steps run
    /// `docker`. That container runs privileged.
    pub docker_in_jobs: bool,
    /// Mount package manager caches from the host.
    pub dep_cache: bool,
    /// Directory, `file://` or `https://` base for shared results.
    pub remote_cache: Option<String>,
    /// Upload this run's results to `remote_cache`.
    pub push_results: bool,
    /// Signs and verifies remote entries. Required with `remote_cache`.
    pub signing_key: Option<Secret>,
    /// Bearer token for an `https://` remote.
    pub remote_token: Option<Secret>,
    /// This run is `wrkflw gate`'s: jobs the sidecar marks `gate: false` are
    /// skipped and left to CI.
    pub gate: bool,
}

/// Why a remote cache setting can't be used.
pub fn validate_remote(accel: &Acceleration) -> Result<(), String> {
    let Some(remote) = &accel.remote_cache else {
        if accel.push_results {
            return Err("--cache-push needs --remote-cache".to_string());
        }
        return Ok(());
    };
    if let Some((scheme, _)) = remote.split_once("://") {
        if !matches!(scheme, "file" | "http" | "https") {
            return Err(format!(
                "--remote-cache doesn't support {}://. Use a directory, a file:// path \
                 or an https:// URL",
                scheme
            ));
        }
    }
    if accel.signing_key.as_ref().is_none_or(|k| k.0.is_empty()) {
        return Err(
            "--remote-cache needs WRKFLW_CACHE_SIGNING_KEY: an unsigned result would let \
             anyone who can write to the cache mark a job as passed"
                .to_string(),
        );
    }
    if remote.starts_with("http://") && !is_loopback_url(remote) {
        return Err("--remote-cache must use https unless the host is local".to_string());
    }
    Ok(())
}

/// Whether `url` names this machine, by name or by a loopback address.
fn is_loopback_url(url: &str) -> bool {
    let Ok(url) = reqwest::Url::parse(url) else {
        return false;
    };
    let Some(host) = url.host_str() else {
        return false;
    };
    let bare = host.trim_start_matches('[').trim_end_matches(']');
    match bare.parse::<std::net::IpAddr>() {
        Ok(ip) => ip.is_loopback(),
        Err(_) => host.eq_ignore_ascii_case("localhost"),
    }
}

fn compile_globs(patterns: &[&str]) -> Vec<glob::Pattern> {
    patterns
        .iter()
        .filter_map(|p| glob::Pattern::new(p).ok())
        .collect()
}

fn glob_options() -> glob::MatchOptions {
    glob::MatchOptions {
        case_sensitive: true,
        require_literal_separator: true,
        require_literal_leading_dot: false,
    }
}

fn matches_any(path: &str, patterns: &[glob::Pattern]) -> bool {
    patterns
        .iter()
        .any(|p| p.matches_with(path, glob_options()))
}

/// `hints.paths` plus the files that always count, or `None` for every file.
///
/// The files that always count include every local action `job` uses, as a
/// step loads `uses: ./tools/x` without the job's paths naming it.
fn input_patterns(
    job: &Job,
    hints: Option<&JobHints>,
) -> Result<Option<Vec<glob::Pattern>>, String> {
    let Some(paths) = hints.and_then(|h| h.paths.as_ref()) else {
        return Ok(None);
    };
    let mut patterns = compile_globs(ALWAYS_INPUT);
    patterns.extend(local_action_patterns(job));
    for path in paths {
        patterns.push(
            glob::Pattern::new(path)
                .map_err(|e| format!("bad path '{}' in {}: {}", path, SIDECAR_PATH, e))?,
        );
    }
    Ok(Some(patterns))
}

/// A pattern for the directory of each `uses: ./<dir>` step in `job`.
fn local_action_patterns(job: &Job) -> Vec<glob::Pattern> {
    job.steps
        .iter()
        .filter_map(|step| step.uses.as_deref()?.strip_prefix("./"))
        .filter_map(|dir| {
            let dir = dir.trim_end_matches('/');
            if dir.is_empty() {
                // An action at the repository root can read any file.
                return glob::Pattern::new("**").ok();
            }
            glob::Pattern::new(&format!("{}/**", glob::Pattern::escape(dir))).ok()
        })
        .collect()
}

/// Whether `permissions:`, at the job or workflow level, grants any write.
/// `write-all` is the one string form that does.
fn grants_write(permissions: &serde_yaml::Value) -> bool {
    match permissions {
        serde_yaml::Value::String(all) => all == "write-all",
        serde_yaml::Value::Mapping(scopes) => scopes
            .values()
            .any(|access| access.as_str() == Some("write")),
        _ => false,
    }
}

/// Why a job's result must not be reused, or `None` if it may be.
///
/// wrkflw can't see what a step does, so this rules out what it can detect:
/// a job that touches credentials probably changes something outside the
/// repository, and a skipped job produces no artifacts for downstream jobs.
///
/// A job and workflow with no `permissions:` count as read-only here, though
/// on GitHub that means the repository's default, which is write-all on
/// repositories created before February 2023. Treating that as a write would
/// rule out nearly every job. The README asks for `cache: false` instead.
pub(crate) fn uncacheable_reason(
    job: &Job,
    workflow: &WorkflowDefinition,
    hints: Option<&JobHints>,
) -> Option<&'static str> {
    if hints.is_some_and(|h| !h.cache) {
        return Some("cache: false in the sidecar");
    }
    if job.uses.is_some() {
        return Some("a reusable workflow's contents aren't in the fingerprint");
    }
    // A job's own `permissions:` replaces the workflow's, as on GitHub.
    let writes = match &job.permissions {
        Some(own) => own.values().any(|v| v == "write"),
        None => workflow.permissions.as_ref().is_some_and(grants_write),
    };
    if writes {
        return Some("it has write permissions");
    }
    if uses_one_of(job, ARTIFACT_ACTIONS).is_some() {
        return Some("it exchanges artifacts");
    }
    if uses_one_of(job, MUTATING_ACTIONS).is_some() {
        return Some("it uses an action that publishes or writes to GitHub");
    }
    const UNREADABLE: &str = "its definition can't be checked for secrets";
    let Some(env_text) = searchable(&sorted(&workflow.env)) else {
        return Some(UNREADABLE);
    };
    let Some(job_text) = searchable(job) else {
        return Some(UNREADABLE);
    };
    if reads_secrets(job, &job_text, &env_text) {
        return Some("it reads secrets");
    }
    if job.outputs.as_ref().is_some_and(|o| !o.is_empty())
        && job_text.contains("needs.")
        && upstream_reads_secrets(job, workflow, &env_text)
    {
        return Some("its outputs may carry a secret from a job it needs");
    }
    None
}

/// `value` as lowercase JSON, for the text checks below.
fn searchable(value: &impl serde::Serialize) -> Option<String> {
    serde_json::to_string(value)
        .ok()
        .map(|text| text.to_lowercase())
}

/// Whether a job, given its `searchable` text and the workflow `env`'s,
/// refers to a credential.
fn reads_secrets(job: &Job, job_text: &str, env_text: &str) -> bool {
    job.secrets.is_some()
        || SECRET_REFERENCES
            .iter()
            .any(|needle| job_text.contains(needle) || env_text.contains(needle))
}

/// Whether any job `job` needs, directly or not, reads a credential. Stored
/// outputs go to the remote in plain text, so a value passed along must not
/// end up there.
fn upstream_reads_secrets(job: &Job, workflow: &WorkflowDefinition, env_text: &str) -> bool {
    let mut seen = HashSet::new();
    let mut pending: Vec<&String> = job.needs.iter().flatten().collect();
    while let Some(name) = pending.pop() {
        if !seen.insert(name) {
            continue;
        }
        let Some(upstream) = workflow.jobs.get(name) else {
            continue;
        };
        let reads =
            searchable(upstream).is_none_or(|text| reads_secrets(upstream, &text, env_text));
        if reads {
            return true;
        }
        pending.extend(upstream.needs.iter().flatten());
    }
    false
}

fn sorted(map: &HashMap<String, String>) -> BTreeMap<&String, &String> {
    map.iter().collect()
}

/// Sort object keys at every depth. Job maps are `HashMap`s, whose order
/// changes per process, and that would give each run a different key.
fn canonical(value: Value) -> Value {
    match value {
        Value::Object(map) => {
            let ordered: BTreeMap<String, Value> =
                map.into_iter().map(|(k, v)| (k, canonical(v))).collect();
            let mut out = serde_json::Map::new();
            for (k, v) in ordered {
                out.insert(k, v);
            }
            Value::Object(out)
        }
        Value::Array(items) => Value::Array(items.into_iter().map(canonical).collect()),
        other => other,
    }
}

fn hex(bytes: &[u8]) -> String {
    bytes.iter().map(|b| format!("{:02x}", b)).collect()
}

/// Everything that makes a job's result this job's result: the job as
/// `decide` sees it, and the files it reads.
pub(crate) struct KeyInput<'a> {
    pub lookup: &'a Lookup<'a>,
    pub tree: &'a Tree,
    pub patterns: Option<&'a [glob::Pattern]>,
}

/// The fingerprint a stored result is filed under. `None` if the job can't be
/// serialized, which leaves it running as normal.
pub(crate) fn job_key(input: &KeyInput<'_>) -> Option<String> {
    job_key_with(
        input,
        crate::run_options::every_matrix_leg(),
        !crate::runner_user::for_run(),
    )
}

/// [`job_key`], with `every_matrix_leg` saying whether matrix legs that
/// would repeat another leg still run, and `steps_as_root` whether the run
/// keeps steps as root where they would run as the runner user: because it
/// asked to, or because it can't have that user at all (see
/// [`runner_user::for_run`](crate::runner_user::for_run)).
fn job_key_with(
    input: &KeyInput<'_>,
    every_matrix_leg: bool,
    steps_as_root: bool,
) -> Option<String> {
    let lookup = input.lookup;
    let inputs = match input.patterns {
        None => input.tree.digest.clone(),
        Some(patterns) => input.tree.digest_matching(patterns)?,
    };
    let env = |name: &str| lookup.env_context.get(name).cloned();
    let document = serde_json::json!({
        "version": KEY_VERSION,
        "job_name": lookup.job_name,
        "job": serde_json::to_value(lookup.job).ok()?,
        "workflow_env": lookup.workflow.env,
        "workflow_defaults": serde_json::to_value(&lookup.workflow.defaults).ok()?,
        "platform_overrides": lookup.platform_overrides,
        "needs_outputs": lookup.needs_outputs,
        "needs_results": lookup.needs_results,
        "event": env("GITHUB_EVENT_NAME"),
        "ref": env("GITHUB_REF"),
        "base_ref": env("GITHUB_BASE_REF"),
        "runtime": env("WRKFLW_RUNTIME_MODE"),
        "os": std::env::consts::OS,
        "arch": std::env::consts::ARCH,
        "inputs": inputs,
    });
    // A matrix result stored by default may hold legs that were folded into
    // another leg and never ran, so it can't stand in for a run of every leg.
    // Only that mode changes the key, so stored and shared results from a
    // default run keep their keys.
    let mut document = document;
    if every_matrix_leg && lookup.job.matrix_config().is_some() {
        document["every_matrix_leg"] = serde_json::json!(true);
    }
    // A matrix whose macOS legs run on this Mac (`--macos-on-host`) passed
    // on other ground than one whose legs all ran in containers. Only that
    // mode changes the key. A job without a matrix differs by `runtime`.
    if crate::host_jobs::legs_may_run_on_host(lookup.job) {
        document["macos_on_host"] = serde_json::json!(true);
    }
    // A pass with a container per step can rest on each step starting from
    // a clean image, so it can't stand in for a run whose steps share one,
    // or the other way round. Version 3 keys were all of the former.
    if container_policy().is_some_and(|policy| policy.one_container_per_step()) {
        document["step_containers"] = serde_json::json!(true);
    }
    // A pass as root, which may write what the runner user can't, can't
    // stand in for one as that user, or the other way round.
    if steps_as_root {
        document["steps_as_root"] = serde_json::json!(true);
    }
    // A pass whose actions were left unrun, all JavaScript ones or those
    // the user listed, can't stand in for a run of them.
    if !crate::node_actions::enabled() {
        document["js_actions"] = serde_json::json!(false);
    }
    if let Some(skipped) = crate::run_options::skipped_actions_key() {
        document["skipped_actions"] = serde_json::json!(skipped);
    }
    let text = serde_json::to_string(&canonical(document)).ok()?;
    Some(hex(&Sha256::digest(text.as_bytes())))
}

/// State shared by every job of one run.
pub(crate) struct RunState {
    pub accel: Acceleration,
    pub preserve_containers: bool,
    root: PathBuf,
    sidecar: Option<Sidecar>,
    /// `None` unless this run reuses or shares results, so a run that doesn't
    /// never needs a cache directory.
    store: Option<ResultStore>,
    tree: OnceCell<Option<Arc<Tree>>>,
    /// Jobs whose steps really ran, so a dependent can't be skipped as
    /// unaffected when something it consumes has just changed.
    executed: Mutex<HashSet<String>>,
    groups: Mutex<HashMap<String, Arc<Semaphore>>>,
    dep_mounts: OnceLock<DepCacheMounts>,
    /// Host directories whose files this run has already handed to the
    /// runner user, so each shared cache is walked once, not per job.
    handed_over: Mutex<HashSet<PathBuf>>,
    /// Set once the run has said it won't push an emulated result.
    warned_on_host_push: AtomicBool,
}

tokio::task_local! {
    /// `None` inside a called workflow, whose inputs aren't in any fingerprint.
    static RUN: Option<Arc<RunState>>;
    /// Set with [`RUN`], and kept inside a called workflow.
    static CONTAINERS: Arc<ContainerScope>;
}

/// What the jobs of a run, called workflows' included, share about
/// containers.
struct ContainerScope {
    policy: ContainerPolicy,
    /// Set once the run has said its steps run `docker` with no engine.
    warned_of_missing_docker: AtomicBool,
    /// The GitHub token this run gives its jobs, once a job asked for it.
    job_token: tokio::sync::OnceCell<Option<String>>,
    /// This machine's GitHub token for cloning private repositories, once
    /// a clone needed it.
    clone_token: tokio::sync::OnceCell<Option<String>>,
    /// JavaScript actions cloned this run.
    action_clones: crate::node_actions::ActionClones,
    /// Set once the run has said its jobs get the user's own token.
    warned_of_own_token: AtomicBool,
    /// Set once the run has said a remote JavaScript action runs on this
    /// machine as the user.
    warned_of_host_actions: AtomicBool,
    /// Why this run's steps can't run as the runner user, once settled:
    /// `None` when they may.
    runner_user: tokio::sync::OnceCell<Option<crate::runner_user::Ruled>>,
    /// Set once the run has said why its steps run as root.
    warned_of_root_steps: AtomicBool,
}

/// How the jobs of a run use containers. Unlike the rest of the run's
/// state it reaches the jobs of a called workflow, whose steps share a
/// machine on GitHub as any job's do.
#[derive(Debug, Clone, Copy, Default, PartialEq)]
pub(crate) struct ContainerPolicy {
    /// One container per step, not one per job.
    pub step_containers: bool,
    /// Keep a failed step's container, which takes one per step.
    pub preserve_containers: bool,
    /// Start a Docker engine in the container of a job that runs `docker`.
    pub docker_in_jobs: bool,
}

impl ContainerPolicy {
    /// Each `run:` step gets a container of its own, which a kept-for-
    /// debugging container also takes.
    pub(crate) fn one_container_per_step(&self) -> bool {
        self.step_containers || self.preserve_containers
    }
}

/// The directory results are kept in.
fn results_dir() -> Option<PathBuf> {
    if let Some(dir) = std::env::var_os("WRKFLW_RESULTS_DIR") {
        return Some(PathBuf::from(dir));
    }
    Some(dirs::cache_dir()?.join("wrkflw").join("results"))
}

/// Paths in the sidecar and from `git diff` start at the repository root, so
/// neither lines up from a subdirectory.
fn warn_outside_repo_root(root: &Path) {
    let output = std::process::Command::new("git")
        .arg("-C")
        .arg(root)
        .args(["rev-parse", "--show-prefix"])
        .output();
    if let Ok(output) = output {
        let prefix = String::from_utf8_lossy(&output.stdout);
        if output.status.success() && !prefix.trim().is_empty() {
            wrkflw_logging::warning(&format!(
                "Running from {}, not the repository root: {} and changed paths may not apply",
                prefix.trim(),
                SIDECAR_PATH
            ));
        }
    }
}

impl RunState {
    pub(crate) fn new(
        accel: Acceleration,
        preserve_containers: bool,
        root: PathBuf,
    ) -> Result<Self, String> {
        // The CLI validates first, to fail in its output format before the run
        // is recorded. This covers library callers that build the config.
        validate_remote(&accel)?;
        let sidecar = if accel.reuse_results || accel.changed_files.is_some() || accel.gate {
            warn_outside_repo_root(&root);
            Some(Sidecar::load(&root)?)
        } else {
            None
        };
        let store = if accel.reuse_results || accel.remote_cache.is_some() {
            let store_root = results_dir().ok_or(
                "could not determine the cache directory for --reuse-results: \
                 set WRKFLW_RESULTS_DIR",
            )?;
            Some(ResultStore::new(store_root, &accel))
        } else {
            None
        };
        Ok(Self {
            store,
            accel,
            preserve_containers,
            root,
            sidecar,
            tree: OnceCell::new(),
            executed: Mutex::new(HashSet::new()),
            groups: Mutex::new(HashMap::new()),
            dep_mounts: OnceLock::new(),
            handed_over: Mutex::new(HashSet::new()),
            warned_on_host_push: AtomicBool::new(false),
        })
    }

    async fn tree(&self) -> Option<Arc<Tree>> {
        self.tree
            .get_or_init(|| async {
                let root = self.root.clone();
                let hashed =
                    tokio::task::spawn_blocking(move || hash_tree(&root).and_then(Tree::new))
                        .await
                        .ok()
                        .flatten();
                if hashed.is_none() {
                    wrkflw_logging::warning(
                        "Not reusing job results: this directory isn't a git work tree, \
                         or a repository inside it can't be read",
                    );
                }
                hashed.map(Arc::new)
            })
            .await
            .clone()
    }

    fn hints(&self, job_name: &str) -> Option<&JobHints> {
        self.sidecar.as_ref()?.jobs.get(job_name)
    }
}

#[cfg(test)]
impl RunState {
    /// This run, with its `--dep-cache` directories under `base` instead of
    /// `WRKFLW_DEP_CACHE_DIR`, which a test can't set for itself alone.
    pub(crate) fn with_dep_caches_in(self, base: &Path) -> Self {
        let _ = self.dep_mounts.set(dep_cache_dirs_in(base, &self.root));
        self
    }
}

/// Run `future` with this run's state available to the jobs it executes.
pub(crate) async fn with_run_state<F: Future>(state: Arc<RunState>, future: F) -> F::Output {
    // The variables reach a run started from the TUI or `watch` too, which
    // have no flags for these.
    let policy = ContainerPolicy {
        step_containers: state.accel.step_containers
            || crate::run_options::env_flag(crate::run_options::STEP_CONTAINERS_ENV),
        preserve_containers: state.preserve_containers,
        docker_in_jobs: state.accel.docker_in_jobs
            || crate::run_options::env_flag(crate::run_options::DOCKER_IN_JOBS_ENV),
    };
    let scope = Arc::new(ContainerScope {
        policy,
        warned_of_missing_docker: AtomicBool::new(false),
        job_token: tokio::sync::OnceCell::new(),
        clone_token: tokio::sync::OnceCell::new(),
        action_clones: crate::node_actions::ActionClones::default(),
        warned_of_own_token: AtomicBool::new(false),
        warned_of_host_actions: AtomicBool::new(false),
        runner_user: tokio::sync::OnceCell::new(),
        warned_of_root_steps: AtomicBool::new(false),
    });
    CONTAINERS
        .scope(scope, RUN.scope(Some(state), future))
        .await
}

/// Run a called workflow's jobs with none of this applied.
pub(crate) async fn without_run_state<F: Future>(future: F) -> F::Output {
    RUN.scope(None, future).await
}

pub(crate) fn current() -> Option<Arc<RunState>> {
    RUN.try_with(|run| run.clone()).ok().flatten()
}

/// True the first time a run asks, called workflows included, so it says
/// only once that steps run `docker` with no engine there.
pub(crate) fn first_missing_docker_warning() -> bool {
    CONTAINERS
        .try_with(|scope| !scope.warned_of_missing_docker.swap(true, Ordering::Relaxed))
        .unwrap_or(true)
}

/// True the first time a run asks, called workflows included, so it says
/// only once that its jobs get the user's own GitHub token.
pub(crate) fn first_own_token_warning() -> bool {
    CONTAINERS
        .try_with(|scope| !scope.warned_of_own_token.swap(true, Ordering::Relaxed))
        .unwrap_or(true)
}

/// True the first time a run asks, called workflows included, so it says
/// only once that remote JavaScript actions run on this machine.
pub(crate) fn first_host_action_warning() -> bool {
    CONTAINERS
        .try_with(|scope| !scope.warned_of_host_actions.swap(true, Ordering::Relaxed))
        .unwrap_or(true)
}

/// True the first time a run asks, called workflows included, so it says
/// only once why its steps run as root.
pub(crate) fn first_root_steps_warning() -> bool {
    CONTAINERS
        .try_with(|scope| !scope.warned_of_root_steps.swap(true, Ordering::Relaxed))
        .unwrap_or(true)
}

/// Why this run's steps can't run as the runner user, once
/// [`settle_runner_user`] settled it, with `Some(None)` when they may;
/// `None` before, or outside a run.
pub(crate) fn settled_runner_user() -> Option<Option<crate::runner_user::Ruled>> {
    CONTAINERS
        .try_with(|scope| scope.runner_user.get().copied())
        .ok()
        .flatten()
}

/// Settle [`settled_runner_user`] with what `find` says, the first time a
/// job of the run asks. Outside a run nothing is kept.
pub(crate) async fn settle_runner_user<F>(find: impl FnOnce() -> F)
where
    F: Future<Output = Option<crate::runner_user::Ruled>>,
{
    if let Ok(scope) = CONTAINERS.try_with(Arc::clone) {
        scope.runner_user.get_or_init(find).await;
    }
}

/// What `find` gives for this run, the first time a job of it asks:
/// each run, from the TUI or `watch` too, reads a token that may have
/// changed since the last and says once that it hands it out. Outside a
/// run, each call finds it anew.
pub(crate) async fn run_job_token<F>(find: impl FnOnce() -> F) -> Option<String>
where
    F: Future<Output = Option<String>>,
{
    match CONTAINERS.try_with(Arc::clone) {
        Ok(scope) => scope.job_token.get_or_init(find).await.clone(),
        Err(_) => find().await,
    }
}

/// What `find` gives for this run, the first time a clone of it needs a
/// token: see [`crate::run_options::remote_clone_token`]. Outside a run,
/// each call finds it anew.
pub(crate) async fn run_clone_token<F>(find: impl FnOnce() -> F) -> Option<String>
where
    F: Future<Output = Option<String>>,
{
    match CONTAINERS.try_with(Arc::clone) {
        Ok(scope) => scope.clone_token.get_or_init(find).await.clone(),
        Err(_) => find().await,
    }
}

/// This run's clone of the action `key` names, made with `clone` the first
/// time a job asks for it. Outside a run each call clones anew.
pub(crate) async fn run_action_clone<F, Fut>(
    key: &str,
    clone: F,
) -> Result<Arc<tempfile::TempDir>, String>
where
    F: FnOnce(PathBuf) -> Fut,
    Fut: Future<Output = Result<(), String>>,
{
    match CONTAINERS.try_with(Arc::clone) {
        Ok(scope) => scope.action_clones.get_or_clone(key, clone).await,
        Err(_) => {
            crate::node_actions::ActionClones::default()
                .get_or_clone(key, clone)
                .await
        }
    }
}

/// How this run's jobs use containers, or `None` outside a run that has a
/// say in it, as a GitLab pipeline's.
pub(crate) fn container_policy() -> Option<ContainerPolicy> {
    CONTAINERS.try_with(|scope| scope.policy).ok()
}

/// Host and container paths of the `--dep-cache` mounts of a step, for
/// whom `who` says: the set the [runner user](crate::runner_user)'s steps
/// share, or root's own, in root's home and, in the image wrkflw builds,
/// the runner user's too.
pub(crate) fn dep_cache_mounts(
    mounts_are_copies: bool,
    who: DepCacheFor,
) -> Vec<(PathBuf, PathBuf)> {
    let Some(run) = current().filter(|run| run.accel.dep_cache && !mounts_are_copies) else {
        return Vec::new();
    };
    run.dep_mounts
        .get_or_init(|| create_dep_cache_dirs(&run.root))
        .for_steps(who)
}

impl DepCacheMounts {
    fn for_steps(&self, who: DepCacheFor) -> Vec<(PathBuf, PathBuf)> {
        let set = if who.runner_user {
            &self.runner
        } else {
            &self.root
        };
        let homes = if who.runner_home || who.runner_user {
            DEP_CACHE_HOMES
        } else {
            &DEP_CACHE_HOMES[..1]
        };
        set.iter()
            .flat_map(|(host, in_home)| {
                homes
                    .iter()
                    .map(move |home| (host.clone(), Path::new(home).join(in_home)))
            })
            .collect()
    }
}

/// Of `hosts`, those this run hasn't handed to the runner user yet. Outside
/// a run, every one.
pub(crate) fn not_handed_over(hosts: Vec<PathBuf>) -> Vec<PathBuf> {
    match current() {
        Some(run) => {
            let done = run.handed_over.lock().unwrap_or_else(|e| e.into_inner());
            hosts
                .into_iter()
                .filter(|host| !done.contains(host))
                .collect()
        }
        None => hosts,
    }
}

/// Note that `hosts` hold nothing but the runner user's files now.
pub(crate) fn mark_handed_over(hosts: impl IntoIterator<Item = PathBuf>) {
    if let Some(run) = current() {
        run.handed_over
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .extend(hosts);
    }
}

/// The host side of the mounts for the repository at `repo`, under
/// `WRKFLW_DEP_CACHE_DIR` or the cache directory.
fn create_dep_cache_dirs(repo: &Path) -> DepCacheMounts {
    let base = std::env::var_os("WRKFLW_DEP_CACHE_DIR")
        .map(PathBuf::from)
        .or_else(|| dirs::cache_dir().map(|d| d.join("wrkflw").join("deps")));
    match base {
        Some(base) => dep_cache_dirs_in(&base, repo),
        None => DepCacheMounts::default(),
    }
}

/// A short id for the repository at `repo`.
fn repo_cache_id(repo: &Path) -> String {
    let canonical = repo.canonicalize().unwrap_or_else(|_| repo.to_path_buf());
    hex(&Sha256::digest(canonical.to_string_lossy().as_bytes()))[..16].to_string()
}

/// Each repository gets caches of its own under `base`. Steps can write to
/// them, so a job in one repository must not leave packages that another
/// repository's jobs then install.
fn dep_cache_dirs_in(base: &Path, repo: &Path) -> DepCacheMounts {
    let repo_dir = base.join(repo_cache_id(repo));
    let created = |dir: &Path| {
        DEP_CACHE_DIRS
            .iter()
            .filter_map(|(name, in_home)| {
                let host = dir.join(name);
                std::fs::create_dir_all(&host).ok()?;
                Some((host, *in_home))
            })
            .collect::<Vec<_>>()
    };
    DepCacheMounts {
        runner: created(&repo_dir),
        root: created(&repo_dir.join(ROOT_DEP_CACHE_DIR)),
    }
}

/// What to do with a job before running it.
pub(crate) enum Decision {
    /// Run it. A `Some` key means a pass can be stored under it.
    Run { key: Option<String> },
    /// Don't run it: this result stands.
    Reuse { jobs: Vec<StoredJob>, why: Reused },
    /// Don't run it, and report it skipped, as a false `if:` would.
    Skip { reason: String },
}

/// Why a job's earlier result stands in for running it.
pub(crate) enum Reused {
    /// `--affected`: none of its paths changed.
    Unaffected,
    /// A pass is stored under this fingerprint.
    Stored { key: String },
}

impl Reused {
    /// How reports name the reason: `stored` or `unaffected`.
    pub(crate) fn kind(&self) -> &'static str {
        match self {
            Reused::Unaffected => "unaffected",
            Reused::Stored { .. } => "stored",
        }
    }

    /// The one step reports show for the job.
    pub(crate) fn step(&self) -> &'static str {
        match self {
            Reused::Unaffected => "Not affected",
            Reused::Stored { .. } => "Reuse result",
        }
    }

    pub(crate) fn note(&self) -> String {
        match self {
            Reused::Unaffected => {
                "Not affected: no changed file is among this job's paths".to_string()
            }
            Reused::Stored { key } => format!("Reused the passing result {}", &key[..12]),
        }
    }
}

pub(crate) struct Lookup<'a> {
    pub job_name: &'a str,
    pub job: &'a Job,
    pub workflow: &'a WorkflowDefinition,
    pub env_context: &'a HashMap<String, String>,
    pub platform_overrides: &'a HashMap<String, String>,
    pub needs_outputs: &'a HashMap<String, HashMap<String, String>>,
    pub needs_results: &'a HashMap<String, String>,
    /// A partial run proves nothing about the job, so it can't reuse or store.
    pub partial: bool,
}

/// The `--affected` verdict: true when `hints` names the job's inputs, no
/// changed file is among them, nothing it consumes just ran, and everything
/// it needs passed.
fn unaffected(
    job_name: &str,
    job: &Job,
    hints: Option<&JobHints>,
    changed: &[String],
    executed: &HashSet<String>,
    needs_results: &HashMap<String, String>,
) -> Result<bool, String> {
    // A job with outputs has to run: a dependent needs the values.
    if job.outputs.as_ref().is_some_and(|o| !o.is_empty()) || job.matrix_config().is_some() {
        return Ok(false);
    }
    // A job that runs after a failure, such as one with `if: always()`, is
    // in a state the base never proved, so it can't stand in as a pass.
    let needs_passed = job
        .needs
        .iter()
        .flatten()
        .all(|need| needs_results.get(need).map(String::as_str) == Some("success"));
    if !needs_passed {
        return Ok(false);
    }
    let Some(patterns) = input_patterns(job, hints)? else {
        return Ok(false);
    };
    let needs_ran = job
        .needs
        .iter()
        .flatten()
        .any(|need| executed.contains(need));
    let touched = changed.iter().any(|path| matches_any(path, &patterns));
    let workflows_changed = changed
        .iter()
        .any(|path| matches_any(path, &compile_globs(NEVER_INPUT)));
    wrkflw_logging::debug(&format!(
        "--affected '{}': touched={} needs_ran={} workflows_changed={}",
        job_name, touched, needs_ran, workflows_changed
    ));
    Ok(!touched && !needs_ran && !workflows_changed)
}

/// Decide whether `lookup`'s job has to run.
pub(crate) async fn decide(lookup: Lookup<'_>) -> Decision {
    let run_it = Decision::Run { key: None };
    let Ok(run) = RUN.try_with(Option::clone) else {
        // Every run sets the scope, even if to `None`, so this is a job that
        // left the run's task and lost reuse, sessions and services with it.
        wrkflw_logging::warning(&format!(
            "Job '{}' lost its run state: acceleration doesn't apply to it",
            lookup.job_name
        ));
        return run_it;
    };
    let Some(run) = run else {
        return run_it;
    };
    let hints = run.hints(lookup.job_name);
    if run.accel.gate && hints.is_some_and(|h| !h.gate) {
        return Decision::Skip {
            reason: format!("gate: false in {}, so it's left to CI", SIDECAR_PATH),
        };
    }
    // A key is needed to reuse a result, and also to publish one: a trusted
    // CI that always runs fresh still has to file what it ran.
    let wants_key = run.accel.reuse_results || run.accel.push_results;
    if !(wants_key || run.accel.changed_files.is_some()) || lookup.partial {
        return run_it;
    }
    if let Some(reason) = uncacheable_reason(lookup.job, lookup.workflow, hints) {
        wrkflw_logging::info(&format!(
            "Job '{}' always runs: {}",
            lookup.job_name, reason
        ));
        if run.accel.gate && hints.is_none_or(|h| h.cache) {
            wrkflw_logging::warning(&format!(
                "Job '{}' runs on every push from here, with this machine's credentials. \
                 If it deploys or publishes, set `gate: false` for it in {}",
                lookup.job_name, SIDECAR_PATH
            ));
        }
        return run_it;
    }
    let patterns = match input_patterns(lookup.job, hints) {
        Ok(patterns) => patterns,
        Err(e) => {
            wrkflw_logging::warning(&format!("{}. Running job '{}'", e, lookup.job_name));
            return run_it;
        }
    };
    if let Some(changed) = &run.accel.changed_files {
        let executed = run
            .executed
            .lock()
            .map(|set| set.clone())
            .unwrap_or_default();
        if matches!(
            unaffected(
                lookup.job_name,
                lookup.job,
                hints,
                changed,
                &executed,
                lookup.needs_results
            ),
            Ok(true)
        ) {
            return Decision::Reuse {
                jobs: vec![StoredJob {
                    name: lookup.job_name.to_string(),
                    canonical_name: lookup.job_name.to_string(),
                    outputs: HashMap::new(),
                    actions_not_run: Vec::new(),
                    repeated_from: None,
                }],
                why: Reused::Unaffected,
            };
        }
    }
    let Some(store) = run.store.as_ref().filter(|_| wants_key) else {
        return run_it;
    };
    let Some(tree) = run.tree().await else {
        return run_it;
    };
    let key = job_key(&KeyInput {
        lookup: &lookup,
        tree: &tree,
        patterns: patterns.as_deref(),
    });
    let Some(key) = key else {
        return run_it;
    };
    if !run.accel.reuse_results {
        return Decision::Run { key: Some(key) };
    }
    if let Some(entry) = store.load(&key).await {
        return Decision::Reuse {
            jobs: entry.jobs,
            why: Reused::Stored { key },
        };
    }
    Decision::Run { key: Some(key) }
}

/// Note that `job_name` really ran.
pub(crate) fn mark_executed(job_name: &str) {
    if let Some(run) = current() {
        if let Ok(mut executed) = run.executed.lock() {
            executed.insert(job_name.to_string());
        }
    }
}

/// Store a passing result under the key `decide` returned. A job that ran on
/// the host (`on_host`), or any leg of it did, passed with that host's
/// tools, so its result stays on this machine.
pub(crate) async fn store(key: &str, jobs: Vec<StoredJob>, on_host: bool) {
    let Some(run) = current() else { return };
    let Some(store) = &run.store else { return };
    if on_host && run.accel.push_results && !run.warned_on_host_push.swap(true, Ordering::Relaxed) {
        wrkflw_logging::warning(
            "Not pushing results from this run: emulation depends on this machine's tools",
        );
    }
    store.save(key, jobs, !on_host).await;
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::TempDir;

    fn job(yaml: &str) -> Job {
        serde_yaml::from_str(yaml).unwrap()
    }

    fn workflow() -> WorkflowDefinition {
        serde_yaml::from_str("name: t\non: push\njobs: {}\n").unwrap()
    }

    fn git(dir: &Path, args: &[&str]) {
        let status = std::process::Command::new("git")
            .arg("-C")
            .arg(dir)
            .args(["-c", "user.name=t", "-c", "user.email=t@example.com"])
            .args(args)
            .stdout(std::process::Stdio::null())
            .stderr(std::process::Stdio::null())
            .status()
            .unwrap();
        assert!(status.success());
    }

    fn repo() -> TempDir {
        let dir = TempDir::new().unwrap();
        git(dir.path(), &["init", "-q"]);
        std::fs::create_dir_all(dir.path().join("src")).unwrap();
        std::fs::create_dir_all(dir.path().join("docs")).unwrap();
        std::fs::write(dir.path().join("src/a.rs"), "one").unwrap();
        std::fs::write(dir.path().join("docs/a.md"), "one").unwrap();
        std::fs::write(dir.path().join(".gitignore"), "ignored\n").unwrap();
        git(dir.path(), &["add", "."]);
        dir
    }

    fn key_for(
        job_def: &Job,
        wf: &WorkflowDefinition,
        tree: &TreeHashes,
        paths: Option<Vec<String>>,
    ) -> String {
        key_for_mode(job_def, wf, tree, paths, false)
    }

    fn key_for_mode(
        job_def: &Job,
        wf: &WorkflowDefinition,
        tree: &TreeHashes,
        paths: Option<Vec<String>>,
        every_matrix_leg: bool,
    ) -> String {
        key_for_user(job_def, wf, tree, paths, every_matrix_leg, false)
    }

    fn key_for_user(
        job_def: &Job,
        wf: &WorkflowDefinition,
        tree: &TreeHashes,
        paths: Option<Vec<String>>,
        every_matrix_leg: bool,
        steps_as_root: bool,
    ) -> String {
        let hints = JobHints {
            paths,
            ..Default::default()
        };
        let patterns = input_patterns(job_def, Some(&hints)).unwrap();
        let empty_env = HashMap::new();
        let empty_outputs = HashMap::new();
        let empty_results = HashMap::new();
        let lookup = Lookup {
            job_name: "build",
            job: job_def,
            workflow: wf,
            env_context: &empty_env,
            platform_overrides: &empty_env,
            needs_outputs: &empty_outputs,
            needs_results: &empty_results,
            partial: false,
        };
        job_key_with(
            &KeyInput {
                lookup: &lookup,
                tree: &Tree::new(tree.clone()).unwrap(),
                patterns: patterns.as_deref(),
            },
            every_matrix_leg,
            steps_as_root,
        )
        .unwrap()
    }

    /// A pass with steps kept as root isn't reused for a run whose steps
    /// run as the runner user, or the other way round.
    #[test]
    fn a_pass_as_root_is_kept_apart_from_one_as_the_runner_user() {
        let dir = repo();
        let wf = workflow();
        let tree = hash_tree(dir.path()).unwrap();
        let plain = job("steps:\n  - run: make\n");
        assert_ne!(
            key_for_user(&plain, &wf, &tree, None, false, false),
            key_for_user(&plain, &wf, &tree, None, false, true)
        );
    }

    /// A matrix whose macOS legs run on this Mac isn't reused for a run with
    /// every leg in a container, or the other way round: the hint to rerun
    /// with `--macos-on-host` must actually run them.
    #[tokio::test]
    async fn a_matrix_run_with_legs_on_the_host_is_kept_apart() {
        let dir = repo();
        let wf = workflow();
        let tree = hash_tree(dir.path()).unwrap();
        let matrix = job(
            "runs-on: ${{ matrix.os }}\nstrategy:\n  matrix:\n    os: [ubuntu-latest, macos-15]\nsteps:\n  - run: make\n",
        );
        let in_containers = key_for_mode(&matrix, &wf, &tree, None, false);
        let host = crate::host_jobs::HostJobs::new(
            Box::new(wrkflw_runtime::emulation::EmulationRuntime::new()),
            &HashMap::new(),
            HashMap::new(),
        );
        let on_host = crate::host_jobs::scope(Some(std::sync::Arc::new(host)), async {
            key_for_mode(&matrix, &wf, &tree, None, false)
        })
        .await;
        assert_ne!(in_containers, on_host);
    }

    #[test]
    fn running_every_matrix_leg_does_not_reuse_a_folded_result() {
        let dir = repo();
        let wf = workflow();
        let tree = hash_tree(dir.path()).unwrap();
        let matrix = job(
            "runs-on: ${{ matrix.os }}\nstrategy:\n  matrix:\n    os: [a, b]\nsteps:\n  - run: make\n",
        );
        assert_ne!(
            key_for_mode(&matrix, &wf, &tree, None, false),
            key_for_mode(&matrix, &wf, &tree, None, true)
        );
        // A job without a matrix has nothing to fold, so its key is shared.
        let plain = job("steps:\n  - run: make\n");
        assert_eq!(
            key_for_mode(&plain, &wf, &tree, None, false),
            key_for_mode(&plain, &wf, &tree, None, true)
        );
    }

    #[tokio::test]
    async fn a_pass_with_a_container_per_step_is_kept_apart() {
        let dir = repo();
        let wf = workflow();
        let tree = hash_tree(dir.path()).unwrap();
        let definition = job("steps:\n  - run: make\n");
        let key_in = |accel: Acceleration, preserve_containers: bool| {
            let state =
                RunState::new(accel, preserve_containers, dir.path().to_path_buf()).unwrap();
            let (definition, wf, tree) = (&definition, &wf, &tree);
            with_run_state(Arc::new(state), async move {
                key_for(definition, wf, tree, None)
            })
        };
        let shared = key_in(Acceleration::default(), false).await;
        let per_step = key_in(
            Acceleration {
                step_containers: true,
                ..Default::default()
            },
            false,
        )
        .await;
        let kept = key_in(Acceleration::default(), true).await;
        assert_ne!(shared, per_step);
        // A kept-for-debugging container is one per step too.
        assert_eq!(per_step, kept);
    }

    #[test]
    fn the_key_changes_with_a_source_file_but_not_an_ignored_one() {
        let dir = repo();
        let wf = workflow();
        let definition = job("steps:\n  - run: make\n");
        let before = key_for(&definition, &wf, &hash_tree(dir.path()).unwrap(), None);
        std::fs::write(dir.path().join("ignored"), "noise").unwrap();
        let ignored = key_for(&definition, &wf, &hash_tree(dir.path()).unwrap(), None);
        assert_eq!(before, ignored);
        std::fs::write(dir.path().join("src/a.rs"), "two").unwrap();
        let changed = key_for(&definition, &wf, &hash_tree(dir.path()).unwrap(), None);
        assert_ne!(before, changed);
    }

    #[test]
    fn an_untracked_file_is_an_input() {
        let dir = repo();
        let wf = workflow();
        let definition = job("steps:\n  - run: make\n");
        let before = key_for(&definition, &wf, &hash_tree(dir.path()).unwrap(), None);
        std::fs::write(dir.path().join("src/new.rs"), "new").unwrap();
        let after = key_for(&definition, &wf, &hash_tree(dir.path()).unwrap(), None);
        assert_ne!(before, after);
    }

    #[test]
    fn a_path_hint_narrows_the_inputs() {
        let dir = repo();
        let wf = workflow();
        let definition = job("steps:\n  - run: make\n");
        let hint = || Some(vec!["src/**".to_string()]);
        let before = key_for(&definition, &wf, &hash_tree(dir.path()).unwrap(), hint());
        std::fs::write(dir.path().join("docs/a.md"), "two").unwrap();
        let docs = key_for(&definition, &wf, &hash_tree(dir.path()).unwrap(), hint());
        assert_eq!(before, docs);
        std::fs::write(dir.path().join("src/a.rs"), "two").unwrap();
        let source = key_for(&definition, &wf, &hash_tree(dir.path()).unwrap(), hint());
        assert_ne!(before, source);
    }

    #[test]
    fn a_composite_action_counts_even_with_a_path_hint() {
        let dir = repo();
        std::fs::create_dir_all(dir.path().join(".github/actions/x")).unwrap();
        std::fs::write(dir.path().join(".github/actions/x/action.yml"), "a").unwrap();
        git(dir.path(), &["add", "."]);
        let wf = workflow();
        let definition = job("steps:\n  - run: make\n");
        let hint = || Some(vec!["src/**".to_string()]);
        let before = key_for(&definition, &wf, &hash_tree(dir.path()).unwrap(), hint());
        std::fs::write(dir.path().join(".github/actions/x/action.yml"), "b").unwrap();
        let after = key_for(&definition, &wf, &hash_tree(dir.path()).unwrap(), hint());
        assert_ne!(before, after);
    }

    #[test]
    fn the_key_changes_with_the_job_and_is_stable_across_map_order() {
        let dir = repo();
        let tree = hash_tree(dir.path()).unwrap();
        let wf = workflow();
        let first = job("env: {A: '1', B: '2', C: '3', D: '4'}\nsteps:\n  - run: make\n");
        let second = job("env: {D: '4', C: '3', B: '2', A: '1'}\nsteps:\n  - run: make\n");
        assert_eq!(
            key_for(&first, &wf, &tree, None),
            key_for(&second, &wf, &tree, None)
        );
        let other = job("env: {A: '1', B: '2', C: '3', D: '4'}\nsteps:\n  - run: make test\n");
        assert_ne!(
            key_for(&first, &wf, &tree, None),
            key_for(&other, &wf, &tree, None)
        );
    }

    #[test]
    fn jobs_with_effects_are_not_cacheable() {
        let wf = workflow();
        for (yaml, needle) in [
            ("steps:\n  - run: echo ${{ secrets.TOKEN }}\n", "secrets"),
            ("steps:\n  - run: echo ${{ github.token }}\n", "secrets"),
            (
                "permissions: {contents: write}\nsteps:\n  - run: x\n",
                "write",
            ),
            (
                "steps:\n  - uses: actions/upload-artifact@v4\n",
                "artifacts",
            ),
            (
                "steps:\n  - uses: actions/download-artifact@v4\n",
                "artifacts",
            ),
            (
                "steps:\n  - uses: actions/upload-pages-artifact@v3\n",
                "artifacts",
            ),
            (
                "steps:\n  - uses: actions/upload-artifact/merge@v4\n",
                "artifacts",
            ),
            (
                "steps:\n  - uses: Actions/Upload-Artifact@v4\n",
                "artifacts",
            ),
            ("uses: ./.github/workflows/x.yml\n", "reusable"),
        ] {
            let reason = uncacheable_reason(&job(yaml), &wf, None).expect(yaml);
            assert!(reason.contains(needle), "{}: {}", yaml, reason);
        }
        assert!(uncacheable_reason(&job("steps:\n  - run: x\n"), &wf, None).is_none());
    }

    #[test]
    fn outputs_passed_along_from_a_job_with_secrets_are_not_cacheable() {
        let wf: WorkflowDefinition = serde_yaml::from_str(
            r#"
name: t
on: push
jobs:
  login:
    steps:
      - run: echo ${{ secrets.TOKEN }}
  middle:
    needs: login
    steps:
      - run: x
  relay:
    needs: middle
    outputs: {v: '${{ needs.middle.outputs.v }}'}
    steps:
      - run: x
  plain:
    needs: middle
    steps:
      - run: x
"#,
        )
        .unwrap();
        let reason = uncacheable_reason(&wf.jobs["relay"], &wf, None).unwrap();
        assert!(reason.contains("outputs"), "{}", reason);
        assert!(uncacheable_reason(&wf.jobs["plain"], &wf, None).is_none());
    }

    #[test]
    fn committing_a_tree_keeps_its_key() {
        let dir = repo();
        git(dir.path(), &["commit", "-qm", "base"]);
        let wf = workflow();
        let definition = job("steps:\n  - run: make\n");
        std::fs::write(dir.path().join("src/a.rs"), "edited").unwrap();
        std::fs::write(dir.path().join("src/new.rs"), "new").unwrap();
        let dirty = key_for(&definition, &wf, &hash_tree(dir.path()).unwrap(), None);
        git(dir.path(), &["add", "."]);
        git(dir.path(), &["commit", "-qm", "edit"]);
        let clean = key_for(&definition, &wf, &hash_tree(dir.path()).unwrap(), None);
        assert_eq!(dirty, clean);
    }

    #[test]
    fn the_tree_covers_deletions_and_the_executable_bit() {
        let dir = repo();
        git(dir.path(), &["commit", "-qm", "base"]);
        let before = hash_tree(dir.path()).unwrap();
        std::fs::remove_file(dir.path().join("docs/a.md")).unwrap();
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            let script = dir.path().join("src/a.rs");
            std::fs::set_permissions(&script, std::fs::Permissions::from_mode(0o755)).unwrap();
        }
        let after = hash_tree(dir.path()).unwrap();
        assert_ne!(before["docs/a.md"], after["docs/a.md"]);
        #[cfg(unix)]
        assert_ne!(before["src/a.rs"], after["src/a.rs"]);
    }

    #[tokio::test]
    async fn affected_alone_stores_nothing() {
        let wf = workflow();
        let definition = job("steps:\n  - run: x\n");
        let accel = Acceleration {
            changed_files: Some(vec!["src/a.rs".to_string()]),
            ..Default::default()
        };
        let state = RunState::new(accel, false, PathBuf::from(".")).unwrap();
        let empty = HashMap::new();
        let decision = with_run_state(
            Arc::new(state),
            decide(Lookup {
                job_name: "a",
                job: &definition,
                workflow: &wf,
                env_context: &empty,
                platform_overrides: &empty,
                needs_outputs: &HashMap::new(),
                needs_results: &empty,
                partial: false,
            }),
        )
        .await;
        assert!(matches!(decision, Decision::Run { key: None }));
    }

    /// A run in `dir`, a git work tree, that keeps results in `results`.
    fn state_in(dir: &Path, results: &Path, accel: &Acceleration) -> RunState {
        let mut state = RunState::new(accel.clone(), false, dir.to_path_buf()).unwrap();
        state.store = Some(ResultStore::new(results.to_path_buf(), accel));
        state
    }

    /// `decide` for a one-step job in `dir`.
    async fn decide_in(dir: &Path, results: &Path, accel: &Acceleration) -> Decision {
        let wf = workflow();
        let definition = job("steps:\n  - run: x\n");
        let state = state_in(dir, results, accel);
        let empty = HashMap::new();
        with_run_state(
            Arc::new(state),
            decide(Lookup {
                job_name: "a",
                job: &definition,
                workflow: &wf,
                env_context: &empty,
                platform_overrides: &empty,
                needs_outputs: &HashMap::new(),
                needs_results: &empty,
                partial: false,
            }),
        )
        .await
    }

    #[tokio::test]
    async fn pushing_files_a_result_without_reusing_one() {
        let dir = repo();
        let results = TempDir::new().unwrap();
        // Outside the work tree, whose files are inputs.
        let remote = TempDir::new().unwrap();
        let pushing = Acceleration {
            remote_cache: Some(remote.path().display().to_string()),
            push_results: true,
            signing_key: Some(Secret("k".to_string())),
            ..Default::default()
        };
        let Decision::Run { key: Some(key) } =
            decide_in(dir.path(), results.path(), &pushing).await
        else {
            panic!("a pushing run needs a key to file its result under");
        };
        // A stored pass must not stop a run that didn't ask for reuse.
        let state = state_in(dir.path(), results.path(), &pushing);
        state.store.as_ref().unwrap().save(&key, vec![], true).await;
        assert!(matches!(
            decide_in(dir.path(), results.path(), &pushing).await,
            Decision::Run { key: Some(_) }
        ));
        let reusing = Acceleration {
            reuse_results: true,
            ..pushing
        };
        assert!(matches!(
            decide_in(dir.path(), results.path(), &reusing).await,
            Decision::Reuse { .. }
        ));
    }

    #[test]
    fn secrets_without_spaces_are_still_secrets() {
        let wf = workflow();
        for yaml in [
            "steps:\n  - run: echo ${{secrets}}\n",
            "steps:\n  - run: echo ${{secrets.TOKEN}}\n",
        ] {
            assert!(
                uncacheable_reason(&job(yaml), &wf, None).is_some(),
                "{}",
                yaml
            );
        }
    }

    #[test]
    fn a_run_without_reuse_needs_no_result_store() {
        let state = RunState::new(Acceleration::default(), false, PathBuf::from(".")).unwrap();
        assert!(state.store.is_none());
        let reuse = Acceleration {
            reuse_results: true,
            ..Default::default()
        };
        let state = RunState::new(reuse, false, PathBuf::from(".")).unwrap();
        assert!(state.store.is_some());
    }

    #[test]
    fn a_repository_inside_the_tree_is_hashed_by_its_contents() {
        let dir = repo();
        let nested = dir.path().join("vendor/lib");
        std::fs::create_dir_all(&nested).unwrap();
        git(&nested, &["init", "-q"]);
        std::fs::write(nested.join("lib.rs"), "one").unwrap();
        git(&nested, &["add", "."]);
        git(&nested, &["commit", "-qm", "lib"]);
        let first = hash_tree(dir.path()).unwrap();
        let entry = first
            .keys()
            .find(|path| path.starts_with("vendor/lib"))
            .expect("the nested repository is an input")
            .clone();
        std::fs::write(nested.join("lib.rs"), "two").unwrap();
        let edited = hash_tree(dir.path()).unwrap();
        assert_ne!(first[&entry], edited[&entry]);
        std::fs::write(nested.join("new.rs"), "new").unwrap();
        let added = hash_tree(dir.path()).unwrap();
        assert_ne!(edited[&entry], added[&entry]);
    }

    #[test]
    fn workflow_env_secrets_make_a_job_uncacheable() {
        let wf: WorkflowDefinition =
            serde_yaml::from_str("name: t\non: push\nenv:\n  T: ${{ secrets.TOKEN }}\njobs: {}\n")
                .unwrap();
        assert!(uncacheable_reason(&job("steps:\n  - run: x\n"), &wf, None).is_some());
    }

    #[test]
    fn the_sidecar_can_opt_a_job_out() {
        let hints = JobHints {
            paths: None,
            cache: false,
            ..Default::default()
        };
        let reason = uncacheable_reason(&job("steps:\n  - run: x\n"), &workflow(), Some(&hints));
        assert_eq!(reason, Some("cache: false in the sidecar"));
    }

    #[test]
    fn a_broken_sidecar_is_an_error_and_a_missing_one_is_not() {
        let dir = TempDir::new().unwrap();
        assert_eq!(Sidecar::load(dir.path()).unwrap(), Sidecar::default());
        std::fs::create_dir_all(dir.path().join(".wrkflw")).unwrap();
        std::fs::write(
            dir.path().join(SIDECAR_PATH),
            "jobs:\n  a:\n    pathz: []\n",
        )
        .unwrap();
        assert!(Sidecar::load(dir.path()).is_err());
    }

    #[test]
    fn a_wildcard_without_a_slash_is_flagged_as_root_only() {
        let sidecar: Sidecar = serde_yaml::from_str(
            "jobs:\n  a: {paths: ['*.rs', 'src/**', 'Cargo.lock', '**/*.md']}\n",
        )
        .unwrap();
        assert_eq!(sidecar.root_only_paths(), [("a", "*.rs")]);
    }

    #[test]
    fn unaffected_requires_hints_and_no_touched_path() {
        let definition = job("steps:\n  - run: x\n");
        let hints = JobHints {
            paths: Some(vec!["src/**".to_string()]),
            cache: true,
            ..Default::default()
        };
        let none = HashSet::new();
        let docs = ["docs/a.md".to_string()];
        let source = ["src/deep/a.rs".to_string()];
        assert!(unaffected(
            "a",
            &definition,
            Some(&hints),
            &docs,
            &none,
            &HashMap::new()
        )
        .unwrap());
        assert!(!unaffected(
            "a",
            &definition,
            Some(&hints),
            &source,
            &none,
            &HashMap::new()
        )
        .unwrap());
        assert!(!unaffected("a", &definition, None, &docs, &none, &HashMap::new()).unwrap());
    }

    #[test]
    fn a_workflow_change_affects_every_job() {
        let definition = job("steps:\n  - run: x\n");
        let hints = JobHints {
            paths: Some(vec!["src/**".to_string()]),
            cache: true,
            ..Default::default()
        };
        let changed = [".github/workflows/ci.yml".to_string()];
        assert!(!unaffected(
            "a",
            &definition,
            Some(&hints),
            &changed,
            &HashSet::new(),
            &HashMap::new()
        )
        .unwrap());
    }

    #[test]
    fn an_upstream_run_or_declared_outputs_keep_a_job_affected() {
        let hints = JobHints {
            paths: Some(vec!["src/**".to_string()]),
            cache: true,
            ..Default::default()
        };
        let docs = ["docs/a.md".to_string()];
        let dependent = job("needs: build\nsteps:\n  - run: x\n");
        let ran = HashSet::from(["build".to_string()]);
        assert!(!unaffected("a", &dependent, Some(&hints), &docs, &ran, &passed()).unwrap());
        let with_outputs = job("outputs: {v: '1'}\nsteps:\n  - run: x\n");
        assert!(!unaffected(
            "a",
            &with_outputs,
            Some(&hints),
            &docs,
            &HashSet::new(),
            &HashMap::new()
        )
        .unwrap());
    }

    #[test]
    fn workflow_permissions_count_unless_the_job_sets_its_own() {
        let with = |permissions: &str| -> WorkflowDefinition {
            serde_yaml::from_str(&format!(
                "name: t\non: push\npermissions: {}\njobs: {{}}\n",
                permissions
            ))
            .unwrap()
        };
        let plain = job("steps:\n  - run: x\n");
        for writes in ["{contents: write}", "write-all"] {
            let reason = uncacheable_reason(&plain, &with(writes), None);
            assert_eq!(reason, Some("it has write permissions"), "{}", writes);
        }
        for reads in ["{contents: read}", "read-all", "{}"] {
            assert!(
                uncacheable_reason(&plain, &with(reads), None).is_none(),
                "{}",
                reads
            );
        }
        let reads_only = job("permissions: {contents: read}\nsteps:\n  - run: x\n");
        assert!(uncacheable_reason(&reads_only, &with("write-all"), None).is_none());
    }

    #[test]
    fn a_local_action_counts_even_with_a_path_hint() {
        let dir = repo();
        std::fs::create_dir_all(dir.path().join("tools/act")).unwrap();
        std::fs::write(dir.path().join("tools/act/action.yml"), "a").unwrap();
        git(dir.path(), &["add", "."]);
        let wf = workflow();
        let definition = job("steps:\n  - uses: ./tools/act/\n");
        let hint = || Some(vec!["src/**".to_string()]);
        let before = key_for(&definition, &wf, &hash_tree(dir.path()).unwrap(), hint());
        std::fs::write(dir.path().join("tools/act/action.yml"), "b").unwrap();
        let after = key_for(&definition, &wf, &hash_tree(dir.path()).unwrap(), hint());
        assert_ne!(before, after);
        let patterns = local_action_patterns(&job("steps:\n  - uses: ./\n"));
        assert!(matches_any("any/file", &patterns));
    }

    fn passed() -> HashMap<String, String> {
        HashMap::from([("build".to_string(), "success".to_string())])
    }

    #[test]
    fn a_need_that_did_not_pass_keeps_a_job_affected() {
        let hints = JobHints {
            paths: Some(vec!["src/**".to_string()]),
            cache: true,
            ..Default::default()
        };
        let docs = ["docs/a.md".to_string()];
        let after_failure = job("needs: build\nif: always()\nsteps:\n  - run: x\n");
        let none = HashSet::new();
        let failed = HashMap::from([("build".to_string(), "failure".to_string())]);
        assert!(!unaffected("a", &after_failure, Some(&hints), &docs, &none, &failed).unwrap());
        let skipped = HashMap::from([("build".to_string(), "skipped".to_string())]);
        assert!(!unaffected("a", &after_failure, Some(&hints), &docs, &none, &skipped).unwrap());
        assert!(unaffected("a", &after_failure, Some(&hints), &docs, &none, &passed()).unwrap());
    }

    #[test]
    fn remote_settings_are_validated() {
        let base = Acceleration {
            remote_cache: Some("https://cache.example.com".to_string()),
            signing_key: Some(Secret("k".to_string())),
            ..Default::default()
        };
        assert!(validate_remote(&base).is_ok());
        let unsigned = Acceleration {
            signing_key: None,
            ..base.clone()
        };
        assert!(validate_remote(&unsigned)
            .unwrap_err()
            .contains("SIGNING_KEY"));
        let plain = Acceleration {
            remote_cache: Some("http://cache.example.com".to_string()),
            ..base.clone()
        };
        assert!(validate_remote(&plain).unwrap_err().contains("https"));
        for local in [
            "http://localhost:9090",
            "http://127.0.0.1/x",
            "http://127.1.2.3",
            "http://[::1]:9090/x",
        ] {
            let accel = Acceleration {
                remote_cache: Some(local.to_string()),
                ..base.clone()
            };
            assert!(validate_remote(&accel).is_ok(), "{}", local);
        }
        for remote in [
            "http://localhost.evil.com",
            "http://localhost@evil.com/",
            "http://[::2]",
            "s3://bucket/results",
            "gs://bucket/results",
            "HTTPS://cache.example.com",
        ] {
            let accel = Acceleration {
                remote_cache: Some(remote.to_string()),
                ..base.clone()
            };
            assert!(validate_remote(&accel).is_err(), "{}", remote);
        }
        let push_only = Acceleration {
            push_results: true,
            ..Default::default()
        };
        assert!(validate_remote(&push_only).is_err());
    }

    #[test]
    fn jobs_sharing_paths_share_one_digest_of_them() {
        let dir = repo();
        let tree = Tree::new(hash_tree(dir.path()).unwrap()).unwrap();
        let src = compile_globs(&["src/**"]);
        let docs = compile_globs(&["docs/**"]);
        let first = tree.digest_matching(&src).unwrap();
        assert_eq!(tree.selected.lock().unwrap().len(), 1);
        assert_eq!(tree.digest_matching(&src).unwrap(), first);
        assert_eq!(tree.selected.lock().unwrap().len(), 1);
        assert_ne!(tree.digest_matching(&docs).unwrap(), first);
    }

    #[test]
    fn actions_that_publish_make_a_job_uncacheable() {
        let wf = workflow();
        for yaml in [
            "steps:\n  - uses: actions/github-script@v7\n    with: {script: 'x'}\n",
            "steps:\n  - uses: Softprops/Action-GH-Release@v2\n",
            "steps:\n  - uses: peter-evans/create-pull-request@v6\n",
            "steps:\n  - uses: x/y@v1\n    with: {github-token: abc}\n",
        ] {
            assert!(
                uncacheable_reason(&job(yaml), &wf, None).is_some(),
                "{}",
                yaml
            );
        }
        for yaml in [
            "steps:\n  - uses: actions/checkout@v4\n",
            "steps:\n  - uses: actions/github-scripts-helper@v1\n",
        ] {
            assert!(
                uncacheable_reason(&job(yaml), &wf, None).is_none(),
                "{}",
                yaml
            );
        }
    }

    #[tokio::test]
    async fn the_gate_skips_a_job_the_sidecar_leaves_to_ci() {
        let dir = repo();
        std::fs::create_dir_all(dir.path().join(".wrkflw")).unwrap();
        std::fs::write(dir.path().join(SIDECAR_PATH), "jobs:\n  a: {gate: false}\n").unwrap();
        let results = TempDir::new().unwrap();
        let gate = Acceleration {
            reuse_results: true,
            gate: true,
            ..Default::default()
        };
        assert!(matches!(
            decide_in(dir.path(), results.path(), &gate).await,
            Decision::Skip { .. }
        ));
        let plain = Acceleration {
            gate: false,
            ..gate
        };
        assert!(matches!(
            decide_in(dir.path(), results.path(), &plain).await,
            Decision::Run { .. }
        ));
    }

    #[test]
    fn a_file_url_is_a_valid_remote() {
        let accel = Acceleration {
            remote_cache: Some("file:///srv/wrkflw".to_string()),
            signing_key: Some(Secret("k".to_string())),
            ..Default::default()
        };
        assert!(validate_remote(&accel).is_ok());
    }

    #[test]
    fn each_repository_gets_its_own_dependency_caches() {
        let base = TempDir::new().unwrap();
        let first = TempDir::new().unwrap();
        let second = TempDir::new().unwrap();
        let hosts = |repo: &Path| -> Vec<PathBuf> {
            let mounts = dep_cache_dirs_in(base.path(), repo);
            let mut hosts: Vec<PathBuf> = mounts
                .runner
                .iter()
                .chain(&mounts.root)
                .map(|(host, _)| host.clone())
                .collect();
            hosts.dedup();
            hosts
        };
        let mine = hosts(first.path());
        // One set for steps that may run as the runner user, one for root.
        assert_eq!(mine.len(), 2 * DEP_CACHE_DIRS.len());
        let mounts = dep_cache_dirs_in(base.path(), first.path());
        let cargo = |who: DepCacheFor| -> Vec<(bool, PathBuf)> {
            mounts
                .for_steps(who)
                .into_iter()
                .filter(|(host, _)| host.ends_with("cargo-registry"))
                .map(|(host, container)| (mounts.runner.iter().any(|(h, _)| *h == host), container))
                .collect()
        };
        let both_homes = |runner_set: bool| {
            vec![
                (runner_set, PathBuf::from("/root/.cargo/registry")),
                (runner_set, PathBuf::from("/home/runner/.cargo/registry")),
            ]
        };
        // A step that may run as the runner user gets that user's set, where
        // root keeps it and where the runner user does: it runs as one or
        // the other.
        let runner = DepCacheFor {
            runner_user: true,
            runner_home: true,
        };
        assert_eq!(cargo(runner), both_homes(true));
        // A step that runs as root in the image wrkflw builds gets root's own
        // set, from host directories no runner-user step writes, in both
        // homes too: `CARGO_HOME` is the runner user's there, for root too.
        let root_in_built_image = DepCacheFor {
            runner_user: false,
            runner_home: true,
        };
        assert_eq!(cargo(root_in_built_image), both_homes(false));
        // Any other image gets root's set in root's home only.
        let root_elsewhere = DepCacheFor {
            runner_user: false,
            runner_home: false,
        };
        let roots = mounts.for_steps(root_elsewhere);
        assert_eq!(roots.len(), DEP_CACHE_DIRS.len());
        assert!(
            roots.iter().all(|(_, c)| c.starts_with("/root")),
            "{roots:?}"
        );
        assert!(
            roots
                .iter()
                .all(|(host, _)| mounts.runner.iter().all(|(theirs, _)| theirs != host)),
            "{mounts:?}"
        );
        assert!(mine.iter().all(|dir| dir.is_dir()));
        let theirs = hosts(second.path());
        assert!(mine.iter().all(|dir| !theirs.contains(dir)));
        // The same repository, by another spelling, keeps its caches.
        assert_eq!(hosts(&first.path().join(".")), mine);
    }

    #[test]
    fn the_signing_key_stays_out_of_debug_output() {
        let accel = Acceleration {
            signing_key: Some(Secret("hunter2".to_string())),
            ..Default::default()
        };
        assert!(!format!("{:?}", accel).contains("hunter2"));
    }
}
