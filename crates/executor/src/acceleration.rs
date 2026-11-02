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

use hmac::{Hmac, Mac};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use sha2::{Digest, Sha256};
use std::collections::{BTreeMap, HashMap, HashSet};
use std::future::Future;
use std::io::Read;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};
use tokio::sync::{OnceCell, Semaphore};
use wrkflw_parser::workflow::{Job, WorkflowDefinition};

/// The sidecar that holds what a workflow file can't: GitHub rejects unknown
/// keys in workflow YAML, so job hints can't live next to the job.
pub const SIDECAR_PATH: &str = ".wrkflw/affected.yml";

/// Bump when the fingerprint's inputs change, so old entries stop matching.
const KEY_VERSION: u32 = 1;
const ENTRY_SCHEMA: u32 = 1;
/// Images and floating action tags aren't in the key, so a hit must not live
/// forever. A week bounds that drift.
const DEFAULT_TTL_SECS: i64 = 7 * 24 * 60 * 60;
const REMOTE_TIMEOUT: std::time::Duration = std::time::Duration::from_secs(10);

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
    "github.token",
    "github_token",
];

/// A secret that must not reach logs through `Debug`.
#[derive(Clone, Default, PartialEq)]
pub struct SigningKey(pub String);

impl std::fmt::Debug for SigningKey {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("SigningKey([REDACTED])")
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
    pub signing_key: Option<SigningKey>,
    /// Bearer token for an `https://` remote.
    pub remote_token: Option<SigningKey>,
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
    let job_text = serde_json::to_string(job).ok()?.to_lowercase();
    let env_text = serde_json::to_string(&sorted(&workflow.env))
        .ok()?
        .to_lowercase();
    if job.secrets.is_some()
        || SECRET_REFERENCES
            .iter()
            .any(|needle| job_text.contains(needle) || env_text.contains(needle))
    {
        return Some("it reads secrets");
    }
    None
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

/// Hash the files checkout would hand to a job. `None` outside a git work
/// tree, where there's no reliable list of source files.
pub(crate) fn hash_tree(root: &Path) -> Option<TreeHashes> {
    let output = std::process::Command::new("git")
        .arg("-C")
        .arg(root)
        .args([
            "ls-files",
            "-z",
            "--cached",
            "--others",
            "--exclude-standard",
        ])
        .output()
        .ok()?;
    if !output.status.success() {
        return None;
    }
    let excluded = compile_globs(NEVER_INPUT);
    let mut hashes = TreeHashes::new();
    for raw in output.stdout.split(|b| *b == 0).filter(|p| !p.is_empty()) {
        let relative = String::from_utf8_lossy(raw).into_owned();
        if matches_any(&relative, &excluded) {
            continue;
        }
        hashes.insert(relative.clone(), hash_path(&root.join(&relative)));
    }
    Some(hashes)
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

/// Everything that makes a job's result this job's result.
pub(crate) struct KeyInput<'a> {
    pub job_name: &'a str,
    pub job: &'a Job,
    pub workflow: &'a WorkflowDefinition,
    pub env_context: &'a HashMap<String, String>,
    pub platform_overrides: &'a HashMap<String, String>,
    pub needs_outputs: &'a HashMap<String, HashMap<String, String>>,
    pub needs_results: &'a HashMap<String, String>,
    pub tree: &'a TreeHashes,
    pub patterns: Option<&'a [glob::Pattern]>,
}

/// The fingerprint a stored result is filed under. `None` if the job can't be
/// serialized, which leaves it running as normal.
pub(crate) fn job_key(input: &KeyInput<'_>) -> Option<String> {
    let inputs: BTreeMap<&String, &String> = input
        .tree
        .iter()
        .filter(|(path, _)| input.patterns.is_none_or(|p| matches_any(path, p)))
        .collect();
    let env = |name: &str| input.env_context.get(name).cloned();
    let document = serde_json::json!({
        "version": KEY_VERSION,
        "job_name": input.job_name,
        "job": serde_json::to_value(input.job).ok()?,
        "workflow_env": input.workflow.env,
        "workflow_defaults": serde_json::to_value(&input.workflow.defaults).ok()?,
        "platform_overrides": input.platform_overrides,
        "needs_outputs": input.needs_outputs,
        "needs_results": input.needs_results,
        "event": env("GITHUB_EVENT_NAME"),
        "ref": env("GITHUB_REF"),
        "base_ref": env("GITHUB_BASE_REF"),
        "runtime": env("WRKFLW_RUNTIME_MODE"),
        "inputs": inputs,
    });
    let text = serde_json::to_string(&canonical(document)).ok()?;
    Some(hex(&Sha256::digest(text.as_bytes())))
}

/// One job or matrix leg of a stored result.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub(crate) struct StoredJob {
    pub name: String,
    pub canonical_name: String,
    pub outputs: HashMap<String, String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub(crate) struct Entry {
    pub schema: u32,
    /// Repeated from the file name, so a renamed file can't answer another key.
    pub key: String,
    pub created_at: i64,
    pub jobs: Vec<StoredJob>,
}

impl Entry {
    fn is_usable(&self, key: &str, now: i64) -> bool {
        self.schema == ENTRY_SCHEMA
            && self.key == key
            && now.saturating_sub(self.created_at) <= ttl_secs()
    }
}

fn ttl_secs() -> i64 {
    std::env::var("WRKFLW_RESULT_TTL_SECS")
        .ok()
        .and_then(|v| v.parse().ok())
        .unwrap_or(DEFAULT_TTL_SECS)
}

/// The wire form of an entry. The signature covers the exact bytes of `body`.
#[derive(Serialize, Deserialize)]
struct SignedEntry {
    body: String,
    signature: String,
}

type HmacSha256 = Hmac<Sha256>;

fn sign(key: &SigningKey, body: &str) -> Option<String> {
    let mut mac = HmacSha256::new_from_slice(key.0.as_bytes()).ok()?;
    mac.update(body.as_bytes());
    Some(hex(&mac.finalize().into_bytes()))
}

fn unhex(text: &str) -> Option<Vec<u8>> {
    if !text.len().is_multiple_of(2) || !text.is_ascii() {
        return None;
    }
    (0..text.len())
        .step_by(2)
        .map(|i| u8::from_str_radix(&text[i..i + 2], 16).ok())
        .collect()
}

/// The entry in `signed`, if `key` made it.
fn verify(key: &SigningKey, signed: &SignedEntry) -> Option<Entry> {
    let mut mac = HmacSha256::new_from_slice(key.0.as_bytes()).ok()?;
    mac.update(signed.body.as_bytes());
    // `verify_slice` compares in constant time.
    mac.verify_slice(&unhex(&signed.signature)?).ok()?;
    serde_json::from_str(&signed.body).ok()
}

/// Results on this machine, and optionally a shared location.
pub(crate) struct ResultStore {
    root: PathBuf,
    remote: Option<Remote>,
    push: bool,
    key: Option<SigningKey>,
}

enum Remote {
    Dir(PathBuf),
    Http {
        base: String,
        client: reqwest::Client,
        token: Option<SigningKey>,
    },
}

impl Remote {
    fn parse(location: &str, token: Option<SigningKey>) -> Option<Self> {
        if location.starts_with("http://") || location.starts_with("https://") {
            let client = reqwest::Client::builder()
                .timeout(REMOTE_TIMEOUT)
                .build()
                .ok()?;
            return Some(Remote::Http {
                base: location.trim_end_matches('/').to_string(),
                client,
                token,
            });
        }
        let path = location.strip_prefix("file://").unwrap_or(location);
        Some(Remote::Dir(PathBuf::from(path)))
    }

