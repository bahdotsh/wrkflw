---
name: wrkflw
description: Validate, plan and run GitHub Actions or GitLab CI workflows locally, and diagnose failed runs from recorded history. Use when editing workflow YAML, when a CI failure needs reproducing, or before pushing a workflow change.
---

# wrkflw

Check workflow changes locally before pushing them. Use the wrkflw MCP tools when they are available, otherwise the CLI with `--format json`.

1. Validate after every YAML edit with `validate_workflows`, or `wrkflw --format json validate`. Each issue has a rule, severity, YAML path, line and sometimes a hint. A `warning` leaves the file valid but is usually worth fixing.
2. Before running, call `plan_workflow`, or `wrkflw --format json plan <file>`. It explains whether the trigger fires, lists jobs in order and names `missing_secrets` without executing anything. On a detached HEAD, or for a push to another branch, pass `branch` (`--branch`), or `tag` (`--tag`) for a tag push.
3. Run with `run_workflow`, or `wrkflw --format json run <file> --intent "<why>"`. Always say why, because the reason is stored on the run. Through MCP, also pass your session id as `session` if you know it, such as `$CLAUDE_CODE_SESSION_ID`.
4. On success, read `verdict`. `verified` means every job ran as the OS it asked for and every `uses:` step ran its action. `partial` means something under `unverified` wasn't checked: a job that ran as another OS, so its OS-specific steps weren't checked, or a step whose action didn't run (an entry of `kind` `action_not_run`, with `step` and `uses`), so whatever that action installs or checks is missing. Say which. `nothing_checked` means nothing ran, so the pass proves nothing.
   On failure, read the digest in the result. It names the failed step, script, exit code, earlier exported env and output tail. If its `cause` is `out_of_memory`, the machine failed, not the code. If it is `killed`, a process got SIGKILL: usually memory, unless a step kills processes itself. If it is `setup_failed`, wrkflw couldn't set the job up and no step ran, so the code isn't at fault.
5. Before rerunning, check `list_runs` and `get_run_failure`. The answer may already be recorded.
6. Before pushing, call `gate`, or `wrkflw --format json gate`. It runs what CI would run for the push, reusing jobs that already passed on the same files, and exits 1 if anything fails. On a feature branch, a workflow that runs no job for a push is checked as a pull request into the default branch. Read `verdict`: `passed` is false when nothing was checked. Each workflow's `failure` holds its digest.
7. If wrkflw and GitHub disagree about a finished run, check out its commit and run `wrkflw --format json compare <run id or URL>`. It lists each job's result on both sides, with a note on why they may differ.

For `pull_request`, pass `event` and `base_branch`. If the workflow has a `paths:` filter, also pass `diff` or `changed_files`. Add `activity_type` if the workflow filters on `types:`.

A missing secret is empty, as on GitHub. Set an environment variable with its name to provide a value.

`status: skipped` means the trigger did not match, not that the workflow passed. Read `skip_reason`.

Running a workflow executes its code. Don't use the `emulation` runtime on code you don't trust.
