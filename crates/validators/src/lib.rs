// validators crate

mod actions;
mod gitlab;
mod jobs;
mod matrix;
mod steps;
mod triggers;

pub use actions::validate_action_reference;
pub use gitlab::validate_gitlab_pipeline;
pub use jobs::validate_jobs;
pub use matrix::validate_matrix;
pub use steps::validate_steps;
pub use triggers::validate_triggers;

use serde_yaml::Value;
use wrkflw_models::ValidationResult;

/// Check whether a YAML value is a string containing GitHub Actions expression syntax (`${{ }}`).
fn is_expression_string(v: &Value) -> bool {
    matches!(v, Value::String(s) if s.contains("${{") && s.contains("}}"))
}

/// Validate that an `env` value is a mapping (or an expression string).
/// `context` describes where the env was found, e.g. "Job 'build'" or "Job 'build', step 3".
pub fn validate_env(env: &Value, context: &str, result: &mut ValidationResult) {
    if env.is_mapping() || is_expression_string(env) {
        return;
    }
    let kind = yaml_type_name(env);
    result.add_issue(format!(
        "{}: 'env' must be a mapping of key-value pairs, not a {}",
        context, kind
    ));
}

fn yaml_type_name(v: &Value) -> &'static str {
    match v {
        Value::String(_) => "string",
        Value::Number(_) => "number",
        Value::Bool(_) => "boolean",
        Value::Sequence(_) => "sequence",
        Value::Null => "null",
        Value::Mapping(_) => "mapping",
        Value::Tagged(_) => "tagged value",
    }
}
