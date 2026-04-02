## wrkflw-executor

The execution engine that runs GitHub Actions workflows locally (Docker, Podman, or emulation).

- Job graph execution with `needs` ordering and parallel independent jobs
- Docker/Podman container steps and emulation mode
- Run individual jobs via `target_job` / `--job` flag
- GitHub Actions environment file support (`GITHUB_OUTPUT`, `GITHUB_ENV`, `GITHUB_PATH`, `GITHUB_STEP_SUMMARY`) with read-back
- Docker-based action resolution (container, JavaScript, composite, local)
- Job-level `container:` directive support
- **Used by**: `wrkflw` CLI and TUI

### API sketch

```rust
use wrkflw_executor::{execute_workflow, ExecutionConfig, RuntimeType};

let cfg = ExecutionConfig {
    runtime: RuntimeType::Docker,
    verbose: true,
    preserve_containers_on_failure: false,
    target_job: Some("build".to_string()), // run a single job
};

let workflow_path = std::path::Path::new(".github/workflows/ci.yml");
let result = execute_workflow(workflow_path, cfg).await?;
println!("workflow status: {:?}", result.summary_status);
```

Prefer using the `wrkflw` binary for a complete UX across validation, execution, and logs.
