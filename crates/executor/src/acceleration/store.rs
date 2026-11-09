//! Stored job results: on this machine, and optionally in a shared location
//! whose entries are HMAC-signed.

use super::{hex, Acceleration, Secret};
use hmac::{Hmac, Mac};
use serde::{Deserialize, Serialize};
use sha2::Sha256;
use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};

/// 2: jobs carry the actions they passed without running. Entries from 1
/// can't say, so they are not reused.
const ENTRY_SCHEMA: u32 = 2;
/// Images and floating action tags aren't in the key, so a hit must not live
/// forever. A week bounds that drift.
const DEFAULT_TTL_SECS: i64 = 7 * 24 * 60 * 60;
const REMOTE_TIMEOUT: std::time::Duration = std::time::Duration::from_secs(10);

/// One job or matrix leg of a stored result.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub(crate) struct StoredJob {
    pub name: String,
    pub canonical_name: String,
    pub outputs: HashMap<String, String>,
    /// Steps that passed without running their action. A reused job reports
    /// them again, so reuse never turns a partial pass into a verified one.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub actions_not_run: Vec<crate::action_fidelity::UnexecutedAction>,
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
    parse_ttl(std::env::var("WRKFLW_RESULT_TTL_SECS").ok().as_deref())
}

/// `WRKFLW_RESULT_TTL_SECS` as seconds. Unset, unparsable or negative means
/// the default, saying so for the last two so a typo doesn't go unseen.
fn parse_ttl(value: Option<&str>) -> i64 {
    let Some(value) = value else {
        return DEFAULT_TTL_SECS;
    };
    match value.trim().parse::<i64>() {
        Ok(secs) if secs >= 0 => secs,
        _ => {
            wrkflw_logging::warning(&format!(
                "Ignoring WRKFLW_RESULT_TTL_SECS={}: not a whole number of seconds",
                value
            ));
            DEFAULT_TTL_SECS
        }
    }
}

/// The wire form of an entry. The signature covers the exact bytes of `body`.
#[derive(Serialize, Deserialize)]
struct SignedEntry {
    body: String,
    signature: String,
}

type HmacSha256 = Hmac<Sha256>;

