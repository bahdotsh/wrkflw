#[cfg(test)]
mod tests {
    use crate::evaluator::evaluate_workflow_file;
    use std::fs;
    use tempfile::tempdir;

    #[test]
    fn test_reusable_workflow_validation() {
        let temp_dir = tempdir().unwrap();
        let workflow_path = temp_dir.path().join("test-workflow.yml");

        // Create a workflow file that uses reusable workflows
        let content = r#"
on:
  pull_request:
    branches:
      - main

jobs:
  call-workflow-1-in-local-repo:
    uses: octo-org/this-repo/.github/workflows/workflow-1.yml@172239021f7ba04fe7327647b213799853a9eb89
  call-workflow-2-in-local-repo:
    uses: ./path/to/workflow.yml
    with:
      username: mona
    secrets:
      token: ${{ secrets.TOKEN }}
"#;

        fs::write(&workflow_path, content).unwrap();

        // Validate the workflow
        let result = evaluate_workflow_file(&workflow_path, false).unwrap();

        // Should be valid since we've fixed the validation to handle reusable workflows
        assert!(
            result.is_valid,
            "Workflow should be valid, but got issues: {:?}",
            result.issues
        );
        assert!(result.issues.is_empty());

        // Create an invalid reusable workflow (bad format for 'uses')
        let invalid_content = r#"
on:
  pull_request:
    branches:
      - main

jobs:
  call-workflow-invalid:
    uses: invalid-format
"#;

        fs::write(&workflow_path, invalid_content).unwrap();

        // Validate the workflow
        let result = evaluate_workflow_file(&workflow_path, false).unwrap();

        // Should be invalid due to the bad format
        assert!(!result.is_valid);
        assert!(result
            .issues
            .iter()
            .any(|issue| issue.contains("Invalid reusable workflow reference format")));
    }
}
