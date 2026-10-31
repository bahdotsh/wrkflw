use serde_yaml::Value;
use wrkflw_models::ValidationResult;

pub fn validate_matrix(matrix: &Value, result: &mut ValidationResult) {
    // A whole matrix given as an expression is only known at run time
    if let Some(expr) = matrix.as_str().filter(|s| wrkflw_matrix::is_expression(s)) {
        if let Err(e) = wrkflw_matrix::check_strategy_expression(expr) {
            result.add_issue(format!("matrix {}", e));
        }
        return;
    }

    // Check if matrix is a mapping
    if !matrix.is_mapping() {
        result.add_issue("Matrix must be a mapping".to_string());
        return;
    }

    // Check for include and exclude sections
    if let Some(include) = matrix.get("include") {
        validate_include_exclude(include, "include", result);
    }

    if let Some(exclude) = matrix.get("exclude") {
        validate_include_exclude(exclude, "exclude", result);
    }

    // Check max-parallel
    if let Some(max_parallel) = matrix.get("max-parallel") {
        if !max_parallel.is_number() {
            result.add_issue("max-parallel must be a number".to_string());
        } else if let Some(value) = max_parallel.as_u64() {
            if value == 0 {
                result.add_issue("max-parallel must be greater than 0".to_string());
            }
        }
    }

    // Check fail-fast
    if let Some(fail_fast) = matrix.get("fail-fast") {
        if !fail_fast.is_bool() {
            result.add_issue("fail-fast must be a boolean".to_string());
        }
    }

    // Validate the main matrix parameters (excluding special keywords)
    let special_keys = ["include", "exclude", "max-parallel", "fail-fast"];

    // Use if let to avoid unwrap
    if let Some(mapping) = matrix.as_mapping() {
        for (key, value) in mapping {
            // Safely get the key string, using an empty string as fallback
            let key_str = key.as_str().unwrap_or("");
            if !special_keys.contains(&key_str) {
                validate_matrix_parameter(key_str, value, result);
            }
        }
    } else {
        // This is a safeguard, though we already checked if it's a mapping above
        result.add_issue("Failed to process matrix mapping".to_string());
    }
}

fn validate_include_exclude(section: &Value, section_name: &str, result: &mut ValidationResult) {
    // An expression such as `${{ fromJSON(...) }}` is only known at run time
    if let Some(expr) = section.as_str().filter(|s| wrkflw_matrix::is_expression(s)) {
        if let Err(e) = wrkflw_matrix::check_strategy_expression(expr) {
            result.add_issue(format!("{} {}", section_name, e));
        }
        return;
    }

    if !section.is_sequence() {
        result.add_issue(format!("{} must be an array of objects", section_name));
        return;
    }

    // Check each item in the include/exclude array
    // Use if let to avoid unwrap
    if let Some(sequence) = section.as_sequence() {
        for (index, item) in sequence.iter().enumerate() {
            if !item.is_mapping() {
                result.add_issue(format!(
                    "{} item at index {} must be an object",
                    section_name, index
                ));
            }
        }
    } else {
        // This is a safeguard, though we already checked if it's a sequence above
        result.add_issue(format!("Failed to process {} sequence", section_name));
    }
}

fn validate_matrix_parameter(name: &str, value: &Value, result: &mut ValidationResult) {
    // Basic matrix parameters should be arrays or simple values
    match value {
        Value::Sequence(_) => {
            // Check that each item in the array has a consistent type
            if let Some(seq) = value.as_sequence() {
                if !seq.is_empty() {
                    let first_type = get_value_type(&seq[0]);

                    for (i, item) in seq.iter().enumerate().skip(1) {
                        let item_type = get_value_type(item);
                        if item_type != first_type {
                            result.add_issue(format!(
                                "Matrix parameter '{}' has inconsistent types: item at index {} is {}, but expected {}",
                                name, i, item_type, first_type
                            ));
                        }
                    }
                }
            }
        }
        Value::Mapping(_) => {
            // For object-based parameters, make sure they have valid structure
            // Here we just check if it's a mapping, but could add more validation
        }
        // Other types (string, number, bool) are valid as single values
        _ => (),
    }
}

fn get_value_type(value: &Value) -> &'static str {
    match value {
        Value::Null => "null",
        Value::Bool(_) => "boolean",
        Value::Number(_) => "number",
        Value::String(_) => "string",
        Value::Sequence(_) => "array",
        Value::Mapping(_) => "object",
        _ => "unknown",
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn issues_for(yaml: &str) -> Vec<String> {
        let matrix: Value = serde_yaml::from_str(yaml).unwrap();
        let mut result = ValidationResult::new();
        validate_matrix(&matrix, &mut result);
        result.issues
    }

    #[test]
    fn accepts_expression_include_and_exclude() {
        let issues = issues_for(
            "os: [ubuntu]\ninclude: ${{ fromJSON(needs.a.outputs.m) }}\nexclude: ${{ fromJSON(inputs.x) }}\n",
        );
        assert!(issues.is_empty(), "{:?}", issues);
    }

    #[test]
    fn rejects_contexts_unavailable_in_strategy() {
        let issues = issues_for("os: [ubuntu]\nexclude: ${{ fromJSON(env.X) }}\n");
        assert_eq!(issues.len(), 1, "{:?}", issues);
        assert!(issues[0].starts_with("exclude "), "{:?}", issues);
        assert!(issues[0].contains("uses env"), "{:?}", issues);
    }

    #[test]
    fn accepts_a_whole_matrix_expression() {
        assert!(issues_for("${{ fromJSON(needs.a.outputs.m) }}").is_empty());
    }

    #[test]
    fn rejects_whole_matrix_expression_with_unavailable_context() {
        let issues = issues_for("${{ fromJSON(env.X) }}");
        assert_eq!(issues.len(), 1, "{:?}", issues);
        assert!(issues[0].contains("uses env"), "{:?}", issues);
    }

    #[test]
    fn rejects_plain_string_matrix() {
        assert_eq!(
            issues_for("nope"),
            vec!["Matrix must be a mapping".to_string()]
        );
    }

    #[test]
    fn rejects_plain_string_include() {
        assert_eq!(
            issues_for("os: [ubuntu]\ninclude: not-an-expression\n"),
            vec!["include must be an array of objects".to_string()]
        );
    }

    #[test]
    fn rejects_non_object_include_items() {
        assert_eq!(
            issues_for("os: [ubuntu]\ninclude:\n  - ubuntu\n"),
            vec!["include item at index 0 must be an object".to_string()]
        );
    }
}
