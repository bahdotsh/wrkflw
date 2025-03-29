use crate::parser::workflow::WorkflowDefinition;
use chrono::Utc;
use std::collections::HashMap;

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
