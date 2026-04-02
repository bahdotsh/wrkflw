# WRKFLW

[![Crates.io](https://img.shields.io/crates/v/wrkflw)](https://crates.io/crates/wrkflw)
[![License](https://img.shields.io/crates/l/wrkflw)](LICENSE)
[![Build Status](https://img.shields.io/github/actions/workflow/status/bahdotsh/wrkflw/ci.yml?branch=main)](https://github.com/bahdotsh/wrkflw/actions/workflows/ci.yml)
[![Downloads](https://img.shields.io/crates/d/wrkflw)](https://crates.io/crates/wrkflw)

A command-line tool for validating and executing GitHub Actions workflows locally. Test your workflows on your machine before pushing to GitHub.

![WRKFLW Demo](demo.gif)

## Features

- **TUI interface** — interactive terminal UI for browsing, running, and monitoring workflows
- **Workflow validation** — syntax checks, structural validation, and composite action input cross-checking with CI/CD-friendly exit codes
- **Local execution** — run workflows using Docker, Podman, or emulation mode (no containers)
- **Job selection** — run individual jobs with `--job` flag or via TUI job selection mode
- **Job dependency resolution** — automatic ordering based on `needs` with parallel execution of independent jobs
- **Action support** — Docker container actions, JavaScript actions, composite actions, and local actions
- **Reusable workflows** — execute caller jobs via `jobs.<id>.uses` (local or `owner/repo/path@ref`)
- **GitHub context emulation** — environment variables, `GITHUB_OUTPUT`, `GITHUB_ENV`, `GITHUB_PATH`, `GITHUB_STEP_SUMMARY`
- **Matrix builds** — full support for `include`, `exclude`, `max-parallel`, and `fail-fast`
- **Secrets management** — multiple providers (env, file, Vault, AWS, Azure, GCP) with masking and encryption
- **Remote triggering** — trigger `workflow_dispatch` runs on GitHub or GitLab pipelines
- **GitLab support** — validate and trigger GitLab CI pipelines

## Installation

```bash
cargo install wrkflw
```

Or build from source:

```bash
git clone https://github.com/bahdotsh/wrkflw.git
cd wrkflw
cargo build --release
```

## Quick Start

```bash
# Launch the TUI (auto-detects .github/workflows)
wrkflw

# Validate workflows
wrkflw validate

# Run a workflow
wrkflw run .github/workflows/ci.yml
```

## Usage

### Validation

```bash
# Validate all workflows in .github/workflows
wrkflw validate

# Validate specific files or directories
wrkflw validate path/to/workflow.yml
wrkflw validate path/to/workflows/

# Validate multiple paths
wrkflw validate flow-1.yml flow-2.yml path/to/workflows/

# GitLab pipelines
wrkflw validate .gitlab-ci.yml --gitlab

# Verbose output
wrkflw validate --verbose path/to/workflow.yml
```

**Exit codes:** `0` = all valid, `1` = validation failures, `2` = usage error. Use `--no-exit-code` to disable.

### Execution

```bash
# Run with Docker (default)
wrkflw run .github/workflows/ci.yml

# Run with Podman
wrkflw run --runtime podman .github/workflows/ci.yml

# Run in emulation mode (no containers)
wrkflw run --runtime emulation .github/workflows/ci.yml

# Run a specific job
wrkflw run --job build .github/workflows/ci.yml

# List jobs in a workflow
wrkflw run --jobs .github/workflows/ci.yml

# Preserve failed containers for debugging
wrkflw run --preserve-containers-on-failure .github/workflows/ci.yml
```

### TUI

```bash
# Open TUI with default directory
wrkflw tui

# Open with specific runtime
wrkflw tui --runtime podman
```

**Controls:**

| Key | Action |
|-----|--------|
| `Tab` / `1-4` | Switch tabs (Workflows, Execution, Logs, Help) |
| `Up/Down` or `j/k` | Navigate |
| `Space` | Toggle selection |
| `Enter` | Run / View details |
| `r` | Run selected workflows |
| `a` / `n` | Select all / Deselect all |
| `e` | Cycle runtime (Docker / Podman / Emulation) |
| `v` | Toggle Execution / Validation mode |
| `t` | Trigger remote workflow |
| `q` / `Esc` | Quit / Back |

### Remote Triggering

Trigger `workflow_dispatch` events on GitHub or GitLab.

```bash
# GitHub (requires GITHUB_TOKEN env var)
wrkflw trigger workflow-name --branch main --input key=value

# GitLab (requires GITLAB_TOKEN env var)
wrkflw trigger-gitlab --branch main --variable key=value
```

## Runtime Modes

| Mode | Description | Best for |
|------|-------------|----------|
| **Docker** (default) | Full container isolation, closest to GitHub runners | Production, CI/CD |
| **Podman** | Rootless containers, no daemon required | Security-conscious environments |
| **Emulation** | Runs directly on host, no containers needed | Quick local testing |

## Reusable Workflows

```yaml
jobs:
  call-local:
    uses: ./.github/workflows/shared.yml

  call-remote:
    uses: my-org/my-repo/.github/workflows/shared.yml@v1
    with:
      foo: bar
    secrets:
      token: ${{ secrets.MY_TOKEN }}
```

- Local refs resolve relative to the working directory
- Remote refs are shallow-cloned at the specified `@ref`
- `with:` entries become `INPUT_<KEY>` env vars; `secrets:` become `SECRET_<KEY>`

**Limitations:** outputs from called workflows are not propagated back; `secrets: inherit` is not supported; private repos for remote `uses:` are not yet supported.

## Secrets Management

WRKFLW supports GitHub Actions-compatible `${{ secrets.* }}` syntax with multiple providers:

```bash
# Environment variables (simplest)
export GITHUB_TOKEN="ghp_..."
wrkflw run .github/workflows/ci.yml

# File-based secrets (JSON, YAML, or .env format)
# Configure in ~/.wrkflw/secrets.yml
```

Supported providers: environment variables, file-based, HashiCorp Vault, AWS Secrets Manager, Azure Key Vault, Google Cloud Secret Manager. See the [secrets demo](examples/secrets-demo/) for detailed examples.

## Limitations

### Supported
- Workflow syntax validation with exit codes
- Job dependency resolution and parallel execution
- Matrix builds, environment variables, GitHub context
- Container, JavaScript, composite, and local actions
- Reusable workflows (caller jobs)
- Environment files (`GITHUB_OUTPUT`, `GITHUB_ENV`, `GITHUB_PATH`, `GITHUB_STEP_SUMMARY`)
- TUI and CLI interfaces
- Container cleanup (even on Ctrl+C)

### Not Supported
- GitHub encrypted secrets and fine-grained permissions
- `actions/cache` (no persistent cache between runs)
- Artifact upload/download between jobs
- Event triggers other than `workflow_dispatch`
- Windows and macOS runners
- Job/step timeouts, concurrency, and cancellation
- Service containers in emulation mode
- Reusable workflow output propagation (`needs.<id>.outputs.*`)

## Project Structure

WRKFLW is organized as a Cargo workspace with focused crates:

| Crate | Purpose |
|-------|---------|
| `wrkflw` | CLI binary and library entry point |
| `wrkflw-executor` | Workflow execution engine |
| `wrkflw-parser` | Workflow file parsing and schema validation |
| `wrkflw-evaluator` | Structural evaluation of workflow files |
| `wrkflw-validators` | Validation rules for jobs, steps, triggers |
| `wrkflw-runtime` | Container and emulation runtime abstractions |
| `wrkflw-ui` | Terminal user interface |
| `wrkflw-models` | Shared data structures |
| `wrkflw-matrix` | Matrix expansion utilities |
| `wrkflw-secrets` | Secrets management with multiple providers |
| `wrkflw-github` | GitHub API integration |
| `wrkflw-gitlab` | GitLab API integration |
| `wrkflw-logging` | In-memory logging for TUI/CLI |
| `wrkflw-utils` | Shared helpers |

## License

MIT License - see [LICENSE](LICENSE) for details.
