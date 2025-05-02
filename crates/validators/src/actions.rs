use models::ValidationResult;

pub fn validate_action_reference(
    action_ref: &str,
    job_name: &str,
    step_idx: usize,
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

    // For local actions, verify the path exists
    if is_local_action {
        let action_path = std::path::Path::new(action_ref);
        if !action_path.exists() {
            // We can't reliably check this during validation since the working directory
            // might not be the repository root, but we'll add a warning
            result.add_issue(format!(
                "Job '{}', step {}: Local action path '{}' may not exist at runtime",
                job_name,
                step_idx + 1,
                action_ref
            ));
        }
    }
}
