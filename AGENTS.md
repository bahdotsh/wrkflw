# wrkflw for coding agents

wrkflw validates and runs GitHub Actions and GitLab CI workflows locally. Use it to check a workflow change before pushing.

## Loop

1. After editing YAML, run `wrkflw --format json validate`.
2. Run `wrkflw --format json plan <file>` to see job order, matrix legs and whether a trigger fires.
3. Run `wrkflw --format json run <file> --intent "why you are running this"`.
4. On failure, read the `failure` object in the final `run_finished` line. Don't rerun to see the error again.
5. For earlier runs, use `wrkflw runs list`, `runs failure <id>` and `runs logs <id> --step <name>`.

Prefer the MCP server if your client supports MCP. `wrkflw mcp install` registers it with Claude Code, Codex, Cursor, Windsurf, VS Code, Gemini CLI, opencode, Claude Desktop, GitHub Copilot CLI, Grok Build, Kiro CLI, pi and Zed. Pass a name, such as `wrkflw mcp install codex`, for one agent, and `--dry-run` to see the change first. By hand:

```json
{"mcpServers": {"wrkflw": {"command": "wrkflw", "args": ["mcp", "serve"]}}}
```

## Before pushing

Run `wrkflw gate`. It runs what a push would run, reusing jobs that already passed on the same files, and exits 1 if anything fails. Add `--reuse-results` to `wrkflw run` to get the same reuse while iterating. A reused job reports one `Reuse result` step and an unaffected job one `Not affected` step, so their output isn't there to read. See "Spending less on CI" in the README for `.wrkflw/affected.yml`.

## Simulating events

- `--event pull_request` needs `--base-branch main`. Without it the command fails, on purpose.
- Add `--activity-type opened` if the workflow uses `types:`.
- If the event has a `paths:` filter, pair `--event` with `--diff` or `--changed-files a,b`, or the command fails, because the filter would have nothing to match.
- A skipped run exits 0 with `run_finished` status `skipped`. Check `status`, not only the exit code.

## Output contract

`--format json` is a global flag. stdout carries only JSON and logs go to stderr. Every object has `schema_version` and `type`.

| Command | stdout |
| --- | --- |
| `run`, `watch`, `runs replay` | One object per line: `run_started`, `job_started`, `step_started`, `step_finished`, `job_finished`, `run_finished`. `watch` adds `watch_started` and `watch_cycle`. |
| `validate`, `list`, `plan`, `runs ...`, `clean` | One object. |
| Any error | `{"type": "error", "message": "..."}`, exit 1. |

`step_finished.output` holds the last 64 KiB of a step's output, delivered when the step ends. Output is not streamed while a step runs.

`clean` prints `{"removed": n, "bytes": n, "in_use": n, "failed": [{"path": "...", "error": "..."}]}`: workspaces removed, the bytes they held, workspaces left because a run holds them, and workspaces that could not be removed. It exits 1 when `failed` is not empty, after cleaning the rest.

A `job_finished` with `repeated_from` is a matrix leg that did not run, because locally it would only have repeated the named leg. Its `status` is that leg's and its `duration_ms` is 0. Pass `--every-matrix-leg` to run every leg.

`failures` has one entry per distinct failure, not per failed job, so count `jobs` on `run_finished` instead. `also_failed_in` lists matrix legs where the same step failed with the same exit code and output, ignoring durations, and `command` is the first leg's script. `error_lines` holds lines above `output_tail` that look like errors. `note` warns about the run itself, such as `--from-step` having skipped earlier steps.

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
- `wrkflw gate` prints text even with `--format json`, and the MCP server has no gate tool. Use its exit code, then `wrkflw --format json runs failure latest` for the cause.
- Parallel jobs run one after another in emulation, so `--runtime emulation` won't show a race between jobs.
- `--from-step` needs `--job`. Earlier steps are skipped apart from `actions/checkout`, so their outputs and files are missing. Use it to iterate on a step that doesn't depend on them.
- `runs replay` reruns against the current working tree. Compare `git_sha` and `git_dirty` on the record if the result differs.
- `--runtime emulation` runs steps directly on this machine. Use a container runtime or `secure-emulation` for code you don't trust.
- `plan` reports a matrix computed by expressions as `dynamic`, because it is only known at run time.
