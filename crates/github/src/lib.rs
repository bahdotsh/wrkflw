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

        // Get the default branch
        let branch_output = Command::new("git")
            .args(["rev-parse", "--abbrev-ref", "HEAD"])
            .output()
            .map_err(|e| {
                GithubError::GitParseError(format!("Failed to execute git command: {}", e))
            })?;

        if !branch_output.status.success() {
            return Err(GithubError::GitParseError(
                "Failed to get current branch".to_string(),
            ));
        }

        let default_branch = String::from_utf8_lossy(&branch_output.stdout)
            .trim()
            .to_string();

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
    println!("Repository: {}/{}", repo_info.owner, repo_info.repo);

    // Prepare the request payload
    let branch_ref = branch.unwrap_or(&repo_info.default_branch);
    println!("Using branch: {}", branch_ref);

    // Extract just the workflow name from the path if it's a full path
    let workflow_name = if workflow_name.contains('/') {
        Path::new(workflow_name)
            .file_stem()
            .and_then(|s| s.to_str())
            .ok_or_else(|| GithubError::GitParseError("Invalid workflow name".to_string()))?
    } else {
        workflow_name
    };

    println!("Using workflow name: {}", workflow_name);

    // Create simplified payload
    let mut payload = serde_json::json!({
        "ref": branch_ref
    });

    // Add inputs if provided
    if let Some(input_map) = inputs {
        payload["inputs"] = serde_json::json!(input_map);
        println!("With inputs: {:?}", input_map);
    }

    // Send the workflow_dispatch event
    let url = format!(
        "https://api.github.com/repos/{}/{}/actions/workflows/{}.yml/dispatches",
        repo_info.owner, repo_info.repo, workflow_name
    );

    println!("Triggering workflow at URL: {}", url);

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

    println!("Workflow triggered successfully!");
    println!(
        "View runs at: https://github.com/{}/{}/actions/workflows/{}.yml",
        repo_info.owner, repo_info.repo, workflow_name
    );

    // Attempt to verify the workflow was actually triggered
    match list_recent_workflow_runs(&repo_info, workflow_name, &token).await {
        Ok(runs) => {
            if !runs.is_empty() {
                println!("\nRecent runs of this workflow:");
                for run in runs.iter().take(3) {
                    println!(
                        "- Run #{} ({}): {}",
                        run.get("id").and_then(|id| id.as_u64()).unwrap_or(0),
                        run.get("status")
                            .and_then(|s| s.as_str())
                            .unwrap_or("unknown"),
                        run.get("html_url").and_then(|u| u.as_str()).unwrap_or("")
                    );
                }
            } else {
                println!("\nNo recent runs found. The workflow might still be initializing.");
                println!(
                    "Check GitHub UI in a few moments: https://github.com/{}/{}/actions",
                    repo_info.owner, repo_info.repo
                );
            }
        }
        Err(e) => {
            println!("\nCould not fetch recent workflow runs: {}", e);
            println!("This doesn't mean the trigger failed - check GitHub UI: https://github.com/{}/{}/actions", 
                     repo_info.owner, repo_info.repo);
        }
    }

    Ok(())
}

/// List recent workflow runs for a specific workflow
async fn list_recent_workflow_runs(
    repo_info: &RepoInfo,
    workflow_name: &str,
    token: &str,
) -> Result<Vec<serde_json::Value>, GithubError> {
    // Extract just the workflow name from the path if it's a full path
    let workflow_name = if workflow_name.contains('/') {
        Path::new(workflow_name)
            .file_stem()
            .and_then(|s| s.to_str())
            .ok_or_else(|| GithubError::GitParseError("Invalid workflow name".to_string()))?
    } else {
        workflow_name
    };

    // Get recent workflow runs via GitHub API
    let url = format!(
        "https://api.github.com/repos/{}/{}/actions/workflows/{}.yml/runs?per_page=5",
        repo_info.owner, repo_info.repo, workflow_name
    );

    let curl_output = Command::new("curl")
        .arg("-s")
        .arg("-H")
        .arg(format!("Authorization: Bearer {}", token))
        .arg("-H")
        .arg("Accept: application/vnd.github.v3+json")
        .arg(&url)
        .output()
        .map_err(|e| GithubError::GitParseError(format!("Failed to execute curl: {}", e)))?;

    if !curl_output.status.success() {
        let error_message = String::from_utf8_lossy(&curl_output.stderr).to_string();
        return Err(GithubError::GitParseError(format!(
            "Failed to list workflow runs: {}",
            error_message
        )));
    }

    let response_body = String::from_utf8_lossy(&curl_output.stdout).to_string();
    let parsed: serde_json::Value = serde_json::from_str(&response_body)
        .map_err(|e| GithubError::GitParseError(format!("Failed to parse workflow runs: {}", e)))?;

    // Extract the workflow runs from the response
    if let Some(workflow_runs) = parsed.get("workflow_runs").and_then(|wr| wr.as_array()) {
        Ok(workflow_runs.clone())
    } else {
        Ok(Vec::new())
    }
}
