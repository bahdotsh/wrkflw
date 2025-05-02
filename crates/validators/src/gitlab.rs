use models::gitlab::{Job, Pipeline};
use models::ValidationResult;
use std::collections::HashMap;

/// Validate a GitLab CI/CD pipeline
pub fn validate_gitlab_pipeline(pipeline: &Pipeline) -> ValidationResult {
    let mut result = ValidationResult::new();

    // Basic structure validation
    if pipeline.jobs.is_empty() {
        result.add_issue("Pipeline must contain at least one job".to_string());
    }

    // Validate jobs
    validate_jobs(&pipeline.jobs, &mut result);

    // Validate stages if defined
    if let Some(stages) = &pipeline.stages {
        validate_stages(stages, &pipeline.jobs, &mut result);
    }

    // Validate dependencies
    validate_dependencies(&pipeline.jobs, &mut result);

    // Validate extends
    validate_extends(&pipeline.jobs, &mut result);

    // Validate artifacts
    validate_artifacts(&pipeline.jobs, &mut result);

    result
}

/// Validate GitLab CI/CD jobs
fn validate_jobs(jobs: &HashMap<String, Job>, result: &mut ValidationResult) {
    for (job_name, job) in jobs {
        // Skip template jobs
        if let Some(true) = job.template {
            continue;
        }

        // Check for script or extends
        if job.script.is_none() && job.extends.is_none() {
            result.add_issue(format!(
                "Job '{}' must have a script section or extend another job",
                job_name
            ));
        }

        // Check when value if present
        if let Some(when) = &job.when {
            match when.as_str() {
                "on_success" | "on_failure" | "always" | "manual" | "never" => {
                    // Valid when value
                }
                _ => {
                    result.add_issue(format!(
                        "Job '{}' has invalid 'when' value: '{}'. Valid values are: on_success, on_failure, always, manual, never",
                        job_name, when
                    ));
                }
            }
        }

        // Check retry configuration
        if let Some(retry) = &job.retry {
            match retry {
                models::gitlab::Retry::MaxAttempts(attempts) => {
                    if *attempts > 10 {
                        result.add_issue(format!(
                            "Job '{}' has excessive retry count: {}. Consider reducing to avoid resource waste",
                            job_name, attempts
                        ));
                    }
                }
                models::gitlab::Retry::Detailed { max, when: _ } => {
                    if *max > 10 {
                        result.add_issue(format!(
                            "Job '{}' has excessive retry count: {}. Consider reducing to avoid resource waste",
                            job_name, max
                        ));
                    }
                }
            }
        }
    }
}

/// Validate GitLab CI/CD stages
fn validate_stages(stages: &[String], jobs: &HashMap<String, Job>, result: &mut ValidationResult) {
    // Check that all jobs reference existing stages
    for (job_name, job) in jobs {
        if let Some(stage) = &job.stage {
            if !stages.contains(stage) {
                result.add_issue(format!(
                    "Job '{}' references undefined stage '{}'. Available stages are: {}",
                    job_name,
                    stage,
                    stages.join(", ")
                ));
            }
        }
    }

    // Check for unused stages
    for stage in stages {
        let used = jobs.values().any(|job| {
            if let Some(job_stage) = &job.stage {
                job_stage == stage
            } else {
                false
            }
        });

        if !used {
            result.add_issue(format!(
                "Stage '{}' is defined but not used by any job",
                stage
            ));
        }
    }
}

/// Validate GitLab CI/CD job dependencies
fn validate_dependencies(jobs: &HashMap<String, Job>, result: &mut ValidationResult) {
    for (job_name, job) in jobs {
        if let Some(dependencies) = &job.dependencies {
            for dependency in dependencies {
                if !jobs.contains_key(dependency) {
                    result.add_issue(format!(
                        "Job '{}' depends on undefined job '{}'",
                        job_name, dependency
                    ));
                } else if job_name == dependency {
                    result.add_issue(format!("Job '{}' cannot depend on itself", job_name));
                }
            }
        }
    }
}

/// Validate GitLab CI/CD job extends
fn validate_extends(jobs: &HashMap<String, Job>, result: &mut ValidationResult) {
    // Check for circular extends
    for (job_name, job) in jobs {
        if let Some(extends) = &job.extends {
            // Check that all extended jobs exist
            for extend in extends {
                if !jobs.contains_key(extend) {
                    result.add_issue(format!(
                        "Job '{}' extends undefined job '{}'",
                        job_name, extend
                    ));
                    continue;
                }

                // Check for circular extends
                let mut visited = vec![job_name.clone()];
                check_circular_extends(extend, jobs, &mut visited, result);
            }
        }
    }
}

/// Helper function to detect circular extends
fn check_circular_extends(
    job_name: &str,
    jobs: &HashMap<String, Job>,
    visited: &mut Vec<String>,
    result: &mut ValidationResult,
) {
    visited.push(job_name.to_string());

    if let Some(job) = jobs.get(job_name) {
        if let Some(extends) = &job.extends {
            for extend in extends {
                if visited.contains(&extend.to_string()) {
                    // Circular dependency detected
                    let cycle = visited
                        .iter()
                        .skip(visited.iter().position(|x| x == extend).unwrap())
                        .chain(std::iter::once(extend))
                        .cloned()
                        .collect::<Vec<_>>()
                        .join(" -> ");

                    result.add_issue(format!("Circular extends detected: {}", cycle));
                    return;
                }

                check_circular_extends(extend, jobs, visited, result);
            }
        }
    }

    visited.pop();
}

/// Validate GitLab CI/CD job artifacts
fn validate_artifacts(jobs: &HashMap<String, Job>, result: &mut ValidationResult) {
    for (job_name, job) in jobs {
        if let Some(artifacts) = &job.artifacts {
            // Check that paths are specified
            if let Some(paths) = &artifacts.paths {
                if paths.is_empty() {
                    result.add_issue(format!(
                        "Job '{}' has artifacts section with empty paths",
                        job_name
                    ));
                }
            } else {
                result.add_issue(format!(
                    "Job '{}' has artifacts section without specifying paths",
                    job_name
                ));
            }

            // Check for valid 'when' value if present
            if let Some(when) = &artifacts.when {
                match when.as_str() {
                    "on_success" | "on_failure" | "always" => {
                        // Valid when value
                    }
                    _ => {
                        result.add_issue(format!(
                            "Job '{}' has artifacts with invalid 'when' value: '{}'. Valid values are: on_success, on_failure, always",
                            job_name, when
                        ));
                    }
                }
            }
        }
    }
}
