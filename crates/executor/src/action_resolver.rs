use once_cell::sync::Lazy;
use std::collections::{HashMap, VecDeque};
use std::ffi::OsString;
use std::path::{Path, PathBuf};
use tokio::sync::RwLock;

/// Maximum number of entries in the action resolution cache.
const MAX_CACHE_ENTRIES: usize = 256;

/// Represents the type of a GitHub Action as declared in its action.yml `runs.using` field.
#[derive(Debug, Clone)]
pub enum ActionType {
    Node {
        version: u32,
    },
    /// A Docker action that references a registry image (e.g., `rust:latest`).
    Docker {
        image: String,
    },
    /// A Docker action that bundles its own Dockerfile and needs to be built.
    DockerBuild,
    Composite,
}

/// Result of resolving a remote action's action.yml.
#[derive(Debug, Clone)]
pub struct ResolvedAction {
    pub action_type: ActionType,
    /// The raw parsed action.yml, available for composite action execution.
    pub definition: Option<serde_yaml::Value>,
}

/// Bounded FIFO cache for successfully resolved actions keyed by "owner/repo@version".
/// Only successful resolutions are cached — transient failures are not persisted
/// so that retries can succeed if network conditions improve.
/// Eviction is insertion-order (FIFO), not access-order, which is sufficient here
/// because actions are typically resolved once per workflow run.
struct BoundedCache {
    map: HashMap<String, ResolvedAction>,
    /// Insertion order for FIFO eviction (oldest at front).
    order: VecDeque<String>,
}

impl BoundedCache {
    fn new() -> Self {
        Self {
            map: HashMap::new(),
            order: VecDeque::new(),
        }
    }

    fn get(&self, key: &str) -> Option<&ResolvedAction> {
        self.map.get(key)
    }

    #[allow(clippy::map_entry)]
    fn insert(&mut self, key: String, value: ResolvedAction) {
        if self.map.contains_key(&key) {
            // Already cached — update value, don't change insertion order
            self.map.insert(key, value);
            return;
        }
        // Evict oldest entries if at capacity
        while self.map.len() >= MAX_CACHE_ENTRIES {
            if let Some(oldest) = self.order.pop_front() {
                self.map.remove(&oldest);
            }
        }
        self.order.push_back(key.clone());
        self.map.insert(key, value);
    }
}

static ACTION_CACHE: Lazy<RwLock<BoundedCache>> = Lazy::new(|| RwLock::new(BoundedCache::new()));

/// How long to wait for an action's metadata. Results are cached, so this is
/// paid once per action. The fallback ignores the action's inputs, so a slow
/// network is better waited for than given up on: some take 7s or more just
/// to connect to GitHub.
const FETCH_TIMEOUT: std::time::Duration = std::time::Duration::from_secs(20);

/// Shared HTTP client to avoid repeated TLS initialization.
static HTTP_CLIENT: Lazy<reqwest::Client> = Lazy::new(|| {
    reqwest::Client::builder()
        .timeout(FETCH_TIMEOUT)
        .user_agent("wrkflw")
        .build()
        .expect("Failed to create HTTP client")
});

/// Shared no-redirect HTTP client for authenticated requests.
/// Prevents leaking the GITHUB_TOKEN to redirect targets (e.g., CDN hosts).
/// Reused across requests to avoid per-request TLS initialization.
static NO_REDIRECT_CLIENT: Lazy<reqwest::Client> = Lazy::new(|| {
    reqwest::Client::builder()
        .timeout(FETCH_TIMEOUT)
        .user_agent("wrkflw")
        .redirect(reqwest::redirect::Policy::none())
        .build()
        .expect("Failed to create no-redirect HTTP client")
});

const GITHUB_RAW_BASE_URL: &str = "https://raw.githubusercontent.com";

/// Disables the on-disk action metadata cache when set to anything but `0`
/// or the empty string.
const NO_ACTION_CACHE_ENV: &str = "WRKFLW_NO_ACTION_CACHE";

/// Overrides the directory the on-disk action metadata cache lives in.
const ACTION_CACHE_DIR_ENV: &str = "WRKFLW_ACTION_CACHE_DIR";

/// Fetch and parse `action.yml` (or `action.yaml`) from a remote GitHub repository.
///
/// `sub_path` is the optional path within the repo (e.g., for `owner/repo/path@ref`,
/// `sub_path` is `Some("path")`). When present, the action metadata is fetched from
/// `{repo}/{version}/{sub_path}/action.yml` instead of `{repo}/{version}/action.yml`.
///
/// `action.yaml` is only tried when the server answered `404` for `action.yml`.
/// A timeout, connection error or any other HTTP error is returned as is, so a
/// slow network costs one timeout per action rather than two.
///
/// Successful resolutions are kept in memory for the life of the process. When
/// `version` is a full 40-hex commit SHA — and therefore immutable — the raw
/// action YAML is also persisted to disk and read back on later runs before
/// anything is fetched. Floating refs such as `v4` or `main` are never written
/// to disk because they move. The disk cache lives in
/// `$WRKFLW_ACTION_CACHE_DIR`, or `<cache dir>/wrkflw/action-metadata/` where
/// `<cache dir>` is `$XDG_CACHE_HOME` (or `~/.cache`) on Linux and
/// `~/Library/Caches` on macOS. Set `WRKFLW_NO_ACTION_CACHE=1` to disable it.
/// A missing, unreadable or corrupt cache file is ignored and the action is
/// fetched again; the disk cache never fails a resolution.
///
/// Returns `Ok(ResolvedAction)` on success, or `Err` if the action metadata cannot be
/// fetched or parsed. Callers should fall back to hardcoded image mappings on error.
pub async fn resolve_remote_action(
    repo: &str,
    version: &str,
    sub_path: Option<&str>,
) -> Result<ResolvedAction, String> {
    let token = std::env::var("GITHUB_TOKEN").ok();
    let disk_cache_dir = action_cache_dir();
    resolve_with(
        GITHUB_RAW_BASE_URL,
        disk_cache_dir.as_deref(),
        repo,
        version,
        sub_path,
        token.as_deref(),
    )
    .await
}

