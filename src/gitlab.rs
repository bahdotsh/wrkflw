use lazy_static::lazy_static;
use regex::Regex;
use reqwest::header;
use std::collections::HashMap;
use std::path::Path;
use std::process::Command;
use thiserror::Error;

#[derive(Error, Debug)]
pub enum GitlabError {
    #[error("HTTP error: {0}")]
    RequestError(#[from] reqwest::Error),

    #[error("IO error: {0}")]
    IoError(#[from] std::io::Error),

    #[error("Failed to parse Git repository URL: {0}")]
    GitParseError(String),

    #[error("GitLab token not found. Please set GITLAB_TOKEN environment variable")]
    TokenNotFound,

    #[error("API error: {status} - {message}")]
    ApiError { status: u16, message: String },
}

/// Information about a GitLab repository
#[derive(Debug, Clone)]
pub struct RepoInfo {
    pub namespace: String,
    pub project: String,
    pub default_branch: String,
}

lazy_static! {
    static ref GITLAB_REPO_REGEX: Regex =
        Regex::new(r"(?:https://gitlab\.com/|git@gitlab\.com:)([^/]+)/([^/.]+)(?:\.git)?")
            .expect("Failed to compile GitLab repo regex - this is a critical error");
}

/// Extract repository information from the current git repository for GitLab
pub fn get_repo_info() -> Result<RepoInfo, GitlabError> {
    let output = Command::new("git")
        .args(["remote", "get-url", "origin"])
        .output()
        .map_err(|e| GitlabError::GitParseError(format!("Failed to execute git command: {}", e)))?;

    if !output.status.success() {
        return Err(GitlabError::GitParseError(
            "Failed to get git origin URL. Are you in a git repository?".to_string(),
        ));
    }

    let url = String::from_utf8_lossy(&output.stdout).trim().to_string();

    if let Some(captures) = GITLAB_REPO_REGEX.captures(&url) {
        let namespace = captures
            .get(1)
            .ok_or_else(|| {
                GitlabError::GitParseError(
                    "Unable to extract namespace from GitLab URL".to_string(),
                )
            })?
            .as_str()
            .to_string();

        let project = captures
            .get(2)
            .ok_or_else(|| {
                GitlabError::GitParseError(
                    "Unable to extract project name from GitLab URL".to_string(),
                )
            })?
            .as_str()
            .to_string();

        // Get the default branch
        let branch_output = Command::new("git")
            .args(["rev-parse", "--abbrev-ref", "HEAD"])
            .output()
            .map_err(|e| {
                GitlabError::GitParseError(format!("Failed to execute git command: {}", e))
            })?;

        if !branch_output.status.success() {
            return Err(GitlabError::GitParseError(
                "Failed to get current branch".to_string(),
            ));
        }

        let default_branch = String::from_utf8_lossy(&branch_output.stdout)
            .trim()
            .to_string();

        Ok(RepoInfo {
            namespace,
            project,
            default_branch,
        })
    } else {
        Err(GitlabError::GitParseError(format!(
            "URL '{}' is not a valid GitLab repository URL",
            url
        )))
    }
}

/// Get the list of available pipeline files in the repository
pub async fn list_pipelines(_repo_info: &RepoInfo) -> Result<Vec<String>, GitlabError> {
    // GitLab CI/CD pipelines are defined in .gitlab-ci.yml files
    let pipeline_file = Path::new(".gitlab-ci.yml");

    if !pipeline_file.exists() {
        return Err(GitlabError::IoError(std::io::Error::new(
            std::io::ErrorKind::NotFound,
            "GitLab CI/CD pipeline file not found (.gitlab-ci.yml)",
        )));
    }

    // In GitLab, there's typically a single pipeline file with multiple jobs
    // Return a list with just that file name
    Ok(vec!["gitlab-ci".to_string()])
}

/// Trigger a pipeline on GitLab
pub async fn trigger_pipeline(
    branch: Option<&str>,
    variables: Option<HashMap<String, String>>,
) -> Result<(), GitlabError> {
    // Get GitLab token from environment
    let token = std::env::var("GITLAB_TOKEN").map_err(|_| GitlabError::TokenNotFound)?;

    // Trim the token to remove any leading or trailing whitespace
    let trimmed_token = token.trim();

    // Get repository information
    let repo_info = get_repo_info()?;
    println!(
        "GitLab Repository: {}/{}",
        repo_info.namespace, repo_info.project
    );

    // Prepare the request payload
    let branch_ref = branch.unwrap_or(&repo_info.default_branch);
    println!("Using branch: {}", branch_ref);

    // Create simplified payload
    let mut payload = serde_json::json!({
        "ref": branch_ref
    });

    // Add variables if provided
    if let Some(vars_map) = variables {
        // GitLab expects variables in a specific format
        let formatted_vars: Vec<serde_json::Value> = vars_map
            .iter()
            .map(|(key, value)| {
                serde_json::json!({
                    "key": key,
                    "value": value
                })
            })
            .collect();

        payload["variables"] = serde_json::json!(formatted_vars);
        println!("With variables: {:?}", vars_map);
    }

    // URL encode the namespace and project for use in URL
    let encoded_namespace = urlencoding::encode(&repo_info.namespace);
    let encoded_project = urlencoding::encode(&repo_info.project);

    // Send the pipeline trigger request
    let url = format!(
        "https://gitlab.com/api/v4/projects/{encoded_namespace}%2F{encoded_project}/pipeline",
        encoded_namespace = encoded_namespace,
        encoded_project = encoded_project,
    );

    println!("Triggering pipeline at URL: {}", url);

    // Create a reqwest client
    let client = reqwest::Client::new();

    // Send the request using reqwest
    let response = client
        .post(&url)
        .header("PRIVATE-TOKEN", trimmed_token)
        .header(header::CONTENT_TYPE, "application/json")
        .json(&payload)
        .send()
        .await
        .map_err(GitlabError::RequestError)?;

    if !response.status().is_success() {
        let status = response.status().as_u16();
        let error_message = response
            .text()
            .await
            .unwrap_or_else(|_| format!("Unknown error (HTTP {})", status));

        // Add more detailed error information
        let error_details = if status == 404 {
            "Project not found or token doesn't have access to it. This could be due to:\n\
             1. The project doesn't exist\n\
             2. The GitLab token doesn't have sufficient permissions\n\
             Please check:\n\
             - The repository URL is correct\n\
             - Your GitLab token has the correct scope (api access)\n\
             - Your token has access to the project"
        } else if status == 401 {
            "Unauthorized. Your GitLab token may be invalid or expired."
        } else {
            &error_message
        };

        return Err(GitlabError::ApiError {
            status,
            message: error_details.to_string(),
        });
    }

    // Parse response to get pipeline ID
    let pipeline_info: serde_json::Value = response.json().await?;
    let pipeline_id = pipeline_info["id"].as_i64().unwrap_or(0);
    let pipeline_url = format!(
        "https://gitlab.com/{}/{}/pipelines/{}",
        repo_info.namespace, repo_info.project, pipeline_id
    );

    println!("Pipeline triggered successfully!");
    println!("View pipeline at: {}", pipeline_url);

    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_parse_gitlab_url_https() {
        let url = "https://gitlab.com/mygroup/myproject.git";
        assert!(GITLAB_REPO_REGEX.is_match(url));

        let captures = GITLAB_REPO_REGEX.captures(url).unwrap();
        assert_eq!(captures.get(1).unwrap().as_str(), "mygroup");
        assert_eq!(captures.get(2).unwrap().as_str(), "myproject");
    }

    #[test]
    fn test_parse_gitlab_url_ssh() {
        let url = "git@gitlab.com:mygroup/myproject.git";
        assert!(GITLAB_REPO_REGEX.is_match(url));

        let captures = GITLAB_REPO_REGEX.captures(url).unwrap();
        assert_eq!(captures.get(1).unwrap().as_str(), "mygroup");
        assert_eq!(captures.get(2).unwrap().as_str(), "myproject");
    }

    #[test]
    fn test_parse_gitlab_url_no_git_extension() {
        let url = "https://gitlab.com/mygroup/myproject";
        assert!(GITLAB_REPO_REGEX.is_match(url));

        let captures = GITLAB_REPO_REGEX.captures(url).unwrap();
        assert_eq!(captures.get(1).unwrap().as_str(), "mygroup");
        assert_eq!(captures.get(2).unwrap().as_str(), "myproject");
    }

    #[test]
    fn test_parse_invalid_url() {
        let url = "https://github.com/myuser/myrepo.git";
        assert!(!GITLAB_REPO_REGEX.is_match(url));
    }
}
