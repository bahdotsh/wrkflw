---
name: wrkflw
description: Validate, plan and run GitHub Actions or GitLab CI workflows locally, and diagnose failed runs from recorded history. Use when editing workflow YAML, when a CI failure needs reproducing, or before pushing a workflow change.
---

# wrkflw

Check workflow changes locally before pushing them. Use the wrkflw MCP tools when they are available, otherwise the CLI with `--format json`.

1. Validate after every YAML edit with `validate_workflows`, or `wrkflw --format json validate`. Each issue has a rule, severity, YAML path, line and sometimes a hint. A `warning` leaves the file valid but is usually worth fixing.
2. Before running, call `plan_workflow`, or `wrkflw --format json plan <file>`. It explains whether the trigger fires, lists jobs in order and names `missing_secrets` without executing anything.
3. Run with `run_workflow`, or `wrkflw --format json run <file> --intent "<why>"`. Always say why, because the reason is stored on the run.
4. On failure, read the digest in the result. It names the failed step, script, exit code, earlier exported env and output tail.
5. Before rerunning, check `list_runs` and `get_run_failure`. The answer may already be recorded.

For `pull_request`, pass `event` and `base_branch`. If the workflow has a `paths:` filter, also pass `diff` or `changed_files`. Add `activity_type` if the workflow filters on `types:`.

A missing secret is empty, as on GitHub. Set an environment variable with its name to provide a value.

`status: skipped` means the trigger did not match, not that the workflow passed. Read `skip_reason`.

Running a workflow executes its code. Don't use the `emulation` runtime on code you don't trust.
