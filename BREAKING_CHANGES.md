# Breaking Changes

## A pass that skipped an action is `partial`, and library API changes (unreleased)

A `uses:` step whose action wrkflw didn't run now makes a passing run's `verdict` `partial` instead of `verified`, with an entry `{kind: "action_not_run", job, step, uses, reason}` under `unverified`. Every `unverified` entry now has a `kind`, and the entries for a job run as another OS have `kind: "runner"`. That covers a JavaScript action wrkflw has no implementation of, such as `codecov/codecov-action`, `actions/github-script` or a local `./` action whose `action.yml` runs on Node, and a setup action whose runtime isn't in the job's image: one in a job with `container:`, one inside a composite action whose runtime the job's own steps didn't set up, one whose version wrkflw can't install, and every setup action under emulation. `node-version` and `.nvmrc` aliases such as `lts/*`, `lts/iron` and `node` are now installed rather than ignored, and an `lts/` codename wrkflw doesn't know is refused, as setup-node refuses it, instead of installing the newest LTS. A `go-version` or `go.mod` that names a line such as `1.22` installs that line's newest release. A `*-version-file` must be a path inside the repository. An action inside a composite action is reported on the step that calls the composite action. Workflows that use such actions and used to report `verified` now report `partial`, and `wrkflw gate` prints its partial warning for them. Its exit code is unchanged.

`actions/setup-python` installs Python with uv instead of the deadsnakes PPA, under `/opt/uv/python` so steps that run as another user or set `HOME` find it, and a `python-version-file` pointing at `pyproject.toml` installs the newest release `requires-python` allows, as the hosted action does. `pip install` works in it, as on GitHub. The image built for a job with setup actions also gets `less` and, unless the job sets up Node or the base image has it, Node.js 22, as GitHub's Ubuntu runners have. Combined runtime images get new tags, so the first run after upgrading builds them again, and that build takes a little longer.

`wrkflw run` and `wrkflw plan` take `--branch` and `--tag`, the branch or tag the event is for, and the MCP `run_workflow` and `plan_workflow` tools take `branch` and `tag`. `wrkflw compare` passes the run's branch, or its tag for a run a tag push started, so a detached checkout of the run's commit fires the same `branches:` and `tags:` filters.

`wrkflw compare` and `wrkflw gate` pass `--platform` on to the runs they start, and the pre-push hook that `gate --install-hook` writes keeps it. Both used to accept the flag and ignore it, so their runs used the built-in images.

A job that fails before any step runs, because its image can't be built or pulled or its job container or network doesn't start, gets `cause: {kind: "setup_failed"}` in the failure digest, and `compare` says so in its note instead of reporting a plain failure.

For crates that use wrkflw as a library:

- `wrkflw_runtime::container::ContainerRuntime` requires `Sync`, and gains `image_id` and `identifies_images`, both with defaults. An implementation that isn't `Sync` no longer compiles.
- `wrkflw_executor::ExecutionConfig` gains `event_name`, `git_ref` (a full ref such as `refs/heads/main` or `refs/tags/v1.0`) and `base_branch`, and `StepResult` gains `actions_not_run`. Struct literals need the new fields.
- `wrkflw_executor::environment::create_github_context` takes a `RunRef`. Without `--event`, a workflow whose first trigger is a pull request now gets `GITHUB_HEAD_REF` and `GITHUB_BASE_REF` like one run with `--event pull_request`.
- `ExecutionEvent` gains `ActionNotRun`. An exhaustive `match` on it needs an arm.
- `wrkflw_executor::workspace::CleanReport` gains `volumes`, and `clean`'s JSON line a `volumes` count.

A job reused with `--reuse-results`, as `wrkflw gate` does, reports the actions it passed without running again, so a pass that was `partial` stays `partial`. Stored results from earlier versions can't say, so they aren't reused and the job runs once more. A step whose action ran its `command` input in its place, and one whose `action.yml` couldn't be fetched, are reported as not run too.

`wrkflw run` and `wrkflw plan` refuse `--tag` with a pull request `--event`, which is always for a branch. `--branch` and `--tag` drop a leading `refs/heads/` or `refs/tags/`, and refuse a name git wouldn't allow for a ref instead of quietly missing every `branches:` filter.

