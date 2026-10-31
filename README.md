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
- **Kubernetes execution** — run each step as a pod in your own cluster, with node selectors and GPU resource requests
- **Diff-aware filtering** — skip workflows whose `on:` block doesn't match the simulated event and changed file set
- **Watch mode** — rerun workflows automatically on file changes, with trigger-aware filtering
- **Job selection** — run individual jobs with `--job` or via TUI job selection mode
- **Job dependency resolution** — automatic ordering based on `needs` with parallel execution of independent jobs
- **Expression evaluator** — evaluates `${{ ... }}` expressions including `toJSON`, `fromJSON`, `contains`, `startsWith`, etc.
- **Action support** — Docker container actions, JavaScript actions, composite actions (with output propagation), and local actions
- **Reusable workflows** — execute caller jobs via `jobs.<id>.uses` (local or `owner/repo/path@ref`) with output propagation
- **Artifacts, cache, and inter-job outputs** — `actions/upload-artifact`, `actions/download-artifact`, `actions/cache`, and `needs.<id>.outputs.*`
- **GitHub context emulation** — environment variables, `GITHUB_OUTPUT`, `GITHUB_ENV`, `GITHUB_PATH`, `GITHUB_STEP_SUMMARY`
- **Matrix builds** — full support for `include`, `exclude`, `max-parallel`, and `fail-fast`, including a whole matrix, an axis or `include`/`exclude` computed by an earlier job (`${{ fromJSON(needs.setup.outputs.matrix) }}`). An axis expression must evaluate to an array
- **Secrets management** — multiple providers (env, file, Vault, AWS, Azure, GCP) with masking and AES-256-GCM encrypted storage
- **Remote triggering** — trigger `workflow_dispatch` runs on GitHub or GitLab pipelines
- **GitLab support** — validate and trigger GitLab CI pipelines
- **Gitea support** — discover, validate, and run workflows from `.gitea/workflows`, including the `gitea` expression context

## Not yet supported

