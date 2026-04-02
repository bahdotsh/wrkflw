// UI utilities
use crate::models::{Workflow, WorkflowStatus};
use std::path::{Path, PathBuf};
use wrkflw_parser::workflow::parse_workflow;
use wrkflw_utils::is_workflow_file;

/// Parse a workflow file and return sorted job names, or an empty vec on failure.
pub fn extract_job_names(path: &Path) -> Vec<String> {
    parse_workflow(path)
        .map(|wf| {
            let mut names: Vec<String> = wf.jobs.keys().cloned().collect();
            names.sort();
            names
        })
        .unwrap_or_default()
}

/// Find and load all workflow files in a directory
pub fn load_workflows(dir_path: &Path) -> Vec<Workflow> {
    let mut workflows = Vec::new();

    // Default path is .github/workflows
    let default_workflows_dir = Path::new(".github").join("workflows");
    let is_default_dir = dir_path == default_workflows_dir || dir_path.ends_with("workflows");

    if let Ok(entries) = std::fs::read_dir(dir_path) {
        for entry in entries.flatten() {
            let path = entry.path();
            if path.is_file() && (is_workflow_file(&path) || !is_default_dir) {
                // Get just the base name without extension
                let name = path.file_stem().map_or_else(
                    || "[unknown]".to_string(),
                    |fname| fname.to_string_lossy().into_owned(),
                );

                let job_names = extract_job_names(&path);

                workflows.push(Workflow {
                    name,
                    path,
                    selected: false,
                    status: WorkflowStatus::NotStarted,
                    execution_details: None,
                    job_names,
                });
            }
        }
    }

    // Check for GitLab CI pipeline file in the root directory if we're in the default GitHub workflows dir
    if is_default_dir {
        // Look for .gitlab-ci.yml in the repository root
        let gitlab_ci_path = PathBuf::from(".gitlab-ci.yml");
        if gitlab_ci_path.exists() && gitlab_ci_path.is_file() {
            let job_names = extract_job_names(&gitlab_ci_path);

            workflows.push(Workflow {
                name: "gitlab-ci".to_string(),
                path: gitlab_ci_path,
                selected: false,
                status: WorkflowStatus::NotStarted,
                execution_details: None,
                job_names,
            });
        }
    }

    // Sort workflows by name
    workflows.sort_by(|a, b| a.name.cmp(&b.name));
    workflows
}
