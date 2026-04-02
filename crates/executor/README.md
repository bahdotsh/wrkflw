## wrkflw-executor

The execution engine that runs GitHub Actions workflows locally (Docker, Podman, or emulation).

- **Features**:
  - Job graph execution with `needs` ordering and parallelism
  - Docker/Podman container steps and emulation mode
  - Basic environment/context wiring compatible with Actions
- **Used by**: `wrkflw` CLI and TUI

### API sketch

```rust
use wrkflw_executor::{execute_workflow, ExecutionConfig, RuntimeType};

let cfg = ExecutionConfig {
    runtime: RuntimeType::Docker,
    verbose: true,
    preserve_containers_on_failure: false,
    target_job: None,
};

// Path to a workflow YAML
let workflow_path = std::path::Path::new(".github/workflows/ci.yml");

let result = execute_workflow(workflow_path, cfg).await?;
println!("workflow status: {:?}", result.summary_status);
```

Prefer using the `wrkflw` binary for a complete UX across validation, execution, and logs.