fn sign(key: &Secret, body: &str) -> Option<String> {
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
fn verify(key: &Secret, signed: &SignedEntry) -> Option<Entry> {
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
    key: Option<Secret>,
    /// Set after the remote fails once, so later jobs don't each wait out
    /// its timeout. A run reads it again next time.
    remote_down: AtomicBool,
    /// Set once this run has swept expired entries from `root`.
    pruned: AtomicBool,
}

enum Remote {
    Dir(PathBuf),
    Http {
        base: String,
        client: reqwest::Client,
        token: Option<Secret>,
    },
}

impl Remote {
    fn parse(location: &str, token: Option<Secret>) -> Option<Self> {
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
        // `validate_remote` refuses other schemes. Never read one as a path.
        if location.contains("://") && !location.starts_with("file://") {
            return None;
        }
        let path = location.strip_prefix("file://").unwrap_or(location);
        Some(Remote::Dir(PathBuf::from(path)))
    }

    /// The entry text for `key`. `Ok(None)` is a miss, `Err` means the
    /// remote itself is failing.
    async fn get(&self, key: &str) -> Result<Option<String>, String> {
        match self {
            Remote::Dir(dir) => Ok(tokio::fs::read_to_string(dir.join(entry_file(key)))
                .await
                .ok()),
            Remote::Http {
                base,
                client,
                token,
            } => {
                let mut request = client.get(format!("{}/{}", base, entry_file(key)));
                if let Some(token) = token {
                    request = request.bearer_auth(&token.0);
                }
                let response = request.send().await.map_err(|e| e.to_string())?;
                let status = response.status();
                // Only "not there" is a miss. Anything else, a refused token
                // above all, would otherwise make every job ask and run cold
                // without a word.
                if status == reqwest::StatusCode::NOT_FOUND {
                    return Ok(None);
                }
                if !status.is_success() {
                    return Err(format!("HTTP {}", status));
                }
                response.text().await.map(Some).map_err(|e| e.to_string())
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
            remote_down: AtomicBool::new(false),
            pruned: AtomicBool::new(false),
        }
    }

    /// Remove local entries, and temporary files a crash left, older than the
    /// TTL. Reads already ignore them, so this only bounds the directory,
    /// which would otherwise grow with every run. Once per run, and by
    /// modification time, so no entry is opened.
    fn prune_expired(&self) {
        if self.pruned.swap(true, Ordering::Relaxed) {
            return;
        }
        let Ok(entries) = std::fs::read_dir(&self.root) else {
            return;
        };
        let max_age = std::time::Duration::from_secs(ttl_secs() as u64);
        let now = std::time::SystemTime::now();
        for entry in entries.flatten() {
            let path = entry.path();
            let ours = path
                .file_name()
                .and_then(|name| name.to_str())
                .is_some_and(|name| name.ends_with(".json") || name.ends_with(".tmp"));
            let expired = entry
                .metadata()
                .and_then(|meta| meta.modified())
                .ok()
                .and_then(|modified| now.duration_since(modified).ok())
                .is_some_and(|age| age > max_age);
            if ours && expired {
                let _ = std::fs::remove_file(&path);
            }
        }
    }

    /// The remote, unless it has already failed in this run.
    fn live_remote(&self) -> Option<&Remote> {
        self.remote
            .as_ref()
            .filter(|_| !self.remote_down.load(Ordering::Relaxed))
    }

    /// Stop using the remote for this run, saying so once.
    fn remote_failed(&self, error: &str) {
        if !self.remote_down.swap(true, Ordering::Relaxed) {
            wrkflw_logging::warning(&format!(
                "Remote cache failed ({}): not using it for the rest of this run",
                error
            ));
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
        let text = match self.live_remote()?.get(key).await {
            Ok(text) => text?,
            Err(e) => {
                self.remote_failed(&e);
                return None;
            }
        };
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
        self.prune_expired();
    }

    /// Store a passing result, and upload it if `shareable` and this run
    /// pushes. Failing to do so never fails the job.
    pub(crate) async fn save(&self, key: &str, jobs: Vec<StoredJob>, shareable: bool) {
        let entry = Entry {
            schema: ENTRY_SCHEMA,
            key: key.to_string(),
            created_at: chrono::Utc::now().timestamp(),
            jobs,
        };
        self.save_local(&entry);
        if !self.push || !shareable {
            return;
        }
        let (Some(remote), Some(signing_key)) = (self.live_remote(), &self.key) else {
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
            self.remote_failed(&format!("pushing {}: {}", key, e));
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::TempDir;

    fn store_with(dir: &Path, accel: &Acceleration) -> ResultStore {
        ResultStore::new(dir.join("local"), accel)
    }

    fn stored() -> Vec<StoredJob> {
        vec![StoredJob {
            name: "build".to_string(),
            canonical_name: "build".to_string(),
            outputs: HashMap::from([("v".to_string(), "1".to_string())]),
            actions_not_run: Vec::new(),
        }]
    }

    fn remote_accel(dir: &Path, key: &str) -> Acceleration {
        Acceleration {
            remote_cache: Some(dir.join("remote").display().to_string()),
            push_results: true,
            signing_key: Some(Secret(key.to_string())),
            ..Default::default()
        }
    }

    #[tokio::test]
    async fn a_stored_result_comes_back_and_a_copy_under_another_key_does_not() {
        let dir = TempDir::new().unwrap();
        let store = store_with(dir.path(), &Acceleration::default());
        store.save("aaa", stored(), true).await;
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
        store.save("aaa", stored(), true).await;
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
        writer.save("aaa", stored(), true).await;
        std::fs::remove_dir_all(dir.path().join("local")).unwrap();

        let forged = store_with(dir.path(), &remote_accel(dir.path(), "wrong"));
        assert!(forged.load("aaa").await.is_none());

        let reader = store_with(dir.path(), &remote_accel(dir.path(), "right"));
        assert_eq!(reader.load("aaa").await.unwrap().jobs, stored());
    }

    #[tokio::test]
    async fn a_result_that_is_not_shareable_stays_local() {
        let dir = TempDir::new().unwrap();
        let store = store_with(dir.path(), &remote_accel(dir.path(), "k"));
        store.save("aaa", stored(), false).await;
        assert!(dir.path().join("local/aaa.json").exists());
        assert!(!dir.path().join("remote/aaa.json").exists());
    }

    #[tokio::test]
    async fn a_tampered_remote_body_is_rejected() {
        let dir = TempDir::new().unwrap();
        let store = store_with(dir.path(), &remote_accel(dir.path(), "k"));
        store.save("aaa", stored(), true).await;
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
            signing_key: Some(Secret("k".to_string())),
            remote_token: Some(Secret("tok".to_string())),
            ..Default::default()
        };
        Mock::given(method("PUT"))
            .and(path("/aaa.json"))
            .and(header("authorization", "Bearer tok"))
            .respond_with(ResponseTemplate::new(201))
            .expect(1)
            .mount(&server)
            .await;
        store_with(dir.path(), &accel)
            .save("aaa", stored(), true)
            .await;
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
        assert!(!reader.remote_down.load(Ordering::Relaxed));
    }

    #[tokio::test]
    async fn an_unreachable_remote_is_tried_once_per_run() {
        // A port that was free a moment ago refuses connections.
        let port = std::net::TcpListener::bind("127.0.0.1:0")
            .unwrap()
            .local_addr()
            .unwrap()
            .port();
        let dir = TempDir::new().unwrap();
        let accel = Acceleration {
            remote_cache: Some(format!("http://127.0.0.1:{}", port)),
            signing_key: Some(Secret("k".to_string())),
            ..Default::default()
        };
        let store = store_with(dir.path(), &accel);
        assert!(store.load("aaa").await.is_none());
        assert!(store.remote_down.load(Ordering::Relaxed));
        assert!(store.live_remote().is_none());
    }

    #[tokio::test]
    async fn a_refused_token_stops_the_remote_instead_of_missing_quietly() {
        use wiremock::matchers::method;
        use wiremock::{Mock, MockServer, ResponseTemplate};

        let server = MockServer::start().await;
        Mock::given(method("GET"))
            .respond_with(ResponseTemplate::new(401))
            .mount(&server)
            .await;
        let dir = TempDir::new().unwrap();
        let accel = Acceleration {
            remote_cache: Some(server.uri()),
            signing_key: Some(Secret("k".to_string())),
            remote_token: Some(Secret("stale".to_string())),
            ..Default::default()
        };
        let store = store_with(dir.path(), &accel);
        assert!(store.load("aaa").await.is_none());
        assert!(store.remote_down.load(Ordering::Relaxed));
        assert!(store.load("bbb").await.is_none());
        assert_eq!(server.received_requests().await.unwrap().len(), 1);
    }

    #[tokio::test]
    async fn a_failing_server_stops_the_remote_for_the_run() {
        use wiremock::matchers::method;
        use wiremock::{Mock, MockServer, ResponseTemplate};

        let server = MockServer::start().await;
        Mock::given(method("GET"))
            .respond_with(ResponseTemplate::new(503))
            .mount(&server)
            .await;
        let dir = TempDir::new().unwrap();
        let accel = Acceleration {
            remote_cache: Some(server.uri()),
            signing_key: Some(Secret("k".to_string())),
            ..Default::default()
        };
        let store = store_with(dir.path(), &accel);
        assert!(store.load("aaa").await.is_none());
        assert!(store.remote_down.load(Ordering::Relaxed));
        assert!(store.load("bbb").await.is_none());
        assert_eq!(server.received_requests().await.unwrap().len(), 1);
    }

    #[test]
    fn an_unknown_scheme_is_never_read_as_a_directory() {
        assert!(Remote::parse("s3://bucket/x", None).is_none());
        assert!(matches!(
            Remote::parse("file:///srv/x", None),
            Some(Remote::Dir(path)) if path == Path::new("/srv/x")
        ));
    }

    #[tokio::test]
    async fn storing_a_result_sweeps_out_expired_ones() {
        let dir = TempDir::new().unwrap();
        let local = dir.path().join("local");
        std::fs::create_dir_all(&local).unwrap();
        let old = local.join("old.json");
        let crashed = local.join(".x.tmp");
        let unrelated = local.join("notes.txt");
        for path in [&old, &crashed, &unrelated] {
            std::fs::write(path, "{}").unwrap();
            let long_ago = std::time::SystemTime::now()
                - std::time::Duration::from_secs(DEFAULT_TTL_SECS as u64 + 3600);
            std::fs::File::options()
                .write(true)
                .open(path)
                .unwrap()
                .set_modified(long_ago)
                .unwrap();
        }
        let store = store_with(dir.path(), &Acceleration::default());
        store.save("fresh", stored(), true).await;
        assert!(local.join("fresh.json").exists());
        assert!(!old.exists());
        assert!(!crashed.exists());
        assert!(unrelated.exists());
    }

    #[test]
    fn the_ttl_comes_from_the_environment_or_the_default() {
        assert_eq!(parse_ttl(None), DEFAULT_TTL_SECS);
        assert_eq!(parse_ttl(Some("60")), 60);
        assert_eq!(parse_ttl(Some(" 0 ")), 0);
        assert_eq!(parse_ttl(Some("-5")), DEFAULT_TTL_SECS);
        assert_eq!(parse_ttl(Some("1w")), DEFAULT_TTL_SECS);
    }
}
