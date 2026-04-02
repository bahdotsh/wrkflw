## wrkflw-ui

Terminal user interface for browsing workflows, running them, and viewing logs.

- Tabs: Workflows, Execution, Logs, Help
- Job selection mode: pick and run individual jobs within a workflow
- Hotkeys: `1-4`, `Tab`, `Enter`, `r`, `R`, `t`, `v`, `e`, `q`, etc.
- Optional: enabled via the `tui` cargo feature flag
- Integrates with `wrkflw-executor` and `wrkflw-logging`

### Example

```rust
use std::path::PathBuf;
use wrkflw_executor::RuntimeType;
use wrkflw_ui::run_wrkflw_tui;

# tokio_test::block_on(async {
let path = PathBuf::from(".github/workflows");
run_wrkflw_tui(Some(&path), RuntimeType::Docker, true, false).await?;
# Ok::<_, Box<dyn std::error::Error>>(())
# })?;
```

Most users should run the `wrkflw` binary and select TUI mode: `wrkflw tui`.
