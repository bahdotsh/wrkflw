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

mod store;

use serde::Deserialize;
use serde_json::Value;
use sha2::{Digest, Sha256};
use std::collections::{BTreeMap, HashMap, HashSet};
use std::future::Future;
use std::io::{Read, Write};
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex, OnceLock};
use store::ResultStore;
pub(crate) use store::StoredJob;
use tokio::sync::{OnceCell, Semaphore};
use wrkflw_parser::workflow::{Job, WorkflowDefinition};

/// The sidecar that holds what a workflow file can't: GitHub rejects unknown
/// keys in workflow YAML, so job hints can't live next to the job.
pub const SIDECAR_PATH: &str = ".wrkflw/affected.yml";

/// Bump when the fingerprint's inputs change, so old entries stop matching.
const KEY_VERSION: u32 = 1;

/// Host directories mounted into containers under `--dep-cache`.
const DEP_CACHE_DIRS: &[(&str, &str)] = &[
    ("cargo-registry", "/root/.cargo/registry"),
    ("cargo-git", "/root/.cargo/git"),
    ("npm", "/root/.npm"),
    ("pip", "/root/.cache/pip"),
    ("go-mod", "/root/go/pkg/mod"),
];

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
];

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
    /// Run a job's `run:` steps in one container instead of one per step.
    pub job_container: bool,
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
}

/// Why a remote cache setting can't be used.
pub fn validate_remote(accel: &Acceleration) -> Result<(), String> {
    let Some(remote) = &accel.remote_cache else {
        if accel.push_results {
            return Err("--cache-push needs --remote-cache".to_string());
        }
        return Ok(());
    };
    if accel.signing_key.as_ref().is_none_or(|k| k.0.is_empty()) {
        return Err(
            "--remote-cache needs WRKFLW_CACHE_SIGNING_KEY: an unsigned result would let \
             anyone who can write to the cache mark a job as passed"
                .to_string(),
        );
    }
    if let Some(rest) = remote.strip_prefix("http://") {
        let host = rest.split(['/', ':']).next().unwrap_or_default();
        if !matches!(host, "localhost" | "127.0.0.1") {
            return Err("--remote-cache must use https unless the host is local".to_string());
        }
    }
    Ok(())
}

/// One job's entry in `.wrkflw/affected.yml`.
#[derive(Debug, Clone, Default, Deserialize, PartialEq)]
#[serde(deny_unknown_fields)]
pub struct JobHints {
    /// Globs of the files this job reads, relative to the repository root.
    #[serde(default)]
    pub paths: Option<Vec<String>>,
    /// False never reuses this job, for a job with effects wrkflw can't see.
    #[serde(default = "default_true")]
    pub cache: bool,
}

fn default_true() -> bool {
    true
}

#[derive(Debug, Clone, Default, Deserialize, PartialEq)]
#[serde(deny_unknown_fields)]
pub struct Sidecar {
    #[serde(default)]
    pub jobs: HashMap<String, JobHints>,
}

