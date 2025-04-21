use serde::Serialize;
use std::path::Path;
use thiserror::Error;
use reqwest::{Client, header};
use std::fs;
use std::process::Command;
use std::collections::HashMap;
use regex::Regex;

#[derive(Error, Debug)]
pub enum GithubError {
    #[error("HTTP error: {0}")]
    RequestError(#[from] reqwest::Error),
    
    #[error("IO error: {0}")]
    IoError(#[from] std::io::Error),
    
    #[error("Failed to parse Git repository URL: {0}")]
    GitParseError(String),
    
    #[error("Failed to extract repository info: {0}")]
    RepoInfoError(String),
    
    #[error("GitHub token not found. Please set GITHUB_TOKEN environment variable")]
    TokenNotFound,
    
    #[error("Failed to parse workflow file: {0}")]
    WorkflowParseError(String),
    
    #[error("API error: {status} - {message}")]
    ApiError { status: u16, message: String },
}

#[derive(Debug, Serialize)]
struct WorkflowDispatchPayload {
    #[serde(rename = "ref")]
    reference: String,
    inputs: Option<HashMap<String, String>>,
}

/// Information about a GitHub repository
#[derive(Debug, Clone)]
pub struct RepoInfo {
    pub owner: String,
    pub repo: String,
    pub default_branch: String,
}

/// Extract repository information from the current git repository
pub fn get_repo_info() -> Result<RepoInfo, GithubError> {
    // Get the remote URL
    let output = Command::new("git")
        .args(["remote", "get-url", "origin"])
        .output()?;
    
    if !output.status.success() {
        return Err(GithubError::GitParseError(
            "Failed to get git remote URL".to_string(),
        ));
    }
    
    let url = String::from_utf8_lossy(&output.stdout).trim().to_string();
    
    // Extract owner and repo from the URL using regex
    let re = Regex::new(r"(?:https://github\.com/|git@github\.com:)([^/]+)/([^/.]+)(?:\.git)?").unwrap();
    let captures = re.captures(&url).ok_or_else(|| {
        GithubError::GitParseError(format!("Could not parse GitHub URL: {}", url))
    })?;
    
    let owner = captures.get(1).unwrap().as_str().to_string();
    let repo = captures.get(2).unwrap().as_str().to_string();
    
    // Get the default branch
    let branch_output = Command::new("git")
        .args(["rev-parse", "--abbrev-ref", "HEAD"])
        .output()?;
    
    if !branch_output.status.success() {
        return Err(GithubError::GitParseError(
            "Failed to get current branch".to_string(),
        ));
    }
    
    let default_branch = String::from_utf8_lossy(&branch_output.stdout).trim().to_string();
    
    Ok(RepoInfo {
        owner,
        repo,
        default_branch,
    })
}

/// Get the list of available workflows in the repository
pub async fn list_workflows(_repo_info: &RepoInfo) -> Result<Vec<String>, GithubError> {
    let workflows_dir = Path::new(".github/workflows");
    
    if !workflows_dir.exists() {
        return Err(GithubError::IoError(
            std::io::Error::new(std::io::ErrorKind::NotFound, "Workflows directory not found")
        ));
    }
    
    let mut workflow_names = Vec::new();
    
    for entry in fs::read_dir(workflows_dir)? {
        let entry = entry?;
        let path = entry.path();
        
        if path.is_file() && path.extension().map_or(false, |ext| ext == "yml" || ext == "yaml") {
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
    let token = std::env::var("GITHUB_TOKEN")
        .map_err(|_| GithubError::TokenNotFound)?;
    
    // Get repository information
    let repo_info = get_repo_info()?;
    
    // Create HTTP client with GitHub token
    let mut headers = header::HeaderMap::new();
    headers.insert(
        header::ACCEPT,
        header::HeaderValue::from_static("application/vnd.github.v3+json"),
    );
    headers.insert(
        header::USER_AGENT,
        header::HeaderValue::from_static("wrkflw-cli"),
    );
    
    let client = Client::builder()
        .default_headers(headers)
        .build()?;
    
    // Prepare the request payload
    let branch_ref = branch.unwrap_or(&repo_info.default_branch);
    
    let payload = WorkflowDispatchPayload {
        reference: branch_ref.to_string(),
        inputs,
    };
    
    // Send the workflow_dispatch event
    let url = format!(
        "https://api.github.com/repos/{}/{}/actions/workflows/{}.yml/dispatches",
        repo_info.owner, repo_info.repo, workflow_name
    );
    
    let response = client
        .post(&url)
        .bearer_auth(token)
        .json(&payload)
        .send()
        .await?;
    
    if !response.status().is_success() {
        let status = response.status().as_u16();
        let error_message = response.text().await.unwrap_or_else(|_| {
            format!("Unknown error (HTTP {})", status)
        });
        
        return Err(GithubError::ApiError {
            status,
            message: error_message,
        });
    }
    
    Ok(())
} 