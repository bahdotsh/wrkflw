# Usage

## Validation

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

## Execution

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

# Set GitLab pipeline variables, which rules and scripts both see
wrkflw run --variable CI_COMMIT_TAG=v1.0.0 .gitlab-ci.yml

# List workflows and their jobs
wrkflw list --jobs

# Preserve failed containers for debugging
wrkflw run --preserve-containers-on-failure .github/workflows/ci.yml
```

## Custom runner images

By default `runs-on` maps to a small built-in image, so `ubuntu-latest` runs in `ubuntu:latest`, which has few of the tools a GitHub runner has. Use `-P`/`--platform` to pick the image for a label, for example the larger images made for [act](https://github.com/nektos/act):

```bash
wrkflw run -P ubuntu-latest=catthehacker/ubuntu:act-latest .github/workflows/ci.yml

# Repeat the flag or separate entries with commas
wrkflw run -P ubuntu-latest=my/image:1 -P ubuntu-22.04=my/image:2 .github/workflows/ci.yml

# Set it once for every command, including the TUI and watch mode
export WRKFLW_PLATFORM=ubuntu-latest=catthehacker/ubuntu:act-latest
```

- A job-level `container:` still wins over `--platform`, which wins over the built-in image.
- The built-in image is the base image plus tools the hosted Ubuntu runner has and steps call without installing: `sudo`, `jq`, `gh`, `unzip`, `zip`, `wget`, `xz`, `zstd`, `file`, `ssh`, `rsync`, `git-lfs`, `python3` with `pip`, a C toolchain and Node.js. It is built once and shared by every job. `gh` comes from cli.github.com in a layer of its own: when that can't be reached the job runs without it, with a warning, and the next run tries again. When the tools can't be installed at all, a job with nothing else to set up runs on the bare base image.
- Steps of a job in the built-in image run as `runner`, as on GitHub: a user with `/home/runner` for its home and `sudo` without a password, so a test that expects a permission error gets one. rustup and cargo are in its home, and it owns what `pip install` and `npm install -g` write to. On Linux the user has your uid, so what a step writes in the workspace stays yours; elsewhere it has GitHub's, 1001. Files an earlier run left in the workspace or the `--dep-cache` directories as root's are handed to that user before the first step. Steps run as root instead, with one warning per run, where that user can't write the job's files: under a rootless engine (not a rootless Podman whose containers keep your ids, with `userns = "keep-id"` or `PODMAN_USERNS=keep-id`), or when wrkflw itself runs as root on Linux. That is found before the first job, so their results are still kept and reused with `--reuse-results`, as root's. A job whose container shows another reason only once it is up, such as files an earlier run left that can't be handed over, runs as root with a warning of its own, and its result isn't kept. `WRKFLW_STEPS_AS_ROOT=1` runs them as root everywhere. A job `container:`, an image mapped with `--platform`, a step in a container of its own, and the Kubernetes and emulation runtimes are unchanged: they run as the image's user, or as you.
- The built-in image's packages, tools, Node.js, GitHub CLI and `runner` user are published with each release as `ghcr.io/bahdotsh/wrkflw-runner`, so a first run pulls them instead of installing them, and builds only the job's setup runtimes on top. Its tag is a hash of `docker/runner.Dockerfile`, the text this wrkflw would build itself. On Linux the `runner` user is given your uid in one more layer. The latest release's image is rebuilt each week with current packages, under the same tag, for machines that pull it for the first time: a machine keeps the copy it pulled, as it keeps an image it built, until you remove the `ghcr.io/bahdotsh/wrkflw-runner` image and the `wrkflw-combined:base-*` image made from it, and the next run pulls the current one and builds the job images on it anew. Where it can't be pulled, wrkflw builds the whole image here as before: when the registry refused it (a wrkflw whose Dockerfile was never published) it doesn't try again for a day, and when it couldn't be reached (offline, or a firewall in front of ghcr.io) for an hour. A job that sets up Node.js, a job on another base, and Kubernetes still build the whole image. `WRKFLW_PREBUILT_IMAGES=0` always builds here, and `WRKFLW_RUNNER_IMAGE` names another base image to start from.
- An image you map with `--platform` gets none of these tools, also when a setup action's runtime is installed on it. Map `ubuntu-latest=ubuntu:latest` to run on the bare Ubuntu image.
- Labels match case-insensitively, and the last value given for a label wins. Every label in `runs-on: [self-hosted, linux]` is tried, and `runs-on: ${{ matrix.os }}` is resolved per matrix combination.
- Runtimes from `actions/setup-*` are installed on top of your image, so the image must be Debian or Ubuntu based for those jobs.
- `--platform` replaces `WRKFLW_PLATFORM` entirely when both are set. It has no effect in emulation mode.

## Faster local reruns

Two defaults make a rerun cheaper than a hosted runner's cold start. Both work the same for any language or toolchain.

- **Each GitHub Actions job keeps its workspace.** GitLab pipelines still start each run in an empty workspace. Checkout syncs the source files into it, and leaves alone anything the repository ignores, such as build output, `node_modules` or a virtualenv. The second run of a build is therefore incremental. Unchanged files keep their timestamps, and files deleted from the repository are deleted from the workspace too. Workspaces live under your cache directory in `wrkflw/workspaces` (`~/Library/Caches` on macOS, `~/.cache` on Linux), one per repository, workflow file and job, and apart for a job run on this machine (emulation, or `--macos-on-host`) and the same job in a container: what a build leaves for one is wrong for the other. Upgrading to 0.9.0, a job run with `--runtime emulation` starts from an empty workspace once, and `wrkflw clean` removes its old one. Set `WRKFLW_WORKSPACES_DIR` to keep them somewhere else, such as a larger disk. `--fresh-workspace` or `WRKFLW_FRESH_WORKSPACE=1` gives every job an empty workspace, as a hosted runner does.
- **Matrix legs that only pick a runner run once.** Locally, `runs-on: ${{ matrix.os }}` maps every OS to the same image. If a leg differs from another only in values that `runs-on` reads, or that nothing in the job reads, such as a `name` the job's display name uses, and at least one of them picks the runner, and the two would run in the same image, that leg reports the other's result instead of repeating it. Legs on the same runner always run, so `attempt: [1, 2, 3]` still repeats a job. Its history record and its `job_finished` event carry `repeated_from` with the leg it repeats. A leg whose workflow reads the value, or `strategy.job-index`, or which runs in a different image (see `--platform`), still runs, as do all legs of a `runs-on` that reads the matrix by index (`matrix['os']`). Only the workflow file is read: a script that reads `$MATRIX_OS` from a file of its own isn't seen, so pass `--every-matrix-leg` for such a job. `--every-matrix-leg` or `WRKFLW_EVERY_MATRIX_LEG=1` runs them all.

What a kept workspace means in practice:

- In a git repository, checkout brings every file git tracks, even one an ignore rule matches, as a clone would. Untracked files follow git's ignore rules: every `.gitignore` in the tree, `.git/info/exclude` and your global excludes file. Tracked dotfiles such as `.cargo/config.toml` are synced, and removed once the repository deletes them. Hidden files a tool leaves in the workspace, such as `.pytest_cache`, are kept. wrkflw records what git tracked at each checkout in a file beside the workspace, never by running git in the workspace, whose `.git` a step can change. Outside a git repository, or without `git` installed, only the ignore rules apply, and most hidden files are skipped.
- Checkout copies the repository's symlinks as links, with the same target, as git checks them out, and never follows one. A link a step left where the repository has a file or directory is replaced, so a step can't make the next checkout write or delete outside the workspace.
- `.git` is mirrored so `git describe` and `git diff` work. Its objects are hard-linked where the cache directory shares a filesystem with the repository, so they take no extra space. Git never rewrites an object in place, so the links are safe for git itself. A step that writes into `.git/objects` directly, or changes permissions or owners there, changes those files in your repository too, since a hard link is the same file. Hooks are left out, as in a fresh clone, and so are the `core.hooksPath`, `core.fsmonitor` and `core.worktree` settings and any `include` or `includeIf` section, which would run programs from your machine, point outside the workspace or read another config file.
- A job that never runs `actions/checkout` finds whatever its last run left behind, not an empty directory. The run log says when a job reuses its workspace.
- Files a step writes, including credentials such as `.npmrc` or a kubeconfig, stay in the workspace until the next run overwrites them or you clean it. Use `--fresh-workspace` for jobs where that matters.
- If the cache directory is inside the directory you run from, as when you run from your home directory, checkout leaves the workspaces out.
- `wrkflw clean` removes every kept workspace no run is using and says how much space it freed. It leaves each job's small lock file, so a run starting at the same moment still has the workspace to itself. Deleting the directory by hand is also safe; each job then starts cold once.
- `cargo test` in this repository sets `WRKFLW_FRESH_WORKSPACE=1` through `.cargo/config.toml`, so tests never keep workspaces. The same applies to `cargo run`; set `WRKFLW_FRESH_WORKSPACE=0` to try kept workspaces from a source build.

While a step runs, the TUI shows what it prints: the Run tab's Output pane shows the full output, and the Log story shows its last lines. Error, warning and notice annotations appear as `Error: …` lines, as on GitHub. Other workflow commands are hidden, and every line is masked before it is shown. An `::add-mask::` value is masked from the line after it; lines printed before it are already shown. A progress bar that redraws its line with carriage returns shows as one line, in its latest state once the line ends. The Output pane says how many earlier lines it isn't showing.

wrkflw's own log, shown in the TUI and with `--verbose`, masks every secret, every `::add-mask::` value, and each `--variable` value whose name suggests a credential (a part ending in `TOKEN`, `SECRET`, `PASSWORD`, `KEY` and the like) or that is 16 characters or longer. Other `--variable` values, such as `BRANCH=main`, stay readable there. A value stays masked until the next run starts, so a session that runs many workflows, as the TUI does, doesn't mask one run's values in another's log. The log names the environment variables a container gets, never their values. Step output, both live in the TUI and as stored, is masked for every `--variable` value.

## Trigger-aware execution

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

See [BREAKING_CHANGES.md](../BREAKING_CHANGES.md) for full migration notes.

## Spending less on CI

All of this is opt-in. Without these flags a run behaves as it always has.

```bash
# Check everything CI would run for this push, reusing what already passed.
# On a feature branch, a workflow a push doesn't fire is checked as a pull
# request into the default branch. The result says how much the pass proves:
# verified, partial (some jobs ran as another OS, or passed a step whose
# action wrkflw can't run) or nothing_checked.
wrkflw gate

