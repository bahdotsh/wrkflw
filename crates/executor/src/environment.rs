use chrono::Utc;
use serde_json;
use serde_yaml::Value;
use std::{collections::HashMap, fs, io, path::Path};
use wrkflw_matrix::MatrixCombination;
use wrkflw_parser::workflow::WorkflowDefinition;

pub fn setup_github_environment_files(workspace_dir: &Path) -> io::Result<()> {
    // Create necessary directories
    let github_dir = workspace_dir.join("github");
    fs::create_dir_all(&github_dir)?;

    // Create common GitHub environment files
    let github_output = github_dir.join("output");
    let github_env = github_dir.join("env");
    let github_path = github_dir.join("path");
    let github_step_summary = github_dir.join("step_summary");

    // Initialize files with empty content
    fs::write(&github_output, "")?;
    fs::write(&github_env, "")?;
    fs::write(&github_path, "")?;
    fs::write(&github_step_summary, "")?;

    Ok(())
}

pub fn create_github_context(
    workflow: &WorkflowDefinition,
    workspace_dir: &Path,
) -> HashMap<String, String> {
    let mut env = HashMap::new();

    // Basic GitHub environment variables
    env.insert("GITHUB_WORKFLOW".to_string(), workflow.name.clone());
    env.insert("GITHUB_ACTION".to_string(), "run".to_string());
    env.insert("GITHUB_REPOSITORY".to_string(), get_repo_name());
    env.insert("GITHUB_EVENT_NAME".to_string(), get_event_name(workflow));
    env.insert("GITHUB_WORKSPACE".to_string(), get_workspace_path());
    env.insert("GITHUB_SHA".to_string(), get_current_sha());
    env.insert("GITHUB_REF".to_string(), get_current_ref());

    // File paths for GitHub Actions
    env.insert(
        "GITHUB_OUTPUT".to_string(),
        workspace_dir
            .join("github")
            .join("output")
            .to_string_lossy()
            .to_string(),
    );
    env.insert(
        "GITHUB_ENV".to_string(),
        workspace_dir
            .join("github")
            .join("env")
            .to_string_lossy()
            .to_string(),
    );
    env.insert(
        "GITHUB_PATH".to_string(),
        workspace_dir
            .join("github")
            .join("path")
            .to_string_lossy()
            .to_string(),
    );
    env.insert(
        "GITHUB_STEP_SUMMARY".to_string(),
        workspace_dir
            .join("github")
            .join("step_summary")
            .to_string_lossy()
            .to_string(),
    );

    // Time-related variables
    let now = Utc::now();
    env.insert("GITHUB_RUN_ID".to_string(), format!("{}", now.timestamp()));
    env.insert("GITHUB_RUN_NUMBER".to_string(), "1".to_string());
    env.insert("GITHUB_RUN_ATTEMPT".to_string(), "1".to_string());

    // CI detection variables
    env.insert("GITHUB_ACTIONS".to_string(), "true".to_string());
    env.insert("CI".to_string(), "true".to_string());

    // GitHub URLs
    env.insert(
        "GITHUB_SERVER_URL".to_string(),
        "https://github.com".to_string(),
    );
    env.insert(
        "GITHUB_API_URL".to_string(),
        "https://api.github.com".to_string(),
    );
    env.insert(
        "GITHUB_GRAPHQL_URL".to_string(),
        "https://api.github.com/graphql".to_string(),
    );

    // Ref-derived variables
    let full_ref = env.get("GITHUB_REF").cloned().unwrap_or_default();
    env.insert("GITHUB_REF_NAME".to_string(), get_ref_name(&full_ref));
    env.insert("GITHUB_REF_TYPE".to_string(), get_ref_type(&full_ref));

    // PR-related variables (empty for local runs)
    env.insert("GITHUB_HEAD_REF".to_string(), String::new());
    env.insert("GITHUB_BASE_REF".to_string(), String::new());

    // Actor-related variables
    let actor = get_actor();
    env.insert("GITHUB_ACTOR".to_string(), actor.clone());
    env.insert("GITHUB_TRIGGERING_ACTOR".to_string(), actor);

    // Repository owner
    let repo = env.get("GITHUB_REPOSITORY").cloned().unwrap_or_default();
    env.insert(
        "GITHUB_REPOSITORY_OWNER".to_string(),
        get_repository_owner(&repo),
    );

    // Miscellaneous
    env.insert("GITHUB_RETENTION_DAYS".to_string(), "90".to_string());

    // Runner variables
    env.insert("RUNNER_OS".to_string(), get_runner_os());
    env.insert("RUNNER_ARCH".to_string(), get_runner_arch());
    env.insert("RUNNER_NAME".to_string(), "wrkflw-local".to_string());
    env.insert("RUNNER_ENVIRONMENT".to_string(), "local".to_string());
    env.insert("RUNNER_TEMP".to_string(), get_temp_dir());
    env.insert("RUNNER_TOOL_CACHE".to_string(), get_tool_cache_dir());

    env
}

/// Add job-specific context variables to the environment
pub fn add_job_context(env: &mut HashMap<String, String>, job_name: &str) {
    env.insert("GITHUB_JOB".to_string(), job_name.to_string());
}