/// [`resolve_remote_action`] with its environment made explicit, so tests can
/// point it at a local server and a temporary cache directory.
async fn resolve_with(
    base_url: &str,
    disk_cache_dir: Option<&Path>,
    repo: &str,
    version: &str,
    sub_path: Option<&str>,
    token: Option<&str>,
) -> Result<ResolvedAction, String> {
    let cache_key = match sub_path {
        Some(p) => format!("{}/{}@{}", repo, p, version),
        None => format!("{}@{}", repo, version),
    };

    // Check cache first (read lock — allows concurrent reads)
    {
        let cache = ACTION_CACHE.read().await;
        if let Some(cached) = cache.get(&cache_key) {
            return Ok(cached.clone());
        }
    }

    // Only immutable refs are persisted: a SHA names one action.yml forever.
    let disk_cache_file = disk_cache_dir
        .filter(|_| is_full_sha(version))
        .map(|dir| dir.join(disk_cache_file_name(repo, sub_path, version)));

    let result = match disk_cache_file
        .as_deref()
        .and_then(read_disk_cache)
        .and_then(|body| parse_action_definition(&body).ok())
    {
        Some(resolved) => Ok(resolved),
        None => {
            let fetched = fetch_action_yaml(base_url, repo, version, sub_path, token).await;
            if let (Ok(body), Some(path)) = (&fetched, disk_cache_file.as_deref()) {
                write_disk_cache(path, body);
            }
            fetched.and_then(|body| parse_action_definition(&body))
        }
    };

    // Only cache successful resolutions — transient failures should be retryable
    if let Ok(ref resolved) = result {
        let mut cache = ACTION_CACHE.write().await;
        cache.insert(cache_key, resolved.clone());
    }

    result
}

/// Fetch the raw text of `action.yml`, falling back to `action.yaml` only when
/// the server says `action.yml` does not exist.
async fn fetch_action_yaml(
    base_url: &str,
    repo: &str,
    version: &str,
    sub_path: Option<&str>,
    token: Option<&str>,
) -> Result<String, String> {
    match fetch_action_file(base_url, repo, version, sub_path, "action.yml", token).await {
        Ok(body) => Ok(body),
        Err(yml_err) if should_try_action_yaml(&yml_err) => {
            fetch_action_file(base_url, repo, version, sub_path, "action.yaml", token)
                .await
                .map_err(|yaml_err| {
                    format!(
                        "Neither action.yml ({}) nor action.yaml ({}) could be resolved",
                        yml_err, yaml_err
                    )
                })
        }
        Err(yml_err) => Err(yml_err.to_string()),
    }
}

/// Why fetching one action metadata file failed.
#[derive(Debug, Clone, PartialEq, Eq)]
enum FetchError {
    /// The server answered `404`: the file does not exist at that path.
    NotFound(String),
    /// Anything else: timeout, connection error, non-404 HTTP status, unreadable body.
    Other(String),
}

impl std::fmt::Display for FetchError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            FetchError::NotFound(msg) | FetchError::Other(msg) => f.write_str(msg),
        }
    }
}

/// Whether a failure on `action.yml` justifies asking for `action.yaml`.
///
/// Only a `404` does: the repository may simply use the other spelling. After
/// a timeout or connection error the second request would most likely fail the
/// same way and double the wait; after a `500` or `403` the spelling is not the
/// problem.
fn should_try_action_yaml(err: &FetchError) -> bool {
    matches!(err, FetchError::NotFound(_))
}

/// Fetch the raw text of one action metadata file.
async fn fetch_action_file(
    base_url: &str,
    repo: &str,
    version: &str,
    sub_path: Option<&str>,
    filename: &str,
    token: Option<&str>,
) -> Result<String, FetchError> {
    let url = match sub_path {
        Some(p) => format!("{}/{}/{}/{}/{}", base_url, repo, version, p, filename),
        None => format!("{}/{}/{}/{}", base_url, repo, version, filename),
    };

    // Try unauthenticated first; only send GITHUB_TOKEN on 404 (private repos).
    let response = HTTP_CLIENT
        .get(&url)
        .send()
        .await
        .map_err(|e| FetchError::Other(format!("Failed to fetch {}: {}", url, e)))?;

    let response = if response.status() == reqwest::StatusCode::NOT_FOUND {
        // Retry with auth if token is available — the repo may be private.
        // NO_REDIRECT_CLIENT prevents leaking the token to a non-GitHub host.
        if let Some(token) = token {
            let auth_response = NO_REDIRECT_CLIENT
                .get(&url)
                .header("Authorization", format!("token {}", token))
                .send()
                .await
                .map_err(|e| FetchError::Other(format!("Failed to fetch {}: {}", url, e)))?;

            // The no-redirect policy prevents token leakage, but the server may
            // legitimately redirect (CDN routing). If we get a 3xx, follow it
            // without the auth header to avoid leaking the token.
            if auth_response.status().is_redirection() {
                if let Some(location) = auth_response.headers().get(reqwest::header::LOCATION) {
                    let redirect_url = location.to_str().map_err(|_| {
                        FetchError::Other("Invalid redirect URL encoding".to_string())
                    })?;
                    HTTP_CLIENT.get(redirect_url).send().await.map_err(|e| {
                        FetchError::Other(format!(
                            "Failed to follow redirect {}: {}",
                            redirect_url, e
                        ))
                    })?
                } else {
                    return Err(FetchError::Other(format!(
                        "HTTP {} (redirect with no Location header) fetching {}",
                        auth_response.status(),
                        url
                    )));
                }
            } else {
                auth_response
            }
        } else {
            response
        }
    } else {
        response
    };

    let status = response.status();
    if status == reqwest::StatusCode::NOT_FOUND {
        return Err(FetchError::NotFound(format!(
            "HTTP {} fetching {}",
            status, url
        )));
    }
    if !status.is_success() {
        return Err(FetchError::Other(format!(
            "HTTP {} fetching {}",
            status, url
        )));
    }

    response
        .text()
        .await
        .map_err(|e| FetchError::Other(format!("Failed to read response body: {}", e)))
}

