# WRKFLW

[![Crates.io](https://img.shields.io/crates/v/wrkflw)](https://crates.io/crates/wrkflw)
[![License](https://img.shields.io/crates/l/wrkflw)](LICENSE)
[![Build Status](https://img.shields.io/github/actions/workflow/status/bahdotsh/wrkflw/ci.yml?branch=main)](https://github.com/bahdotsh/wrkflw/actions/workflows/ci.yml)
[![Downloads](https://img.shields.io/crates/d/wrkflw)](https://crates.io/crates/wrkflw)

A command-line tool for validating and executing GitHub Actions workflows locally. Test your workflows on your machine before pushing to GitHub.

![WRKFLW Demo](demo.gif)

## Features

- **TUI interface** — interactive terminal UI with Workflows, Execution, DAG, Logs, Trigger, Secrets, and Help tabs
- **Workflow validation** — syntax checks, structural validation, and composite action input cross-checking with CI/CD-friendly exit codes
- **Local execution** — Docker, Podman, emulation, or sandboxed **secure emulation** (no containers)
- **Diff-aware filtering** — skip workflows whose `on:` block doesn't match the simulated event and changed file set
- **Watch mode** — rerun workflows automatically on file changes, with trigger-aware filtering
- **Job selection** — run individual jobs with `--job` or via TUI job selection mode
- **Job dependency resolution** — automatic ordering based on `needs` with parallel execution of independent jobs
- **Expression evaluator** — evaluates `${{ ... }}` expressions including `toJSON`, `fromJSON`, `contains`, `startsWith`, etc.
- **Action support** — Docker container actions, JavaScript actions, composite actions (with output propagation), and local actions
- **Reusable workflows** — execute caller jobs via `jobs.<id>.uses` (local or `owner/repo/path@ref`) with output propagation
- **Artifacts, cache, and inter-job outputs** — `actions/upload-artifact`, `actions/download-artifact`, `actions/cache`, and `needs.<id>.outputs.*`
- **GitHub context emulation** — environment variables, `GITHUB_OUTPUT`, `GITHUB_ENV`, `GITHUB_PATH`, `GITHUB_STEP_SUMMARY`
- **Matrix builds** — full support for `include`, `exclude`, `max-parallel`, and `fail-fast`
- **Secrets management** — multiple providers (env, file, Vault, AWS, Azure, GCP) with masking and AES-256-GCM encrypted storage
- **Remote triggering** — trigger `workflow_dispatch` runs on GitHub or GitLab pipelines
- **GitLab support** — validate and trigger GitLab CI pipelines

## Not yet supported

- GitHub encrypted secrets and fine-grained permissions
- Event triggers other than `workflow_dispatch` for the remote `trigger` command
- Private repos for remote `uses:` — reusable workflows clone over unauthenticated HTTPS
- `concurrency:` groups and `cancel-in-progress` — parsed but not enforced
- Service containers — `services:` is parsed but never started, in any runtime
- Windows and macOS runners — `runs-on: windows-*` / `macos-*` is silently mapped to a container image (macOS → a Linux image, Windows → a Windows container that won't run on Linux/macOS hosts). `${{ runner.os }}` reflects the host OS, not `runs-on`.

## Installation

```bash
cargo install wrkflw
```

Or with Homebrew ([formula](https://formulae.brew.sh/formula/wrkflw)):

```bash
brew install wrkflw
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

# Rerun workflows automatically on file changes
wrkflw watch

# List detected workflows and pipelines
wrkflw list
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
# Run with auto-detection (default: tries Docker, then Podman, then emulation)
wrkflw run .github/workflows/ci.yml

# Run with Docker explicitly
wrkflw run --runtime docker .github/workflows/ci.yml

# Run with Podman
wrkflw run --runtime podman .github/workflows/ci.yml

# Run in emulation mode (no containers)
wrkflw run --runtime emulation .github/workflows/ci.yml

# Run in sandboxed secure emulation
wrkflw run --runtime secure-emulation .github/workflows/ci.yml

# Run a specific job
wrkflw run --job build .github/workflows/ci.yml

# List jobs in a workflow
wrkflw run --jobs .github/workflows/ci.yml

# Preserve failed containers for debugging
wrkflw run --preserve-containers-on-failure .github/workflows/ci.yml
```

### Trigger-aware execution

Skip workflows whose `on:` block wouldn't fire for a given event/change set. Strict mode is on by default: `wrkflw run --event …` without `--diff` or `--changed-files` is rejected up front rather than silently skipping every `paths:`-gated workflow.

```bash
# Auto-detect changed files from git (vs origin/HEAD, main/master, or HEAD~1)
wrkflw run --diff --event push .github/workflows/ci.yml

# Pin the diff range
wrkflw run --diff --diff-base main --diff-head HEAD --event push .github/workflows/ci.yml

# Supply changed files explicitly (e.g. from a CI wrapper)
wrkflw run --event push --changed-files src/main.rs,Cargo.toml .github/workflows/ci.yml

# Simulate a pull_request — `--base-branch` is required under strict mode
wrkflw run --event pull_request --base-branch main --diff .github/workflows/ci.yml

# Opt out of strict rejection (legacy warn-and-proceed)
wrkflw run --event push --no-strict-filter .github/workflows/ci.yml
```

See [BREAKING_CHANGES.md](BREAKING_CHANGES.md) for full migration notes.

### Watch mode

```bash
# Watch .github/workflows for changes and rerun affected workflows
wrkflw watch

# Watch a specific path, simulate pull_request, and cap concurrency
wrkflw watch --event pull_request --base-branch main \
    --max-concurrency 2 --debounce 750 .github/workflows

# Ignore extra directories on top of the built-in list
wrkflw watch --ignore-dir .terraform --ignore-dir coverage
```

### TUI

```bash
# Open TUI with default directory
wrkflw tui

# Open with specific runtime
wrkflw tui --runtime podman
```

**Tabs:** Workflows · Execution · DAG · Logs · Trigger · Secrets · Help.

**Controls:**

| Key | Action |
|-----|--------|
| `Tab` / `Shift+Tab` | Switch tabs |
| `1`–`7` | Jump to tab by number |
| `w` / `x` / `l` / `h` | Jump to Workflows / Execution / Logs / Help |
| `↑`/`↓` or `k`/`j` | Navigate / scroll |
| `Space` | Toggle workflow selection |
| `Enter` | Run / view details |
| `r` | Run selected workflows |
| `a` / `n` | Select all / deselect all |
| `Shift+R` | Reset workflow status |
| `Shift+J` | View jobs in workflow |
| `e` | Cycle runtime (Docker / Podman / Emulation / Secure Emulation) |
| `v` | Toggle Execution / Validation mode |
| `d` / `D` | Toggle diff-aware filter / cycle simulated event |
| `t` | Trigger remote workflow |
| `,` | Open Tweaks overlay |
| `?` | Toggle help overlay |
| `q` / `Esc` | Quit / back |

Logs tab adds `s` (search), `f` (filter), `c` (clear), `n` (next match). Trigger tab adds `p` (github↔gitlab), `b` (edit branch), `+` (add input), `c` (copy curl preview).

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
| **Auto** (default) | Detects Docker, then Podman, then falls back to emulation | Most users |
| **Docker** | Full container isolation, closest to GitHub runners | Production, CI/CD |
| **Podman** | Rootless containers, no daemon required | Security-conscious environments |
| **Emulation** | Runs directly on host, no containers needed | Quick local testing |
| **Secure Emulation** | Sandboxed host processes with filesystem/network restrictions | Running untrusted workflows without a container runtime |

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
- Outputs from called jobs are merged back into `needs.<id>.outputs.*`

**Limitations:** private repos for remote `uses:` are not yet supported (the clone is unauthenticated); declared `on.workflow_call.outputs` is approximated by flattening all called-job outputs (the explicit mapping is not yet parsed).

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

## Project Structure

WRKFLW is organized as a Cargo workspace with focused crates:

| Crate | Purpose |
|-------|---------|
| `wrkflw` | CLI binary and library entry point |
| `wrkflw-executor` | Workflow execution engine, expression evaluator, artifact/cache stores |
| `wrkflw-parser` | Workflow file parsing and schema validation |
| `wrkflw-evaluator` | Structural evaluation of workflow files |
| `wrkflw-validators` | Validation rules for jobs, steps, triggers |
| `wrkflw-runtime` | Container and emulation runtime abstractions |
| `wrkflw-trigger-filter` | `on:` block parsing and change-set matching |
| `wrkflw-watcher` | File watcher with trigger-aware re-execution |
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
