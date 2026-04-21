## WRKFLW (CLI and Library)

This crate provides the `wrkflw` command-line interface and a thin library surface that ties together all WRKFLW subcrates. It lets you validate and execute GitHub Actions workflows and GitLab CI pipelines locally, with a built-in TUI for an interactive experience.

- **Validate**: Lints structure and common mistakes in workflow/pipeline files
- **Run**: Executes jobs locally using Docker, Podman, emulation, or secure emulation, with optional diff-aware trigger filtering
- **Watch**: Reruns workflows on file changes with trigger-aware filtering
- **TUI**: Interactive terminal UI for browsing workflows, running, and viewing logs
- **Trigger**: Manually trigger remote runs on GitHub/GitLab
- **List**: Detect workflow and pipeline files in the repo

### Installation

```bash
cargo install wrkflw
```

### Quick start

```bash
# Launch the TUI (auto-loads .github/workflows)
wrkflw

# Validate all workflows in the default directory
wrkflw validate

# Validate a specific file or directory
wrkflw validate .github/workflows/ci.yml
wrkflw validate path/to/workflows

# Validate multiple files and/or directories
wrkflw validate path/to/flow-1.yml path/to/flow-2.yml path/to/workflows

# Run a workflow (Docker by default)
wrkflw run .github/workflows/ci.yml

# Use Podman, emulation, or sandboxed secure emulation instead of Docker
wrkflw run --runtime podman .github/workflows/ci.yml
wrkflw run --runtime emulation .github/workflows/ci.yml
wrkflw run --runtime secure-emulation .github/workflows/ci.yml

# Diff-aware filtering (skip workflows whose on: block doesn't match)
wrkflw run --diff --event push .github/workflows/ci.yml
wrkflw run --event pull_request --base-branch main --diff .github/workflows/ci.yml

# Watch for changes and rerun affected workflows
wrkflw watch

# Open the TUI explicitly
wrkflw tui
wrkflw tui --runtime podman
```

### Commands

- **validate**: Validate workflow/pipeline files and/or directories
  - GitHub (default): `.github/workflows/*.yml`
  - GitLab: `.gitlab-ci.yml` or files ending with `gitlab-ci.yml`
  - Accepts multiple paths in a single invocation
  - Exit code behavior (by default): `1` when any validation failure is detected
  - Flags: `--gitlab`, `--exit-code`, `--no-exit-code`, `--verbose`

- **run**: Execute a workflow or pipeline locally
  - Runtimes: `docker` (default), `podman`, `emulation`, `secure-emulation`
  - Flags: `--runtime`, `--job`, `--jobs`, `--preserve-containers-on-failure`, `--gitlab`, `--verbose`
  - Trigger filter flags: `--event`, `--diff`, `--changed-files`, `--diff-base`, `--diff-head`, `--base-branch`, `--activity-type`, `--strict-filter` (default on), `--no-strict-filter`

- **watch**: Watch a directory and rerun affected workflows on change
  - Flags: `--runtime`, `--debounce`, `--event`, `--max-concurrency`, `--base-branch`, `--activity-type`, `--max-pending-events`, `--ignore-dir`, `--strict-filter` / `--no-strict-filter`

- **tui**: Interactive terminal interface
  - Browse workflows, execute, and inspect logs and job details

- **trigger**: Trigger a GitHub workflow (requires `GITHUB_TOKEN`)
- **trigger-gitlab**: Trigger a GitLab pipeline (requires `GITLAB_TOKEN`)
- **list**: Show detected workflows and pipelines in the repo

### Environment variables

- **GITHUB_TOKEN**: Required for `trigger` when calling GitHub
- **GITLAB_TOKEN**: Required for `trigger-gitlab` (api scope)

### Exit codes

- `validate`: `0` if all pass; `1` if any fail (unless `--no-exit-code`)
- `run`: `0` on success, `1` if execution fails

### Library usage

This crate re-exports subcrates for convenience if you want to embed functionality:

```rust
use std::path::Path;
use wrkflw::executor::{execute_workflow, ExecutionConfig, RuntimeType};

# tokio_test::block_on(async {
let cfg = ExecutionConfig {
    runtime_type: RuntimeType::Docker,
    verbose: true,
    preserve_containers_on_failure: false,
    secrets_config: None,
    show_action_messages: false,
    target_job: None,
};
let result = execute_workflow(Path::new(".github/workflows/ci.yml"), cfg).await?;
for job in &result.jobs {
    println!("{}: {:?}", job.name, job.status);
}
# Ok::<_, Box<dyn std::error::Error>>(())
# })?;
```

You can also run the TUI programmatically:

```rust
use std::path::PathBuf;
use wrkflw::executor::RuntimeType;
use wrkflw::ui::run_wrkflw_tui;

# tokio_test::block_on(async {
let path = PathBuf::from(".github/workflows");
run_wrkflw_tui(Some(&path), RuntimeType::Docker, /* verbose */ true, /* preserve_containers_on_failure */ false, /* show_action_messages */ false).await?;
# Ok::<_, Box<dyn std::error::Error>>(())
# })?;
```

### Notes

- See the repository root README for feature details, limitations, and a full walkthrough.
- Service containers and advanced Actions features are best supported in Docker/Podman modes.
- Emulation mode skips containerized steps and runs commands on the host.