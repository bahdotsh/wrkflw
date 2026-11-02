//! `--format json` for `validate` and `list`.

use crate::is_gitlab_pipeline;
use serde_json::{json, Value};
use std::path::{Path, PathBuf};
use wrkflw_models::ValidationResult;
use wrkflw_runs::SCHEMA_VERSION;

fn yaml_files(dir: &Path) -> std::io::Result<Vec<PathBuf>> {
    let mut files: Vec<PathBuf> = std::fs::read_dir(dir)?
        .filter_map(|entry| entry.ok().map(|e| e.path()))
        .filter(|path| {
            path.is_file()
                && path
                    .extension()
                    .is_some_and(|ext| ext == "yml" || ext == "yaml")
        })
        .collect();
    files.sort();
    Ok(files)
}

fn file_report(
    path: &Path,
    kind: &str,
    outcome: Result<ValidationResult, (&str, String)>,
) -> Value {
    match outcome {
        Ok(result) => json!({
            "path": path,
            "kind": kind,
            "valid": result.is_valid,
            "issues": result.details,
        }),
        Err((rule, message)) => json!({
            "path": path,
            "kind": kind,
            "valid": false,
            "issues": [{
                "rule": rule,
                "severity": "error",
                "message": message,
                "path": null,
                "line": yaml_error_line(&message),
                "hint": null,
            }],
        }),
    }
}

/// serde_yaml reports positions as `at line N column M`.
fn yaml_error_line(message: &str) -> Option<usize> {
    let rest = message.split(" line ").nth(1)?;
    rest.split(|c: char| !c.is_ascii_digit())
        .next()?
        .parse()
        .ok()
}

fn validate_file(path: &Path, force_gitlab: bool) -> Value {
    if force_gitlab || is_gitlab_pipeline(path) {
        let outcome = wrkflw_parser::gitlab::parse_pipeline(path)
            .map(|pipeline| {
                let mut result = wrkflw_validators::validate_gitlab_pipeline(&pipeline);
                if let Ok(source) = std::fs::read_to_string(path) {
                    result.locate_lines(&source);
                }
                result
            })
            .map_err(|e| ("invalid-pipeline", e.to_string()));
        return file_report(path, "gitlab", outcome);
    }
    let outcome =
        wrkflw_evaluator::evaluate_workflow_file(path, false).map_err(|e| ("invalid-yaml", e));
    file_report(path, "github", outcome)
}

/// Validate `paths` and print one JSON document. Returns whether any failed.
pub(crate) fn validate(paths: &[PathBuf], force_gitlab: bool) -> bool {
    let mut files = Vec::new();
    let mut errors = Vec::new();
    for path in paths {
        if path.is_dir() {
            match yaml_files(path) {
                Ok(found) => files.extend(found.iter().map(|f| validate_file(f, force_gitlab))),
                Err(e) => errors.push(format!("Failed to read directory {}: {e}", path.display())),
            }
        } else if path.exists() {
            files.push(validate_file(path, force_gitlab));
        } else {
            errors.push(format!("Path does not exist: {}", path.display()));
        }
    }
    let valid = errors.is_empty() && files.iter().all(|f| f["valid"] == true);
    wrkflw_runs::emit_line(&json!({
        "schema_version": SCHEMA_VERSION,
        "type": "validation",
        "valid": valid,
        "files": files,
        "errors": errors,
    }));
    !valid
}

fn workflow_entry(path: &Path) -> Value {
    match wrkflw_parser::workflow::parse_workflow(path) {
        Ok(workflow) => {
            let mut jobs: Vec<&String> = workflow.jobs.keys().collect();
            jobs.sort();
            json!({"path": path, "kind": "github", "name": workflow.name, "jobs": jobs})
        }
        Err(error) => json!({"path": path, "kind": "github", "error": error}),
    }
}

fn pipeline_entry(path: &Path) -> Value {
    match wrkflw_parser::gitlab::parse_pipeline(path) {
        Ok(pipeline) => {
            let mut jobs: Vec<&String> = pipeline.jobs.keys().collect();
            jobs.sort();
            json!({"path": path, "kind": "gitlab", "jobs": jobs})
        }
        Err(error) => json!({"path": path, "kind": "gitlab", "error": error.to_string()}),
    }
}

/// Print every workflow and pipeline in the repository as one JSON document.
pub(crate) fn list() {
    let mut workflows = Vec::new();
    for dir in wrkflw_utils::default_workflow_dirs() {
        match yaml_files(&dir) {
            Ok(files) => workflows.extend(files.iter().map(|f| workflow_entry(f))),
            Err(e) => wrkflw_logging::warning(&format!("{}: {e}", dir.display())),
        }
    }
    let gitlab = Path::new(".gitlab-ci.yml");
    if gitlab.is_file() {
        workflows.push(pipeline_entry(gitlab));
    }
    wrkflw_runs::emit_line(&json!({
        "schema_version": SCHEMA_VERSION,
        "type": "workflows",
        "workflows": workflows,
    }));
}
