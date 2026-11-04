## WRKFLW (CLI and Library)

This crate provides the `wrkflw` command-line interface and a thin library surface that ties together all WRKFLW subcrates. It lets you validate and execute GitHub Actions, Gitea and GitLab CI workflows locally, with a built-in TUI for an interactive experience.

- **Validate**: Lints structure and common mistakes in workflow/pipeline files
- **Run**: Executes jobs locally using Docker, Podman, Kubernetes, emulation, or secure emulation, with optional diff-aware trigger filtering
- **Gate**: Runs what a push would run, reusing jobs that already passed, and fails if any of it fails
- **Plan**: Shows job order, matrix legs and whether a trigger fires, without running anything
- **Runs**: Inspects, explains and replays recorded runs
- **MCP**: Serves all of this to coding agents over the Model Context Protocol
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
# Launch the TUI (auto-loads .gitea/workflows if it exists, else .github/workflows)
wrkflw

# Validate all workflows in the default directory
wrkflw validate

# Validate a specific file or directory
wrkflw validate .github/workflows/ci.yml
wrkflw validate path/to/workflows

# Validate multiple files and/or directories
wrkflw validate path/to/flow-1.yml path/to/flow-2.yml path/to/workflows

# Run a workflow (auto-detects Docker, then Podman, then falls back to emulation)
wrkflw run .github/workflows/ci.yml

# Pick a runtime explicitly
wrkflw run --runtime podman .github/workflows/ci.yml
wrkflw run --runtime kubernetes --k8s-namespace ci .github/workflows/ci.yml
wrkflw run --runtime emulation .github/workflows/ci.yml
wrkflw run --runtime secure-emulation .github/workflows/ci.yml

# Diff-aware filtering (skip workflows whose on: block doesn't match)
wrkflw run --diff --event push .github/workflows/ci.yml
wrkflw run --event pull_request --base-branch main --diff .github/workflows/ci.yml

# Check everything a push would run, reusing what already passed
wrkflw gate
wrkflw gate --install-hook

# See what a run would do, then explain the last failure
wrkflw plan .github/workflows/ci.yml
wrkflw runs failure latest

# Watch for changes and rerun affected workflows
wrkflw watch

# Open the TUI explicitly
wrkflw tui
wrkflw tui --runtime podman
```

### Commands

- **validate**: Validate workflow/pipeline files and/or directories
  - GitHub (default): `.gitea/workflows` if it exists, else `.github/workflows`
  - GitLab: `.gitlab-ci.yml` or files ending with `gitlab-ci.yml`
  - Accepts multiple paths in a single invocation
  - Exit code behavior (by default): `1` when any validation failure is detected
  - Flags: `--gitlab`, `--exit-code`, `--no-exit-code`, `--verbose`

- **run**: Execute a workflow or pipeline locally
  - Runtimes: `auto` (default), `docker`, `podman`, `kubernetes`, `emulation`, `secure-emulation`. Only `auto` falls back; an explicit runtime that doesn't answer fails the run
  - Flags: `--runtime`, `--job`, `--jobs`, `--from-step`, `--variable`, `--preserve-containers-on-failure`, `--show-action-messages`, `--gitlab`, `--verbose`
  - Trigger filter flags: `--event`, `--diff`, `--changed-files`, `--diff-base`, `--diff-head`, `--base-branch`, `--activity-type`, `--strict-filter` (default on), `--no-strict-filter`
  - CI-saving flags: `--reuse-results`, `--affected`, `--job-container`, `--dep-cache`, `--remote-cache`, `--cache-push`
  - History flags: `--intent`, `--agent`, `--session`, `--no-record`
  - Kubernetes flags: `--k8s-context`, `--k8s-namespace`, `--k8s-node-selector`, `--k8s-resource`, `--k8s-image-pull-secret`, `--k8s-registry`, `--k8s-helper-image`, `--k8s-timeout`, `--k8s-schedule-timeout`

- **gate**: Run every workflow a push would fire, with result reuse, a job container and dependency caches, and exit `1` if anything fails
  - Flags: `--event`, `--base-branch`, `--diff-base`, `--runtime`, `--remote-cache`, `--install-hook`

- **plan**: Show job order, matrix legs, missing secrets and whether the trigger fires, without executing anything
  - Takes the same `--job` and trigger filter flags as `run`

- **runs**: Inspect recorded runs: `list`, `show`, `logs`, `failure` and `replay`

- **mcp**: `serve` runs the MCP server over stdio; `install` registers it with the coding agents on this machine

- **watch**: Watch a directory and rerun affected workflows on change
  - Flags: `--runtime`, `--debounce`, `--event`, `--max-concurrency`, `--base-branch`, `--activity-type`, `--max-pending-events`, `--ignore-dir`, `--strict-filter` / `--no-strict-filter`

- **tui**: Interactive terminal interface
  - Browse workflows, execute, inspect logs, job details and recorded runs

- **trigger**: Trigger a GitHub workflow (requires `GITHUB_TOKEN`)
- **trigger-gitlab**: Trigger a GitLab pipeline (requires `GITLAB_TOKEN`)
- **list**: Show detected workflows and pipelines in the repo

Global flags: `--format json` (stdout carries only JSON, logs go to stderr), `--theme auto|dark|light`, `-P`/`--platform LABEL=IMAGE`, `--verbose`, `--debug`.

### Environment variables

- **GITHUB_TOKEN**: Required for `trigger` when calling GitHub
- **GITLAB_TOKEN**: Required for `trigger-gitlab` (api scope)
- **WRKFLW_PLATFORM**, **WRKFLW_THEME**, **WRKFLW_K8S_\***: Defaults for `--platform`, `--theme` and the `--k8s-*` flags
- **WRKFLW_AGENT**, **WRKFLW_SESSION**, **WRKFLW_INTENT**: Recorded on each run
- **WRKFLW_CACHE_SIGNING_KEY**, **WRKFLW_REMOTE_CACHE_TOKEN**: Sign and authenticate a `--remote-cache`

### Exit codes

- `validate`: `0` if all pass; `1` if any fail (unless `--no-exit-code`)
- `run`, `gate`: `0` on success or a skipped run, `1` if execution fails
- Any command: `2` for a usage error

### Library usage

This crate re-exports subcrates for convenience if you want to embed functionality. The library is named `wrkflw_lib`, because the binary takes `wrkflw`:

```rust
use std::path::Path;
use wrkflw_lib::executor::{execute_workflow, ExecutionConfig, RuntimeType};

# tokio_test::block_on(async {
let cfg = ExecutionConfig {
    runtime_type: RuntimeType::Auto,
    verbose: true,
    ..Default::default()
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
use wrkflw_lib::executor::RuntimeType;
use wrkflw_lib::ui::run_wrkflw_tui;

# tokio_test::block_on(async {
let path = PathBuf::from(".github/workflows");
run_wrkflw_tui(
    Some(&path),
    RuntimeType::Auto,
    /* verbose */ true,
    /* preserve_containers_on_failure */ false,
    /* show_action_messages */ false,
    /* theme */ None,
    /* platform_overrides */ Default::default(),
    /* kubernetes */ Default::default(),
).await?;
# Ok::<_, Box<dyn std::error::Error>>(())
# })?;
```

### Notes

- See the repository root README for feature details, limitations, and a full walkthrough.
- Service containers start only in Docker and Podman modes.
- Emulation mode runs steps directly on this machine. Use a container runtime or `secure-emulation` for code you don't trust.
- See [AGENTS.md](https://github.com/bahdotsh/wrkflw/blob/main/AGENTS.md) for the JSON output contract used by coding agents.