    async fn get(&self, key: &str) -> Option<String> {
        match self {
            Remote::Dir(dir) => tokio::fs::read_to_string(dir.join(entry_file(key)))
                .await
                .ok(),
            Remote::Http {
                base,
                client,
                token,
            } => {
                let mut request = client.get(format!("{}/{}", base, entry_file(key)));
                if let Some(token) = token {
                    request = request.bearer_auth(&token.0);
                }
                let response = request.send().await.ok()?;
                if !response.status().is_success() {
                    return None;
                }
                response.text().await.ok()
            }
        }
    }

    async fn put(&self, key: &str, body: String) -> Result<(), String> {
        match self {
            Remote::Dir(dir) => write_atomically(&dir.join(entry_file(key)), &body),
            Remote::Http {
                base,
                client,
                token,
            } => {
                let mut request = client
                    .put(format!("{}/{}", base, entry_file(key)))
                    .body(body);
                if let Some(token) = token {
                    request = request.bearer_auth(&token.0);
                }
                let response = request.send().await.map_err(|e| e.to_string())?;
                if response.status().is_success() {
                    Ok(())
                } else {
                    Err(format!("HTTP {}", response.status()))
                }
            }
        }
    }
}

fn entry_file(key: &str) -> String {
    format!("{}.json", key)
}

/// A reader never sees half an entry, even with parallel jobs or a crash.
fn write_atomically(path: &Path, body: &str) -> Result<(), String> {
    let parent = path
        .parent()
        .ok_or_else(|| format!("no parent directory for {}", path.display()))?;
    std::fs::create_dir_all(parent).map_err(|e| e.to_string())?;
    let temp = parent.join(format!(".{}.tmp", uuid::Uuid::new_v4()));
    std::fs::write(&temp, body).map_err(|e| e.to_string())?;
    std::fs::rename(&temp, path).map_err(|e| {
        let _ = std::fs::remove_file(&temp);
        e.to_string()
    })
}

impl ResultStore {
    pub(crate) fn new(root: PathBuf, accel: &Acceleration) -> Self {
        Self {
            root,
            remote: accel
                .remote_cache
                .as_deref()
                .and_then(|r| Remote::parse(r, accel.remote_token.clone())),
            push: accel.push_results,
            key: accel.signing_key.clone(),
        }
    }

