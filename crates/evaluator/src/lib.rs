use colored::*;
use serde_yaml::{self, Value};
use std::fs;
use std::path::{Path, PathBuf};

use wrkflw_models::ValidationResult;
use wrkflw_validators::{validate_env, validate_jobs, validate_triggers};

pub fn evaluate_workflow_file(path: &Path, verbose: bool) -> Result<ValidationResult, String> {
    let content = fs::read_to_string(path).map_err(|e| format!("Failed to read file: {}", e))?;

    // Parse YAML content
    let workflow: Value =
        serde_yaml::from_str(&content).map_err(|e| format!("Invalid YAML: {}", e))?;

    let mut result = ValidationResult::new();
    let repo_root = find_repo_root(path);

    // Check for required structure
    if !workflow.is_mapping() {
        result.add_issue("Workflow file is not a valid YAML mapping".to_string());
        return Ok(result);
    }

    // Note: The 'name' field is optional per GitHub Actions specification.
    // When omitted, GitHub displays the workflow file path relative to the repository root.
    // We do not validate name presence as it's not required by the schema.

    // Check if jobs section exists
    match workflow.get("jobs") {
        Some(jobs) if jobs.is_mapping() => {
            validate_jobs(jobs, repo_root.as_deref(), &mut result);
        }
        Some(_) => {
            result.add_issue("'jobs' section is not a mapping".to_string());
        }
        None => {
            result.add_issue("Workflow is missing 'jobs' section".to_string());
        }
    }

    // Validate top-level env is a mapping
    if let Some(env) = workflow.get("env") {
        validate_env(env, "Top-level", &mut result);
    }

    // Check for valid triggers
    match workflow.get("on") {
        Some(on) => {
            validate_triggers(on, &mut result);
        }
        None => {
            result.add_issue("Workflow is missing 'on' section (triggers)".to_string());
        }
    }

    if verbose && result.is_valid {
        println!(
            "{} Validated structure of workflow: {}",
            "✓".green(),
            path.display()
        );
    }

    Ok(result)
}

/// Walk up from the workflow file's directory to find the repository root (.git directory).
/// Returns `None` if no `.git` directory is found.
fn find_repo_root(workflow_path: &Path) -> Option<PathBuf> {
    let canonical = fs::canonicalize(workflow_path).ok()?;
    let mut dir = canonical.parent();
    while let Some(d) = dir {
        if d.join(".git").exists() {
            return Some(d.to_path_buf());
        }
        dir = d.parent();
    }
    None
}
