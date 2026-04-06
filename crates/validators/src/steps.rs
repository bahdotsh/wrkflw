use crate::{validate_action_reference, validate_env};
use serde_yaml::Value;
use std::collections::HashSet;
use std::path::Path;
use wrkflw_models::ValidationResult;

pub fn validate_steps(
    steps: &[Value],
    job_name: &str,
    repo_root: Option<&Path>,
    result: &mut ValidationResult,
) {
    let mut step_ids: HashSet<String> = HashSet::new();

    for (i, step) in steps.iter().enumerate() {
        if let Some(step_map) = step.as_mapping() {
            // A step must have either 'uses' or 'run' (name alone is not sufficient)
            if !step_map.contains_key(Value::String("uses".to_string()))
                && !step_map.contains_key(Value::String("run".to_string()))
            {
                result.add_issue(format!(
                    "Job '{}', step {}: Missing required 'uses' or 'run' field",
                    job_name,
                    i + 1
                ));
            }

            // Check for both 'uses' and 'run' in the same step
            if step_map.contains_key(Value::String("uses".to_string()))
                && step_map.contains_key(Value::String("run".to_string()))
            {
                result.add_issue(format!(
                    "Job '{}', step {}: Contains both 'uses' and 'run' (should only use one)",
                    job_name,
                    i + 1
                ));
            }

            // Check for duplicate step IDs
            if let Some(Value::String(id)) = step_map.get(Value::String("id".to_string())) {
                if !step_ids.insert(id.clone()) {
                    result.add_issue(format!(
                        "Job '{}', step {}: The identifier '{}' may not be used more than once within the same scope",
                        job_name,
                        i + 1,
                        id
                    ));
                }
            }

            // Validate env is a mapping, not a bare string
            if let Some(env_val) = step_map.get(Value::String("env".to_string())) {
                validate_env(
                    env_val,
                    &format!("Job '{}', step {}", job_name, i + 1),
                    result,
                );
            }

            // Validate action reference if 'uses' is present
            if let Some(Value::String(uses)) = step_map.get(Value::String("uses".to_string())) {
                let with_params = step_map
                    .get(Value::String("with".to_string()))
                    .and_then(|v| v.as_mapping());
                validate_action_reference(uses, with_params, job_name, i, repo_root, result);
            }
        } else {
            result.add_issue(format!(
                "Job '{}', step {}: Not a valid mapping",
                job_name,
                i + 1
            ));
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use wrkflw_models::ValidationResult;

    #[test]
    fn test_step_with_only_name_is_invalid() {
        let yaml = r#"
- name: "just a name"
"#;
        let steps: Vec<Value> = serde_yaml::from_str(yaml).unwrap();
        let mut result = ValidationResult::new();
        validate_steps(&steps, "test-job", None, &mut result);

        assert!(!result.is_valid);
        assert!(result
            .issues
            .iter()
            .any(|i| i.contains("Missing required 'uses' or 'run' field")));
    }

    #[test]
    fn test_step_with_run_is_valid() {
        let yaml = r#"
- name: "build"
  run: "cargo build"
"#;
        let steps: Vec<Value> = serde_yaml::from_str(yaml).unwrap();
        let mut result = ValidationResult::new();
        validate_steps(&steps, "test-job", None, &mut result);

        assert!(result.is_valid);
        assert!(result.issues.is_empty());
    }

    #[test]
    fn test_step_env_string_is_invalid() {
        let yaml = r#"
- name: "build"
  run: "cargo build"
  env: VAR=value
"#;
        let steps: Vec<Value> = serde_yaml::from_str(yaml).unwrap();
        let mut result = ValidationResult::new();
        validate_steps(&steps, "test-job", None, &mut result);

        assert!(!result.is_valid);
        assert!(result
            .issues
            .iter()
            .any(|i| i.contains("'env' must be a mapping")));
    }

    #[test]
    fn test_step_env_mapping_is_valid() {
        let yaml = r#"
- name: "build"
  run: "cargo build"
  env:
    MY_VAR: my_value
"#;
        let steps: Vec<Value> = serde_yaml::from_str(yaml).unwrap();
        let mut result = ValidationResult::new();
        validate_steps(&steps, "test-job", None, &mut result);

        assert!(result.is_valid);
        assert!(result.issues.is_empty());
    }

    #[test]
    fn test_step_env_expression_is_valid() {
        let yaml = r#"
- name: "build"
  run: "cargo build"
  env: ${{ fromJSON(needs.setup.outputs.env) }}
"#;
        let steps: Vec<Value> = serde_yaml::from_str(yaml).unwrap();
        let mut result = ValidationResult::new();
        validate_steps(&steps, "test-job", None, &mut result);

        assert!(result.is_valid);
        assert!(result.issues.is_empty());
    }

    #[test]
    fn test_step_with_uses_is_valid() {
        let yaml = r#"
- name: "checkout"
  uses: "actions/checkout@v4"
"#;
        let steps: Vec<Value> = serde_yaml::from_str(yaml).unwrap();
        let mut result = ValidationResult::new();
        validate_steps(&steps, "test-job", None, &mut result);

        assert!(result.is_valid);
        assert!(result.issues.is_empty());
    }
}