impl Sidecar {
    /// A missing file means no hints. A broken one is an error: silently
    /// ignoring `cache: false` would reuse a job the author ruled out.
    pub fn load(root: &Path) -> Result<Self, String> {
        let path = root.join(SIDECAR_PATH);
        match std::fs::read_to_string(&path) {
            Ok(text) if text.trim().is_empty() => Ok(Self::default()),
            Ok(text) => {
                serde_yaml::from_str(&text).map_err(|e| format!("invalid {}: {}", SIDECAR_PATH, e))
            }
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(Self::default()),
            Err(e) => Err(format!("cannot read {}: {}", SIDECAR_PATH, e)),
        }
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
fn input_patterns(hints: Option<&JobHints>) -> Result<Option<Vec<glob::Pattern>>, String> {
    let Some(paths) = hints.and_then(|h| h.paths.as_ref()) else {
        return Ok(None);
    };
    let mut patterns = compile_globs(ALWAYS_INPUT);
    for path in paths {
        patterns.push(
            glob::Pattern::new(path)
                .map_err(|e| format!("bad path '{}' in {}: {}", path, SIDECAR_PATH, e))?,
        );
    }
    Ok(Some(patterns))
}

/// Why a job's result must not be reused, or `None` if it may be.
///
/// wrkflw can't see what a step does, so this rules out what it can detect:
/// a job that touches credentials probably changes something outside the
/// repository, and a skipped job produces no artifacts for downstream jobs.
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
    if job
        .permissions
        .as_ref()
        .is_some_and(|p| p.values().any(|v| v == "write"))
    {
        return Some("it has write permissions");
    }
    if job.steps.iter().any(|step| {
        step.uses.as_deref().is_some_and(|uses| {
            uses.starts_with("actions/upload-artifact")
                || uses.starts_with("actions/download-artifact")
        })
    }) {
        return Some("it exchanges artifacts");
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

/// `path -> content hash` for every tracked or untracked-but-not-ignored file.
pub(crate) type TreeHashes = BTreeMap<String, String>;

/// Identify the files checkout would hand to a job. `None` outside a git work
/// tree, where there's no reliable list of source files.
///
/// Git already knows the blob of every clean tracked file, so only changed and
/// untracked files are read. Those get the blob id too, so committing a tree
/// that passed doesn't change its fingerprint.
///
/// A submodule with changes, or an untracked repository inside this one, is
/// hashed as a tree of its own. `None` if one of those can't be read either.
pub(crate) fn hash_tree(root: &Path) -> Option<TreeHashes> {
    hash_tree_at(root, 0)
}

/// How deep nested repositories are followed before giving up.
const MAX_NESTED_REPOS: usize = 8;

fn hash_tree_at(root: &Path, depth: usize) -> Option<TreeHashes> {
    if depth > MAX_NESTED_REPOS {
        return None;
    }
    let files = git_paths(
        root,
        &[
            "ls-files",
            "-z",
            "--cached",
            "--others",
            "--exclude-standard",
        ],
    )?;
    let dirty: HashSet<String> = git_paths(root, &["ls-files", "-z", "--modified"])?
        .into_iter()
        .collect();
    let index = index_blobs(root)?;
    let excluded = compile_globs(NEVER_INPUT);

    let mut hashes = TreeHashes::new();
    let mut unknown = Vec::new();
    for relative in files {
        if matches_any(&relative, &excluded) {
            continue;
        }
        match index.get(&relative).filter(|_| !dirty.contains(&relative)) {
            Some(blob) => {
                hashes.insert(relative, blob.clone());
            }
            None => unknown.push(relative),
        }
    }

    let (plain, odd): (Vec<String>, Vec<String>) = unknown
        .into_iter()
        .partition(|relative| is_plain_file(&root.join(relative)) && !relative.contains('\n'));
    let blobs = hash_objects(root, &plain);
    for (position, relative) in plain.into_iter().enumerate() {
        let path = root.join(&relative);
        let value = match blobs.as_ref().map(|ids| &ids[position]) {
            Some(id) => format!("{} {}", file_mode(&path, index.get(&relative)), id),
            None => hash_path(&path),
        };
        hashes.insert(relative, value);
    }
    for relative in odd {
        let path = root.join(&relative);
        let value = if is_real_dir(&path) {
            // A changed submodule or an untracked repository. Git lists it as
            // one path, so its contents have to be hashed from inside it.
            let nested = hash_tree_at(&path, depth + 1)?;
            let text = serde_json::to_string(&nested).ok()?;
            format!("repo {}", hex(&Sha256::digest(text.as_bytes())))
        } else {
            hash_path(&path)
        };
        hashes.insert(relative, value);
    }
    Some(hashes)
}

fn is_real_dir(path: &Path) -> bool {
    std::fs::symlink_metadata(path).is_ok_and(|meta| meta.is_dir())
}

fn git_paths(root: &Path, args: &[&str]) -> Option<Vec<String>> {
    let output = std::process::Command::new("git")
        .arg("-C")
        .arg(root)
        .args(args)
        .output()
        .ok()?;
    if !output.status.success() {
        return None;
    }
    Some(
        output
            .stdout
            .split(|b| *b == 0)
            .filter(|path| !path.is_empty())
            .map(|path| String::from_utf8_lossy(path).into_owned())
            .collect(),
    )
}

/// `path -> "mode blob"` for merged index entries. Unmerged paths are absent,
/// so they get read.
fn index_blobs(root: &Path) -> Option<HashMap<String, String>> {
    let mut blobs = HashMap::new();
    for entry in git_paths(root, &["ls-files", "-z", "--stage"])? {
        let Some((meta, path)) = entry.split_once('\t') else {
            continue;
        };
        let mut fields = meta.split(' ');
        if let (Some(mode), Some(blob), Some("0")) = (fields.next(), fields.next(), fields.next()) {
            blobs.insert(path.to_string(), format!("{} {}", mode, blob));
        }
    }
    Some(blobs)
}

fn is_plain_file(path: &Path) -> bool {
    std::fs::symlink_metadata(path).is_ok_and(|meta| meta.file_type().is_file())
}

/// The mode git would record for the file at `path`. `index_entry` is its
/// `"mode blob"` from the index, if it's tracked.
fn file_mode<'a>(path: &Path, index_entry: Option<&'a String>) -> &'a str {
    #[cfg(unix)]
    {
        let _ = index_entry;
        use std::os::unix::fs::PermissionsExt;
        if std::fs::metadata(path).is_ok_and(|meta| meta.permissions().mode() & 0o111 != 0) {
            return "100755";
        }
        "100644"
    }
    // No executable bit to read here, so a tracked file keeps the mode git
    // has for it, as git itself does with `core.fileMode` off. Otherwise an
    // edited script would never match its committed fingerprint.
    #[cfg(not(unix))]
    {
        let _ = path;
        index_entry
            .and_then(|entry| entry.split(' ').next())
            .filter(|mode| *mode == "100755")
            .unwrap_or("100644")
    }
}

/// The blob ids of `paths`, in order. `None` if git can't read one of them,
/// which leaves the caller hashing the content itself.
fn hash_objects(root: &Path, paths: &[String]) -> Option<Vec<String>> {
    if paths.is_empty() {
        return Some(Vec::new());
    }
    let mut child = std::process::Command::new("git")
        .arg("-C")
        .arg(root)
        .args(["hash-object", "--stdin-paths"])
        .stdin(std::process::Stdio::piped())
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::null())
        .spawn()
        .ok()?;
    let mut stdin = child.stdin.take()?;
    let list = paths.join("\n") + "\n";
    // On a thread, as git answers as it reads and a full pipe would block both.
    let writer = std::thread::spawn(move || stdin.write_all(list.as_bytes()));
    let output = child.wait_with_output().ok()?;
    writer.join().ok()?.ok()?;
    if !output.status.success() {
        return None;
    }
    let ids: Vec<String> = String::from_utf8_lossy(&output.stdout)
        .lines()
        .map(str::to_string)
        .collect();
    (ids.len() == paths.len()).then_some(ids)
}

