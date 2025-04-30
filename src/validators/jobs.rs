use crate::models::ValidationResult;
use crate::validators::{validate_matrix, validate_steps};
use serde_yaml::Value;

pub fn validate_jobs(jobs: &Value, result: &mut ValidationResult) {
    if let Value::Mapping(jobs_map) = jobs {
        if jobs_map.is_empty() {
            result.add_issue("'jobs' section is empty".to_string());
            return;
        }

        for (job_name, job_config) in jobs_map {
            if let Some(job_name) = job_name.as_str() {
                if let Some(job_config) = job_config.as_mapping() {
                    // Check if this is a reusable workflow job (has 'uses' field)
                    let is_reusable_workflow =
                        job_config.contains_key(Value::String("uses".to_string()));

                    // Only check for 'runs-on' if it's not a reusable workflow
                    if !is_reusable_workflow
                        && !job_config.contains_key(Value::String("runs-on".to_string()))
                    {
                        result.add_issue(format!("Job '{}' is missing 'runs-on' field", job_name));
                    }

                    // Only check for steps if it's not a reusable workflow
                    if !is_reusable_workflow {
                        match job_config.get(Value::String("steps".to_string())) {
                            Some(Value::Sequence(steps)) => {
                                if steps.is_empty() {
                                    result.add_issue(format!(
                                        "Job '{}' has empty 'steps' section",
                                        job_name
                                    ));
                                } else {
                                    validate_steps(steps, job_name, result);
                                }
                            }
                            Some(_) => {
                                result.add_issue(format!(
                                    "Job '{}': 'steps' section is not a sequence",
                                    job_name
                                ));
                            }
                            None => {
                                result.add_issue(format!(
                                    "Job '{}' is missing 'steps' section",
                                    job_name
                                ));
                            }
                        }
                    } else {
                        // For reusable workflows, validate the 'uses' field format
                        if let Some(Value::String(uses)) =
                            job_config.get(Value::String("uses".to_string()))
                        {
                            // Simple validation for reusable workflow reference format
                            if !uses.contains('/') || !uses.contains('.') {
                                result.add_issue(format!(
                                    "Job '{}': Invalid reusable workflow reference format '{}'",
                                    job_name, uses
                                ));
                            }
                        }
                    }

                    // Check for job dependencies
                    if let Some(Value::Sequence(needs)) =
                        job_config.get(Value::String("needs".to_string()))
                    {
                        for need in needs {
                            if let Some(need_str) = need.as_str() {
                                if !jobs_map.contains_key(Value::String(need_str.to_string())) {
                                    result.add_issue(format!(
                                        "Job '{}' depends on non-existent job '{}'",
                                        job_name, need_str
                                    ));
                                }
                            }
                        }
                    } else if let Some(Value::String(need)) =
                        job_config.get(Value::String("needs".to_string()))
                    {
                        if !jobs_map.contains_key(Value::String(need.clone())) {
                            result.add_issue(format!(
                                "Job '{}' depends on non-existent job '{}'",
                                job_name, need
                            ));
                        }
                    }

                    // Validate matrix configuration if present
                    if let Some(matrix) = job_config.get(Value::String("matrix".to_string())) {
                        validate_matrix(matrix, result);
                    }
                } else {
                    result.add_issue(format!("Job '{}' configuration is not a mapping", job_name));
                }
            }
        }
    }
}
