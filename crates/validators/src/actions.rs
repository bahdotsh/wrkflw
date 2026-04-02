use std::collections::HashSet;
use std::path::Path;
use wrkflw_models::ValidationResult;

pub fn validate_action_reference(
    action_ref: &str,
    with_params: Option<&serde_yaml::Mapping>,
    job_name: &str,
    step_idx: usize,
    repo_root: Option<&Path>,
    result: &mut ValidationResult,
) {
    // Check if it's a local action (starts with ./)
    let is_local_action = action_ref.starts_with("./");

    // For non-local actions, enforce standard format
    if !is_local_action && !action_ref.contains('/') && !action_ref.contains('.') {
        result.add_issue(format!(
            "Job '{}', step {}: Invalid action reference format '{}'",
            job_name,
            step_idx + 1,
            action_ref
        ));
        return;
    }

    // Check for version tag or commit SHA, but only for non-local actions
    if !is_local_action && action_ref.contains('@') {
        let parts: Vec<&str> = action_ref.split('@').collect();
        if parts.len() != 2 || parts[1].is_empty() {
            result.add_issue(format!(
                "Job '{}', step {}: Action '{}' has invalid version/ref format",
                job_name,
                step_idx + 1,
                action_ref
            ));
        }
    } else if !is_local_action {
        // Missing version tag is not recommended for non-local actions
        result.add_issue(format!(
            "Job '{}', step {}: Action '{}' is missing version tag (@v2, @main, etc.)",
            job_name,
            step_idx + 1,
            action_ref
        ));
    }

    // For local actions, validate the path and cross-check inputs
    if is_local_action {
        if let Some(root) = repo_root {
            let relative_path = action_ref.strip_prefix("./").unwrap_or(action_ref);
            let action_dir = root.join(relative_path);

            let action_file = {
                let yml = action_dir.join("action.yml");
                let yaml = action_dir.join("action.yaml");
                if yml.exists() {
                    Some(yml)
                } else if yaml.exists() {
                    Some(yaml)
                } else {
                    None
                }
            };

            match action_file {
                None => {
                    if !action_dir.exists() {
                        result.add_issue(format!(
                            "Job '{}', step {}: Local action path '{}' does not exist",
                            job_name,
                            step_idx + 1,
                            action_ref
                        ));
                    } else {
                        result.add_issue(format!(
                            "Job '{}', step {}: No action.yml or action.yaml found in '{}'",
                            job_name,
                            step_idx + 1,
                            action_ref
                        ));
                    }
                }
                Some(action_file_path) => {
                    validate_local_action_inputs(
                        &action_file_path,
                        with_params,
                        action_ref,
                        job_name,
                        step_idx,
                        result,
                    );
                }
            }
        }
    }
}