`actions/setup-python` with neither `python-version` nor `python-version-file` reads `.python-version`, as the hosted action does. `actions/setup-node` with no version keeps the runner's Node 22 instead of installing 20. An empty `*-version` input, as an unset workflow input leaves it, reads the version file. A `node-version` or `.nvmrc` that pins a full release such as `20.11.0` installs that release from nodejs.org instead of that major's newest. `go-version: oldstable` and Go pre-releases such as `1.22rc1` install; any other Go version wrkflw can't name is refused before the image build instead of failing it. Python pre-releases written as setup-python takes them, such as `3.14.0-alpha.1`, reach uv in its spelling. An `actions-rs/cargo` step that runs its `command` on the host in a container runtime is reported as not run. `compare` and `gate` pass every global flag they are given, such as `--every-matrix-leg`, `--fresh-workspace` and the `--k8s-*` settings, on to their runs, and the pre-push hook keeps them, not only `--platform`.

## `wrkflw gate` checks pull request workflows, and fails when it checked nothing (unreleased)

Without `--event`, the gate used to check every workflow as a push. On a feature branch, CI that only runs on pushes to `main` and on pull requests was skipped, and the gate passed with nothing run. Now a workflow that runs no job for a push, because it doesn't fire or every job is skipped by `if:`, is checked again as a pull request into the default branch, or into `--base-branch`. Pass `--event push` for the old behaviour.

The gate's JSON gains `verdict`, and each workflow gains the `event` and `base_branch` it was checked as. `passed` is false when no job ran for any workflow, with `verdict: "nothing_checked"`. The exit code is still 0 then, so a push no CI runs for isn't blocked. A script that read `passed: true` as "safe to push" now sees `false` in that case. The gate's JSON hasn't been in a release, so `schema_version` stays 1: read `verdict` to tell why `passed` is false. A workflow checked as a pull request after the push ran no job carries `fell_back_from`, with the push's `event`, `run_id` and `skip_reason`.

## `run_finished` says how much a pass proves (unreleased)

`run_finished` gains `verdict`: `verified`, `partial`, `nothing_checked`, `failed`, `error`, `skipped` or `interrupted`. `status` is unchanged. In text mode, a pass with jobs that ran as another OS, or with no job run, no longer prints the plain success line.

A job that didn't run because an earlier result stands has `reused` on `job_finished` and in `run_finished.jobs`, `stored` or `unaffected`. Failure digests gain an optional `cause`, `out_of_memory` when the output says memory ran out or `killed` for a SIGKILL without a stated reason, and a matrix leg that repeated a failed leg is listed under the original's `also_failed_in` instead of as a failure of its own. `error_lines` and `output_tail` no longer hold terminal colour codes, and `error_lines` only keeps lines with a whole error word that isn't a count of zero.

The canned "Common causes for cargo command failures" text is gone from failed cargo steps. It blamed the code for failures that came from the machine.

## Matrix `include` follows GitHub's rules (unreleased)

An `include` entry is now added to every original combination whose matrix values it doesn't overwrite, and becomes a combination of its own only when it fits none. Entries are never merged into combinations an earlier `include` created. An entry with only new keys used to become a combination of its own, and now adds its keys to every combination. A workflow planned with fewer or different legs than GitHub runs now gets the same ones.

## Matrix legs that only differ in runner and an unread label run once (unreleased)

A leg that differs from another in a value `runs-on` reads and in values nothing in the job reads, such as a `name` used only by the job's display name, now reports that leg's result instead of running. Such legs used to run. `--every-matrix-leg` runs them all.

## Off Linux, steps see the workspace in a container volume (unreleased)

On macOS and Windows, Docker and Podman steps no longer bind-mount the job's workspace directory. They see a container volume that wrkflw copies changes into before each step and back out after it, so file names that aren't UTF-8 and names that differ only in case work as on GitHub. Each step starts and ends a little later, by the time the copies take. The copies need the image `ubuntu:latest`, or the one `WRKFLW_WORKSPACE_HELPER_IMAGE` names. If the volume can't be set up, the job mounts the directory as before and says so. Files in the volume belong to root, so a step image that runs as another user can't change them, as on GitHub when the user's ID differs from the runner's. On Windows, whose files have no permission bits, files go into the volume executable, as the file share showed them. Set `WRKFLW_WORKSPACE_MOUNT=bind` to keep the old behaviour. A custom helper image needs GNU tar 1.32 or later.

## Docker and Podman runs limit parallel jobs on a small VM (unreleased)