    /// A usable entry for `key`, trying this machine before the remote.
    pub(crate) async fn load(&self, key: &str) -> Option<Entry> {
        let now = chrono::Utc::now().timestamp();
        let local = tokio::fs::read_to_string(self.root.join(entry_file(key)))
            .await
            .ok()
            .and_then(|text| serde_json::from_str::<Entry>(&text).ok())
            .filter(|entry| entry.is_usable(key, now));
        if local.is_some() {
            return local;
        }
        let text = self.remote.as_ref()?.get(key).await?;
        let signed: SignedEntry = serde_json::from_str(&text).ok()?;
        let entry = verify(self.key.as_ref()?, &signed);
        if entry.is_none() {
            wrkflw_logging::warning(&format!(
                "Ignoring remote result {}: its signature doesn't verify",
                key
            ));
        }
        let entry = entry.filter(|entry| entry.is_usable(key, now))?;
        self.save_local(&entry);
        Some(entry)
    }

    fn save_local(&self, entry: &Entry) {
        let written = serde_json::to_string(entry)
            .map_err(|e| e.to_string())
            .and_then(|body| write_atomically(&self.root.join(entry_file(&entry.key)), &body));
        if let Err(e) = written {
            wrkflw_logging::warning(&format!("Could not store job result: {}", e));
        }
    }