/// Whether `version` is a full 40-hex commit SHA, the only kind of ref whose
/// `action.yml` can never change.
fn is_full_sha(version: &str) -> bool {
    version.len() == 40 && version.bytes().all(|b| b.is_ascii_hexdigit())
}

/// The directory the on-disk action metadata cache lives in, or `None` when
/// it is disabled or no cache directory can be found.
fn action_cache_dir() -> Option<PathBuf> {
    action_cache_dir_from(
        std::env::var_os(NO_ACTION_CACHE_ENV),
        std::env::var_os(ACTION_CACHE_DIR_ENV),
        dirs::cache_dir(),
    )
}

/// [`action_cache_dir`] on explicit inputs: the value of `WRKFLW_NO_ACTION_CACHE`,
/// the value of `WRKFLW_ACTION_CACHE_DIR`, and the platform cache directory.
fn action_cache_dir_from(
    no_cache: Option<OsString>,
    override_dir: Option<OsString>,
    platform_cache_dir: Option<PathBuf>,
) -> Option<PathBuf> {
    if no_cache.is_some_and(|v| !v.is_empty() && v != "0") {
        return None;
    }
    if let Some(dir) = override_dir.filter(|d| !d.is_empty()) {
        return Some(PathBuf::from(dir));
    }
    platform_cache_dir.map(|d| d.join("wrkflw").join("action-metadata"))
}

/// The cache file name for one action at one commit.
///
/// A readable, sanitized prefix is kept for debugging; uniqueness comes from the
/// SHA-256 of the three parts, so no input can escape the cache directory or
/// collide with another action.
fn disk_cache_file_name(repo: &str, sub_path: Option<&str>, version: &str) -> String {
    use sha2::{Digest, Sha256};

    let mut hasher = Sha256::new();
    hasher.update(repo.as_bytes());
    hasher.update(b"\n");
    hasher.update(sub_path.unwrap_or("").as_bytes());
    hasher.update(b"\n");
    hasher.update(version.as_bytes());
    let digest: String = hasher
        .finalize()
        .iter()
        .map(|b| format!("{:02x}", b))
        .collect();

    let readable: String = repo
        .chars()
        .chain(
            sub_path
                .into_iter()
                .flat_map(|p| "/".chars().chain(p.chars())),
        )
        .map(|c| {
            if c.is_ascii_alphanumeric() || c == '-' {
                c
            } else {
                '_'
            }
        })
        .take(64)
        .collect();

    format!("{}-{}.yml", readable, digest)
}

/// The cached action YAML at `path`, or `None` if there is none or it cannot be read.
fn read_disk_cache(path: &Path) -> Option<String> {
    std::fs::read_to_string(path)
        .ok()
        .filter(|body| !body.trim().is_empty())
}

/// Persist `body` at `path`. Written to a temporary file in the same directory
/// and renamed into place, so a concurrent reader never sees a partial file.
/// Failures are ignored: the cache is an optimization, not a requirement.
fn write_disk_cache(path: &Path, body: &str) {
    let Some(dir) = path.parent() else {
        return;
    };
    if std::fs::create_dir_all(dir).is_err() {
        return;
    }
    let Ok(mut file) = tempfile::NamedTempFile::new_in(dir) else {
        return;
    };
    if std::io::Write::write_all(&mut file, body.as_bytes()).is_err() {
        return;
    }
    let _ = file.persist(path);
}

/// Parse an action.yml body and extract the action type from the `runs` section.
fn parse_action_definition(content: &str) -> Result<ResolvedAction, String> {
    let def: serde_yaml::Value =
        serde_yaml::from_str(content).map_err(|e| format!("Invalid action YAML: {}", e))?;

    let runs = def
        .get("runs")
        .ok_or_else(|| "action.yml missing 'runs' section".to_string())?;

    let using = runs
        .get("using")
        .and_then(|v| v.as_str())
        .ok_or_else(|| "action.yml missing 'runs.using' field".to_string())?;

    let action_type = parse_using(using, runs)?;

    Ok(ResolvedAction {
        action_type,
        definition: Some(def),
    })
}

