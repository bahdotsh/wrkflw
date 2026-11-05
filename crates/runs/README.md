## wrkflw-runs

Run history for wrkflw: what ran, what it printed, and why it failed. It lets an
agent answer "why did the last run fail?" with one read instead of a rerun.

- Records every run under `$WRKFLW_HOME/runs` (the platform's local data
  directory by default), keeping the newest 500 or `WRKFLW_RUNS_KEEP`
- Stores per-step logs with known secrets masked, plus the agent, session and
  intent that started the run
- Builds failure digests: the failed step, its script, exit code, env exported
  by earlier steps and the output tail, with identical matrix failures folded
- Emits the NDJSON events behind `--format json` (`run_started` … `run_finished`)
- **Used by**: `wrkflw run`, `wrkflw runs`, `wrkflw mcp serve` and the TUI's History tab

### Example

```rust
use wrkflw_runs::{failure_digest, render_digest, RunStore, DEFAULT_TAIL_LINES};

let store = RunStore::open_default().expect("no data directory");
for run in store.list() {
    if let Some(digest) = failure_digest(&store, &run, DEFAULT_TAIL_LINES) {
        println!("{}", render_digest(&digest));
    }
}
```

Set `WRKFLW_NO_HISTORY=1` or pass `--no-record` to skip recording. Most users
should read history through `wrkflw runs` rather than this crate.
