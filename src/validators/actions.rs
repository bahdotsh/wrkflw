use crate::models::ValidationResult;

pub fn validate_action_reference(
    action_ref: &str,
    job_name: &str,
    step_idx: usize,
    result: &mut ValidationResult,
) {
    // Check for valid action reference formats
    if !action_ref.contains('/') && !action_ref.contains('.') {
        result.add_issue(format!(
            "Job '{}', step {}: Invalid action reference format '{}'",
            job_name,
            step_idx + 1,
            action_ref
        ));
        return;
    }

    // Check for version tag or commit SHA
    if action_ref.contains('@') {
        let parts: Vec<&str> = action_ref.split('@').collect();
        if parts.len() != 2 || parts[1].is_empty() {
            result.add_issue(format!(
                "Job '{}', step {}: Action '{}' has invalid version/ref format",
                job_name,
                step_idx + 1,
                action_ref
            ));
        }
    } else {
        // Missing version tag is not recommended
        result.add_issue(format!(
            "Job '{}', step {}: Action '{}' is missing version tag (@v2, @main, etc.)",
            job_name,
            step_idx + 1,
            action_ref
        ));
    }
}
