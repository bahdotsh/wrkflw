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
    let client = reqwest::Client::new();

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

    let client = reqwest::Client::new();
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

#[cfg(test)]
mod tests {
    use super::*;

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
