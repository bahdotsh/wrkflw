// github crate

use lazy_static::lazy_static;
use regex::Regex;
use reqwest::header;
use serde_json::{self};
use std::collections::HashMap;
use std::fs;
use std::path::Path;
use std::process::Command;
use thiserror::Error;

#[derive(Error, Debug)]
pub enum GithubError {
    #[error("HTTP error: {0}")]
    RequestError(#[from] reqwest::Error),

    #[error("IO error: {0}")]
    IoError(#[from] std::io::Error),

    #[error("Failed to parse Git repository URL: {0}")]
    GitParseError(String),

    #[error("GitHub token not found. Please set GITHUB_TOKEN environment variable")]
    TokenNotFound,

    #[error("API error: {status} - {message}")]
    ApiError { status: u16, message: String },

    #[error("Unexpected API response: {0}")]
    UnexpectedResponse(String),
}

/// Whether a pull request event started `run`.
fn is_pull_request_run(run: &serde_json::Value) -> bool {
    run["event"]
        .as_str()
        .is_some_and(|event| event.starts_with("pull_request"))
}

/// The branch the pull request behind `run` goes into, as the run lists it.
///
/// Only a pull request's run has one. A push lists every open pull request
/// whose head is the pushed commit, a fork's included, and it ran on the
/// branch, not on a merge into that pull request's base.
fn listed_base_branch(run: &serde_json::Value) -> Option<String> {
    if !is_pull_request_run(run) {
        return None;
    }
    run["pull_requests"][0]["base"]["ref"]
        .as_str()
        .map(str::to_string)
}

/// How long one request to GitHub may take, connecting included, before it
/// fails instead of hanging the command.
const REQUEST_TIMEOUT: std::time::Duration = std::time::Duration::from_secs(30);

/// The client every request to GitHub goes through, with a timeout.
fn http_client() -> Result<reqwest::Client, GithubError> {
    Ok(reqwest::Client::builder()
        .timeout(REQUEST_TIMEOUT)
        .connect_timeout(std::time::Duration::from_secs(10))
        .build()?)
}

/// `text` when it is a full commit id: 40 hex digits for SHA-1, 64 for
/// SHA-256. Ids from the API become arguments to git and to a child run, so
/// nothing else passes.
fn commit_id(text: &str) -> Option<String> {
    (matches!(text.len(), 40 | 64) && text.bytes().all(|b| b.is_ascii_hexdigit()))
        .then(|| text.to_ascii_lowercase())
}

/// Information about a GitHub repository
#[derive(Debug, Clone)]
pub struct RepoInfo {
    pub owner: String,
    pub repo: String,
    pub default_branch: String,
}

lazy_static! {
    static ref GITHUB_REPO_REGEX: Regex =
        Regex::new(r"(?:https://github\.com/|git@github\.com:)([^/]+)/([^/.]+)(?:\.git)?")
            .expect("Failed to compile GitHub repo regex - this is a critical error");
}

/// Extract repository information from the current git repository
pub fn get_repo_info() -> Result<RepoInfo, GithubError> {
    let output = Command::new("git")
        .args(["remote", "get-url", "origin"])
        .output()
        .map_err(|e| GithubError::GitParseError(format!("Failed to execute git command: {}", e)))?;

    if !output.status.success() {
        return Err(GithubError::GitParseError(
            "Failed to get git origin URL. Are you in a git repository?".to_string(),
        ));
    }

    let url = String::from_utf8_lossy(&output.stdout).trim().to_string();

    if let Some(captures) = GITHUB_REPO_REGEX.captures(&url) {
        let owner = captures
            .get(1)
            .ok_or_else(|| {
                GithubError::GitParseError("Unable to extract owner from GitHub URL".to_string())
            })?
            .as_str()
            .to_string();

        let repo = captures
            .get(2)
            .ok_or_else(|| {
                GithubError::GitParseError(
                    "Unable to extract repo name from GitHub URL".to_string(),
                )
            })?
            .as_str()
            .to_string();

        // Get the default branch (try remote HEAD first, fall back to current branch)
        let default_branch = Command::new("git")
            .args(["symbolic-ref", "refs/remotes/origin/HEAD"])
            .output()
            .ok()
            .filter(|o| o.status.success())
            .map(|o| {
                let full_ref = String::from_utf8_lossy(&o.stdout).trim().to_string();
                full_ref
                    .strip_prefix("refs/remotes/origin/")
                    .unwrap_or(&full_ref)
                    .to_string()
            })
            .unwrap_or_else(|| {
                // Fall back to current branch
                Command::new("git")
                    .args(["rev-parse", "--abbrev-ref", "HEAD"])
                    .output()
                    .ok()
                    .filter(|o| o.status.success())
                    .map(|o| String::from_utf8_lossy(&o.stdout).trim().to_string())
                    .unwrap_or_else(|| "main".to_string())
            });

        Ok(RepoInfo {
            owner,
            repo,
            default_branch,
        })
    } else {
        Err(GithubError::GitParseError(format!(
            "URL '{}' is not a valid GitHub repository URL",
            url
        )))
    }
}

/// Normalize a user-facing workflow identifier into the path segment
/// GitHub's `workflow_dispatch` endpoint expects as `{workflow_file_name}`
/// in `/repos/{owner}/{repo}/actions/workflows/{workflow_file_name}/dispatches`.
///
/// - Drops any directory prefix: `"release/prod.yml"` → `"prod.yml"`.
/// - Preserves an existing `.yml` or `.yaml` suffix so workflows stored
///   as `.yaml` don't have `.yml` tacked on.
/// - Appends `.yml` when no extension is present so the result is
///   always a valid filename reference.
/// - Returns `None` for inputs with no extractable basename (empty
///   string, bare path separator, trailing slash).
///
/// Used both by [`trigger_workflow`] to build the real dispatch URL
/// and by the TUI's Trigger-tab curl preview via this crate's public
/// API, so the preview and the actual POST land on the same endpoint.
pub fn workflow_dispatch_path_segment(name: &str) -> Option<String> {
    let basename = name.rsplit(['/', '\\']).next()?;
    if basename.is_empty() {
        return None;
    }
    if basename.ends_with(".yml") || basename.ends_with(".yaml") {
        Some(basename.to_string())
    } else {
        Some(format!("{basename}.yml"))
    }
}

/// Get the list of available workflows in the repository
pub async fn list_workflows(_repo_info: &RepoInfo) -> Result<Vec<String>, GithubError> {
    let workflows_dir = Path::new(".github/workflows");

    if !workflows_dir.exists() {
        return Err(GithubError::IoError(std::io::Error::new(
            std::io::ErrorKind::NotFound,
            "Workflows directory not found",
        )));
    }

    let mut workflow_names = Vec::new();

    for entry in fs::read_dir(workflows_dir)? {
        let entry = entry?;
        let path = entry.path();

        if path.is_file()
            && path
                .extension()
                .is_some_and(|ext| ext == "yml" || ext == "yaml")
        {
            if let Some(file_name) = path.file_stem() {
                if let Some(name) = file_name.to_str() {
                    workflow_names.push(name.to_string());
                }
            }
        }
    }

    Ok(workflow_names)
}

/// Trigger a workflow on GitHub
pub async fn trigger_workflow(
    workflow_name: &str,
    branch: Option<&str>,
    inputs: Option<HashMap<String, String>>,
) -> Result<(), GithubError> {
    // Get GitHub token from environment
    let token = std::env::var("GITHUB_TOKEN").map_err(|_| GithubError::TokenNotFound)?;

    // Trim the token to remove any leading or trailing whitespace
    let trimmed_token = token.trim();

    // Convert token to HeaderValue
    let token_header = header::HeaderValue::from_str(&format!("Bearer {}", trimmed_token))
        .map_err(|_| GithubError::GitParseError("Invalid token format".to_string()))?;

    // Get repository information
    let repo_info = get_repo_info()?;
    wrkflw_logging::info(&format!(
        "Repository: {}/{}",
        repo_info.owner, repo_info.repo
    ));

    // Prepare the request payload
    let branch_ref = branch.unwrap_or(&repo_info.default_branch);
    wrkflw_logging::info(&format!("Using branch: {}", branch_ref));

    // Normalize the user-facing identifier into the dispatch URL
    // segment. Handles subdir prefixes (drop) and missing extensions
    // (append `.yml`) so `"ci"`, `"ci.yml"`, `"ci.yaml"`, and
    // `"release/prod.yml"` all produce the same URL shape the REST
    // API expects. The TUI preview goes through the same helper so
    // a copy-pasted curl lands on the same endpoint.
    let workflow_segment = workflow_dispatch_path_segment(workflow_name)
        .ok_or_else(|| GithubError::GitParseError("Invalid workflow name".to_string()))?;

    wrkflw_logging::info(&format!("Using workflow file: {}", workflow_segment));

    // Create simplified payload
    let mut payload = serde_json::json!({
        "ref": branch_ref
    });

    // Add inputs if provided
    if let Some(input_map) = inputs {
        payload["inputs"] = serde_json::json!(input_map);
        wrkflw_logging::info(&format!("With inputs: {:?}", input_map));
    }

    // Send the workflow_dispatch event
    let url = format!(
        "https://api.github.com/repos/{}/{}/actions/workflows/{}/dispatches",
        repo_info.owner, repo_info.repo, workflow_segment
    );

    wrkflw_logging::info(&format!("Triggering workflow at URL: {}", url));

    // Create a reqwest client
    let client = http_client()?;

    // Send the request using reqwest
    let response = client
        .post(&url)
        .header(header::AUTHORIZATION, token_header)
        .header(header::ACCEPT, "application/vnd.github.v3+json")
        .header(header::CONTENT_TYPE, "application/json")
        .header(header::USER_AGENT, "wrkflw-cli")
        .json(&payload)
        .send()
        .await
        .map_err(GithubError::RequestError)?;

    if !response.status().is_success() {
        let status = response.status().as_u16();
        let error_message = response
            .text()
            .await
            .unwrap_or_else(|_| format!("Unknown error (HTTP {})", status));

        // Add more detailed error information
        let error_details = if status == 500 {
            "Internal server error from GitHub. This could be due to:\n\
             1. The workflow file doesn't exist in the repository\n\
             2. The GitHub token doesn't have sufficient permissions\n\
             3. There's an issue with the workflow file itself\n\
             Please check:\n\
             - The workflow file exists at .github/workflows/rust.yml\n\
             - Your GitHub token has the 'workflow' scope\n\
             - The workflow file is valid YAML"
        } else {
            &error_message
        };

        return Err(GithubError::ApiError {
            status,
            message: error_details.to_string(),
        });
    }

    wrkflw_logging::info("Workflow triggered successfully!");
    wrkflw_logging::info(&format!(
        "View runs at: https://github.com/{}/{}/actions/workflows/{}",
        repo_info.owner, repo_info.repo, workflow_segment
    ));

    // Attempt to verify the workflow was actually triggered
    match list_recent_workflow_runs(&repo_info, &workflow_segment, &token).await {
        Ok(runs) => {
            if !runs.is_empty() {
                wrkflw_logging::info("Recent runs of this workflow:");
                for run in runs.iter().take(3) {
                    wrkflw_logging::info(&format!(
                        "- Run #{} ({}): {}",
                        run.get("id").and_then(|id| id.as_u64()).unwrap_or(0),
                        run.get("status")
                            .and_then(|s| s.as_str())
                            .unwrap_or("unknown"),
                        run.get("html_url").and_then(|u| u.as_str()).unwrap_or("")
                    ));
                }
            } else {
                wrkflw_logging::info(
                    "No recent runs found. The workflow might still be initializing.",
                );
                wrkflw_logging::info(&format!(
                    "Check GitHub UI in a few moments: https://github.com/{}/{}/actions",
                    repo_info.owner, repo_info.repo
                ));
            }
        }
        Err(e) => {
            wrkflw_logging::warning(&format!("Could not fetch recent workflow runs: {}", e));
            wrkflw_logging::info(&format!(
                "This doesn't mean the trigger failed - check GitHub UI: https://github.com/{}/{}/actions",
                repo_info.owner, repo_info.repo
            ));
        }
    }

    Ok(())
}

/// List recent workflow runs for a specific workflow. `workflow_segment`
/// must already be the basename-form produced by
/// [`workflow_dispatch_path_segment`] (e.g. `"ci.yml"`), not the raw
/// user-facing identifier — the caller has already normalized it.
async fn list_recent_workflow_runs(
    repo_info: &RepoInfo,
    workflow_segment: &str,
    token: &str,
) -> Result<Vec<serde_json::Value>, GithubError> {
    // Get recent workflow runs via GitHub API
    let url = format!(
        "https://api.github.com/repos/{}/{}/actions/workflows/{}/runs?per_page=5",
        repo_info.owner, repo_info.repo, workflow_segment
    );

    let client = http_client()?;
    let response = client
        .get(&url)
        .header(header::AUTHORIZATION, format!("Bearer {}", token))
        .header(header::ACCEPT, "application/vnd.github.v3+json")
        .header(header::USER_AGENT, "wrkflw-cli")
        .send()
        .await
        .map_err(GithubError::RequestError)?;

    if !response.status().is_success() {
        let status = response.status().as_u16();
        let error_message = response
            .text()
            .await
            .unwrap_or_else(|_| format!("Unknown error (HTTP {})", status));
        return Err(GithubError::ApiError {
            status,
            message: error_message,
        });
    }

    let parsed: serde_json::Value = response
        .json()
        .await
        .map_err(|e| GithubError::GitParseError(format!("Failed to parse workflow runs: {}", e)))?;

    // Extract the workflow runs from the response
    if let Some(workflow_runs) = parsed.get("workflow_runs").and_then(|wr| wr.as_array()) {
        Ok(workflow_runs.clone())
    } else {
        Ok(Vec::new())
    }
}

/// One job of a run on GitHub.
#[derive(Debug, Clone, PartialEq, serde::Serialize)]
pub struct RemoteJob {
    /// As GitHub shows it: the job's `name:`, or its id, with matrix values.
    pub name: String,
    /// `success`, `failure`, `cancelled`, `skipped`, or `None` while running.
    pub conclusion: Option<String>,
}

/// A workflow run on GitHub, as `wrkflw compare` needs it.
#[derive(Debug, Clone, serde::Serialize)]
pub struct RemoteRun {
    pub id: u64,
    pub owner: String,
    pub repo: String,
    pub html_url: String,
    pub head_sha: String,
    pub head_branch: Option<String>,
    /// For a push, the commit the branch was at before it: forty zeros when
    /// the push created the branch.
    pub before: Option<String>,
    pub event: String,
    /// The workflow file, such as `.github/workflows/ci.yml`.
    pub path: String,
    pub conclusion: Option<String>,
    /// For a pull request, the branch it goes into. Once the pull request is
    /// closed GitHub no longer says, and this is the default branch.
    pub base_branch: Option<String>,
    pub jobs: Vec<RemoteJob>,
}

/// A run named by its id, or by its URL, which also names the repository.
#[derive(Debug, Clone, PartialEq)]
pub struct RunReference {
    pub owner: Option<String>,
    pub repo: Option<String>,
    pub id: u64,
}

/// Read `123`, or `https://github.com/o/r/actions/runs/123` with anything after.
pub fn parse_run_reference(text: &str) -> Option<RunReference> {
    let text = text.trim();
    if let Ok(id) = text.parse::<u64>() {
        return Some(RunReference {
            owner: None,
            repo: None,
            id,
        });
    }
    let rest = text
        .strip_prefix("https://github.com/")
        .or_else(|| text.strip_prefix("http://github.com/"))
        .or_else(|| text.strip_prefix("github.com/"))?;
    let parts: Vec<&str> = rest.split('/').collect();
    // Owner and repository go into the API path, so only names GitHub
    // allows get there.
    let is_name = |part: &str| {
        !part.is_empty()
            && part != "."
            && part != ".."
            && part
                .chars()
                .all(|c| c.is_ascii_alphanumeric() || matches!(c, '-' | '_' | '.'))
    };
    match parts.as_slice() {
        [owner, repo, "actions", "runs", id, ..] if is_name(owner) && is_name(repo) => {
            Some(RunReference {
                owner: Some(owner.to_string()),
                repo: Some(repo.to_string()),
                id: id.parse().ok()?,
            })
        }
        _ => None,
    }
}

/// A token for the API: `GITHUB_TOKEN`, else what `gh auth token` prints.
/// `None` is fine for a public repository.
fn api_token() -> Option<String> {
    if let Ok(token) = std::env::var("GITHUB_TOKEN") {
        let token = token.trim().to_string();
        if !token.is_empty() {
            return Some(token);
        }
    }
    let output = Command::new("gh").args(["auth", "token"]).output().ok()?;
    let token = String::from_utf8_lossy(&output.stdout).trim().to_string();
    (output.status.success() && !token.is_empty()).then_some(token)
}

async fn api_get(
    client: &reqwest::Client,
    path: &str,
    token: Option<&str>,
) -> Result<serde_json::Value, GithubError> {
    let mut request = client
        .get(format!("https://api.github.com{path}"))
        .header(header::ACCEPT, "application/vnd.github+json")
        .header(header::USER_AGENT, "wrkflw-cli");
    if let Some(token) = token {
        request = request.header(header::AUTHORIZATION, format!("Bearer {token}"));
    }
    let response = request.send().await?;
    if !response.status().is_success() {
        let status = response.status().as_u16();
        let message = response
            .text()
            .await
            .unwrap_or_else(|_| format!("HTTP {status}"));
        return Err(GithubError::ApiError { status, message });
    }
    Ok(response.json().await?)
}

/// Fetch run `reference` and its jobs. The repository is the one the URL
/// names, else this checkout's `origin`.
pub async fn fetch_run(reference: &RunReference) -> Result<RemoteRun, GithubError> {
    let (owner, repo) = match (&reference.owner, &reference.repo) {
        (Some(owner), Some(repo)) => (owner.clone(), repo.clone()),
        _ => {
            let info = get_repo_info()?;
            (info.owner, info.repo)
        }
    };
    // Read before the first request, on a blocking thread: `gh auth token`
    // is a subprocess. One client serves every request of the fetch.
    let token = tokio::task::spawn_blocking(api_token).await.ok().flatten();
    let client = http_client()?;
    let base = format!("/repos/{owner}/{repo}/actions/runs/{}", reference.id);
    let jobs_page = |page: usize| {
        let path = format!("{base}/jobs?per_page=100&page={page}");
        let (client, token) = (&client, token.as_deref());
        async move { api_get(client, &path, token).await }
    };
    // The run and its first page of jobs together: neither needs the other,
    // and the local run waits for both.
    let (run, first_jobs) = tokio::join!(api_get(&client, &base, token.as_deref()), jobs_page(1));
    let run = run?;
    let mut listed = first_jobs?;
    let mut jobs = Vec::new();
    let mut page = 1;
    loop {
        let batch = listed["jobs"].as_array().cloned().unwrap_or_default();
        jobs.extend(batch.iter().map(|job| RemoteJob {
            name: job["name"].as_str().unwrap_or_default().to_string(),
            conclusion: job["conclusion"].as_str().map(str::to_string),
        }));
        let total = listed["total_count"].as_u64().unwrap_or(0) as usize;
        if batch.is_empty() || jobs.len() >= total {
            break;
        }
        page += 1;
        listed = jobs_page(page).await?;
    }
    let text = |key: &str| run[key].as_str().map(str::to_string);
    // GitHub empties `pull_requests` once the pull request is merged or
    // closed. Most pull requests go into the default branch, so fall back to
    // it rather than leave the event unrunnable.
    let base_branch = match listed_base_branch(&run) {
        Some(base) => Some(base),
        None if is_pull_request_run(&run) => {
            api_get(&client, &format!("/repos/{owner}/{repo}"), token.as_deref())
                .await
                .ok()
                .and_then(|r| r["default_branch"].as_str().map(str::to_string))
        }
        None => None,
    };
    // A run doesn't say what a push pushed over, but its check suite does.
    // Best effort: without it, `compare` diffs as `--diff` would.
    let before = match (text("event").as_deref(), run["check_suite_id"].as_u64()) {
        (Some("push"), Some(suite)) => api_get(
            &client,
            &format!("/repos/{owner}/{repo}/check-suites/{suite}"),
            token.as_deref(),
        )
        .await
        .ok()
        .and_then(|suite| suite["before"].as_str().and_then(commit_id)),
        _ => None,
    };
    let head_sha = text("head_sha")
        .as_deref()
        .and_then(commit_id)
        .ok_or_else(|| {
            GithubError::UnexpectedResponse(format!(
                "run {} has no valid head_sha: {}",
                reference.id, run["head_sha"]
            ))
        })?;
    Ok(RemoteRun {
        id: reference.id,
        owner,
        repo,
        html_url: text("html_url").unwrap_or_default(),
        head_sha,
        head_branch: text("head_branch"),
        before,
        event: text("event").unwrap_or_default(),
        path: text("path").unwrap_or_default(),
        conclusion: text("conclusion"),
        base_branch,
        jobs,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn only_a_pull_request_run_takes_a_base_branch_from_its_pull_requests() {
        let listed = serde_json::json!([{"base": {"ref": "main"}}]);
        let pull_request = serde_json::json!({"event": "pull_request", "pull_requests": listed});
        assert_eq!(listed_base_branch(&pull_request), Some("main".to_string()));
        // A push lists a fork's pull request whose head is the pushed commit.
        let push = serde_json::json!({"event": "push", "pull_requests": listed});
        assert_eq!(listed_base_branch(&push), None);
        let merged = serde_json::json!({"event": "pull_request_target", "pull_requests": []});
        assert_eq!(listed_base_branch(&merged), None);
        assert!(is_pull_request_run(&merged));
    }

    #[test]
    fn only_a_full_hex_commit_id_is_taken_from_the_api() {
        let sha1 = "0123456789abcdef0123456789ABCDEF01234567";
        assert_eq!(commit_id(sha1), Some(sha1.to_ascii_lowercase()));
        assert!(commit_id(&"a".repeat(64)).is_some());
        assert_eq!(commit_id(&"0".repeat(40)), Some("0".repeat(40)));
        assert_eq!(commit_id("abc123"), None);
        assert_eq!(
            commit_id("--upload-pack=x0000000000000000000000000000"),
            None
        );
        assert_eq!(commit_id(&format!("{}g", "a".repeat(39))), None);
        assert_eq!(commit_id(""), None);
    }

    #[test]
    fn a_run_is_named_by_id_or_url() {
        assert_eq!(
            parse_run_reference("42"),
            Some(RunReference {
                owner: None,
                repo: None,
                id: 42
            })
        );
        assert_eq!(
            parse_run_reference("https://github.com/o/r/actions/runs/7/job/9"),
            Some(RunReference {
                owner: Some("o".into()),
                repo: Some("r".into()),
                id: 7
            })
        );
        assert_eq!(parse_run_reference("https://github.com/o/r/pull/7"), None);
        assert_eq!(parse_run_reference("nope"), None);
        assert_eq!(
            parse_run_reference("https://github.com/o/r?x=1/actions/runs/7"),
            None
        );
        assert_eq!(
            parse_run_reference("https://github.com/../r/actions/runs/7"),
            None
        );
    }

    #[test]
    fn workflow_dispatch_path_segment_keeps_yml_extension() {
        assert_eq!(
            workflow_dispatch_path_segment("ci.yml"),
            Some("ci.yml".into())
        );
    }

    #[test]
    fn workflow_dispatch_path_segment_keeps_yaml_extension() {
        // Regression: the old dispatcher unconditionally appended
        // `.yml`, turning `ci.yaml` into `ci.yaml.yml` which was a
        // guaranteed 404. The helper must round-trip the `.yaml`
        // form untouched.
        assert_eq!(
            workflow_dispatch_path_segment("ci.yaml"),
            Some("ci.yaml".into())
        );
    }

    #[test]
    fn workflow_dispatch_path_segment_appends_yml_when_missing() {
        assert_eq!(workflow_dispatch_path_segment("ci"), Some("ci.yml".into()));
    }

    #[test]
    fn workflow_dispatch_path_segment_strips_subdir_prefix() {
        // GitHub does not support subdirs under `.github/workflows/`,
        // but the caller may pass a filesystem-like path. The helper
        // drops everything before the final path separator so the
        // segment always addresses the workflow by basename.
        assert_eq!(
            workflow_dispatch_path_segment("release/prod.yml"),
            Some("prod.yml".into())
        );
        assert_eq!(
            workflow_dispatch_path_segment("deep/nested/ci.yaml"),
            Some("ci.yaml".into())
        );
    }

    #[test]
    fn workflow_dispatch_path_segment_rejects_inputs_with_no_basename() {
        assert_eq!(workflow_dispatch_path_segment(""), None);
        assert_eq!(workflow_dispatch_path_segment("/"), None);
        assert_eq!(workflow_dispatch_path_segment("foo/"), None);
    }

    #[test]
    fn workflow_dispatch_path_segment_matches_across_dispatcher_and_preview() {
        // The whole point of the helper is that the preview and the
        // real dispatcher produce the same URL segment for the same
        // input. Pin the identities the Trigger-tab curl preview
        // depends on so a refactor in either place can't silently
        // drift them apart.
        for input in [
            "ci",
            "ci.yml",
            "ci.yaml",
            "release/prod.yml",
            "deep/nested/ci.yaml",
            "has spaces.yml",
            "weird;name.yml",
        ] {
            let segment = workflow_dispatch_path_segment(input)
                .unwrap_or_else(|| panic!("helper produced None for {:?}", input));
            assert!(
                !segment.contains('/') && !segment.contains('\\'),
                "segment {:?} for input {:?} must be a bare basename",
                segment,
                input
            );
            assert!(
                segment.ends_with(".yml") || segment.ends_with(".yaml"),
                "segment {:?} for input {:?} must carry an extension",
                segment,
                input
            );
        }
    }
}