    /// Store a passing result. Failing to do so never fails the job.
    pub(crate) async fn save(&self, key: &str, jobs: Vec<StoredJob>) {
        let entry = Entry {
            schema: ENTRY_SCHEMA,
            key: key.to_string(),
            created_at: chrono::Utc::now().timestamp(),
            jobs,
        };
        self.save_local(&entry);
        if !self.push {
            return;
        }
        let (Some(remote), Some(signing_key)) = (&self.remote, &self.key) else {
            return;
        };
        let Ok(body) = serde_json::to_string(&entry) else {
            return;
        };
        let Some(signature) = sign(signing_key, &body) else {
            return;
        };
        let Ok(signed) = serde_json::to_string(&SignedEntry { body, signature }) else {
            return;
        };
        if let Err(e) = remote.put(key, signed).await {
            wrkflw_logging::warning(&format!("Could not push job result {}: {}", key, e));
        }
    }
}

/// State shared by every job of one run.
pub(crate) struct RunState {
    pub accel: Acceleration,
    pub preserve_containers: bool,
    root: PathBuf,
    sidecar: Option<Sidecar>,
    store: ResultStore,
    tree: OnceCell<Option<Arc<TreeHashes>>>,
    /// Jobs whose steps really ran, so a dependent can't be skipped as
    /// unaffected when something it consumes has just changed.
    executed: Mutex<HashSet<String>>,
    groups: Mutex<HashMap<String, Arc<Semaphore>>>,
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

impl RunState {
    pub(crate) fn new(
        accel: Acceleration,
        preserve_containers: bool,
        root: PathBuf,
    ) -> Result<Self, String> {
        let sidecar = if accel.reuse_results || accel.changed_files.is_some() {
            Some(Sidecar::load(&root)?)
        } else {
            None
        };
        let store_root = results_dir().ok_or("could not determine the cache directory")?;
        Ok(Self {
            store: ResultStore::new(store_root, &accel),
            accel,
            preserve_containers,
            root,
            sidecar,
            tree: OnceCell::new(),
            executed: Mutex::new(HashSet::new()),
            groups: Mutex::new(HashMap::new()),
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
                        "Not reusing job results: this directory isn't a git work tree",
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
    if mounts_are_copies || !current().is_some_and(|run| run.accel.dep_cache) {
        return Vec::new();
    }
    let Some(root) = dirs::cache_dir().map(|d| d.join("wrkflw").join("deps")) else {
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

/// The `concurrency:` group of a job: a string, or a mapping with `group`.
fn concurrency_group(job: &Job) -> Option<String> {
    match job.concurrency.as_ref()? {
        serde_yaml::Value::String(group) => Some(group.clone()),
        serde_yaml::Value::Mapping(map) => {
            if map
                .get("cancel-in-progress")
                .is_some_and(|v| v.as_bool() != Some(false))
            {
                wrkflw_logging::warning(
                    "cancel-in-progress has no effect locally: jobs in a group wait their turn",
                );
            }
            map.get("group")?.as_str().map(str::to_string)
        }
        _ => None,
    }
}

/// Wait until no other job in this run holds `job`'s concurrency group.
///
/// Groups compare as written, so expressions aren't evaluated. Equal text
/// serializes, which is the safe direction. `cancel-in-progress` doesn't
/// apply: a local run has no earlier run of the same group to cancel.
pub(crate) async fn acquire_concurrency(job: &Job) -> Option<tokio::sync::OwnedSemaphorePermit> {
    let group = concurrency_group(job)?;
    let run = current()?;
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
    /// Don't run it: this result stands.
    Reuse { jobs: Vec<StoredJob>, note: String },
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
    let Some(run) = current() else {
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
            };
        }
    }
    let Some(tree) = run.tree().await else {
        return run_it;
    };
    let key = job_key(&KeyInput {
        job_name: lookup.job_name,
        job: lookup.job,
        workflow: lookup.workflow,
        env_context: lookup.env_context,
        platform_overrides: lookup.platform_overrides,
        needs_outputs: lookup.needs_outputs,
        needs_results: lookup.needs_results,
        tree: &tree,
        patterns: patterns.as_deref(),
    });
    let Some(key) = key else {
        return run_it;
    };
    if run.accel.reuse_results {
        if let Some(entry) = run.store.load(&key).await {
            return Decision::Reuse {
                jobs: entry.jobs,
                note: format!("Reused the passing result {}", &key[..12]),
            };
        }
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

/// Store a passing result under the key `decide` returned.
pub(crate) async fn store(key: &str, jobs: Vec<StoredJob>) {
    if let Some(run) = current() {
        run.store.save(key, jobs).await;
    }
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
        job_key(&KeyInput {
            job_name: "build",
            job: job_def,
            workflow: wf,
            env_context: &empty_env,
            platform_overrides: &empty_env,
            needs_outputs: &empty_outputs,
            needs_results: &empty_results,
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

    fn store_with(dir: &Path, accel: &Acceleration) -> ResultStore {
        ResultStore::new(dir.join("local"), accel)
    }

    fn stored() -> Vec<StoredJob> {
        vec![StoredJob {
            name: "build".to_string(),
            canonical_name: "build".to_string(),
            outputs: HashMap::from([("v".to_string(), "1".to_string())]),
        }]
    }

    fn remote_accel(dir: &Path, key: &str) -> Acceleration {
        Acceleration {
            remote_cache: Some(dir.join("remote").display().to_string()),
            push_results: true,
            signing_key: Some(SigningKey(key.to_string())),
            ..Default::default()
        }
    }

    #[tokio::test]
    async fn a_stored_result_comes_back_and_a_copy_under_another_key_does_not() {
        let dir = TempDir::new().unwrap();
        let store = store_with(dir.path(), &Acceleration::default());
        store.save("aaa", stored()).await;
        assert_eq!(store.load("aaa").await.unwrap().jobs, stored());
        std::fs::copy(
            dir.path().join("local/aaa.json"),
            dir.path().join("local/bbb.json"),
        )
        .unwrap();
        assert!(store.load("bbb").await.is_none());
    }

    #[tokio::test]
    async fn an_expired_entry_is_a_miss() {
        let dir = TempDir::new().unwrap();
        let store = store_with(dir.path(), &Acceleration::default());
        store.save("aaa", stored()).await;
        let path = dir.path().join("local/aaa.json");
        let mut entry: Entry =
            serde_json::from_str(&std::fs::read_to_string(&path).unwrap()).unwrap();
        entry.created_at -= DEFAULT_TTL_SECS + 3600;
        std::fs::write(&path, serde_json::to_string(&entry).unwrap()).unwrap();
        assert!(store.load("aaa").await.is_none());
    }

    #[tokio::test]
    async fn a_remote_result_reaches_another_machine_only_with_the_right_key() {
        let dir = TempDir::new().unwrap();
        let writer = store_with(dir.path(), &remote_accel(dir.path(), "right"));
        writer.save("aaa", stored()).await;
        std::fs::remove_dir_all(dir.path().join("local")).unwrap();

        let forged = store_with(dir.path(), &remote_accel(dir.path(), "wrong"));
        assert!(forged.load("aaa").await.is_none());

        let reader = store_with(dir.path(), &remote_accel(dir.path(), "right"));
        assert_eq!(reader.load("aaa").await.unwrap().jobs, stored());
    }

    #[tokio::test]
    async fn a_tampered_remote_body_is_rejected() {
        let dir = TempDir::new().unwrap();
        let store = store_with(dir.path(), &remote_accel(dir.path(), "k"));
        store.save("aaa", stored()).await;
        std::fs::remove_dir_all(dir.path().join("local")).unwrap();
        let path = dir.path().join("remote/aaa.json");
        let text = std::fs::read_to_string(&path).unwrap();
        std::fs::write(&path, text.replace("build", "other")).unwrap();
        assert!(store.load("aaa").await.is_none());
    }

    #[tokio::test]
    async fn an_http_remote_round_trips_with_the_token() {
        use wiremock::matchers::{header, method, path};
        use wiremock::{Mock, MockServer, ResponseTemplate};

        let server = MockServer::start().await;
        let dir = TempDir::new().unwrap();
        let accel = Acceleration {
            remote_cache: Some(server.uri()),
            push_results: true,
            signing_key: Some(SigningKey("k".to_string())),
            remote_token: Some(SigningKey("tok".to_string())),
            ..Default::default()
        };
        Mock::given(method("PUT"))
            .and(path("/aaa.json"))
            .and(header("authorization", "Bearer tok"))
            .respond_with(ResponseTemplate::new(201))
            .expect(1)
            .mount(&server)
            .await;
        store_with(dir.path(), &accel).save("aaa", stored()).await;
        let requests = server.received_requests().await.unwrap();
        let uploaded = String::from_utf8(requests[0].body.clone()).unwrap();

        Mock::given(method("GET"))
            .and(path("/aaa.json"))
            .respond_with(ResponseTemplate::new(200).set_body_string(uploaded))
            .mount(&server)
            .await;
        std::fs::remove_dir_all(dir.path().join("local")).unwrap();
        let reader = store_with(dir.path(), &accel);
        assert_eq!(reader.load("aaa").await.unwrap().jobs, stored());
        // A miss and an outage both mean the job runs.
        assert!(reader.load("bbb").await.is_none());
    }

    #[test]
    fn remote_settings_are_validated() {
        let base = Acceleration {
            remote_cache: Some("https://cache.example.com".to_string()),
            signing_key: Some(SigningKey("k".to_string())),
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
            signing_key: Some(SigningKey("hunter2".to_string())),
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
                let _permit = acquire_concurrency(definition).await;
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
        assert_eq!(
            concurrency_group(&job("concurrency: db\nsteps: []\n")).as_deref(),
            Some("db")
        );
        assert_eq!(
            concurrency_group(&job("concurrency:\n  group: db\nsteps: []\n")).as_deref(),
            Some("db")
        );
        assert_eq!(concurrency_group(&job("steps: []\n")), None);
    }
}