When the Docker Desktop or Podman VM has less than 14 GB of memory, a run warns and sets aside the VM's memory divided by the number of whole 4 GB in it for each job, counting every job of the run together. A job measured on an earlier run sets aside half again its peak plus 256 MB instead, so light jobs run side by side from their second run. Measurements are kept in `job-memory.json` in the cache directory, and a job not run for 90 days is forgotten. Jobs share the VM's memory less a tenth of it, at least 1 GiB, kept for the engine and image builds. Only a job that passed records its peak, and a step killed for memory forgets the measurements of every job running beside it. A step the engine reports as killed for memory, or whose output shows a process killed with SIGKILL as cargo and gcc print it, forgets its job's measurements. Runs can take longer there. A Docker or Podman daemon on Linux without a VM is warned about but keeps its parallelism. Set `WRKFLW_MAX_PARALLEL_JOBS` to choose the limit on either.

## Smaller schema changes (unreleased)

A workflow without `name:` is named by its path, such as `.github/workflows/ci.yml`. It used to fail to parse. `strategy:` no longer needs a `matrix`. A top-level `cache-mode`, `deployment` on an `environment`, the `code-quality` permission, an empty env value and an expression in `timeout-minutes` are accepted. An expression timeout runs under the default timeout.

## An explicit `--runtime docker` or `--runtime podman` no longer falls back to the host (unreleased)

If Docker or Podman didn't answer within about a second, for example while Docker Desktop woke from Resource Saver, an explicit `--runtime docker` or `--runtime podman` ran every step on this machine and reported success. The TUI did the same after its runtime probe. Now the availability check waits longer, and a runtime you named that doesn't answer fails the run with a message saying what to start.

Only `--runtime auto`, the default, still falls back to emulation. If you relied on the fallback, drop `--runtime` or pass `--runtime emulation`.

The same release also changes what container steps see:

- Folders a step adds to `$GITHUB_PATH` go in front of the image's own `PATH`. They used to replace it with this machine's `PATH`, so tools in the image could vanish.
- Docker pulls each image once per run instead of before every step. A floating tag such as `ubuntu:latest` that moves during a run isn't picked up until the next run.
- `hashFiles('dir/**')` hashes the files under the directory. It used to return an empty string, so cache keys built from it change.

## `services:` start and can fail a job (unreleased)

On Docker and Podman, a job's `services:` now start, and its steps run in one container with them. A service that can't start or never turns healthy fails the job's container steps. These jobs used to log a warning and run without the service. Emulation, secure emulation, Kubernetes and called workflows still warn.

For such a job, this also changes how its steps run:

- Steps share one container, so files a step leaves outside the workspace are there for the next one. Each step used to start clean.
- The job's image needs `sh` and `env`, which the shared container uses to run steps. An image without them fails the job.
- `--preserve-containers-on-failure` doesn't keep the shared container or the services.
- A step that times out leaves a fresh container, and restarted services, for the next step, as the old one may still be running it. Data a step put in a service is gone after that.
- A `shell: sh` step in a Debian or Ubuntu image can't pass a variable whose name isn't an identifier, such as `INPUT_MY-INPUT`, on to its children, as dash drops it. This matches GitHub.
- Two services whose `ports:` name the same container port fail the job, as services share one network.
- A service's `volumes:` can't mount a host path, and its `options:` only take flags that shape the service container. `--privileged`, `-v`, `--mount`, `--network`, `--pid`, `--cap-add` and the like fail the job, as they would let workflow YAML reach this machine.

## Job-level `concurrency:` is enforced within a run (unreleased)

Jobs that share a `concurrency:` group now run one at a time, where they used to overlap. `cancel-in-progress` has no effect, and a run takes longer if it relied on the overlap.

## New public fields on `ExecutionConfig`, `Job` and `WorkflowDefinition` (unreleased)

`wrkflw_executor::ExecutionConfig` gains `acceleration`, `wrkflw_parser::workflow::Job` gains `concurrency`, and `wrkflw_parser::workflow::WorkflowDefinition` gains `permissions`. Code that builds any of them with a struct literal has to set the new field. `Default::default()` is what `acceleration` was before, and `None` is what `concurrency` and `permissions` were.

## A missing secret no longer fails a `run:` step (unreleased)

A `${{ secrets.X }}` that no provider has now expands to an empty string in `run:`, as it already did in `env:` and as it does on GitHub. The step used to fail with `Secret not found`. Anything relying on that failure should check for the value itself. A provider that fails for any other reason still fails the step.