/// Map the `runs.using` value to an `ActionType`.
fn parse_using(using: &str, runs: &serde_yaml::Value) -> Result<ActionType, String> {
    match using {
        "composite" => Ok(ActionType::Composite),

        "docker" => {
            let image = runs
                .get("image")
                .and_then(|v| v.as_str())
                .ok_or_else(|| "Docker action missing 'runs.image' field".to_string())?;

            // Strip "docker://" prefix if present (some actions use it, some don't)
            let image = image.trim_start_matches("docker://");

            // If the image is "Dockerfile" or a relative path, it means the action
            // bundles its own Dockerfile that needs to be built — not pulled from a registry.
            if image == "Dockerfile"
                || image.starts_with("./")
                || image.starts_with("../")
                || image.ends_with("/Dockerfile")
            {
                Ok(ActionType::DockerBuild)
            } else {
                Ok(ActionType::Docker {
                    image: image.to_string(),
                })
            }
        }

        s if s.starts_with("node") => {
            let version_str = s.trim_start_matches("node");
            let version: u32 = version_str.parse().map_err(|_| {
                format!(
                    "Invalid node version in runs.using '{}': expected 'node<N>' (e.g., 'node20')",
                    s
                )
            })?;
            Ok(ActionType::Node { version })
        }

        other => Err(format!("Unknown runs.using value: {}", other)),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_parse_node_action() {
        let yaml = r#"
name: 'My Action'
runs:
  using: 'node20'
  main: 'index.js'
"#;
        let resolved = parse_action_definition(yaml).unwrap();
        match resolved.action_type {
            ActionType::Node { version } => assert_eq!(version, 20),
            other => panic!("Expected Node action, got {:?}", other),
        }
    }

    #[test]
    fn test_parse_docker_action() {
        let yaml = r#"
name: 'Docker Action'
runs:
  using: 'docker'
  image: 'docker://rust:latest'
"#;
        let resolved = parse_action_definition(yaml).unwrap();
        match &resolved.action_type {
            ActionType::Docker { image } => assert_eq!(image, "rust:latest"),
            other => panic!("Expected Docker action, got {:?}", other),
        }
    }

    #[test]
    fn test_parse_docker_action_with_dockerfile() {
        let yaml = r#"
name: 'Docker Action'
runs:
  using: 'docker'
  image: 'Dockerfile'
"#;
        let resolved = parse_action_definition(yaml).unwrap();
        assert!(
            matches!(resolved.action_type, ActionType::DockerBuild),
            "Expected DockerBuild, got {:?}",
            resolved.action_type
        );
    }

    #[test]
    fn test_parse_docker_action_with_relative_dockerfile() {
        let yaml = r#"
name: 'Docker Action'
runs:
  using: 'docker'
  image: './docker/Dockerfile'
"#;
        let resolved = parse_action_definition(yaml).unwrap();
        assert!(
            matches!(resolved.action_type, ActionType::DockerBuild),
            "Expected DockerBuild, got {:?}",
            resolved.action_type
        );
    }

    #[test]
    fn test_parse_composite_action() {
        let yaml = r#"
name: 'Composite Action'
runs:
  using: 'composite'
  steps:
    - run: echo hello
"#;
        let resolved = parse_action_definition(yaml).unwrap();
        assert!(matches!(resolved.action_type, ActionType::Composite));
    }

    #[test]
    fn test_parse_missing_runs() {
        let yaml = r#"
name: 'Bad Action'
"#;
        assert!(parse_action_definition(yaml).is_err());
    }

    #[test]
    fn test_parse_node16_action() {
        let yaml = r#"
name: 'Legacy Node Action'
runs:
  using: 'node16'
  main: 'index.js'
"#;
        let resolved = parse_action_definition(yaml).unwrap();
        match resolved.action_type {
            ActionType::Node { version } => assert_eq!(version, 16),
            other => panic!("Expected Node 16, got {:?}", other),
        }
    }

    #[test]
    fn test_parse_unknown_using_value() {
        let yaml = r#"
name: 'Unknown Action'
runs:
  using: 'python3'
"#;
        let err = parse_action_definition(yaml).unwrap_err();
        assert!(err.contains("Unknown runs.using value"));
    }

    #[test]
    fn test_parse_missing_using_field() {
        let yaml = r#"
name: 'Bad Action'
runs:
  main: 'index.js'
"#;
        let err = parse_action_definition(yaml).unwrap_err();
        assert!(err.contains("runs.using"));
    }

    #[test]
    fn test_parse_docker_missing_image() {
        let yaml = r#"
name: 'Bad Docker Action'
runs:
  using: 'docker'
"#;
        let err = parse_action_definition(yaml).unwrap_err();
        assert!(err.contains("runs.image"));
    }

    #[test]
    fn test_parse_docker_with_docker_prefix_and_dockerfile() {
        let yaml = r#"
name: 'Docker Action'
runs:
  using: 'docker'
  image: 'docker://Dockerfile'
"#;
        let resolved = parse_action_definition(yaml).unwrap();
        assert!(
            matches!(resolved.action_type, ActionType::DockerBuild),
            "docker://Dockerfile should be DockerBuild, got {:?}",
            resolved.action_type
        );
    }

    #[test]
    fn test_resolved_action_has_definition() {
        let yaml = r#"
name: 'My Action'
description: 'Test'
runs:
  using: 'node20'
  main: 'index.js'
"#;
        let resolved = parse_action_definition(yaml).unwrap();
        let def = resolved.definition.unwrap();
        assert_eq!(def.get("name").unwrap().as_str().unwrap(), "My Action");
    }

    #[test]
    fn test_parse_malformed_node_version_returns_error() {
        let yaml = r#"
name: 'Bad Node Action'
runs:
  using: 'nodefoo'
  main: 'index.js'
"#;
        let err = parse_action_definition(yaml).unwrap_err();
        assert!(
            err.contains("Invalid node version"),
            "Expected error about invalid node version, got: {}",
            err
        );
    }

    #[test]
    fn test_parse_bare_node_returns_error() {
        let yaml = r#"
name: 'Bare Node Action'
runs:
  using: 'node'
  main: 'index.js'
"#;
        let err = parse_action_definition(yaml).unwrap_err();
        assert!(
            err.contains("Invalid node version"),
            "Expected error about invalid node version, got: {}",
            err
        );
    }

    #[tokio::test]
    async fn test_cache_respects_max_capacity() {
        let mut cache = BoundedCache::new();
        // Fill beyond capacity
        for i in 0..MAX_CACHE_ENTRIES + 10 {
            cache.insert(
                format!("owner/repo@v{}", i),
                ResolvedAction {
                    action_type: ActionType::Node { version: 20 },
                    definition: None,
                },
            );
        }
        assert!(
            cache.map.len() <= MAX_CACHE_ENTRIES,
            "Cache size {} exceeds max {}",
            cache.map.len(),
            MAX_CACHE_ENTRIES
        );
        // Oldest entries should have been evicted
        assert!(cache.get("owner/repo@v0").is_none());
        // Newest entries should still be present
        assert!(cache
            .get(&format!("owner/repo@v{}", MAX_CACHE_ENTRIES + 9))
            .is_some());
    }

    #[tokio::test]
    async fn test_cache_duplicate_insert_does_not_grow() {
        let mut cache = BoundedCache::new();
        cache.insert(
            "owner/repo@v1".to_string(),
            ResolvedAction {
                action_type: ActionType::Node { version: 20 },
                definition: None,
            },
        );
        cache.insert(
            "owner/repo@v1".to_string(),
            ResolvedAction {
                action_type: ActionType::Node { version: 16 },
                definition: None,
            },
        );
        assert_eq!(cache.map.len(), 1);
        // Value should be updated
        match &cache.get("owner/repo@v1").unwrap().action_type {
            ActionType::Node { version } => assert_eq!(*version, 16),
            other => panic!("Expected Node, got {:?}", other),
        }
    }

    #[test]
    fn full_sha_is_detected() {
        assert!(is_full_sha("11bd71901bbe5b1630ceea73d27597364c9af683"));
        assert!(is_full_sha("11BD71901BBE5B1630CEEA73D27597364C9AF683"));
        assert!(!is_full_sha("v4"));
        assert!(!is_full_sha("main"));
        assert!(!is_full_sha("11bd719"));
        assert!(!is_full_sha("11bd71901bbe5b1630ceea73d27597364c9af68g"));
        assert!(!is_full_sha("11bd71901bbe5b1630ceea73d27597364c9af6833"));
    }

    #[test]
    fn disk_cache_file_name_is_a_single_safe_component() {
        let name = disk_cache_file_name(
            "actions/checkout",
            Some("../../etc"),
            "11bd71901bbe5b1630ceea73d27597364c9af683",
        );
        assert!(!name.contains('/'), "{name}");
        assert!(!name.contains('\\'), "{name}");
        assert!(!name.contains(".."), "{name}");
        assert!(name.starts_with("actions_checkout_______etc-"), "{name}");
        assert!(name.ends_with(".yml"), "{name}");
        assert!(name.len() < 200, "{name}");
    }

    #[test]
    fn disk_cache_file_name_distinguishes_every_part() {
        let sha_a = "11bd71901bbe5b1630ceea73d27597364c9af683";
        let sha_b = "0000000000000000000000000000000000000000";
        let base = disk_cache_file_name("owner/repo", None, sha_a);
        assert_eq!(base, disk_cache_file_name("owner/repo", None, sha_a));
        assert_ne!(base, disk_cache_file_name("owner/repo", Some("sub"), sha_a));
        assert_ne!(base, disk_cache_file_name("owner/repo", None, sha_b));
        assert_ne!(base, disk_cache_file_name("owner/other", None, sha_a));
        // Sanitization must not make distinct inputs collide.
        assert_ne!(
            disk_cache_file_name("owner/repo", Some("a/b"), sha_a),
            disk_cache_file_name("owner/repo", Some("a_b"), sha_a)
        );
    }

    #[test]
    fn disk_cache_round_trip() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("nested").join("entry.yml");

        assert_eq!(read_disk_cache(&path), None);

        write_disk_cache(&path, "name: x\nruns:\n  using: node20\n");
        assert_eq!(
            read_disk_cache(&path).as_deref(),
            Some("name: x\nruns:\n  using: node20\n")
        );

        // No stray temporary files are left next to the entry.
        let entries: Vec<_> = std::fs::read_dir(path.parent().unwrap())
            .unwrap()
            .map(|e| e.unwrap().file_name())
            .collect();
        assert_eq!(entries, vec![std::ffi::OsString::from("entry.yml")]);
    }

    #[test]
    fn disk_cache_ignores_empty_and_unreadable_files() {
        let dir = tempfile::tempdir().unwrap();
        let empty = dir.path().join("empty.yml");
        std::fs::write(&empty, "   \n").unwrap();
        assert_eq!(read_disk_cache(&empty), None);

        // A directory where a file is expected cannot be read.
        let as_dir = dir.path().join("dir.yml");
        std::fs::create_dir(&as_dir).unwrap();
        assert_eq!(read_disk_cache(&as_dir), None);

        // Writing into an unwritable location does not panic.
        write_disk_cache(&as_dir.join("x").join("y.yml"), "body");
        write_disk_cache(Path::new(""), "body");
    }

    #[test]
    fn action_cache_dir_resolution() {
        let platform = Some(PathBuf::from("/cache"));
        let expected = PathBuf::from("/cache/wrkflw/action-metadata");

        assert_eq!(
            action_cache_dir_from(None, None, platform.clone()),
            Some(expected.clone())
        );
        assert_eq!(
            action_cache_dir_from(Some("".into()), None, platform.clone()),
            Some(expected.clone())
        );
        assert_eq!(
            action_cache_dir_from(Some("0".into()), None, platform.clone()),
            Some(expected)
        );
        assert_eq!(
            action_cache_dir_from(Some("1".into()), None, platform.clone()),
            None
        );
        assert_eq!(
            action_cache_dir_from(Some("true".into()), Some("/x".into()), platform.clone()),
            None
        );
        assert_eq!(
            action_cache_dir_from(None, Some("/custom".into()), platform.clone()),
            Some(PathBuf::from("/custom"))
        );
        assert_eq!(action_cache_dir_from(None, None, None), None);
    }

    #[test]
    fn only_not_found_falls_back_to_action_yaml() {
        assert!(should_try_action_yaml(&FetchError::NotFound(
            "HTTP 404 Not Found fetching x".into()
        )));
        assert!(!should_try_action_yaml(&FetchError::Other(
            "Failed to fetch x: operation timed out".into()
        )));
        assert!(!should_try_action_yaml(&FetchError::Other(
            "HTTP 500 Internal Server Error fetching x".into()
        )));
        assert!(!should_try_action_yaml(&FetchError::Other(
            "HTTP 403 Forbidden fetching x".into()
        )));
    }

    /// Tests for `fetch_action_file` and `resolve_with` HTTP behavior using wiremock.
    ///
    /// Token is passed as a parameter, so no env mutation is needed. Each test
    /// uses its own repository name because the in-memory cache is process-wide.
    mod fetch_tests {
        use super::super::*;
        use wiremock::matchers::{header_exists, method, path};
        use wiremock::{Mock, MockServer, ResponseTemplate};

        const ACTION_YML_BODY: &str =
            "name: Test Action\nruns:\n  using: 'node20'\n  main: 'index.js'\n";
        const COMPOSITE_BODY: &str = "name: Composite\nruns:\n  using: 'composite'\n  steps: []\n";
        const SHA: &str = "11bd71901bbe5b1630ceea73d27597364c9af683";

        async fn requests_to(server: &MockServer, request_path: &str) -> usize {
            server
                .received_requests()
                .await
                .unwrap()
                .iter()
                .filter(|r| r.url.path() == request_path)
                .count()
        }

        #[tokio::test]
        async fn fetch_success_parses_action_yml() {
            let server = MockServer::start().await;

            Mock::given(method("GET"))
                .and(path("/owner/repo/v1/action.yml"))
                .respond_with(ResponseTemplate::new(200).set_body_string(ACTION_YML_BODY))
                .mount(&server)
                .await;

            let body =
                fetch_action_file(&server.uri(), "owner/repo", "v1", None, "action.yml", None)
                    .await
                    .unwrap();

            let resolved = parse_action_definition(&body).unwrap();
            match resolved.action_type {
                ActionType::Node { version } => assert_eq!(version, 20),
                other => panic!("Expected Node action, got {:?}", other),
            }
        }

        #[tokio::test]
        async fn fetch_with_sub_path() {
            let server = MockServer::start().await;

            Mock::given(method("GET"))
                .and(path("/owner/repo/v1/my/action/action.yml"))
                .respond_with(ResponseTemplate::new(200).set_body_string(ACTION_YML_BODY))
                .mount(&server)
                .await;

            let body = fetch_action_file(
                &server.uri(),
                "owner/repo",
                "v1",
                Some("my/action"),
                "action.yml",
                None,
            )
            .await
            .unwrap();

            assert_eq!(body, ACTION_YML_BODY);
        }

        #[tokio::test]
        async fn fetch_404_without_token_is_not_found() {
            let server = MockServer::start().await;

            Mock::given(method("GET"))
                .and(path("/owner/repo/v1/action.yml"))
                .respond_with(ResponseTemplate::new(404))
                .mount(&server)
                .await;

            let err =
                fetch_action_file(&server.uri(), "owner/repo", "v1", None, "action.yml", None)
                    .await
                    .unwrap_err();

            assert!(matches!(err, FetchError::NotFound(_)), "{err:?}");
            assert!(err.to_string().contains("404"), "{err}");
        }

        #[tokio::test]
        async fn fetch_500_is_other() {
            let server = MockServer::start().await;

            Mock::given(method("GET"))
                .and(path("/owner/repo/v1/action.yml"))
                .respond_with(ResponseTemplate::new(500))
                .mount(&server)
                .await;

            let err =
                fetch_action_file(&server.uri(), "owner/repo", "v1", None, "action.yml", None)
                    .await
                    .unwrap_err();

            assert!(matches!(err, FetchError::Other(_)), "{err:?}");
            assert!(err.to_string().contains("500"), "{err}");
        }

        #[tokio::test]
        async fn fetch_connection_error_is_other() {
            // Bind and immediately drop a listener so the port refuses connections.
            let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
            let addr = listener.local_addr().unwrap();
            drop(listener);

            let err = fetch_action_file(
                &format!("http://{}", addr),
                "owner/repo",
                "v1",
                None,
                "action.yml",
                None,
            )
            .await
            .unwrap_err();

            assert!(matches!(err, FetchError::Other(_)), "{err:?}");
            assert!(!should_try_action_yaml(&err));
        }

        #[tokio::test]
        async fn resolve_falls_back_to_action_yaml_on_404() {
            let server = MockServer::start().await;

            Mock::given(method("GET"))
                .and(path("/fallback/yaml/v1/action.yml"))
                .respond_with(ResponseTemplate::new(404))
                .mount(&server)
                .await;
            Mock::given(method("GET"))
                .and(path("/fallback/yaml/v1/action.yaml"))
                .respond_with(ResponseTemplate::new(200).set_body_string(COMPOSITE_BODY))
                .mount(&server)
                .await;

            let resolved = resolve_with(&server.uri(), None, "fallback/yaml", "v1", None, None)
                .await
                .unwrap();

            assert!(matches!(resolved.action_type, ActionType::Composite));
            assert_eq!(
                requests_to(&server, "/fallback/yaml/v1/action.yml").await,
                1
            );
            assert_eq!(
                requests_to(&server, "/fallback/yaml/v1/action.yaml").await,
                1
            );
        }

        #[tokio::test]
        async fn resolve_does_not_try_action_yaml_after_server_error() {
            let server = MockServer::start().await;

            Mock::given(method("GET"))
                .and(path("/fallback/error/v1/action.yml"))
                .respond_with(ResponseTemplate::new(500))
                .mount(&server)
                .await;
            Mock::given(method("GET"))
                .and(path("/fallback/error/v1/action.yaml"))
                .respond_with(ResponseTemplate::new(200).set_body_string(COMPOSITE_BODY))
                .mount(&server)
                .await;

            let err = resolve_with(&server.uri(), None, "fallback/error", "v1", None, None)
                .await
                .unwrap_err();

            assert!(err.contains("500"), "{err}");
            assert!(!err.contains("action.yaml"), "{err}");
            assert_eq!(
                requests_to(&server, "/fallback/error/v1/action.yml").await,
                1
            );
            assert_eq!(
                requests_to(&server, "/fallback/error/v1/action.yaml").await,
                0
            );
        }

        #[tokio::test]
        async fn resolve_reports_both_when_neither_spelling_exists() {
            let server = MockServer::start().await;

            Mock::given(method("GET"))
                .respond_with(ResponseTemplate::new(404))
                .mount(&server)
                .await;

            let err = resolve_with(&server.uri(), None, "fallback/none", "v1", None, None)
                .await
                .unwrap_err();

            assert!(err.contains("Neither action.yml"), "{err}");
            assert!(err.contains("action.yaml"), "{err}");
        }

        #[tokio::test]
        async fn resolve_persists_sha_pinned_action_to_disk() {
            let server = MockServer::start().await;
            let dir = tempfile::tempdir().unwrap();

            Mock::given(method("GET"))
                .and(path(format!("/disk/persist/{SHA}/action.yml")))
                .respond_with(ResponseTemplate::new(200).set_body_string(ACTION_YML_BODY))
                .mount(&server)
                .await;

            let resolved = resolve_with(
                &server.uri(),
                Some(dir.path()),
                "disk/persist",
                SHA,
                None,
                None,
            )
            .await
            .unwrap();
            assert!(matches!(
                resolved.action_type,
                ActionType::Node { version: 20 }
            ));

            let file = dir
                .path()
                .join(disk_cache_file_name("disk/persist", None, SHA));
            assert_eq!(
                std::fs::read_to_string(&file).unwrap(),
                ACTION_YML_BODY,
                "raw body is stored verbatim"
            );
        }

        #[tokio::test]
        async fn resolve_does_not_persist_floating_refs() {
            let server = MockServer::start().await;
            let dir = tempfile::tempdir().unwrap();

            Mock::given(method("GET"))
                .and(path("/disk/floating/v4/action.yml"))
                .respond_with(ResponseTemplate::new(200).set_body_string(ACTION_YML_BODY))
                .mount(&server)
                .await;

            resolve_with(
                &server.uri(),
                Some(dir.path()),
                "disk/floating",
                "v4",
                None,
                None,
            )
            .await
            .unwrap();

            assert_eq!(
                std::fs::read_dir(dir.path()).unwrap().count(),
                0,
                "floating refs must not be written to disk"
            );
        }

        #[tokio::test]
        async fn resolve_reads_sha_pinned_action_from_disk_without_fetching() {
            let server = MockServer::start().await;
            let dir = tempfile::tempdir().unwrap();

            // The server would answer, but it must never be asked.
            Mock::given(method("GET"))
                .respond_with(ResponseTemplate::new(200).set_body_string(ACTION_YML_BODY))
                .mount(&server)
                .await;

            let file = dir
                .path()
                .join(disk_cache_file_name("disk/hit", Some("sub"), SHA));
            std::fs::write(&file, COMPOSITE_BODY).unwrap();

            let resolved = resolve_with(
                &server.uri(),
                Some(dir.path()),
                "disk/hit",
                SHA,
                Some("sub"),
                None,
            )
            .await
            .unwrap();

            assert!(matches!(resolved.action_type, ActionType::Composite));
            assert_eq!(server.received_requests().await.unwrap().len(), 0);
        }

        #[tokio::test]
        async fn resolve_ignores_corrupt_disk_cache_and_refetches() {
            let server = MockServer::start().await;
            let dir = tempfile::tempdir().unwrap();

            Mock::given(method("GET"))
                .and(path(format!("/disk/corrupt/{SHA}/action.yml")))
                .respond_with(ResponseTemplate::new(200).set_body_string(ACTION_YML_BODY))
                .mount(&server)
                .await;

            let file = dir
                .path()
                .join(disk_cache_file_name("disk/corrupt", None, SHA));
            std::fs::write(&file, "runs: [not: valid").unwrap();

            let resolved = resolve_with(
                &server.uri(),
                Some(dir.path()),
                "disk/corrupt",
                SHA,
                None,
                None,
            )
            .await
            .unwrap();

            assert!(matches!(
                resolved.action_type,
                ActionType::Node { version: 20 }
            ));
            assert_eq!(
                requests_to(&server, &format!("/disk/corrupt/{SHA}/action.yml")).await,
                1
            );
            assert_eq!(
                std::fs::read_to_string(&file).unwrap(),
                ACTION_YML_BODY,
                "the corrupt entry is replaced by the fetched body"
            );
        }

        #[tokio::test]
        async fn resolve_without_disk_cache_dir_only_fetches() {
            let server = MockServer::start().await;

            Mock::given(method("GET"))
                .and(path(format!("/disk/disabled/{SHA}/action.yml")))
                .respond_with(ResponseTemplate::new(200).set_body_string(ACTION_YML_BODY))
                .mount(&server)
                .await;

            let resolved = resolve_with(&server.uri(), None, "disk/disabled", SHA, None, None)
                .await
                .unwrap();
            assert!(matches!(
                resolved.action_type,
                ActionType::Node { version: 20 }
            ));
        }

        /// Verifies the security-critical property: when the auth request gets a
        /// redirect response (e.g., to a CDN), the redirect is followed WITHOUT
        /// the Authorization header, preventing the GITHUB_TOKEN from leaking
        /// to a non-GitHub host.
        #[tokio::test]
        async fn auth_redirect_does_not_leak_token() {
            let server = MockServer::start().await;

            // 1. Unauthenticated request → 404 (triggers auth retry).
            //    Mounted first so it has lowest priority in wiremock's LIFO matching.
            Mock::given(method("GET"))
                .and(path("/owner/repo/v1/action.yml"))
                .respond_with(ResponseTemplate::new(404))
                .up_to_n_times(1)
                .mount(&server)
                .await;

            // 2. Authenticated retry → 302 redirect to a different path.
            let redirect_url = format!("{}/cdn/redirected/action.yml", server.uri());
            Mock::given(method("GET"))
                .and(path("/owner/repo/v1/action.yml"))
                .and(header_exists("Authorization"))
                .respond_with(
                    ResponseTemplate::new(302).insert_header("Location", redirect_url.as_str()),
                )
                .mount(&server)
                .await;

            // 3. Redirect target → 200 with action.yml body.
            Mock::given(method("GET"))
                .and(path("/cdn/redirected/action.yml"))
                .respond_with(ResponseTemplate::new(200).set_body_string(ACTION_YML_BODY))
                .mount(&server)
                .await;

            let body = fetch_action_file(
                &server.uri(),
                "owner/repo",
                "v1",
                None,
                "action.yml",
                Some("ghp_test_token_for_redirect_test"),
            )
            .await
            .unwrap();

            // The resolution should succeed via the redirect path
            assert_eq!(body, ACTION_YML_BODY);

            // Verify the redirect request did NOT include the Authorization header.
            // This is the core security invariant: tokens must not leak to redirect targets.
            let requests = server.received_requests().await.unwrap();
            let redirect_req = requests
                .iter()
                .find(|r| r.url.path() == "/cdn/redirected/action.yml")
                .expect("Expected a request to the redirect target");
            let has_auth = redirect_req
                .headers
                .iter()
                .any(|(name, _)| name.as_str() == "authorization");
            assert!(
                !has_auth,
                "GITHUB_TOKEN leaked to redirect target! Authorization header found on redirect request."
            );
        }

        #[tokio::test]
        async fn auth_retry_on_404_with_token_succeeds() {
            let server = MockServer::start().await;

            // 1. Unauthenticated → 404
            Mock::given(method("GET"))
                .and(path("/owner/repo/v1/action.yml"))
                .respond_with(ResponseTemplate::new(404))
                .up_to_n_times(1)
                .mount(&server)
                .await;

            // 2. Authenticated → 200 (private repo, no redirect)
            Mock::given(method("GET"))
                .and(path("/owner/repo/v1/action.yml"))
                .and(header_exists("Authorization"))
                .respond_with(ResponseTemplate::new(200).set_body_string(ACTION_YML_BODY))
                .mount(&server)
                .await;

            let body = fetch_action_file(
                &server.uri(),
                "owner/repo",
                "v1",
                None,
                "action.yml",
                Some("ghp_test_token_for_auth_test"),
            )
            .await
            .unwrap();

            assert_eq!(body, ACTION_YML_BODY);
        }

        #[tokio::test]
        async fn auth_retry_404_is_still_not_found() {
            let server = MockServer::start().await;

            Mock::given(method("GET"))
                .and(path("/owner/repo/v1/action.yml"))
                .respond_with(ResponseTemplate::new(404))
                .mount(&server)
                .await;

            let err = fetch_action_file(
                &server.uri(),
                "owner/repo",
                "v1",
                None,
                "action.yml",
                Some("ghp_test_token_for_404_test"),
            )
            .await
            .unwrap_err();

            assert!(matches!(err, FetchError::NotFound(_)), "{err:?}");
            assert_eq!(requests_to(&server, "/owner/repo/v1/action.yml").await, 2);
        }
    }
}
