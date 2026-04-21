## wrkflw-watcher

File-system watcher with trigger-aware workflow execution. Backs the
`wrkflw watch` subcommand.

- Debounced change detection via `notify`
- Per-workflow trigger cache (built on top of `wrkflw-trigger-filter`) so
  only workflows whose `on:` block matches the change set are rerun
- Built-in ignore list (`.git`, `target`, `node_modules`, `.build`, `build`,
  `dist`, `__pycache__`, `.tox`, `.mypy_cache`, `.pytest_cache`, `.venv`,
  `venv`) plus user-supplied `--ignore-dir` values
- Concurrency cap per cycle, pending-event bound, and strict-filter mode
  that rejects degraded event contexts with a loud error
- Graceful shutdown via the shared shutdown signal

Consumers: `wrkflw` CLI (`watch` subcommand). Prefer the CLI unless you are
embedding the watcher into another tool.