# Run it on every push (an existing hook is left alone). The hook keeps the
# --runtime and --remote-cache given here, and compares against what the
# remote already has for the pushed branch. It refuses a core.hooksPath
# outside the repository, which other repositories share. It checks the
# working tree, so uncommitted changes count, and it warns when there are any.
wrkflw gate --install-hook

# The same pieces, one at a time
wrkflw run .github/workflows/ci.yml --reuse-results
wrkflw run .github/workflows/ci.yml --event push --affected
wrkflw run .github/workflows/ci.yml --dep-cache
```

- **`--reuse-results`** skips a job when an identical job already passed on identical inputs. The fingerprint covers the job definition, workflow `env` and `defaults`, `needs` outputs, event, ref, runtime, operating system, CPU architecture, `--platform` overrides and file contents. Stored outputs still reach dependent jobs. Results expire after 7 days, or `WRKFLW_RESULT_TTL_SECS`, because images and floating action tags such as `@v4` aren't in the fingerprint.
- **`--affected`** also skips a job when none of its declared paths changed, even with no stored result. It trusts the base you compare against to be green. The pre-push hook compares against the remote tip of the branch you push, which is green only if that push's CI passed. If it may not have, run `wrkflw gate --diff-base origin/main` by hand. It needs `--event`, and only applies to jobs listed in `.wrkflw/affected.yml`. Such a job counts as passed for its dependents and reports one `Not affected` step. A job still runs if it has `outputs` or a matrix, if a job it needs ran or didn't pass, or if a workflow file changed.
- **One container per job** is how a job's `run:` steps run on Docker and Podman. What a step leaves outside the workspace, such as a tool from `go install`, `apt-get install` or `rustup toolchain add`, is there for the next step, as on GitHub. A step that times out may still be running in that container, so the next step gets a fresh one, and a job's services restart with it. `--step-containers`, or `WRKFLW_STEP_CONTAINERS=1`, gives each step a container of its own instead, as `--preserve-containers-on-failure` does. The variable also reaches runs started from the TUI and `wrkflw watch`. `--job-container` is still accepted and changes nothing, even beside `--step-containers`.
- **`--docker-in-jobs`**, or `WRKFLW_DOCKER_IN_JOBS=1`, starts a Docker engine inside the container of each job whose `run:` steps run `docker`, as hosted runners have one. That container runs privileged, so it is off unless you ask: only use it for workflows you trust. The engine is the job's own, so `docker run -v $(pwd):/src` mounts the job's workspace, and its images and containers go with the job. Each such job says when its container starts privileged, so a variable left set in a shell doesn't grant it unseen. The variable also reaches runs started from the TUI and `wrkflw watch`. Without the flag such a step fails with `docker: command not found`, and the run says so before it starts. It works on Docker only: a rootless Podman container can't run an engine, and the run says so. An image you map with `-P` gets no engine added either, as it gets no runner tool: it must bring its own. A job counts as running `docker` when its `run:` script calls it as a shell would, including through `sudo`, `env`, `timeout` or `nice`, inside `$(...)`, in `bash -c '...'` and in a heredoc a shell reads; `command -v docker`, a quoted string, a `case` pattern and a heredoc that `cat` writes don't count. A `docker` run by `make`, by a script the step calls or through a variable isn't seen, and that job gets neither the engine nor the warning. The same reading decides which jobs get the runner's Rust, which an image you map with `-P`, or a `rust` image, doesn't get: those keep their own.
- **`--dep-cache`** mounts host directories over the cargo, npm, pip and Go caches in root's home directory and, in the built-in image, in the `runner` user's too, so it helps steps that run as either. An image you map with `--platform` only gets root's: its own user's home is left alone. Steps that always run as root, as in a job `container:`, a mapped image, a step in a container of its own or a run with `WRKFLW_STEPS_AS_ROOT=1`, get a second set of directories, so they never leave root's files where a `runner` step must write. In the built-in image that set goes in the `runner` user's home too, as `CARGO_HOME` is there for root as well. The host directories live under `WRKFLW_DEP_CACHE_DIR` if set, or the user cache directory, with one set per repository. Steps can write to them, so a job you don't trust can leave packages there for later runs of the same repository, but not for other repositories. They are wrkflw's own directories, not your `~/.cargo` or `~/.npm`.

Reuse is only as safe as its inputs. wrkflw always runs a job that reads secrets or a `github-token` input, has `write` permissions of its own or from the workflow's `permissions:`, exchanges artifacts, uses a known publishing action such as `actions/github-script`, `softprops/action-gh-release` or `docker/build-push-action`, or calls a reusable workflow, because those can have effects it can't see. It also always runs a job with `outputs` that uses `needs` when a job upstream reads secrets, so a passed-along secret doesn't get stored. A job with no `permissions:`, in a workflow with none, counts as read-only, though on GitHub it gets the repository's default token permissions, which are write-all for repositories created before February 2023. For anything else with outside effects, set `cache: false`. That includes a job that gets its credentials from the machine rather than from `secrets`, such as a deploy that uses `~/.aws`, `gh auth` or `~/.npmrc` in emulation: wrkflw can't see those, so it would reuse the result and the deploy would silently not happen.

By default every file in the repository is an input, so any change reruns every job. `.wrkflw/affected.yml` narrows that. It lives outside the workflow file because GitHub rejects unknown keys there.

```yaml
jobs:
  test:
    paths: ['src/**', 'Cargo.lock']   # what this job reads
  publish-preview:
    cache: false                      # always run: never reuse or skip it
  deploy:
    gate: false                       # leave it to CI: `wrkflw gate` skips it
