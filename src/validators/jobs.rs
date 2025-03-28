use crate::models::ValidationResult;
use crate::validators::validate_steps;
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
                    // Check for required 'runs-on'
                    if !job_config.contains_key(&Value::String("runs-on".to_string())) {
                        result.add_issue(format!("Job '{}' is missing 'runs-on' field", job_name));
                    }

                    // Check for steps
                    match job_config.get(&Value::String("steps".to_string())) {
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

                    // Check for job dependencies
                    if let Some(Value::Sequence(needs)) =
                        job_config.get(&Value::String("needs".to_string()))
                    {
                        for need in needs {
                            if let Some(need_str) = need.as_str() {
                                if !jobs_map.contains_key(&Value::String(need_str.to_string())) {
                                    result.add_issue(format!(
                                        "Job '{}' depends on non-existent job '{}'",
                                        job_name, need_str
                                    ));
                                }
                            }
                        }
                    } else if let Some(Value::String(need)) =
                        job_config.get(&Value::String("needs".to_string()))
                    {
                        if !jobs_map.contains_key(&Value::String(need.clone())) {
                            result.add_issue(format!(
                                "Job '{}' depends on non-existent job '{}'",
                                job_name, need
                            ));
                        }
                    }
                } else {
                    result.add_issue(format!("Job '{}' configuration is not a mapping", job_name));
                }
            }
        }
    }
}
