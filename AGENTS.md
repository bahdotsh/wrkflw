# wrkflw for coding agents

wrkflw validates and runs GitHub Actions and GitLab CI workflows locally. Use it to check a workflow change before pushing.

## Loop

1. After editing YAML, run `wrkflw --format json validate`.
2. Run `wrkflw --format json plan <file>` to see job order, matrix legs and whether a trigger fires.
3. Run `wrkflw --format json run <file> --intent "why you are running this"`.
4. On failure, read the `failure` object in the final `run_finished` line. Don't rerun to see the error again.
5. For earlier runs, use `wrkflw runs list`, `runs failure <id>` and `runs logs <id> --step <name>`.

Prefer the MCP server if your client supports MCP:

```json
{"mcpServers": {"wrkflw": {"command": "wrkflw", "args": ["mcp", "serve"]}}}
```

## Simulating events

- `--event pull_request` needs `--base-branch main`. Without it the command fails, on purpose.
- Add `--activity-type opened` if the workflow uses `types:`.
- Pair `--event` with `--diff` or `--changed-files a,b`, or `paths:` filters have nothing to match.
- A skipped run exits 0 with `run_finished` status `skipped`. Check `status`, not only the exit code.

## Output contract

`--format json` is a global flag. stdout carries only JSON and logs go to stderr. Every object has `schema_version` and `type`.

| Command | stdout |
| --- | --- |
| `run`, `watch`, `runs replay` | One object per line: `run_started`, `job_started`, `step_started`, `step_finished`, `job_finished`, `run_finished`. `watch` adds `watch_started` and `watch_cycle`. |
| `validate`, `list`, `plan`, `runs ...` | One object. |
| Any error | `{"type": "error", "message": "..."}`, exit 1. |

`step_finished.output` holds the last 64 KiB of a step's output, delivered when the step ends. Output is not streamed while a step runs.

Exit codes: 0 for success or skipped, 1 for a failed run or failed validation, 2 for a usage error.

## History

Runs are stored under `$WRKFLW_HOME/runs`, defaulting to the platform's local data directory. The newest 500 are kept, or `WRKFLW_RUNS_KEEP`. Use `--no-record` or `WRKFLW_NO_HISTORY=1` to skip recording.

Known secrets and `::add-mask::` values are masked before output is stored. `--variable` values are never stored, so pass them again to `runs replay`. Logs can still hold anything a step printed that wrkflw did not recognise as a secret, so run directories are readable only by the owner on Unix. On Windows they rely on the data directory's permissions.

Identify yourself with `WRKFLW_AGENT=<name>` or `--agent`. `WRKFLW_INTENT` or `--intent` records why you ran it.

## Limits

- `--from-step` needs `--job`. Earlier steps are skipped apart from `actions/checkout`, so their outputs and files are missing. Use it to iterate on a step that doesn't depend on them.
- `runs replay` reruns against the current working tree. Compare `git_sha` and `git_dirty` on the record if the result differs.
- `--runtime emulation` runs steps directly on this machine. Use a container runtime or `secure-emulation` for code you don't trust.
- `plan` reports a matrix computed by expressions as `dynamic`, because it is only known at run time.
