# wrkflw for coding agents

wrkflw validates and runs GitHub Actions and GitLab CI workflows locally. Use it to check a workflow change before pushing.

## Loop

1. After editing YAML, run `wrkflw --format json validate`.
2. Run `wrkflw --format json plan <file>` to see job order, matrix legs, whether a trigger fires, and under `unverified` which jobs would run as another OS than `runs-on` asks for. Pass the `--runtime` you will run with.
3. Run `wrkflw --format json run <file> --intent "why you are running this"`.
4. On failure, read the `failure` object in the final `run_finished` line. Don't rerun to see the error again.
5. For earlier runs, use `wrkflw runs list`, `runs failure <id>` and `runs logs <id> --step <name>`.
6. Before calling a pass done, read `verdict` on `run_finished`, not only `status`. `verified` means every job that ran did so as the OS its `runs-on` asked for. `partial` means some didn't: they are listed under `unverified`, such as `windows-latest` run as Linux, so steps gated on `runner.os` and OS-specific behaviour were not checked. `nothing_checked` means it passed because no job ran. Say which, instead of claiming CI will pass.

Prefer the MCP server if your client supports MCP. `wrkflw mcp install` registers it with Claude Code, Codex, Cursor, Windsurf, VS Code, Gemini CLI, opencode, Claude Desktop, GitHub Copilot CLI, Grok Build, Kiro CLI, pi and Zed. Pass a name, such as `wrkflw mcp install codex`, for one agent, and `--dry-run` to see the change first. By hand:

```json
{"mcpServers": {"wrkflw": {"command": "wrkflw", "args": ["mcp", "serve"]}}}
```

## Before pushing

Run `wrkflw --format json gate`, or the MCP `gate` tool. It runs what CI would run for your push, reusing jobs that already passed on the same files, and exits 1 if anything fails.

Without `--event`, each workflow is checked as a push. On a branch other than the default one, a workflow that runs no job for a push, because it doesn't fire or every job is skipped, such as CI that runs on pushes to `main` and on pull requests, is checked again as a pull request into the default branch (`origin/HEAD`, else `main` or `master`). Pass `--base-branch` for another target, or `--event` to check one event only.

It prints one `gate` object: `verdict`, `passed`, `failed` (paths), `unverified` (with `workflow`), `uncommitted_changes`, and per workflow its `status`, `verdict`, the `event` (and `base_branch`) it was checked as, `run_id`, `jobs` and `failure` digest. A workflow checked as a pull request because the push ran no job has `fell_back_from`: the push's `event`, `run_id` and `skip_reason`. `verdict` is `verified`, `partial` (passed, with jobs under `unverified`), `nothing_checked` (no job ran for any workflow) or `failed`. `passed` is true only when something ran and nothing failed. A gate that checked nothing exits 0, so a push that no CI runs for isn't blocked, and prints a warning on stderr. In text mode a `partial` gate says so in its summary line instead of the plain `gate passed`.

Add `--reuse-results` to `wrkflw run`, or `reuse_results` to the MCP `run_workflow`, to get the same reuse while iterating. A job that didn't run because an earlier result stands has `reused` on its `job_finished` and `run_finished.jobs` entry: `stored` for a pass reused under the same fingerprint, `unaffected` for a job `--affected` found no change for. Its recorded run keeps one `Reuse result` or `Not affected` step, so its output isn't there to read. See "Spending less on CI" in the README for `.wrkflw/affected.yml`.

## Checking wrkflw against GitHub

`wrkflw compare <run id or URL>` runs a workflow locally as a finished GitHub Actions run did, for the same event, and lists each job's result on both sides. The working tree must be the commit the run tested: check it out first, in a worktree if you like. It prints one `compare` object: `agree`, `disagreements`, `basis` (`jobs` when jobs were paired by name, `run` when only the run's result could be compared, `none` when nothing could), a `problem` saying why the comparison says less than a per-job one would, the GitHub run, the local run's `status` and `verdict`, and per job `github_name`, `local_name`, `github`, `local`, `agree` (`null` when GitHub cancelled or skipped it before running) and a `note` on why they may differ, such as a job run as another OS or killed for lack of memory. It exits 0 only when something was compared and nothing differed: a local run that errored, ran no job, or can't be compared at all never agrees. A job only one side has a result for, such as a called workflow's job, is named in `problem`: a failure there counts as a disagreement, and the runs' own results are compared too, so a run GitHub failed in a job wrkflw couldn't pair never agrees with a local pass. It reads the API with `GITHUB_TOKEN`, or the token `gh auth token` prints, or none for a public repository.

## Simulating events

- `--event pull_request` needs `--base-branch main`. Without it the command fails, on purpose.
- Add `--activity-type opened` if the workflow uses `types:`.
- If the event has a `paths:` filter, pair `--event` with `--diff` or `--changed-files a,b`, or the command fails, because the filter would have nothing to match.
- A skipped run exits 0 with `run_finished` status `skipped`. Check `status`, not only the exit code.

## Output contract

`--format json` is a global flag. stdout carries only JSON and logs go to stderr. Every object has `schema_version` and `type`.

| Command | stdout |
| --- | --- |
| `run`, `watch`, `runs replay` | One object per line: `run_started`, `job_started`, `step_started`, `step_output` (with `--stream-output`), `step_finished`, `job_finished`, `run_finished`. `watch` adds `watch_started` and `watch_cycle`. |
| `validate`, `list`, `plan`, `runs ...`, `clean`, `gate` | One object. |
| Any error | `{"type": "error", "message": "..."}`, exit 1. |

`step_finished.output` holds the last 64 KiB of a step's output, delivered when the step ends. To follow a long step before it ends, add `--stream-output` to `run`: it adds `step_output` lines, `{job, step, index, lines}`, as the step prints, masked like the rest. Without it no `step_output` lines are printed.

