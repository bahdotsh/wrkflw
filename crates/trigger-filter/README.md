## wrkflw-trigger-filter

Parses a GitHub Actions `on:` block and decides whether a workflow would fire
for a given event context and changed file set. Powers `wrkflw run --event …`
and `wrkflw watch`.

- Parses `push`, `pull_request`, `pull_request_target`, `workflow_dispatch`,
  `schedule`, and the other documented GHA events
- Matches `branches`, `branches-ignore`, `tags`, `tags-ignore`, `paths`,
  `paths-ignore`, and `types:` filters
- Auto-detects the diff base (`origin/HEAD`, `main`, `master`, then `HEAD~1`)
  when invoked without an explicit base ref
- Returns a structured reason on skip so the CLI/TUI can explain *why* a
  workflow was filtered out

Consumers: `wrkflw` CLI (`run`, `watch`) and `wrkflw-watcher`. Most users
should reach this functionality through the `--event` / `--diff` flags rather
than depending on the crate directly.