fn hash_path(path: &Path) -> String {
    let mut hasher = Sha256::new();
    match std::fs::symlink_metadata(path) {
        // A tracked file deleted in the work tree differs from its content.
        Err(_) => hasher.update(b"missing"),
        Ok(meta) if meta.file_type().is_symlink() => {
            hasher.update(b"link:");
            if let Ok(target) = std::fs::read_link(path) {
                hasher.update(target.to_string_lossy().as_bytes());
            }
        }
        Ok(meta) if meta.is_dir() => hasher.update(b"dir"),
        Ok(meta) => {
            #[cfg(unix)]
            {
                use std::os::unix::fs::PermissionsExt;
                // Whether a script is executable changes what a job does.
                hasher.update(if meta.permissions().mode() & 0o111 != 0 {
                    b"x"
                } else {
                    b"-"
                });
            }
            match std::fs::File::open(path) {
                Ok(mut file) => {
                    let mut buffer = [0u8; 64 * 1024];
                    loop {
                        match file.read(&mut buffer) {
                            Ok(0) => break,
                            Ok(n) => hasher.update(&buffer[..n]),
                            Err(_) => {
                                hasher.update(b"unreadable");
                                break;
                            }
                        }
                    }
                }
                Err(_) => hasher.update(b"unreadable"),
            }
        }
    }
    hex(&hasher.finalize())
}

