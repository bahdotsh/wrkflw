use serde_yaml::Value;
use crate::models::ValidationResult;

pub fn validate_matrix(matrix: &Value, result: &mut ValidationResult) {
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
    for (key, value) in matrix.as_mapping().unwrap() {
        let key_str = key.as_str().unwrap_or("");
        if !special_keys.contains(&key_str) {
            validate_matrix_parameter(key_str, value, result);
        }
    }
}

fn validate_include_exclude(section: &Value, section_name: &str, result: &mut ValidationResult) {
    if !section.is_sequence() {
        result.add_issue(format!("{} must be an array of objects", section_name));
        return;
    }

    // Check each item in the include/exclude array
    for (index, item) in section.as_sequence().unwrap().iter().enumerate() {
        if !item.is_mapping() {
            result.add_issue(format!(
                "{} item at index {} must be an object",
                section_name, index
            ));
        }
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