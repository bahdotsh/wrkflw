## wrkflw-executor

The execution engine that runs GitHub Actions, Gitea and GitLab CI workflows locally (Docker, Podman, Kubernetes, emulation, or secure emulation).

- Job graph execution with `needs` ordering and parallel independent jobs
- Docker/Podman container steps, emulation, and sandboxed secure emulation
- Run individual jobs via `target_job` / `--job` flag
- GitHub Actions environment file support (`GITHUB_OUTPUT`, `GITHUB_ENV`, `GITHUB_PATH`, `GITHUB_STEP_SUMMARY`) with read-back
- `${{ ... }}` expression evaluator (`toJSON`, `fromJSON`, `contains`, `startsWith`, `success()`, `failure()`, etc.) with GitHub / env / matrix / secrets / needs / steps context
- Action resolution for container, JavaScript, composite (with output propagation), and local actions
- Job-level `container:` directive support, `services:` on Docker and Podman with health checks, and `--platform` image overrides for `runs-on`
- Job-level `continue-on-error` (also as an expression) and job-level `concurrency:` groups within a run
- Matrices computed by expressions, including from an earlier job's outputs
- Result reuse (`acceleration.reuse_results`): a job whose fingerprint matches a passing run is skipped, with its outputs replayed, from a local store or a signed remote cache
- Affected-job skipping from `.wrkflw/affected.yml`, shared dependency caches (`acceleration.dep_cache`), and the opt-outs and opt-ins of one container per job (`acceleration.step_containers`, `acceleration.docker_in_jobs`)
- Kubernetes execution: each step runs as a pod in the cluster `kubectl` points at, with the workspace synced in and out
- GitLab CI pipelines, including `parallel`, `parallel:matrix`, `rules`, `when` and stages
- Local `actions/upload-artifact`, `actions/download-artifact`, and `actions/cache` via shared artifact and cache stores
- Reusable workflow execution (`jobs.<id>.uses`, local or `owner/repo/path@ref`) with output aggregation into `needs.<id>.outputs.*`
- **Used by**: `wrkflw` CLI and TUI

### API sketch

```rust
use wrkflw_executor::{execute_workflow, ExecutionConfig, RuntimeType};

let cfg = ExecutionConfig {
    runtime_type: RuntimeType::Auto, // Docker, then Podman, then emulation
    verbose: true,
    target_job: Some("build".to_string()), // run a single job
    ..Default::default()
};

let workflow_path = std::path::Path::new(".github/workflows/ci.yml");
let result = execute_workflow(workflow_path, cfg).await?;
for job in &result.jobs {
    println!("{}: {:?}", job.name, job.status);
}
```

Prefer using the `wrkflw` binary for a complete UX across validation, execution, and logs.
