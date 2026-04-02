## wrkflw-evaluator

Small, focused helper for statically evaluating GitHub Actions workflow files.

- **Purpose**: Fast structural checks (e.g., `name`, `on`, `jobs`) and composite action input cross-checking before deeper validation/execution
- **Used by**: `wrkflw` CLI and TUI during validation flows

### Example

```rust
use std::path::Path;

let result = wrkflw_evaluator::evaluate_workflow_file(
    Path::new(".github/workflows/ci.yml"),
    /* verbose */ true,
).expect("evaluation failed");

if result.is_valid {
    println!("Workflow looks structurally sound");
} else {
    for issue in result.issues {
        println!("- {}", issue);
    }
}
```

### Notes
- This crate focuses on structural checks; deeper rules live in `wrkflw-validators`.
- Most consumers should prefer the top-level `wrkflw` CLI for end-to-end UX.