/// Add matrix context variables to the environment
pub fn add_matrix_context(
    env: &mut HashMap<String, String>,
    matrix_combination: &MatrixCombination,
) {
    // Add each matrix parameter as an environment variable
    for (key, value) in &matrix_combination.values {
        let env_key = format!("MATRIX_{}", key.to_uppercase());
        let env_value = value_to_string(value);
        env.insert(env_key, env_value);
    }

    // Also serialize the whole matrix as JSON for potential use
    if let Ok(json_value) = serde_json::to_string(&matrix_combination.values) {
        env.insert("MATRIX_CONTEXT".to_string(), json_value);
    }
}

/// Convert a serde_yaml::Value to a string for environment variables
fn value_to_string(value: &Value) -> String {
    match value {
        Value::String(s) => s.clone(),
        Value::Number(n) => n.to_string(),
        Value::Bool(b) => b.to_string(),
        Value::Sequence(seq) => {
            let items = seq
                .iter()
                .map(value_to_string)
                .collect::<Vec<_>>()
                .join(",");
            items
        }
        Value::Mapping(map) => {
            let items = map
                .iter()
                .map(|(k, v)| format!("{}={}", value_to_string(k), value_to_string(v)))
                .collect::<Vec<_>>()
                .join(",");
            items
        }
        Value::Null => "".to_string(),
        _ => "".to_string(),
    }
}

fn get_repo_name() -> String {
    // Try to detect from git if available
    if let Ok(output) = std::process::Command::new("git")
        .args(["remote", "get-url", "origin"])
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
        .args(["rev-parse", "HEAD"])
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
        .args(["symbolic-ref", "--short", "HEAD"])
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

fn get_runner_os() -> String {
    match std::env::consts::OS {
        "macos" => "macOS".to_string(),
        "linux" => "Linux".to_string(),
        "windows" => "Windows".to_string(),
        other => other.to_string(),
    }
}

fn get_runner_arch() -> String {
    match std::env::consts::ARCH {
        "x86_64" | "x86" => "X64".to_string(),
        "aarch64" => "ARM64".to_string(),
        other => other.to_string(),
    }
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

fn get_ref_name(full_ref: &str) -> String {
    if let Some(name) = full_ref.strip_prefix("refs/heads/") {
        name.to_string()
    } else if let Some(name) = full_ref.strip_prefix("refs/tags/") {
        name.to_string()
    } else if let Some(name) = full_ref.strip_prefix("refs/pull/") {
        name.to_string()
    } else {
        full_ref.to_string()
    }
}

fn get_ref_type(full_ref: &str) -> String {
    if full_ref.starts_with("refs/tags/") {
        "tag".to_string()
    } else {
        "branch".to_string()
    }
}

fn get_repository_owner(repo: &str) -> String {
    repo.split('/').next().unwrap_or("").to_string()
}

fn get_actor() -> String {
    // Try git config user.name first
    if let Ok(output) = std::process::Command::new("git")
        .args(["config", "user.name"])
        .output()
    {
        if output.status.success() {
            let name = String::from_utf8_lossy(&output.stdout).trim().to_string();
            if !name.is_empty() {
                return name;
            }
        }
    }

    // Fall back to $USER or $USERNAME
    if let Ok(user) = std::env::var("USER") {
        if !user.is_empty() {
            return user;
        }
    }
    if let Ok(user) = std::env::var("USERNAME") {
        if !user.is_empty() {
            return user;
        }
    }

    "wrkflw".to_string()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ref_name_strips_heads_prefix() {
        assert_eq!(get_ref_name("refs/heads/main"), "main");
        assert_eq!(get_ref_name("refs/heads/feature/foo"), "feature/foo");
    }

    #[test]
    fn ref_name_strips_tags_prefix() {
        assert_eq!(get_ref_name("refs/tags/v1.0.0"), "v1.0.0");
    }

    #[test]
    fn ref_name_returns_input_for_unknown_prefix() {
        assert_eq!(get_ref_name("some/other/ref"), "some/other/ref");
    }

    #[test]
    fn ref_type_detects_tag() {
        assert_eq!(get_ref_type("refs/tags/v1.0.0"), "tag");
    }

    #[test]
    fn ref_type_defaults_to_branch() {
        assert_eq!(get_ref_type("refs/heads/main"), "branch");
        assert_eq!(get_ref_type("something-else"), "branch");
    }

    #[test]
    fn repository_owner_extracts_owner() {
        assert_eq!(get_repository_owner("octocat/hello-world"), "octocat");
    }

    #[test]
    fn repository_owner_handles_no_slash() {
        assert_eq!(get_repository_owner("myrepo"), "myrepo");
    }

    #[test]
    fn repository_owner_handles_empty() {
        assert_eq!(get_repository_owner(""), "");
    }

    #[test]
    fn extract_repo_from_ssh_url() {
        assert_eq!(
            extract_repo_from_url("git@github.com:owner/repo.git"),
            Some("owner/repo".to_string())
        );
    }

    #[test]
    fn extract_repo_from_https_url() {
        assert_eq!(
            extract_repo_from_url("https://github.com/owner/repo.git"),
            Some("owner/repo".to_string())
        );
    }

    #[test]
    fn extract_repo_from_invalid_url() {
        assert_eq!(extract_repo_from_url("not-a-url"), None);
    }
}
