## wrkflw-ui

Terminal user interface for browsing workflows, running them, and viewing logs.

- Tabs: Workflows, Execution, DAG, Logs, Trigger, Secrets, Help
- Job selection mode: pick and run individual jobs within a workflow
- Diff-aware trigger filter (`d` / `D`) and Tweaks overlay (`,`)
- Log search (`s`), filter (`f`), and match navigation (`n`)
- Runtime cycling across Docker / Podman / Emulation / Secure Emulation (`e`)
- Hotkeys: `1`–`7` for tab jumps, `Tab`/`Shift+Tab`, `w/x/l/h` shortcuts, `Enter`, `r`, `Shift+R`, `Shift+J`, `t`, `v`, `?`, `q`, etc.
- Optional: enabled via the `tui` cargo feature flag
- Integrates with `wrkflw-executor` and `wrkflw-logging`

### Example

```rust
use std::path::PathBuf;
use wrkflw_executor::RuntimeType;
use wrkflw_ui::run_wrkflw_tui;

# tokio_test::block_on(async {
let path = PathBuf::from(".github/workflows");
run_wrkflw_tui(Some(&path), RuntimeType::Docker, /* verbose */ true, /* preserve_containers_on_failure */ false, /* show_action_messages */ false).await?;
# Ok::<_, Box<dyn std::error::Error>>(())
# })?;
```

Most users should run the `wrkflw` binary and select TUI mode: `wrkflw tui`.