```

`wrkflw gate` runs every job a push would, with this machine's credentials, and it can't tell a deploy from a test. Mark deploys, releases and anything else that must only run in CI with `gate: false`. The gate reports them as skipped, and skips the jobs that need them too. It warns about a job that always runs because it reads secrets or publishes, unless that job has `gate: false` or `cache: false`.

Paths you leave out go unnoticed, so list everything the job reads. `*` doesn't cross `/`, so `*.rs` only matches files at the root. Use `**/*.rs`, and wrkflw warns about a wildcard path with no `/`. `.github/actions/**`, `.gitea/actions/**`, this file and the directory of each local action the job uses, such as `uses: ./tools/release`, always count.

### Sharing results across machines

```bash
export WRKFLW_CACHE_SIGNING_KEY=...       # required
export WRKFLW_REMOTE_CACHE_TOKEN=...      # optional bearer token

# In trusted CI on your main branch: run every job and publish its result
wrkflw run ci.yml --remote-cache https://cache.example.com/wrkflw --cache-push

# Anywhere else: read
wrkflw run ci.yml --reuse-results --remote-cache https://cache.example.com/wrkflw
```

The location is a directory, a `file://` path, or an `https://` base that serves `GET` and `PUT` on `<base>/<key>.json`. Entries are HMAC-signed, and one that doesn't verify is ignored. The key is symmetric, so anyone who can read results with it could also forge one given write access to the store. A forged result does more than mark a job as passed: its outputs reach `run:` scripts of dependent jobs, so the key plus write access means running commands on every machine that reads the store. Keep store write access to trusted CI, and don't expose the key to pull requests from forks.

`--cache-push` publishes every passing job, with or without `--reuse-results`. Add `--reuse-results` to also skip jobs the store already has. If the remote fails to answer, or refuses the token, wrkflw warns, stops using it for the rest of that run and runs the jobs. Only a 404 counts as a miss.

Local results older than the TTL are removed the first time a run stores one.

Stored outputs aren't encrypted. A job that runs in emulation never pushes, because its result depends on the tools of that machine. wrkflw removes `WRKFLW_CACHE_SIGNING_KEY` and `WRKFLW_REMOTE_CACHE_TOKEN` from its own environment at startup, so steps it runs, in emulation or not, never see them.

## Watch mode

```bash
# Watch .github/workflows for changes and rerun affected workflows
wrkflw watch

# Watch a specific path, simulate pull_request, and cap concurrency
wrkflw watch --event pull_request --base-branch main \
    --max-concurrency 2 --debounce 750 .github/workflows

# Ignore extra directories on top of the built-in list
wrkflw watch --ignore-dir .terraform --ignore-dir coverage
```

## TUI

```bash
# Open TUI with default directory
wrkflw tui

# Open with specific runtime
wrkflw tui --runtime podman

# Force a color theme
wrkflw tui --theme light
```

The TUI follows your terminal's background and uses a light or dark palette to match. Set `--theme` or `WRKFLW_THEME` to `auto`, `dark` or `light` to override it. The theme is chosen at startup, so restart the TUI after switching your terminal's theme.

**Tabs:** Workflows · Run (jobs, graph and log views of the current run) · History · Dispatch · Setup (runtime and secret providers). Help opens over any tab with `?`.

**Controls:**

| Key | Action |
|-----|--------|
| `Tab` / `Shift+Tab` | Switch tabs |
| `1`–`5` | Jump to tab by number |
| `w` | Jump to Workflows |
| `x` / `g` / `l` | Run tab: jobs / graph / log view |
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
| `t` | Dispatch the workflow on GitHub / GitLab |
| `,` | Open Tweaks overlay |
| `?` / `h` | Toggle help overlay |
| `q` / `Ctrl+C` | Quit |
| `Esc` | Back (quits from the top level) |

Run → jobs: `Enter` inspects a job's steps, `Tab` switches between Output and Matrix, `PgUp`/`PgDn` scroll a step's output and `End` returns to its tail. Run → graph adds `g` (graph↔list), `←`/`→` (scroll stages), `↑`/`↓` (previous/next workflow), `Enter` (back to jobs). Run → log adds `o` (story↔raw log), `s` (search), `f` (filter), `c` (clear), `n` (next match), `PgUp`/`PgDn` (page) and `Home`/`End` (first line / follow new lines). Dispatch tab adds `p` (github↔gitlab), `b` (edit branch), `i` (edit inputs), `+` (add input), `c` (copy curl preview); while a field is being edited, `Tab` moves between fields and `Enter` or `Esc` stops editing.

The layout adapts to the terminal size: side panes drop out and tabs collapse to numbers on narrow terminals, and step output keeps the tool's own colours.

Drag with the mouse to select text; it is copied to the clipboard when you release. A selection stays inside the pane it started in, a click or any key clears it, and the wheel scrolls. Hold your terminal's bypass modifier (usually Shift, or Option in iTerm2) to use its native selection instead. Over SSH, copying relies on your terminal supporting OSC 52.

## Remote Triggering

Trigger `workflow_dispatch` events on GitHub or GitLab.

```bash
# GitHub (requires GITHUB_TOKEN env var)
wrkflw trigger workflow-name --branch main --input key=value

# GitLab (requires GITLAB_TOKEN env var)
wrkflw trigger-gitlab --branch main --variable key=value
```
