# wrkflw v0.9.0: a local pass that means GitHub will pass

Hello everyone,

v0.9.0 is out. Since v0.8.0 the goal has been one thing: when `wrkflw` says a workflow passes, GitHub should say so too, and faster than pushing and waiting.

## How close it gets

We rerun finished GitHub runs from nine open-source repositories (click, flask, cobra, chalk, ky, commander.js, rust-lang/log, itoa, fatih/color) with `wrkflw compare`, which runs the workflow locally for the same commit and event and compares each job's result with GitHub's. On the last sweep 70 jobs agreed and 4 differed. 18 of the 70 are `macos-*` jobs run as Linux, which agree unverified: they say the job's steps pass, not that they pass on macOS. Seven repositories agree on every job compared. Of the four that differ, one fails on a Rust release that came out after its GitHub run, and three need a GitHub token to verify an attestation.

On pallets/click, a warm local run takes 36 to 38 seconds against GitHub's 65, and a regression injected into `click.echo` was caught in 51.

## What's new

### JavaScript actions run

Actions such as `actions/github-script`, `codecov/codecov-action` or your own `./.github/actions/*` with `runs.using: node20` now run with Node in the job's container, with their inputs, `pre`, `post` and `$GITHUB_STATE`, as GitHub's runner runs them. Before, they passed without running and the run was marked `partial`. `WRKFLW_JS_ACTIONS=0` brings back the old behaviour, and `WRKFLW_SKIP_ACTIONS` leaves only the actions it lists unrun, such as one that deploys.

### Private repositories

A called workflow or action in a private repository now clones, with the `GITHUB_TOKEN` a secret provider gives the job, `GITHUB_TOKEN`/`GH_TOKEN`, or `gh auth token`. The token goes to git as a header, never into a URL, a file or the job.

### A faster first run

On Docker and Podman, a built-in job image on `ubuntu:latest` that doesn't set up Node.js starts from a runner base image published with the release, holding the packages, the runner's tools, Node.js, the GitHub CLI and the `runner` user, so a first run pulls it instead of installing it. Matrix legs run together, light jobs fit side by side by their measured memory, and images build with BuildKit.

### Jobs look like GitHub's runner

Steps run as `runner` with passwordless `sudo`, share one container per job, so what a step installs is there for the next, and get the runner's tools (`gh`, `jq`, `zip`, `python3`, Rust when steps call cargo, Docker with `--docker-in-jobs`). Services start, and composite actions hand `$GITHUB_ENV` and `$GITHUB_PATH` on to the job.

### Honest verdicts

Every run ends with a `verdict`: `verified`, `partial` or `nothing_checked`. `partial` lists what wasn't checked: a Windows or macOS job run as Linux, or a step whose action didn't run. `wrkflw gate` checks a push as CI will see it, reuses jobs that already passed, and says each cause on its own line. `wrkflw compare <run>` checks wrkflw itself against a finished GitHub run.

### For coding agents

`--format json` on every command, a recorded run history with failure digests, `wrkflw plan`, and an MCP server, registered with Claude Code, Codex, Cursor, Copilot CLI, Zed and others.

### And more

A Kubernetes runtime, GitLab `parallel:matrix` and `rules`, Gitea workflows, `-P` platform images, expression-driven matrices, `continue-on-error` on jobs, and a redesigned TUI on ratatui 0.30 with a light theme.

## Breaking changes

Steps run as the `runner` user, JavaScript actions run, a job's steps share one container, `~/.wrkflw` moved to the platform's directories, and `partial` covers skipped actions. Each is in [BREAKING_CHANGES.md](https://github.com/bahdotsh/wrkflw/blob/main/BREAKING_CHANGES.md) with how to keep the old behaviour.

## Install

```sh
cargo install wrkflw
```

or `brew install wrkflw`.

---

Tell me where your workflow behaves differently under wrkflw than on GitHub. `wrkflw compare <run URL>` shows exactly where, and those are the bugs I want.

**Project:** https://github.com/bahdotsh/wrkflw
