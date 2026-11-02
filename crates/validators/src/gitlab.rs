use std::collections::HashMap;
use wrkflw_models::gitlab::{Job, Pipeline};
use wrkflw_models::ValidationResult;

/// Validate a GitLab CI/CD pipeline
pub fn validate_gitlab_pipeline(pipeline: &Pipeline) -> ValidationResult {
    let mut result = ValidationResult::new();

    // Basic structure validation
    if pipeline.jobs.is_empty() {
        result.report("empty-pipeline", "Pipeline must contain at least one job");
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
        if job.script.is_none() && job.extends.is_none() && job.trigger.is_none() {
            result.scoped(job_name.as_str(), |r| {
                r.report(
                    "job-missing-script",
                    format!(
                        "Job '{}' must have a script or trigger section or extend another job",
                        job_name
                    ),
                )
            });
        }

        // Check when value if present
        if let Some(when) = &job.when {
            match when.as_str() {
                "on_success" | "on_failure" | "always" | "manual" | "never" => {
                    // Valid when value
                }
                _ => {
                    let message = format!(
                        "Job '{}' has invalid 'when' value: '{}'. Valid values are: on_success, on_failure, always, manual, never",
                        job_name, when
                    );
                    result.scoped(job_name.as_str(), |r| {
                        r.report_at("invalid-when", "when", message)
                    });
                }
            }
        }

        // Check retry configuration
        if let Some(retry) = &job.retry {
            match retry {
                wrkflw_models::gitlab::Retry::MaxAttempts(attempts) => {
                    if *attempts > 10 {
                        let message = format!(
                            "Job '{}' has excessive retry count: {}. Consider reducing to avoid resource waste",
                            job_name, attempts
                        );
                        result.scoped(job_name.as_str(), |r| {
                            r.report_at("excessive-retry", "retry", message)
                        });
                    }
                }
                wrkflw_models::gitlab::Retry::Detailed { max, when: _ } => {
                    if *max > 10 {
                        let message = format!(
                            "Job '{}' has excessive retry count: {}. Consider reducing to avoid resource waste",
                            job_name, max
                        );
                        result.scoped(job_name.as_str(), |r| {
                            r.report_at("excessive-retry", "retry", message)
                        });
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
                result.scoped(job_name.as_str(), |r| {
                    r.report_at(
                        "undefined-stage",
                        "stage",
                        format!(
                            "Job '{}' references undefined stage '{}'. Available stages are: {}",
                            job_name,
                            stage,
                            stages.join(", ")
                        ),
                    )
                });
            }
        }
    }

    // Check for unused stages
    for (index, stage) in stages.iter().enumerate() {
        let used = jobs.values().any(|job| {
            if let Some(job_stage) = &job.stage {
                job_stage == stage
            } else {
                false
            }
        });

        if !used {
            result.scoped("stages", |r| {
                r.push_index(index);
                r.report(
                    "unused-stage",
                    format!("Stage '{}' is defined but not used by any job", stage),
                );
                r.pop_scope();
            });
        }
    }
}

/// Validate GitLab CI/CD job dependencies
fn validate_dependencies(jobs: &HashMap<String, Job>, result: &mut ValidationResult) {
    for (job_name, job) in jobs {
        if let Some(dependencies) = &job.dependencies {
            for dependency in dependencies {
                if !jobs.contains_key(dependency) {
                    result.scoped(job_name.as_str(), |r| {
                        r.report_at(
                            "undefined-dependency",
                            "dependencies",
                            format!(
                                "Job '{}' depends on undefined job '{}'",
                                job_name, dependency
                            ),
                        )
                    });
                } else if job_name == dependency {
                    result.scoped(job_name.as_str(), |r| {
                        r.report_at(
                            "self-dependency",
                            "dependencies",
                            format!("Job '{}' cannot depend on itself", job_name),
                        )
                    });
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
                    result.scoped(job_name.as_str(), |r| {
                        r.report_at(
                            "undefined-extends",
                            "extends",
                            format!("Job '{}' extends undefined job '{}'", job_name, extend),
                        )
                    });
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

                    result.scoped(job_name, |r| {
                        r.report_at(
                            "circular-extends",
                            "extends",
                            format!("Circular extends detected: {}", cycle),
                        )
                    });
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
            result.push_scope(job_name.as_str());
            result.push_scope("artifacts");

            // Check that paths or reports are specified
            let has_reports = artifacts.reports.is_some();
            if let Some(paths) = &artifacts.paths {
                if paths.is_empty() && !has_reports {
                    result.report_at(
                        "empty-artifact-paths",
                        "paths",
                        format!("Job '{}' has artifacts section with empty paths", job_name),
                    );
                }
            } else if !has_reports {
                result.report_at(
                    "empty-artifact-paths",
                    "paths",
                    format!(
                        "Job '{}' has artifacts section without specifying paths",
                        job_name
                    ),
                );
            }

            // Check for valid 'when' value if present
            if let Some(when) = &artifacts.when {
                match when.as_str() {
                    "on_success" | "on_failure" | "always" => {
                        // Valid when value
                    }
                    _ => {
                        result.report_at("invalid-when", "when", format!(
                            "Job '{}' has artifacts with invalid 'when' value: '{}'. Valid values are: on_success, on_failure, always",
                            job_name, when
                        ));
                    }
                }
            }

            result.pop_scope();
            result.pop_scope();
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use wrkflw_models::UNCLASSIFIED_RULE;

    const PIPELINE: &str = r#"stages:
  - build
  - unused
build:
  stage: ghost
  script: [make]
  when: sometimes
  retry: 11
  dependencies: [missing]
  extends: nope
  artifacts:
    paths: []
    when: nope
noscript:
  stage: build
self:
  stage: build
  script: [make]
  dependencies: [self]
  artifacts:
    when: always
loop-a:
  stage: build
  extends: loop-b
loop-b:
  stage: build
  extends: loop-a
"#;

    fn validate(source: &str) -> ValidationResult {
        let pipeline: Pipeline = serde_yaml::from_str(source).unwrap();
        let mut result = validate_gitlab_pipeline(&pipeline);
        result.locate_lines(source);
        result
    }

    #[test]
    fn job_keys_sit_at_the_document_root() {
        let result = validate(PIPELINE);

        for (rule, path, line) in [
            ("undefined-stage", "build.stage", 5),
            ("invalid-when", "build.when", 7),
            ("excessive-retry", "build.retry", 8),
            ("undefined-dependency", "build.dependencies", 9),
            ("undefined-extends", "build.extends", 10),
            ("empty-artifact-paths", "build.artifacts.paths", 12),
            ("invalid-when", "build.artifacts.when", 13),
            ("job-missing-script", "noscript", 14),
            ("self-dependency", "self.dependencies", 19),
            ("empty-artifact-paths", "self.artifacts.paths", 20),
            ("unused-stage", "stages[1]", 3),
        ] {
            assert!(
                result.details.iter().any(|i| i.rule == rule
                    && i.path.as_deref() == Some(path)
                    && i.line == Some(line)),
                "no {rule} at {path} line {line} in {:#?}",
                result.details
            );
        }

        // Which edge closes the loop depends on HashMap order.
        assert!(result.details.iter().any(|i| i.rule == "circular-extends"
            && matches!(i.path.as_deref(), Some("loop-a.extends" | "loop-b.extends"))));
    }

    #[test]
    fn every_issue_is_classified_and_matches_its_message() {
        for source in [PIPELINE, "image: alpine\n"] {
            let result = validate(source);
            assert_eq!(result.issues.len(), result.details.len());
            for (message, issue) in result.issues.iter().zip(&result.details) {
                assert_eq!(message, &issue.message);
                assert_ne!(issue.rule, UNCLASSIFIED_RULE, "{}", issue.message);
            }
        }
        assert_eq!(
            validate("image: alpine\n").details[0].rule,
            "empty-pipeline"
        );
    }
}