## `wrkflw validate` reports more problems (unreleased)

Files that passed may now fail with `unknown-trigger-filter` and `unterminated-expression`, which GitHub rejects too.

`undefined-step-reference`, `undefined-need-reference` and `undefined-matrix-key` are warnings: GitHub accepts these workflows, so they appear in `issues` with `severity: warning` and leave `valid` true. Every issue now has a `severity`, and code treating any entry in `issues` as a failure should read `valid` instead.

## Failure digests fold matrix legs (unreleased)

Legs of a matrix job whose same step failed with the same exit code and output share one entry in `failures`, with the others in `also_failed_in`. The entry's `command` is the first leg's script. Code counting failed legs should read `jobs` on `run_finished`.

## `--event` needs a change set only for `paths:` filters (unreleased)

`--event` without `--diff` or `--changed-files` used to fail for every workflow under `--strict-filter`. It now fails only if the workflow has a `paths:` or `paths-ignore:` filter on that event.

## Unnamed `uses:` steps report the action (unreleased)

An unnamed step with `uses:` is reported as `actions/checkout@v4` rather than `Step 1`, including in run history. Log file names change with it. Unnamed `run:` steps keep `Step N`. `--from-step` accepts any name a run reports, and still accepts a 1-based position, which is how to reach a repeated action after the first.

## Step output in events and history has relative paths (unreleased)

The job workspace prefix is removed from `step_finished.output` and stored step logs, so `/tmp/.tmpAbC123/src/lib.rs` reads `src/lib.rs`.

## Step output is masked, and `run` steps report their exit code (unreleased)

`StepResult::output` and job logs now have secrets replaced with `***`, including values matching built-in token patterns. Previously nothing masked step output. Code that compared a secret's value against output will no longer match. Step outputs set with `::set-output::` keep their real values.

`run:` step output now starts with an `Exit code: N` line. `StepResult` gained an `exit_code` field holding the same number, so code that builds `StepResult` with a struct literal must set it.

GitLab `--variable` values are masked too, because they are often credentials. A value that is also ordinary text, such as `-V DEPLOY=true`, is replaced wherever that text appears in the output. Values under 3 characters are not masked.

## Container steps are no longer capped at 6 minutes (unreleased)

The Docker and Podman runtimes used to stop every step after 300 to 360 seconds. Steps now run until their job's or step's `timeout-minutes`, which defaults to 360 minutes as on GitHub. A step that hangs now holds its job for that long, so set `timeout-minutes` on steps that can hang.

## TUI: `7` opens the Runs tab, Help moved to `8` (unreleased)

The `h` and `?` shortcuts for Help are unchanged.

## `ValidationResult` gained `details` (unreleased)

Code that builds `ValidationResult` with a struct literal must use `ValidationResult::new()`. `issues` is unchanged.

## Runs are recorded by default (unreleased)

`wrkflw run`, `watch` and TUI runs now write logs to the local data directory, keeping the newest 500. Pass `--no-record` or set `WRKFLW_NO_HISTORY=1` to opt out.

## `actions/upload-artifact` follows GitHub's layout and rules (unreleased)

Artifact paths are now relative to where the `path:` search started, as on
GitHub. After uploading `out/*` or `out/f.txt` and downloading into `got/`, the
file is at `got/f.txt`, not `got/out/f.txt`. Update steps that read the old path.

A step that matches no files now passes with a warning. Set
`if-no-files-found: error` to keep failing, or `ignore` to silence the warning.
A missing artifact otherwise shows up at download, or not at all.

`!` lines apply in order, so a later line can re-include a file. A `path:` line
outside the workspace now fails the step instead of being skipped.

Hidden files are skipped unless `include-hidden-files: true` is set, as in
upload-artifact v4. A file or directory is hidden if its name starts with a dot,
so `path: .` no longer uploads `.git`, and `path: .config/` matches nothing.

Uploading to an existing artifact name now fails the step, as on GitHub. Files
used to be merged into the earlier upload. Give each matrix job its own name, or
set `overwrite: true` to replace the artifact. With `overwrite: true`, parallel
jobs sharing a name keep exactly one job's files, and which one is not defined.

A wildcard line that matches a directory now selects everything below it, as on
GitHub. `packages/*/dist` uploads the contents of each `dist`, `!**/node_modules`
excludes the contents, and `out/*` includes files in subdirectories of `out`.

## `ExecutionConfig` gained a `platform_overrides` field (unreleased)