`clean` prints `{"schema_version": 1, "type": "clean", "removed": n, "bytes": n, "in_use": n, "failed": [{"path": "...", "error": "..."}]}`: workspaces removed, the bytes they held, workspaces left because a run holds them, and workspaces that could not be removed. It exits 1 when `failed` is not empty, after cleaning the rest.

A job that does not run, because of its `if:`, a failed `needs` job, fail-fast, a GitLab rule or `gate: false`, still gets `job_started` and `job_finished`, with `status` `skipped` and `duration_ms` 0.

`run_finished` has `verdict`: `verified`, `partial`, `nothing_checked`, `failed`, `error`, `skipped` or `interrupted` (see the loop above). It has `unverified`, a list of `{job, requested, ran_as, reason}` for jobs that ran as another OS than `runs-on` asked for, empty when there are none. The same job's `job_finished` and its `run_finished.jobs` entry carry `runner: {requested, ran_as}`. A `runs-on` expression that reads only `matrix`, such as `${{ matrix.os || 'ubuntu-latest' }}`, is evaluated per leg. One that reads another context, such as `needs`, is not judged.

A `job_finished` with `repeated_from` is a matrix leg that did not run, because locally it would only have repeated the named leg. Its `status` is that leg's and its `duration_ms` is 0. Pass `--every-matrix-leg` to run every leg.

`failures` has one entry per distinct failure, not per failed job, so count `jobs` on `run_finished` instead. `also_failed_in` lists matrix legs where the same step failed with the same exit code and output, ignoring durations, and legs that repeated a failed leg without running. `command` is the first leg's script. `error_lines` holds lines above `output_tail` that report an error: a whole word such as `error` or `FAILED`, not part of one such as `thiserror`, and not a count of zero such as `0 failed`. Both are plain text, without terminal colours. `cause`, when set, is `{kind, evidence, hint}` for a failure that may be the machine's rather than the code's. `out_of_memory` means the output says memory ran out: give the VM more memory before changing code. `killed` means a process got SIGKILL, such as a linker that printed `signal 9` or a step that exited 137, and the output doesn't say why. In a small Docker VM that is usually memory too, but first check that no step kills processes or stops containers. `note` warns about the run itself, such as `--from-step` having skipped earlier steps.

Workspace paths in step output are relative, so `tests/test_app.py` means that file in your repository.

Exit codes: 0 for success or skipped, 1 for a failed run or failed validation, 2 for a usage error.

## Secrets

A secret no provider has expands to an empty string, as on GitHub. `plan` lists these under `missing_secrets`, and a step that reads one ends its output with a warning naming it. To provide a value, set an environment variable with the secret's exact name, such as `DEPLOY_TOKEN`, before running wrkflw. A secret read from a named provider, as in `secrets.vault:TOKEN`, is listed as `vault:TOKEN` and has to be added to that provider.

## Validation

Each issue has a `severity`. An `error` makes the file invalid. A `warning`, such as a reference to an undefined step, leaves `valid` true because GitHub accepts it, but it usually means the value will be empty.

## History

Runs are stored under `$WRKFLW_HOME/runs`, defaulting to the platform's local data directory. The newest 500 are kept, or `WRKFLW_RUNS_KEEP`. Use `--no-record` or `WRKFLW_NO_HISTORY=1` to skip recording.

Known secrets and `::add-mask::` values are masked before output is stored. `--variable` values are masked in output and never stored, so pass them again to `runs replay`. Logs can still hold anything a step printed that wrkflw did not recognise as a secret, so run directories are readable only by the owner on Unix. On Windows they rely on the data directory's permissions.

Identify yourself with `WRKFLW_AGENT=<name>` or `--agent`. Without either, the agent comes from `AI_AGENT`, or from what Claude Code, Codex, Gemini CLI, opencode, Cursor or Augment set. Other agents aren't detected, so set `WRKFLW_AGENT`. `WRKFLW_INTENT` or `--intent` records why you ran it.

The run also records your session, from `--session`, `WRKFLW_SESSION`, `CLAUDE_CODE_SESSION_ID` or `CODEX_THREAD_ID`. Use `wrkflw runs list --session <id>` to find the runs of one session. Under MCP the server can outlive your session, so the inherited id is ignored: pass `session` to `run_workflow` instead.

## Limits

- `--affected` trusts the paths in `.wrkflw/affected.yml`. A path left out of a job's list won't rerun that job.
- `wrkflw gate` checks the working tree, not the commits you push, and skips jobs marked `gate: false` in `.wrkflw/affected.yml`.
- Docker Desktop and Podman machines often have half the memory of a GitHub runner. With Docker or Podman, a run warns when the VM has less than 14 GB and runs at most one job or matrix leg per 4 GB at a time, counting every job of the run together. A native Linux daemon with less memory is warned about but not limited. Set `WRKFLW_MAX_PARALLEL_JOBS` to choose the limit on either.
- A checkout from a git worktree or submodule gets a real `.git` directory in the job's workspace, on the same branch, so steps that run git work as on GitHub.
- Parallel jobs run one after another in emulation, so `--runtime emulation` won't show a race between jobs.
- `--from-step` needs `--job`. Earlier steps are skipped apart from `actions/checkout`, so their outputs and files are missing. Use it to iterate on a step that doesn't depend on them.
- `runs replay` reruns against the current working tree. Compare `git_sha` and `git_dirty` on the record if the result differs.
- `--runtime emulation` runs steps directly on this machine. Use a container runtime or `secure-emulation` for code you don't trust.
- `plan` reports a matrix computed by expressions as `dynamic`, because it is only known at run time.