/// Parse a local action's action.yml and validate that required inputs are provided.
fn validate_local_action_inputs(
    action_file: &Path,
    with_params: Option<&serde_yaml::Mapping>,
    action_ref: &str,
    job_name: &str,
    step_idx: usize,
    result: &mut ValidationResult,
) {
    let content = match std::fs::read_to_string(action_file) {
        Ok(c) => c,
        Err(_) => return,
    };

    let action_def: serde_yaml::Value = match serde_yaml::from_str(&content) {
        Ok(v) => v,
        Err(_) => {
            result.add_issue(format!(
                "Job '{}', step {}: Failed to parse action definition at '{}'",
                job_name,
                step_idx + 1,
                action_ref
            ));
            return;
        }
    };

    let inputs_map = match action_def.get("inputs").and_then(|v| v.as_mapping()) {
        Some(m) => m,
        None => return,
    };

    let provided_keys: HashSet<String> = with_params
        .map(|m| {
            m.keys()
                .filter_map(|k| k.as_str())
                .map(|s| s.to_lowercase())
                .collect()
        })
        .unwrap_or_default();

    for (input_name, input_def) in inputs_map {
        let input_name_str = match input_name.as_str() {
            Some(s) => s,
            None => continue,
        };

        let is_required = input_def
            .get("required")
            .map(|v| match v {
                serde_yaml::Value::Bool(b) => *b,
                serde_yaml::Value::String(s) => s.eq_ignore_ascii_case("true"),
                _ => false,
            })
            .unwrap_or(false);

        let has_default = input_def.get("default").is_some();

        if is_required && !has_default && !provided_keys.contains(&input_name_str.to_lowercase()) {
            result.add_issue(format!(
                "Job '{}', step {}: Local action '{}' requires input '{}' but it is not provided in 'with'",
                job_name,
                step_idx + 1,
                action_ref,
                input_name_str
            ));
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;
    use tempfile::tempdir;

    #[test]
    fn test_missing_required_input() {
        let dir = tempdir().unwrap();
        let action_dir = dir.path().join("my-action");
        fs::create_dir(&action_dir).unwrap();
        fs::write(
            action_dir.join("action.yml"),
            r#"
name: 'Test Action'
inputs:
  token:
    description: 'A required token'
    required: true
runs:
  using: 'composite'
  steps:
    - run: echo hello
"#,
        )
        .unwrap();

        let mut result = ValidationResult::new();
        validate_action_reference(
            "./my-action",
            None,
            "build",
            0,
            Some(dir.path()),
            &mut result,
        );

        assert!(!result.is_valid);
        assert!(result
            .issues
            .iter()
            .any(|i| i.contains("requires input 'token'")));
    }

    #[test]
    fn test_required_input_provided() {
        let dir = tempdir().unwrap();
        let action_dir = dir.path().join("my-action");
        fs::create_dir(&action_dir).unwrap();
        fs::write(
            action_dir.join("action.yml"),
            r#"
name: 'Test Action'
inputs:
  token:
    description: 'A required token'
    required: true
runs:
  using: 'composite'
  steps:
    - run: echo hello
"#,
        )
        .unwrap();

        let mut with = serde_yaml::Mapping::new();
        with.insert(
            serde_yaml::Value::String("token".to_string()),
            serde_yaml::Value::String("my-secret".to_string()),
        );

        let mut result = ValidationResult::new();
        validate_action_reference(
            "./my-action",
            Some(&with),
            "build",
            0,
            Some(dir.path()),
            &mut result,
        );

        assert!(result.is_valid);
        assert!(result.issues.is_empty());
    }

    #[test]
    fn test_required_input_with_default_passes() {
        let dir = tempdir().unwrap();
        let action_dir = dir.path().join("my-action");
        fs::create_dir(&action_dir).unwrap();
        fs::write(
            action_dir.join("action.yml"),
            r#"
name: 'Test Action'
inputs:
  token:
    description: 'Has a default'
    required: true
    default: 'fallback'
runs:
  using: 'composite'
  steps:
    - run: echo hello
"#,
        )
        .unwrap();

        let mut result = ValidationResult::new();
        validate_action_reference(
            "./my-action",
            None,
            "build",
            0,
            Some(dir.path()),
            &mut result,
        );

        assert!(result.is_valid);
    }

    #[test]
    fn test_no_inputs_section_passes() {
        let dir = tempdir().unwrap();
        let action_dir = dir.path().join("my-action");
        fs::create_dir(&action_dir).unwrap();
        fs::write(
            action_dir.join("action.yml"),
            r#"
name: 'Test Action'
runs:
  using: 'composite'
  steps:
    - run: echo hello
"#,
        )
        .unwrap();

        let mut result = ValidationResult::new();
        validate_action_reference(
            "./my-action",
            None,
            "build",
            0,
            Some(dir.path()),
            &mut result,
        );

        assert!(result.is_valid);
    }

    #[test]
    fn test_action_dir_does_not_exist() {
        let dir = tempdir().unwrap();

        let mut result = ValidationResult::new();
        validate_action_reference(
            "./nonexistent",
            None,
            "build",
            0,
            Some(dir.path()),
            &mut result,
        );

        assert!(!result.is_valid);
        assert!(result.issues.iter().any(|i| i.contains("does not exist")));
    }

    #[test]
    fn test_action_dir_exists_but_no_action_yml() {
        let dir = tempdir().unwrap();
        let action_dir = dir.path().join("my-action");
        fs::create_dir(&action_dir).unwrap();

        let mut result = ValidationResult::new();
        validate_action_reference(
            "./my-action",
            None,
            "build",
            0,
            Some(dir.path()),
            &mut result,
        );

        assert!(!result.is_valid);
        assert!(result
            .issues
            .iter()
            .any(|i| i.contains("No action.yml or action.yaml")));
    }

    #[test]
    fn test_case_insensitive_input_matching() {
        let dir = tempdir().unwrap();
        let action_dir = dir.path().join("my-action");
        fs::create_dir(&action_dir).unwrap();
        fs::write(
            action_dir.join("action.yml"),
            r#"
name: 'Test Action'
inputs:
  My-Token:
    description: 'Mixed case'
    required: true
runs:
  using: 'composite'
  steps:
    - run: echo hello
"#,
        )
        .unwrap();

        let mut with = serde_yaml::Mapping::new();
        with.insert(
            serde_yaml::Value::String("my-token".to_string()),
            serde_yaml::Value::String("value".to_string()),
        );

        let mut result = ValidationResult::new();
        validate_action_reference(
            "./my-action",
            Some(&with),
            "build",
            0,
            Some(dir.path()),
            &mut result,
        );

        assert!(result.is_valid);
    }

    #[test]
    fn test_multiple_missing_required_inputs() {
        let dir = tempdir().unwrap();
        let action_dir = dir.path().join("my-action");
        fs::create_dir(&action_dir).unwrap();
        fs::write(
            action_dir.join("action.yml"),
            r#"
name: 'Test Action'
inputs:
  token:
    description: 'Required'
    required: true
  name:
    description: 'Also required'
    required: true
  optional:
    description: 'Not required'
    required: false
runs:
  using: 'composite'
  steps:
    - run: echo hello
"#,
        )
        .unwrap();

        let mut result = ValidationResult::new();
        validate_action_reference(
            "./my-action",
            None,
            "build",
            0,
            Some(dir.path()),
            &mut result,
        );

        assert!(!result.is_valid);
        assert_eq!(
            result
                .issues
                .iter()
                .filter(|i| i.contains("requires input"))
                .count(),
            2
        );
    }

    #[test]
    fn test_required_as_string_true() {
        let dir = tempdir().unwrap();
        let action_dir = dir.path().join("my-action");
        fs::create_dir(&action_dir).unwrap();
        fs::write(
            action_dir.join("action.yml"),
            r#"
name: 'Test Action'
inputs:
  token:
    description: 'Required as string'
    required: 'true'
runs:
  using: 'composite'
  steps:
    - run: echo hello
"#,
        )
        .unwrap();

        let mut result = ValidationResult::new();
        validate_action_reference(
            "./my-action",
            None,
            "build",
            0,
            Some(dir.path()),
            &mut result,
        );

        assert!(!result.is_valid);
        assert!(result
            .issues
            .iter()
            .any(|i| i.contains("requires input 'token'")));
    }

    #[test]
    fn test_repo_root_none_skips_validation() {
        let mut result = ValidationResult::new();
        validate_action_reference("./my-action", None, "build", 0, None, &mut result);

        assert!(result.is_valid);
    }

    #[test]
    fn test_action_yaml_extension() {
        let dir = tempdir().unwrap();
        let action_dir = dir.path().join("my-action");
        fs::create_dir(&action_dir).unwrap();
        fs::write(
            action_dir.join("action.yaml"),
            r#"
name: 'Test Action'
inputs:
  token:
    description: 'Required'
    required: true
runs:
  using: 'composite'
  steps:
    - run: echo hello
"#,
        )
        .unwrap();

        let mut result = ValidationResult::new();
        validate_action_reference(
            "./my-action",
            None,
            "build",
            0,
            Some(dir.path()),
            &mut result,
        );

        assert!(!result.is_valid);
        assert!(result
            .issues
            .iter()
            .any(|i| i.contains("requires input 'token'")));
    }
}