Code that builds `wrkflw_executor::ExecutionConfig` as a struct literal must set
the new field, or it no longer compiles. `ExecutionConfig` and `RuntimeType` now
implement `Default`, so end a literal with `..Default::default()` to stay
compatible with later additions. The default runtime is `RuntimeType::Auto`.

## Jobs on act images now report real step results (unreleased)

Under the Docker runtime, any image name containing both `catthehacker` and
`act-` was treated as macOS emulation. Each step's command was discarded, so
steps on `ubuntu-*-medium` runners passed without running. Those steps now run
and can fail. Workflows that were passing there may start failing for real.

This also affected `uses:` steps on any runner label, because actions such as
`actions/cache` and the `actions/setup-*` family are sent to
`catthehacker/ubuntu:act-latest`. Their commands now run too, including those
derived from a `with.command` input, such as `npm run`, `cargo` and `pip`.

`RUNNER_OS`, `PATH` and `HOME` are also no longer overridden for those images.

Combined images for `actions/setup-*` jobs now put `/usr/local/bin` and
`/usr/bin` first on `PATH`, so they are rebuilt once on the next run.

## `~/.wrkflw` moved to the platform base directories (unreleased)

wrkflw no longer writes to a hardcoded `~/.wrkflw/`. The new locations follow
the XDG Base Directory spec on Linux and the native conventions elsewhere:

| Purpose | Before | Linux | macOS |
|---------|--------|-------|-------|
| `actions/cache` store | `~/.wrkflw/cache` | `$XDG_CACHE_HOME/wrkflw/cache` (`~/.cache/...`) | `~/Library/Caches/wrkflw/cache` |
| `RUNNER_TOOL_CACHE` | `~/.wrkflw/tools` | `$XDG_CACHE_HOME/wrkflw/tools` (`~/.cache/...`) | `~/Library/Caches/wrkflw/tools` |
| Default `file` secrets provider | `~/.wrkflw/secrets` | `$XDG_CONFIG_HOME/wrkflw/secrets` (`~/.config/...`) | `~/Library/Application Support/wrkflw/secrets` |

### Migration

Existing caches are not carried over: they repopulate on the next run, and the
old directories can be deleted with `rm -rf ~/.wrkflw/cache ~/.wrkflw/tools`.

`~/.wrkflw/secrets` keeps working as long as the new secrets location does not
exist. Move it to adopt the new path. Once the new location exists, the old one
is ignored.

> The entries below shipped in v0.8.0.

## `wrkflw run --event` requires change-set input by default (0.8.0)

`wrkflw run` now supports trigger-aware filtering via `--event`, `--diff`,
and `--changed-files`. When any of those flags is passed, the CLI runs a
prefilter that evaluates the workflow's `on:` block against the simulated
event context before execution — if the triggers don't match, the
workflow is skipped with a clean `exit 0`.

Under the new **`--strict-filter` default (on)**, running with `--event`
alone — without any way for wrkflw to know the change set — is rejected
with `exit 1`. Previously the CLI proceeded silently with an empty
change set, which meant every workflow gated on `paths:` was reported as
"not triggering" for reasons the user could not see.

Strict mode also rejects simulating `pull_request` / `pull_request_target`
without `--base-branch`, because GitHub Actions evaluates `branches:`
filters on PR events against the target branch, and the same silent-skip
mode applies there.

### Why

`wrkflw run --event push` used to proceed with `changed_files = []`,
causing every `paths:`-gated workflow to be silently rejected at
evaluation time. Users would then file "why didn't my workflow fire?"
issues for the non-obvious reason that no change set had been supplied.
Strict mode turns that silent failure into a loud, actionable error up
front — it is the default countermeasure for the same class of silent
skip the rest of the trigger-filter work patched iteratively.

### Impact

Scripts that invoked `wrkflw run --event <name> <workflow>` without
also passing `--diff` or `--changed-files` will now fail with:

```
Error: --event was supplied without --diff or --changed-files, so no
changed files are known and any workflow with a `paths:` filter would
be silently skipped. Pass --diff to auto-detect from git, --changed-files
to supply them explicitly, or --no-strict-filter to proceed anyway.
```

Scripts that invoked `wrkflw run --event pull_request <workflow>` without
`--base-branch` will similarly fail with a pointer at the missing flag.

`wrkflw watch --event pull_request` behaves the same way: missing
`--base-branch` is rejected under strict mode.

