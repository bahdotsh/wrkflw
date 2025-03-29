use crate::parser::workflow::WorkflowDefinition;
use chrono::Utc;
use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::process::Command;

pub fn create_github_context(workflow: &WorkflowDefinition) -> HashMap<String, String> {
    let mut env = HashMap::new();

    // Basic GitHub environment variables
    env.insert("GITHUB_WORKFLOW".to_string(), workflow.name.clone());
    env.insert("GITHUB_ACTION".to_string(), "run".to_string());
    env.insert("GITHUB_ACTOR".to_string(), "wrkflw".to_string());
    env.insert("GITHUB_REPOSITORY".to_string(), get_repo_name());
    env.insert("GITHUB_EVENT_NAME".to_string(), get_event_name(workflow));
    env.insert("GITHUB_WORKSPACE".to_string(), get_workspace_path());
    env.insert("GITHUB_SHA".to_string(), get_current_sha());
    env.insert("GITHUB_REF".to_string(), get_current_ref());

    // Time-related variables
    let now = Utc::now();
    env.insert("GITHUB_RUN_ID".to_string(), format!("{}", now.timestamp()));
    env.insert("GITHUB_RUN_NUMBER".to_string(), "1".to_string());

    // Path-related variables
    env.insert("RUNNER_TEMP".to_string(), get_temp_dir());
    env.insert("RUNNER_TOOL_CACHE".to_string(), get_tool_cache_dir());

    env
}

pub fn setup_git_context(workflow_dir: &Path) -> HashMap<String, String> {
    let mut env = HashMap::new();

    // Try to detect the git repository root
    let repo_root = find_git_repo_root(workflow_dir);

    if let Some(repo_root) = repo_root {
        // Set GITHUB_WORKSPACE to the git repo root
        env.insert(
            "GITHUB_WORKSPACE".to_string(),
            repo_root.to_string_lossy().to_string(),
        );

        // Try to get current branch
        if let Some(branch) = get_current_branch(&repo_root) {
            env.insert("GITHUB_REF".to_string(), format!("refs/heads/{}", branch));
            env.insert("GITHUB_REF_NAME".to_string(), branch.clone());
            env.insert("GITHUB_HEAD_REF".to_string(), branch);
        }

        // Try to get current commit SHA
        if let Some(sha) = get_git_sha(&repo_root) {
            env.insert("GITHUB_SHA".to_string(), sha);
        }

        // Try to get repository name from remote origin
        if let Some(repo) = get_repo_name_from_git(&repo_root) {
            env.insert("GITHUB_REPOSITORY".to_string(), repo);
        }
    }

    env
}

fn get_repo_name() -> String {
    // Try to detect from git if available
    if let Ok(output) = std::process::Command::new("git")
        .args(&["remote", "get-url", "origin"])
        .output()
    {
        if output.status.success() {
            let url = String::from_utf8_lossy(&output.stdout);
            if let Some(repo) = extract_repo_from_url(&url) {
                return repo;
            }
        }
    }

    // Fallback to directory name
    let current_dir = std::env::current_dir().unwrap_or_default();
    format!(
        "wrkflw/{}",
        current_dir
            .file_name()
            .unwrap_or_default()
            .to_string_lossy()
    )
}

fn extract_repo_from_url(url: &str) -> Option<String> {
    // Extract owner/repo from common git URLs
    let url = url.trim();

    // Handle SSH URLs: git@github.com:owner/repo.git
    if url.starts_with("git@") {
        let parts: Vec<&str> = url.split(':').collect();
        if parts.len() == 2 {
            let repo_part = parts[1].trim_end_matches(".git");
            return Some(repo_part.to_string());
        }
    }

    // Handle HTTPS URLs: https://github.com/owner/repo.git
    if url.starts_with("http") {
        let without_protocol = url.split("://").nth(1)?;
        let parts: Vec<&str> = without_protocol.split('/').collect();
        if parts.len() >= 3 {
            let owner = parts[1];
            let repo = parts[2].trim_end_matches(".git");
            return Some(format!("{}/{}", owner, repo));
        }
    }

    None
}

fn get_event_name(workflow: &WorkflowDefinition) -> String {
    // Try to extract from the workflow trigger
    if let Some(first_trigger) = workflow.on.first() {
        return first_trigger.clone();
    }
    "workflow_dispatch".to_string()
}

fn get_workspace_path() -> String {
    std::env::current_dir()
        .unwrap_or_default()
        .to_string_lossy()
        .to_string()
}

fn get_current_sha() -> String {
    if let Ok(output) = std::process::Command::new("git")
        .args(&["rev-parse", "HEAD"])
        .output()
    {
        if output.status.success() {
            return String::from_utf8_lossy(&output.stdout).trim().to_string();
        }
    }

    "0000000000000000000000000000000000000000".to_string()
}

fn get_current_ref() -> String {
    if let Ok(output) = std::process::Command::new("git")
        .args(&["symbolic-ref", "--short", "HEAD"])
        .output()
    {
        if output.status.success() {
            return format!(
                "refs/heads/{}",
                String::from_utf8_lossy(&output.stdout).trim()
            );
        }
    }

    "refs/heads/main".to_string()
}

fn get_temp_dir() -> String {
    let temp_dir = std::env::temp_dir();
    temp_dir.join("wrkflw").to_string_lossy().to_string()
}

fn get_tool_cache_dir() -> String {
    let home_dir = dirs::home_dir().unwrap_or_default();
    home_dir
        .join(".wrkflw")
        .join("tools")
        .to_string_lossy()
        .to_string()
}

fn find_git_repo_root(start_dir: &Path) -> Option<PathBuf> {
    let mut current_dir = start_dir.to_path_buf();

    loop {
        let git_dir = current_dir.join(".git");
        if git_dir.exists() && git_dir.is_dir() {
            return Some(current_dir);
        }

        if !current_dir.pop() {
            // Reached root directory without finding .git
            return None;
        }
    }
}

fn get_current_branch(repo_root: &Path) -> Option<String> {
    let output = Command::new("git")
        .args(&["rev-parse", "--abbrev-ref", "HEAD"])
        .current_dir(repo_root)
        .output();

    match output {
        Ok(output) if output.status.success() => {
            let branch = String::from_utf8_lossy(&output.stdout).trim().to_string();
            Some(branch)
        }
        _ => None,
    }
}

fn get_git_sha(repo_root: &Path) -> Option<String> {
    let output = Command::new("git")
        .args(&["rev-parse", "HEAD"])
        .current_dir(repo_root)
        .output();

    match output {
        Ok(output) if output.status.success() => {
            let sha = String::from_utf8_lossy(&output.stdout).trim().to_string();
            Some(sha)
        }
        _ => None,
    }
}

fn get_repo_name_from_git(repo_root: &Path) -> Option<String> {
    let output = Command::new("git")
        .args(&["remote", "get-url", "origin"])
        .current_dir(repo_root)
        .output();

    match output {
        Ok(output) if output.status.success() => {
            let url = String::from_utf8_lossy(&output.stdout).trim().to_string();
            extract_repo_from_url(&url)
        }
        _ => None,
    }
}
