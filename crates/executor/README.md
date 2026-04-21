## wrkflw-executor

The execution engine that runs GitHub Actions workflows locally (Docker, Podman, emulation, or secure emulation).

- Job graph execution with `needs` ordering and parallel independent jobs
- Docker/Podman container steps, emulation, and sandboxed secure emulation
- Run individual jobs via `target_job` / `--job` flag
- GitHub Actions environment file support (`GITHUB_OUTPUT`, `GITHUB_ENV`, `GITHUB_PATH`, `GITHUB_STEP_SUMMARY`) with read-back
- `${{ ... }}` expression evaluator (`toJSON`, `fromJSON`, `contains`, `startsWith`, `success()`, `failure()`, etc.) with GitHub / env / matrix / secrets / needs / steps context
- Action resolution for container, JavaScript, composite (with output propagation), and local actions
- Job-level `container:` directive support
- Local `actions/upload-artifact`, `actions/download-artifact`, and `actions/cache` via shared artifact and cache stores
- Reusable workflow execution (`jobs.<id>.uses`, local or `owner/repo/path@ref`) with output aggregation into `needs.<id>.outputs.*`
- **Used by**: `wrkflw` CLI and TUI

### API sketch

```rust
use wrkflw_executor::{execute_workflow, ExecutionConfig, RuntimeType};

let cfg = ExecutionConfig {
    runtime_type: RuntimeType::Docker,
    verbose: true,
    preserve_containers_on_failure: false,
    secrets_config: None,
    show_action_messages: false,
    target_job: Some("build".to_string()), // run a single job
};

let workflow_path = std::path::Path::new(".github/workflows/ci.yml");
let result = execute_workflow(workflow_path, cfg).await?;
for job in &result.jobs {
    println!("{}: {:?}", job.name, job.status);
}
```

Prefer using the `wrkflw` binary for a complete UX across validation, execution, and logs.