fn hex(bytes: &[u8]) -> String {
    bytes.iter().map(|b| format!("{:02x}", b)).collect()
}

/// Everything that makes a job's result this job's result: the job as
/// `decide` sees it, and the files it reads.
pub(crate) struct KeyInput<'a> {
    pub lookup: &'a Lookup<'a>,
    pub tree: &'a TreeHashes,
    pub patterns: Option<&'a [glob::Pattern]>,
}

/// The fingerprint a stored result is filed under. `None` if the job can't be
/// serialized, which leaves it running as normal.
pub(crate) fn job_key(input: &KeyInput<'_>) -> Option<String> {
    let lookup = input.lookup;
    let inputs: BTreeMap<&String, &String> = input
        .tree
        .iter()
        .filter(|(path, _)| input.patterns.is_none_or(|p| matches_any(path, p)))
        .collect();
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
    tree: OnceCell<Option<Arc<TreeHashes>>>,
    /// Jobs whose steps really ran, so a dependent can't be skipped as
    /// unaffected when something it consumes has just changed.
    executed: Mutex<HashSet<String>>,
    groups: Mutex<HashMap<String, Arc<Semaphore>>>,
    dep_mounts: OnceLock<Vec<(PathBuf, PathBuf)>>,
}

tokio::task_local! {
    /// `None` inside a called workflow, whose inputs aren't in any fingerprint.
    static RUN: Option<Arc<RunState>>;
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
        validate_remote(&accel)?;
        let sidecar = if accel.reuse_results || accel.changed_files.is_some() {
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
        })
    }

    async fn tree(&self) -> Option<Arc<TreeHashes>> {
        self.tree
            .get_or_init(|| async {
                let root = self.root.clone();
                let hashed = tokio::task::spawn_blocking(move || hash_tree(&root))
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

/// Run `future` with this run's state available to the jobs it executes.
pub(crate) async fn with_run_state<F: Future>(state: Arc<RunState>, future: F) -> F::Output {
    RUN.scope(Some(state), future).await
}

/// Run a called workflow's jobs with none of this applied.
pub(crate) async fn without_run_state<F: Future>(future: F) -> F::Output {
    RUN.scope(None, future).await
}

pub(crate) fn current() -> Option<Arc<RunState>> {
    RUN.try_with(|run| run.clone()).ok().flatten()
}

/// Host and container paths of the `--dep-cache` mounts.
pub(crate) fn dep_cache_mounts(mounts_are_copies: bool) -> Vec<(PathBuf, PathBuf)> {
    let Some(run) = current().filter(|run| run.accel.dep_cache && !mounts_are_copies) else {
        return Vec::new();
    };
    run.dep_mounts.get_or_init(create_dep_cache_dirs).clone()
}

/// The host side of the mounts: `WRKFLW_DEP_CACHE_DIR`, or the cache directory.
fn create_dep_cache_dirs() -> Vec<(PathBuf, PathBuf)> {
    let root = std::env::var_os("WRKFLW_DEP_CACHE_DIR")
        .map(PathBuf::from)
        .or_else(|| dirs::cache_dir().map(|d| d.join("wrkflw").join("deps")));
    let Some(root) = root else {
        return Vec::new();
    };
    DEP_CACHE_DIRS
        .iter()
        .filter_map(|(name, container)| {
            let host = root.join(name);
            std::fs::create_dir_all(&host).ok()?;
            Some((host, PathBuf::from(container)))
        })
        .collect()
}

/// A job's `concurrency:`: a group name, or a mapping with `group` and
/// `cancel-in-progress`.
struct Concurrency {
    group: String,
    /// Asked for, though nothing here can honour it.
    cancel_in_progress: bool,
}

fn concurrency_group(job: &Job) -> Option<Concurrency> {
    match job.concurrency.as_ref()? {
        serde_yaml::Value::String(group) => Some(Concurrency {
            group: group.clone(),
            cancel_in_progress: false,
        }),
        serde_yaml::Value::Mapping(map) => Some(Concurrency {
            group: map.get("group")?.as_str()?.to_string(),
            cancel_in_progress: map
                .get("cancel-in-progress")
                .is_some_and(|v| v.as_bool() != Some(false)),
        }),
        _ => None,
    }
}

/// Wait until no other job in this run holds `job`'s concurrency group.
///
/// Groups compare as written, so expressions aren't evaluated. Equal text
/// serializes, which is the safe direction. `cancel-in-progress` doesn't
/// apply: a local run has no earlier run of the same group to cancel.
pub(crate) async fn acquire_concurrency(
    job_name: &str,
    job: &Job,
) -> Option<tokio::sync::OwnedSemaphorePermit> {
    let Concurrency {
        group,
        cancel_in_progress,
    } = concurrency_group(job)?;
    let run = current()?;
    if cancel_in_progress {
        wrkflw_logging::warning(&format!(
            "Job '{}': cancel-in-progress has no effect locally, so it waits its turn in '{}'",
            job_name, group
        ));
    }
    let semaphore = {
        let mut groups = run.groups.lock().ok()?;
        groups
            .entry(group)
            .or_insert_with(|| Arc::new(Semaphore::new(1)))
            .clone()
    };
    semaphore.acquire_owned().await.ok()
}

/// What to do with a job before running it.
pub(crate) enum Decision {
    /// Run it. A `Some` key means a pass can be stored under it.
    Run { key: Option<String> },
    /// Don't run it: this result stands. `step` names what reports show.
    Reuse {
        jobs: Vec<StoredJob>,
        note: String,
        step: &'static str,
    },
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
/// changed file is among them, and nothing it consumes just ran.
fn unaffected(
    job_name: &str,
    job: &Job,
    hints: Option<&JobHints>,
    changed: &[String],
    executed: &HashSet<String>,
) -> Result<bool, String> {
    // A job with outputs has to run: a dependent needs the values.
    if job.outputs.as_ref().is_some_and(|o| !o.is_empty()) || job.matrix_config().is_some() {
        return Ok(false);
    }
    let Some(patterns) = input_patterns(hints)? else {
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
    let wants_reuse = run.accel.reuse_results || run.accel.changed_files.is_some();
    if !wants_reuse || lookup.partial {
        return run_it;
    }
    let hints = run.hints(lookup.job_name);
    if let Some(reason) = uncacheable_reason(lookup.job, lookup.workflow, hints) {
        wrkflw_logging::info(&format!(
            "Job '{}' always runs: {}",
            lookup.job_name, reason
        ));
        return run_it;
    }
    let patterns = match input_patterns(hints) {
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
            unaffected(lookup.job_name, lookup.job, hints, changed, &executed),
            Ok(true)
        ) {
            return Decision::Reuse {
                jobs: vec![StoredJob {
                    name: lookup.job_name.to_string(),
                    canonical_name: lookup.job_name.to_string(),
                    outputs: HashMap::new(),
                }],
                note: "Not affected: no changed file is among this job's paths".to_string(),
                step: "Not affected",
            };
        }
    }
    let Some(store) = run.store.as_ref().filter(|_| run.accel.reuse_results) else {
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
    if let Some(entry) = store.load(&key).await {
        return Decision::Reuse {
            jobs: entry.jobs,
            note: format!("Reused the passing result {}", &key[..12]),
            step: "Reuse result",
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
/// the host passed with that host's tools, so its result stays on this machine.
pub(crate) async fn store(key: &str, jobs: Vec<StoredJob>, env: &HashMap<String, String>) {
    let Some(run) = current() else { return };
    let Some(store) = &run.store else { return };
    let on_host = env
        .get("WRKFLW_RUNTIME_MODE")
        .is_some_and(|mode| mode.contains("emulation"));
    if on_host && run.accel.push_results {
        wrkflw_logging::warning(
            "Not pushing this job's result: emulation depends on this machine's tools",
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
        let hints = JobHints { paths, cache: true };
        let patterns = input_patterns(Some(&hints)).unwrap();
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
        job_key(&KeyInput {
            lookup: &lookup,
            tree,
            patterns: patterns.as_deref(),
        })
        .unwrap()
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
    fn unaffected_requires_hints_and_no_touched_path() {
        let definition = job("steps:\n  - run: x\n");
        let hints = JobHints {
            paths: Some(vec!["src/**".to_string()]),
            cache: true,
        };
        let none = HashSet::new();
        let docs = ["docs/a.md".to_string()];
        let source = ["src/deep/a.rs".to_string()];
        assert!(unaffected("a", &definition, Some(&hints), &docs, &none).unwrap());
        assert!(!unaffected("a", &definition, Some(&hints), &source, &none).unwrap());
        assert!(!unaffected("a", &definition, None, &docs, &none).unwrap());
    }

    #[test]
    fn a_workflow_change_affects_every_job() {
        let definition = job("steps:\n  - run: x\n");
        let hints = JobHints {
            paths: Some(vec!["src/**".to_string()]),
            cache: true,
        };
        let changed = [".github/workflows/ci.yml".to_string()];
        assert!(!unaffected("a", &definition, Some(&hints), &changed, &HashSet::new()).unwrap());
    }

    #[test]
    fn an_upstream_run_or_declared_outputs_keep_a_job_affected() {
        let hints = JobHints {
            paths: Some(vec!["src/**".to_string()]),
            cache: true,
        };
        let docs = ["docs/a.md".to_string()];
        let dependent = job("needs: build\nsteps:\n  - run: x\n");
        let ran = HashSet::from(["build".to_string()]);
        assert!(!unaffected("a", &dependent, Some(&hints), &docs, &ran).unwrap());
        let with_outputs = job("outputs: {v: '1'}\nsteps:\n  - run: x\n");
        assert!(!unaffected("a", &with_outputs, Some(&hints), &docs, &HashSet::new()).unwrap());
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
        let local = Acceleration {
            remote_cache: Some("http://localhost:9090".to_string()),
            ..base
        };
        assert!(validate_remote(&local).is_ok());
        let push_only = Acceleration {
            push_results: true,
            ..Default::default()
        };
        assert!(validate_remote(&push_only).is_err());
    }

    #[test]
    fn the_signing_key_stays_out_of_debug_output() {
        let accel = Acceleration {
            signing_key: Some(Secret("hunter2".to_string())),
            ..Default::default()
        };
        assert!(!format!("{:?}", accel).contains("hunter2"));
    }

    /// The most jobs inside their critical section at once.
    async fn peak_overlap(first: &str, second: &str) -> usize {
        use std::sync::atomic::{AtomicUsize, Ordering};
        let active = AtomicUsize::new(0);
        let peak = AtomicUsize::new(0);
        let definitions = [job(first), job(second)];
        let state = RunState::new(Acceleration::default(), false, PathBuf::from(".")).unwrap();
        with_run_state(Arc::new(state), async {
            futures::future::join_all(definitions.iter().map(|definition| async {
                let _permit = acquire_concurrency("j", definition).await;
                let now = active.fetch_add(1, Ordering::SeqCst) + 1;
                peak.fetch_max(now, Ordering::SeqCst);
                tokio::time::sleep(std::time::Duration::from_millis(20)).await;
                active.fetch_sub(1, Ordering::SeqCst);
            }))
            .await;
        })
        .await;
        peak.load(Ordering::SeqCst)
    }

    #[tokio::test]
    async fn a_group_serializes_its_jobs_and_other_jobs_overlap() {
        let lock = "concurrency: lock\nsteps: []\n";
        let other = "concurrency: other\nsteps: []\n";
        let free = "steps: []\n";
        assert_eq!(peak_overlap(lock, lock).await, 1);
        assert_eq!(peak_overlap(lock, other).await, 2);
        assert_eq!(peak_overlap(free, free).await, 2);
    }

    #[test]
    fn concurrency_groups_come_from_a_string_or_a_mapping() {
        let group =
            |yaml: &str| concurrency_group(&job(yaml)).map(|c| (c.group, c.cancel_in_progress));
        assert_eq!(
            group("concurrency: db\nsteps: []\n"),
            Some(("db".to_string(), false))
        );
        assert_eq!(
            group("concurrency:\n  group: db\n  cancel-in-progress: true\nsteps: []\n"),
            Some(("db".to_string(), true))
        );
        assert_eq!(group("steps: []\n"), None);
    }
}