### Migration

Pick the option that matches your intent:

- **CI script that wants to run only workflows the diff would trigger:**
  add `--diff` (auto-detect base branch via `origin/HEAD` → `main` →
  `master` → `HEAD~1`) or pin with `--diff-base <ref>`.

  ```bash
  wrkflw run --diff --event push .github/workflows/ci.yml
  ```

- **CI script that has its own change set (e.g. from `git diff` in a
  wrapper):** pass it explicitly.

  ```bash
  wrkflw run --event push --changed-files src/main.rs,Cargo.toml \
      .github/workflows/ci.yml
  ```

- **Simulating a pull request locally:** add `--base-branch`.

  ```bash
  wrkflw run --event pull_request --base-branch main --diff \
      .github/workflows/ci.yml
  ```

- **Legacy warn-and-proceed behavior (not recommended):** opt out with
  `--no-strict-filter`. This restores the pre-strict-filter behavior of
  logging a warning and running every workflow anyway. Use this only if
  your scripts have already adapted to the old silent-skip semantics and
  you cannot change them right now.

### Prefilter exit codes

- `0` — triggers matched, workflow ran to completion (or was skipped
  because its triggers did not match the event). A skipped workflow
  prints `Workflow skipped: <reason>` and exits 0 to match GitHub
  Actions' own "nothing to do" semantics.
- `1` — something went wrong: flag validation, git error, strict-filter
  rejection, or workflow execution failure.

### Affected CLI

- `wrkflw run` — new flags: `--event`, `--diff`, `--changed-files`,
  `--diff-base`, `--diff-head`, `--base-branch`, `--activity-type`,
  `--strict-filter` (default on), `--no-strict-filter`.
- `wrkflw watch` — new subcommand. Same strict-filter gate for
  `--event pull_request` + missing `--base-branch`.

---

## Shell now matches GitHub Actions invocation (0.8.0)

The `bash` shell now executes with `bash --noprofile --norc -e -o pipefail -c`, matching GitHub Actions behavior. The `sh` shell uses `sh -e -c`. This means:

- Scripts exit immediately on the first command that returns a non-zero exit code (`-e` / errexit)
- In bash, a failure in any command of a pipeline causes the whole pipeline to fail (`-o pipefail`)
- User profile/rc files are not sourced (`--noprofile --norc`)

### Why

GitHub Actions runs `bash` steps with `bash --noprofile --norc -e -o pipefail {0}`. The previous wrkflw behavior of `bash -c` (without `-e` or `pipefail`) allowed scripts to silently continue past failing commands, which diverged from GHA semantics and could mask real failures.

### Impact

Multi-command `run:` scripts that relied on intermediate commands failing without aborting the step will now fail at the first non-zero exit. Piped commands where an earlier stage fails will also now fail. For example:

```yaml
- run: |
    false        # This now aborts the step
    echo "This no longer runs"

- run: |
    false | echo "pipeline now fails too"
```

### Migration

If a step intentionally tolerates command failures, either:

- Append `|| true` to the specific command: `might-fail || true`
- Use `continue-on-error: true` on the step
- Add `set +e` or `set +o pipefail` at the top of the script to opt out selectively

---

## EncryptedSecretStore serialization format (0.8.0)

The `EncryptedSecretStore` struct in `crates/secrets/src/storage.rs` has changed its serialization format:

- The shared `nonce` field has been **removed** from the struct.
- Each secret now stores its own random nonce **prepended to the ciphertext** (12 bytes nonce + ciphertext, then base64-encoded).

### Why

The previous design reused a single nonce across all secrets encrypted with the same key. Nonce reuse under AES-GCM is a critical vulnerability — it allows an attacker to XOR ciphertexts to recover plaintext differences and potentially forge authenticated messages.

### Impact

- Any `EncryptedSecretStore` serialized with the old format (containing a top-level `nonce` field) **cannot be deserialized** by the new code.
- Old ciphertexts (which did not have the nonce prepended) **cannot be decrypted** by the new code.

### Migration

There is no automatic migration. Users who have persisted encrypted secret stores must re-create them:

1. Decrypt all secrets using the old code (if still accessible).
2. Upgrade to the new version.
3. Re-encrypt and store the secrets.

### Affected API

- `EncryptedSecretStore::from_data` — dropped the `nonce: String` parameter.
- `EncryptedSecretStore` JSON serialization — no longer includes the `nonce` field.