- GitHub encrypted secrets and fine-grained permissions
- Event triggers other than `workflow_dispatch` for the remote `trigger` command
- Absolute-URL actions such as `uses: https://gitea.com/actions/checkout@v4` — they pass validation, but at run time the URL is not resolved and the step falls back to built-in handling
- Private repos for remote `uses:` — reusable workflows clone over unauthenticated HTTPS
- `concurrency:` groups and `cancel-in-progress` — parsed but not enforced
- Service containers — `services:` is parsed but never started, in any runtime
- Windows and macOS runners — `runs-on: windows-*` / `macos-*` is silently mapped to a container image (macOS → a Linux image, Windows → a Windows container that won't run on Linux/macOS hosts). In container runtimes `${{ runner.os }}` is `Linux` and `runner.temp` and the other runner paths are Linux paths, even for a Windows image, so a job on a Windows container sees the wrong values. In emulation `runner.os` is the host OS.
- `working-directory` outside the workspace — it must stay inside the workspace, so `${{ runner.temp }}/build` fails the step instead of running there.

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
# Launch the TUI (auto-detects .gitea/workflows and .github/workflows)
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
# Validate all workflows in .gitea/workflows and .github/workflows
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

# Run each step as a pod in a Kubernetes cluster
wrkflw run --runtime kubernetes --k8s-namespace ci .github/workflows/ci.yml

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

### Custom runner images

By default `runs-on` maps to a small built-in image, so `ubuntu-latest` runs in `ubuntu:latest`, which has few of the tools a GitHub runner has. Use `-P`/`--platform` to pick the image for a label, for example the larger images made for [act](https://github.com/nektos/act):

```bash
wrkflw run -P ubuntu-latest=catthehacker/ubuntu:act-latest .github/workflows/ci.yml

# Repeat the flag or separate entries with commas
wrkflw run -P ubuntu-latest=my/image:1 -P ubuntu-22.04=my/image:2 .github/workflows/ci.yml

# Set it once for every command, including the TUI and watch mode
export WRKFLW_PLATFORM=ubuntu-latest=catthehacker/ubuntu:act-latest
```

- A job-level `container:` still wins over `--platform`, which wins over the built-in image.
- Labels match case-insensitively, and the last value given for a label wins. Every label in `runs-on: [self-hosted, linux]` is tried, and `runs-on: ${{ matrix.os }}` is resolved per matrix combination.
- Runtimes from `actions/setup-*` are installed on top of your image, so the image must be Debian or Ubuntu based for those jobs.
- `--platform` replaces `WRKFLW_PLATFORM` entirely when both are set. It has no effect in emulation mode.

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

# Force a color theme
wrkflw tui --theme light
```

The TUI follows your terminal's background and uses a light or dark palette to match. Set `--theme` or `WRKFLW_THEME` to `auto`, `dark` or `light` to override it. The theme is chosen at startup, so restart the TUI after switching your terminal's theme.

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
| `e` | Cycle runtime (Docker / Podman / Kubernetes / Secure Emulation / Emulation) |
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
| **Kubernetes** | Each step runs as a pod in a cluster you point `kubectl` at | GPU/XPU jobs, shared build clusters, self-hosted forges |
| **Emulation** | Runs directly on host, no containers needed | Quick local testing |
| **Secure Emulation** | Sandboxed host processes with filesystem/network restrictions | Running untrusted workflows without a container runtime |

### Kubernetes runtime

`--runtime kubernetes` dispatches every step to the cluster `kubectl` is configured for. It is never auto-detected, and it never falls back to running on your machine: if the cluster is unreachable, the run fails.

```bash
# Pick the cluster and namespace, then run as usual
wrkflw run --runtime kubernetes --k8s-context gpu-cluster --k8s-namespace ci .github/workflows/train.yml

# Land on GPU nodes and request an accelerator for every step
wrkflw run --runtime kubernetes \
  --k8s-node-selector accelerator=a100 \
  --k8s-resource nvidia.com/gpu=1 --k8s-resource memory=16Gi \
  .github/workflows/train.yml

# Private registry, and a registry wrkflw can push locally built images to
wrkflw run --runtime kubernetes --k8s-image-pull-secret regcred \
  --k8s-registry registry.example.com/ci .github/workflows/ci.yml

# Every flag has a WRKFLW_K8S_* environment variable, so set them once
export WRKFLW_K8S_NAMESPACE=ci WRKFLW_K8S_RESOURCE=nvidia.com/gpu=1
```

| Flag | Environment variable | Purpose |
|------|----------------------|---------|
| `--k8s-context NAME` | `WRKFLW_K8S_CONTEXT` | kubeconfig context (default: current) |
| `--k8s-namespace NAME` | `WRKFLW_K8S_NAMESPACE` | Namespace pods are created in (default: current) |
| `--k8s-node-selector KEY=VALUE` | `WRKFLW_K8S_NODE_SELECTOR` | `nodeSelector` for every pod; repeatable |
| `--k8s-resource NAME=QUANTITY` | `WRKFLW_K8S_RESOURCE` | Limits and requests for the step container, e.g. `nvidia.com/gpu=1`; repeatable |
| `--k8s-image-pull-secret NAME` | `WRKFLW_K8S_IMAGE_PULL_SECRET` | `imagePullSecrets` for every pod; repeatable |
| `--k8s-registry REGISTRY` | `WRKFLW_K8S_REGISTRY` | Where images built locally are pushed so the cluster can pull them |
| `--k8s-helper-image IMAGE` | `WRKFLW_K8S_HELPER_IMAGE` | Image for the sync helpers (default `busybox:1.36`, pinned by digest; needs `sh` and `tar`) |
| `--k8s-timeout SECONDS` | `WRKFLW_K8S_TIMEOUT` | Per-step limit, pod creation to sync-back (default 21600, the engine's job timeout) |
| `--k8s-schedule-timeout SECONDS` | `WRKFLW_K8S_SCHEDULE_TIMEOUT` | How long a pod may stay unschedulable before the step fails (default 600) |

How a step runs:

1. A pod is created with one `emptyDir` per mount, an init container that waits for a ready marker, the step container, and a small sidecar. Pods are labelled `app.kubernetes.io/managed-by=wrkflw` and `wrkflw.dev/session=<id>`.
2. The workspace, GitHub env files and action checkouts are streamed into the init container as tar archives, then the marker is set and the step starts.
3. After the step exits, its logs are collected and every mount is streamed back out through the sidecar, so `GITHUB_OUTPUT`, artifacts and workspace changes reach the next step.
4. The pod is deleted. With `--preserve-containers-on-failure` a failed pod is kept for `kubectl logs <pod> -c main` and `kubectl exec -it <pod> -c wrkflw-sync -- sh`.

Every pod carries `activeDeadlineSeconds`, set to `--k8s-timeout` (at least 1) plus 30 seconds, so the cluster stops it even if wrkflw is killed. A preserved pod is stopped at that deadline too, and keeps its resources until then, GPUs included. Its logs stay readable until you delete it, but `kubectl exec` no longer works. `--preserve-containers-on-failure` also keeps a pod whose step timed out. After `--k8s-timeout` the deadline is only 30 seconds away, so read its logs instead. After a shorter `timeout-minutes`, `kubectl exec` works until the deadline. Ctrl+C always deletes pods.

Things to know:

- Only `kubectl` is needed on your machine. Jobs that build an image (`actions/setup-*` on a standard runner image, Dockerfile actions) also need Docker or Podman locally and `--k8s-registry`, because the cluster cannot see your local image store.
- Each step syncs the workspace and `RUNNER_TEMP` in and out, so large workspaces and large temporary files cost time per step.
- A failed image pull is retried for 60 seconds before the step fails, so a bad image name takes that long to report.
- Mounts are copies. Job `container.volumes` directories are copied in and written back after each step, so parallel jobs sharing one do not see each other's changes and the last to finish wins. Write-back only adds and updates: a file a step deletes from such a directory stays on the host, and files other processes create there are left alone. Only the job's workspace and `GITHUB_*` files mirror the pod exactly. A single-path volume such as `/cache` lives only in the pod and is never written back. Single files and sockets cannot be mounted and are skipped with a warning, sockets inside a mounted directory are not synced, and named volumes start empty.
- Logs are read from the kubelet when the step ends, so a step that outgrows the node's container log rotation (10 MiB by default) loses its earliest lines.
- A failed status check is retried for 60 seconds, so a brief API server outage does not end a running step. A deleted pod fails the step at once.
- A pod the scheduler cannot place fails the step after `--k8s-schedule-timeout`. Raise it if your autoscaler takes longer than 10 minutes to bring up a node.
- Synced files are owned by root in the pod, so run steps in images that run as root, as GitHub requires for container actions. Other users can read the workspace but not write to it, and namespaces enforcing the `restricted` Pod Security Standard are not supported.
- Stdout and stderr come back as one stream, as Kubernetes logs them.
- Helper containers declare small fixed CPU and memory requests and limits, so namespaces with a `ResourceQuota` accept them.
- Kubernetes validates environment variable names itself, and older clusters reject names Docker accepts. Such a step fails when its pod is created.
- Secrets reach the step as plain pod environment variables, visible to anyone who can read pods in that namespace. Pods do not mount a service account token.
- `--platform` and `runs-on` work as elsewhere: they pick the image. Node placement comes from `--k8s-node-selector`.

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
# On Linux, put them in ~/.config/wrkflw/secrets/ (read by the `file` provider)
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
