# WRKFLW

[![Crates.io](https://img.shields.io/crates/v/wrkflw)](https://crates.io/crates/wrkflw)
[![License](https://img.shields.io/crates/l/wrkflw)](LICENSE)
[![Build Status](https://img.shields.io/github/actions/workflow/status/bahdotsh/wrkflw/ci.yml?branch=main)](https://github.com/bahdotsh/wrkflw/actions/workflows/ci.yml)
[![Downloads](https://img.shields.io/crates/d/wrkflw)](https://crates.io/crates/wrkflw)

**Run your GitHub Actions workflows on your own machine, and get the verdict GitHub would give.**

`wrkflw gate` runs everything CI would run for a push, in Docker or Podman, before the push leaves your machine. Every run ends with a verdict that says how much its pass proves: `verified`, `partial` (and what wasn't checked), or `nothing_checked`. And `wrkflw compare <run URL>` checks wrkflw itself against a finished GitHub run, job by job.

![WRKFLW Demo](demo.gif)

## How close to GitHub

We rerun finished GitHub runs of open-source repositories with `wrkflw compare`. The last sweep (2026-11-19, on the 0.9.0 code): a job agrees when both sides passed or both failed; not compared are the Windows jobs, which run on a Linux stand-in. Jobs agreed include `macos-*` jobs run as Linux, which `compare` counts as agreeing unverified (`--macos-on-host` runs them on a Mac instead). Local times are warm, on a Mac with a 7.7 GB Docker VM.

| Repository | Jobs agreed | Differed | Not compared | Local | GitHub |
|---|---|---|---|---|---|
| pallets/click | 10 | 0 | 1 | 38 s | 65 s |
| chalk/chalk | 2 | 0 | 0 | 13 s | 39 s |
| sindresorhus/ky | 4 | 0 | 0 | 130 s | 187 s |
| rust-lang/log | 10 | 1 | 2 | 362 s | 527 s |
| dtolnay/itoa | 8 | 3 | 0 | 101 s | 346 s |
| tj/commander.js | 6 | 0 | 3 | 67 s | 99 s |
| spf13/cobra | 18 | 0 | 1 | 134 s | 99 s |
| fatih/color | 1 | 0 | 0 | 25 s | 57 s |
| pallets/flask | 11 | 0 | 1 | 32 s | 44 s |

rust-lang/log's one difference is its documentation job, which fails on Rust 1.100, released after its GitHub run, as `stable` floats. itoa's three verify an attestation with the job's GitHub token, which a local job doesn't get unless `WRKFLW_JOB_GITHUB_TOKEN=1` gives it yours. spf13/cobra is the one run slower here than on GitHub: its 19 jobs each get a machine of their own there, and share one 7.7 GB VM here.

These nine repositories are the ones wrkflw has been fixed against. To see how it does on a repository it hasn't seen, [`held_out_runs.txt`](scripts/conformance/held_out_runs.txt) lists fourteen it has never been tuned for, with services, wide OS matrices and a known failure. Their first sweep (2026-11-20, same machine) agreed on 62 jobs and differed on 53:

| Repository | Jobs agreed | Differed | Not compared | Local | GitHub |
|---|---|---|---|---|---|
| pallets/markupsafe | 9 | 0 | 1 | 44 s | 38 s |
| psf/requests | 12 | 6 | 7 | 211 s | 279 s |
| encode/httpx | 4 | 1 | 0 | 116 s | 56 s |
| sharkdp/hyperfine | 4 | 10 | 3 | 369 s | 183 s |
| sharkdp/fd | 5 | 10 | 5 | 486 s | 384 s |
| expressjs/express | 9 | 2 | 9 | 188 s | 125 s |
| sindresorhus/execa | 0 | 6 | 3 | 3562 s | 1149 s |
| urfave/cli | 3 | 1 | 2 | 249 s | 128 s |
| BurntSushi/memchr | 6 | 9 | 2 | 790 s | 608 s |
| go-sql-driver/mysql | 1 | 3 | 24 | 127 s | 671 s |
| jazzband/django-redis | 0 | 3 | 22 | 64 s | 291 s |
| dtolnay/anyhow | 9 | 2 | 1 | 620 s | 449 s |
| go-chi/chi | – | – | – | refused | |
| lib/pq | – | – | – | refused | |

The largest causes: cross-compiling with `cross` on an Apple Silicon Mac, which runs x86_64 tools that fail under Rosetta (hyperfine, fd, memchr); `codecov/codecov-action` stopping at its `github-token` input, which defaults to the token every GitHub job has and is empty in a local job unless `WRKFLW_JOB_GITHUB_TOKEN=1` (execa, urfave/cli); trigger filters wrkflw refuses although GitHub accepts them, `branches: "**"` given as a string and the glob `**.go` (chi, lib/pq, which therefore have no result); a matrix computed by a job that failed locally, which leaves GitHub's matrix legs with nothing to pair with (mysql); and job names written as a folded `>` string that `compare` can't pair (django-redis). Results are kept in [`held_out_expected.tsv`](scripts/conformance/held_out_expected.tsv). See [Conformance](docs/matching-github.md#conformance) to run either sweep yourself.

## Quick Start

```bash
cargo binstall wrkflw          # or: brew install wrkflw, or see Installation

wrkflw doctor                  # what runs here would use, and what would slow them down
wrkflw gate                    # run what CI would run for this push
wrkflw gate --install-hook     # ...and do it before every push
wrkflw run .github/workflows/ci.yml
wrkflw                         # the TUI
```

### Check every push before it leaves

Add the gate to a repository once, and every `git push` first runs what CI would run for it:

```bash
wrkflw doctor                  # Docker answering, with enough memory?
wrkflw gate --install-hook     # writes .git/hooks/pre-push; an existing hook is left alone
```

Then tell it what must only ever run in CI. The gate runs jobs with this machine's credentials and can't tell a deploy from a test, so mark deploys and releases in `.wrkflw/affected.yml`:

```yaml
jobs:
  deploy:
    gate: false     # the gate reports it skipped, with every job that needs it
  release:
    gate: false
```

A push the gate fails is stopped with the job, step and error lines that failed. `git push --no-verify` skips it once. A `partial` pass says on one line each what it didn't check, such as a `macos-*` job run as Linux or an action that didn't run. The hook keeps the `--runtime` and `--remote-cache` you install it with, so jobs that already passed in CI can be [reused](docs/usage.md#sharing-results-across-machines) instead of rerun. See [Spending less on CI](docs/usage.md#spending-less-on-ci) for everything it does.

## Features

- **TUI interface** — interactive terminal UI with Workflows, Run (jobs, graph and log views), History, Dispatch and Setup tabs, plus a Help overlay
- **Workflow validation** — syntax checks, structural validation, and composite action input cross-checking with CI/CD-friendly exit codes
- **Local execution** — Docker, Podman, emulation, or sandboxed **secure emulation** (no containers)
- **Kubernetes execution** — run each step as a pod in your own cluster, with node selectors and GPU resource requests
- **Diff-aware filtering** — skip workflows whose `on:` block doesn't match the simulated event and changed file set
- **Watch mode** — rerun workflows automatically on file changes, with trigger-aware filtering
- **Job selection** — run individual jobs with `--job` or via TUI job selection mode
- **Job dependency resolution** — automatic ordering based on `needs` with parallel execution of independent jobs
- **Expression evaluator** — evaluates `${{ ... }}` expressions including `toJSON`, `fromJSON`, `contains`, `startsWith`, etc.
- **Action support** — JavaScript actions (`runs.using: node*`, run with node in the job's container, with `pre`, `post` and `$GITHUB_STATE`), Docker container actions, composite actions (with output propagation), local actions, and built-in versions of `actions/checkout`, `actions/cache`, the artifact actions and the common setup actions (`setup-node`, `setup-python`, `setup-go`, `setup-java`, `setup-dotnet`, `setup-php`, `rust-toolchain`, `setup-uv`), which install the runtime into the job's image, honouring `*-version-file` inputs. A JavaScript action that finds no `node` in the job's image is not run: its step passes and is reported under `unverified`
- **Reusable workflows** — execute caller jobs via `jobs.<id>.uses` (local or `owner/repo/path@ref`) with output propagation
- **Artifacts, cache, and inter-job outputs** — `actions/upload-artifact`, `actions/download-artifact`, `actions/cache`, and `needs.<id>.outputs.*`
- **GitHub context emulation** — environment variables, `GITHUB_OUTPUT`, `GITHUB_ENV`, `GITHUB_PATH`, `GITHUB_STEP_SUMMARY`
- **Matrix builds** — full support for `include`, `exclude`, `max-parallel`, and `fail-fast`, including a whole matrix, an axis or `include`/`exclude` computed by an earlier job (`${{ fromJSON(needs.setup.outputs.matrix) }}`). An axis expression must evaluate to an array
- **Secrets management** — multiple providers (env, file, Vault, AWS, Azure, GCP) with masking and AES-256-GCM encrypted storage
- **Remote triggering** — trigger `workflow_dispatch` runs on GitHub or GitLab pipelines
- **GitLab support** — validate, run and trigger GitLab CI pipelines, including `parallel`, `parallel:matrix`, `rules`, `when` and stages
- **Faster reruns** — reuse passing job results, skip jobs a change can't affect, run a job in one container, share dependency caches, and gate pushes (see [Spending less on CI](docs/usage.md#spending-less-on-ci))
- **Service containers** — `services:` start next to the job on Docker and Podman, with health checks
- **Agent support** — `--format json` on every command, recorded run history with failure digests, `wrkflw plan`, and an MCP server (see [AGENTS.md](AGENTS.md))
- **Gitea support** — discover, validate, and run workflows from `.gitea/workflows`, including the `gitea` expression context

## Not yet supported

- GitHub encrypted secrets and fine-grained permissions
- Event triggers other than `workflow_dispatch` for the remote `trigger` command
- Absolute-URL actions such as `uses: https://gitea.com/actions/checkout@v4` — they pass validation, but at run time the URL is not resolved and the step falls back to built-in handling
- `cancel-in-progress` and workflow-level `concurrency:` — job-level groups are enforced within a run, but a local run has nothing earlier to cancel
- Service containers outside Docker and Podman — emulation, secure emulation, Kubernetes and jobs inside a called workflow don't start `services:`
- Service settings that reach the host — a service's `volumes:` can only name a volume, not a host path, and its `options:` only take flags that shape the service container itself, such as `--health-cmd`, `--user`, `--tmpfs` and `-e NAME=value`. Anything else, such as `--privileged` or `-v`, fails the job.
- Services that listen on the same port — services share the job container's network, as they do on a GitHub runner, so two Postgres services can't both have 5432. wrkflw refuses two services whose `ports:` name the same container port, and warns about two services of one image.
- A job with `services:` runs its steps in one container with them, which needs `sh` and `env` in the job's image. Files a step leaves outside the workspace are there for the next step, and `--preserve-containers-on-failure` doesn't keep that container.
- Windows and macOS runners — `runs-on: windows-*` / `macos-*` runs as Linux, in the same Ubuntu image as `ubuntu-latest`, as a Linux Docker engine can't start a Windows image.
  - **What you are told.** Every job whose `runs-on` names another OS than it ran as is listed under `unverified` in `run_finished` and in the `gate` result, logged as a warning, and marked on the job's line in the TUI, because its pass says nothing about OS-specific steps. `wrkflw plan` lists the same jobs before a run.
  - **Windows jobs.** A Windows job's steps that call Windows tools fail on the Linux stand-in, so `wrkflw compare` leaves its result out of the agreement; a job whose steps are portable passes, unverified. To run one for real, map the label to an image of your own with `-P`, such as a Windows image on a Windows engine.
  - **macOS jobs on a Mac.** `--macos-on-host` (or `WRKFLW_MACOS_ON_HOST=1`) runs `macos-*` jobs on the Mac itself, under emulation, while the other jobs keep their containers. Their pass then checks macOS, but their steps run with no container around them, so a `brew install` in one changes your machine. They have no terminal to ask for input at, so a step that does, such as `sudo` asking for a password, fails at once instead of waiting: GitHub's macOS runners have passwordless `sudo`, this Mac usually hasn't. The check is of the OS, not the version or architecture: on an Apple Silicon Mac a `macos-13` job, an Intel runner on GitHub, runs on Apple Silicon. Without the flag a macOS job runs as Linux, and `compare` counts its result as agreeing unverified.
  - **`runner` values.** In container runtimes `${{ runner.os }}` is `Linux` and `runner.temp` and the other runner paths are Linux paths, even for a Windows image, so a job on a Windows container sees the wrong values. In emulation `runner.os` is the host OS.
- x86_64 programs on an arm64 machine — GitHub's `ubuntu-*` runners are x86_64, and on an Apple Silicon Mac or another arm64 machine a job's container is arm64. Most steps don't notice, but `cross` and the x86_64-only images and tools it runs fail. `wrkflw compare` says so on the job's line.
- `working-directory` outside the workspace — it must stay inside the workspace, so `${{ runner.temp }}/build` fails the step instead of running there.

## Installation

A prebuilt binary, with no Rust toolchain needed:

```bash
# With cargo-binstall
cargo binstall wrkflw

# Or with the install script (Linux and macOS), which checks the download
# against the release's SHA256SUMS. WRKFLW_VERSION and WRKFLW_INSTALL_DIR
# (default ~/.local/bin) choose the release and where it goes.
curl -fsSL https://raw.githubusercontent.com/bahdotsh/wrkflw/main/scripts/install.sh | sh
```

Each [release](https://github.com/bahdotsh/wrkflw/releases) also has the archives to download by hand, Windows included. Or build it:

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

## More commands

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

# Check the runtime, its memory, the runner base image, a GitHub token and git
wrkflw doctor
```

## Documentation

- [Usage](docs/usage.md): validation, running workflows and jobs, runner images, trigger-aware runs, the push gate and shared results, watch mode, the TUI, remote triggering
- [How a local run matches GitHub's](docs/matching-github.md): `compare`, verdicts, the workspace volume, memory, and the conformance sweeps
- [Runtime modes](docs/runtimes.md): Docker, Podman, Kubernetes, emulation and secure emulation
- [Coding agents](docs/agents.md): JSON output, run history, `plan`, and the MCP server
- [Reusable workflows](docs/reusable-workflows.md)
- [Secrets](docs/secrets.md)
- [Breaking changes](BREAKING_CHANGES.md)

## Project Structure

WRKFLW is organized as a Cargo workspace with focused crates:

| Crate | Purpose |
|-------|---------|
| `wrkflw` | CLI binary and library entry point |
| `wrkflw-executor` | Workflow execution engine, expression evaluator, artifact/cache stores |
| `wrkflw-parser` | Workflow file parsing and schema validation |
| `wrkflw-evaluator` | Structural evaluation of workflow files |
| `wrkflw-validators` | Validation rules for jobs, steps, triggers |
| `wrkflw-runtime` | Container (Docker, Podman) and emulation runtime abstractions |
| `wrkflw-trigger-filter` | `on:` block parsing and change-set matching |
| `wrkflw-watcher` | File watcher with trigger-aware re-execution |
| `wrkflw-runs` | Run history, failure digests and JSON events |
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
