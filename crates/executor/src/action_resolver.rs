use once_cell::sync::Lazy;
use std::collections::{HashMap, VecDeque};
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

/// Fetch and parse `action.yml` (or `action.yaml`) from a remote GitHub repository.
///
/// `sub_path` is the optional path within the repo (e.g., for `owner/repo/path@ref`,
/// `sub_path` is `Some("path")`). When present, the action metadata is fetched from
/// `{repo}/{version}/{sub_path}/action.yml` instead of `{repo}/{version}/action.yml`.
///
/// Returns `Ok(ResolvedAction)` on success, or `Err` if the action metadata cannot be
/// fetched or parsed. Callers should fall back to hardcoded image mappings on error.
pub async fn resolve_remote_action(
    repo: &str,
    version: &str,
    sub_path: Option<&str>,
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

    let token = std::env::var("GITHUB_TOKEN").ok();

    // Try action.yml first, then action.yaml
    let result = match fetch_and_parse(
        GITHUB_RAW_BASE_URL,
        repo,
        version,
        sub_path,
        "action.yml",
        token.as_deref(),
    )
    .await
    {
        Ok(resolved) => Ok(resolved),
        Err(yml_err) => fetch_and_parse(
            GITHUB_RAW_BASE_URL,
            repo,
            version,
            sub_path,
            "action.yaml",
            token.as_deref(),
        )
        .await
        .map_err(|yaml_err| {
            format!(
                "Neither action.yml ({}) nor action.yaml ({}) could be resolved",
                yml_err, yaml_err
            )
        }),
    };

    // Only cache successful resolutions — transient failures should be retryable
    if let Ok(ref resolved) = result {
        let mut cache = ACTION_CACHE.write().await;
        cache.insert(cache_key, resolved.clone());
    }

    result
}

async fn fetch_and_parse(
    base_url: &str,
    repo: &str,
    version: &str,
    sub_path: Option<&str>,
    filename: &str,
    token: Option<&str>,
) -> Result<ResolvedAction, String> {
    let url = match sub_path {
        Some(p) => format!("{}/{}/{}/{}/{}", base_url, repo, version, p, filename),
        None => format!("{}/{}/{}/{}", base_url, repo, version, filename),
    };

    // Try unauthenticated first; only send GITHUB_TOKEN on 404 (private repos).
    let response = HTTP_CLIENT
        .get(&url)
        .send()
        .await
        .map_err(|e| format!("Failed to fetch {}: {}", url, e))?;

    let response =
        if response.status() == reqwest::StatusCode::NOT_FOUND {
            // Retry with auth if token is available — the repo may be private.
            // NO_REDIRECT_CLIENT prevents leaking the token to a non-GitHub host.
            if let Some(token) = token {
                let auth_response = NO_REDIRECT_CLIENT
                    .get(&url)
                    .header("Authorization", format!("token {}", token))
                    .send()
                    .await
                    .map_err(|e| format!("Failed to fetch {}: {}", url, e))?;

                // The no-redirect policy prevents token leakage, but the server may
                // legitimately redirect (CDN routing). If we get a 3xx, follow it
                // without the auth header to avoid leaking the token.
                if auth_response.status().is_redirection() {
                    if let Some(location) = auth_response.headers().get(reqwest::header::LOCATION) {
                        let redirect_url = location
                            .to_str()
                            .map_err(|_| "Invalid redirect URL encoding".to_string())?;
                        HTTP_CLIENT.get(redirect_url).send().await.map_err(|e| {
                            format!("Failed to follow redirect {}: {}", redirect_url, e)
                        })?
                    } else {
                        return Err(format!(
                            "HTTP {} (redirect with no Location header) fetching {}",
                            auth_response.status(),
                            url
                        ));
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

    if !response.status().is_success() {
        return Err(format!("HTTP {} fetching {}", response.status(), url));
    }

    let body = response
        .text()
        .await
        .map_err(|e| format!("Failed to read response body: {}", e))?;

    parse_action_definition(&body)
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

    /// Tests for `fetch_and_parse` HTTP behavior using wiremock.
    ///
    /// Token is passed as a parameter to `fetch_and_parse`, so no env mutation is needed.
    mod fetch_tests {
        use super::super::*;
        use wiremock::matchers::{header_exists, method, path};
        use wiremock::{Mock, MockServer, ResponseTemplate};

        const ACTION_YML_BODY: &str =
            "name: Test Action\nruns:\n  using: 'node20'\n  main: 'index.js'\n";

        #[tokio::test]
        async fn fetch_success_parses_action_yml() {
            let server = MockServer::start().await;

            Mock::given(method("GET"))
                .and(path("/owner/repo/v1/action.yml"))
                .respond_with(ResponseTemplate::new(200).set_body_string(ACTION_YML_BODY))
                .mount(&server)
                .await;

            let result =
                fetch_and_parse(&server.uri(), "owner/repo", "v1", None, "action.yml", None).await;

            let resolved = result.unwrap();
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

            let result = fetch_and_parse(
                &server.uri(),
                "owner/repo",
                "v1",
                Some("my/action"),
                "action.yml",
                None,
            )
            .await;

            let resolved = result.unwrap();
            assert!(matches!(
                resolved.action_type,
                ActionType::Node { version: 20 }
            ));
        }

        #[tokio::test]
        async fn fetch_404_without_token_returns_error() {
            let server = MockServer::start().await;

            Mock::given(method("GET"))
                .and(path("/owner/repo/v1/action.yml"))
                .respond_with(ResponseTemplate::new(404))
                .mount(&server)
                .await;

            let result =
                fetch_and_parse(&server.uri(), "owner/repo", "v1", None, "action.yml", None).await;

            assert!(result.is_err());
            assert!(
                result.as_ref().unwrap_err().contains("404"),
                "Expected 404 in error, got: {}",
                result.unwrap_err()
            );
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

            let result = fetch_and_parse(
                &server.uri(),
                "owner/repo",
                "v1",
                None,
                "action.yml",
                Some("ghp_test_token_for_redirect_test"),
            )
            .await;

            // The resolution should succeed via the redirect path
            let resolved = result.unwrap();
            assert!(matches!(
                resolved.action_type,
                ActionType::Node { version: 20 }
            ));

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

            let result = fetch_and_parse(
                &server.uri(),
                "owner/repo",
                "v1",
                None,
                "action.yml",
                Some("ghp_test_token_for_auth_test"),
            )
            .await;

            let resolved = result.unwrap();
            assert!(matches!(
                resolved.action_type,
                ActionType::Node { version: 20 }
            ));
        }
    }
}
