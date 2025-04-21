use lazy_static::lazy_static;
use regex::Regex;
use serde_yaml::Value;
use std::collections::HashMap;

lazy_static! {
    static ref MATRIX_PATTERN: Regex =
        Regex::new(r"\$\{\{\s*matrix\.([a-zA-Z0-9_]+)\s*\}\}").unwrap();
}

/// Preprocesses a command string to replace GitHub-style matrix variable references
/// with their values from the environment
pub fn preprocess_command(command: &str, matrix_values: &HashMap<String, Value>) -> String {
    // Replace matrix references like ${{ matrix.os }} with their values
    let result = MATRIX_PATTERN.replace_all(command, |caps: &regex::Captures| {
        let var_name = &caps[1];

        // Get the value from matrix context
        if let Some(value) = matrix_values.get(var_name) {
            // Convert value to string
            match value {
                Value::String(s) => s.clone(),
                Value::Number(n) => n.to_string(),
                Value::Bool(b) => b.to_string(),
                _ => format!("\\${{{{ matrix.{} }}}}", var_name), // Escape $ for shell
            }
        } else {
            // Keep original if not found but escape $ to prevent shell errors
            format!("\\${{{{ matrix.{} }}}}", var_name)
        }
    });

    result.into_owned()
}

/// Apply variable substitution to step run commands
pub fn process_step_run(run: &str, matrix_combination: &Option<HashMap<String, Value>>) -> String {
    if let Some(matrix) = matrix_combination {
        preprocess_command(run, matrix)
    } else {
        // Escape $ in GitHub expression syntax to prevent shell interpretation
        MATRIX_PATTERN
            .replace_all(run, |caps: &regex::Captures| {
                let var_name = &caps[1];
                format!("\\${{{{ matrix.{} }}}}", var_name)
            })
            .to_string()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_preprocess_simple_matrix_vars() {
        let mut matrix = HashMap::new();
        matrix.insert("os".to_string(), Value::String("ubuntu-latest".to_string()));
        matrix.insert(
            "node".to_string(),
            Value::Number(serde_yaml::Number::from(14)),
        );

        let cmd = "echo \"Running on ${{ matrix.os }} with Node ${{ matrix.node }}\"";
        let processed = preprocess_command(cmd, &matrix);

        assert_eq!(processed, "echo \"Running on ubuntu-latest with Node 14\"");
    }

    #[test]
    fn test_preprocess_with_missing_vars() {
        let mut matrix = HashMap::new();
        matrix.insert("os".to_string(), Value::String("ubuntu-latest".to_string()));

        let cmd = "echo \"Running on ${{ matrix.os }} with Node ${{ matrix.node }}\"";
        let processed = preprocess_command(cmd, &matrix);

        // Missing vars should be escaped
        assert_eq!(
            processed,
            "echo \"Running on ubuntu-latest with Node \\${{ matrix.node }}\""
        );
    }

    #[test]
    fn test_preprocess_preserves_other_text() {
        let mut matrix = HashMap::new();
        matrix.insert("os".to_string(), Value::String("ubuntu-latest".to_string()));

        let cmd = "echo \"Starting job\" && echo \"OS: ${{ matrix.os }}\" && echo \"Done!\"";
        let processed = preprocess_command(cmd, &matrix);

        assert_eq!(
            processed,
            "echo \"Starting job\" && echo \"OS: ubuntu-latest\" && echo \"Done!\""
        );
    }

    #[test]
    fn test_process_without_matrix() {
        let cmd = "echo \"Value: ${{ matrix.value }}\"";
        let processed = process_step_run(cmd, &None);

        assert_eq!(processed, "echo \"Value: \\${{ matrix.value }}\"");
    }
}
