## wrkflw-validators

Validation utilities for workflows and steps.

- Validates GitHub Actions sections: jobs, steps, actions references, triggers
- GitLab pipeline validation helpers
- Matrix-specific validation

### Example

```rust
use serde_yaml::Value;
use wrkflw_models::ValidationResult;
use wrkflw_validators::{validate_jobs, validate_triggers};

let yaml: Value = serde_yaml::from_str(r#"name: demo
on: [workflow_dispatch]
jobs: { build: { runs-on: ubuntu-latest, steps: [] } }
"#).unwrap();

let mut res = ValidationResult::new();
if let Some(on) = yaml.get("on") {
    validate_triggers(on, &mut res);
}
if let Some(jobs) = yaml.get("jobs") {
    // The repository root lets local `uses: ./...` actions be checked; `None` skips that.
    validate_jobs(jobs, None, &mut res);
}
assert!(res.is_valid);
```
