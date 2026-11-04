## wrkflw-ui

Terminal user interface for browsing workflows, running them, and viewing logs.

- Tabs: Workflows, Execution, DAG, Logs, Trigger, Secrets, Runs, Help
- Runs tab: recorded run history, with the agent and intent behind each run
- Job selection mode: pick and run individual jobs within a workflow
- Diff-aware trigger filter (`d` / `D`) and Tweaks overlay (`,`)
- Log search (`s`), filter (`f`), and match navigation (`n`)
- Runtime cycling across Docker / Podman / Kubernetes / Secure Emulation / Emulation (`e`)
- Hotkeys: `1`–`8` for tab jumps, `Tab`/`Shift+Tab`, `w/x/l/h` shortcuts, `Enter`, `r`, `Shift+R`, `Shift+J`, `t`, `v`, `?`, `q`/`Ctrl+C`, etc.
- Step inspector: Output (with ANSI colours, following the tail; `PgUp`/`PgDn` to scroll) and Matrix with per-combination status
- Responsive layout from small terminals up, built on ratatui 0.30
- Mouse selection: drag to select text, copied to the clipboard on release (OSC 52)
- Light and dark themes, following the terminal background unless `--theme` or `WRKFLW_THEME` says otherwise
- Optional: enabled via the `tui` cargo feature flag
- Integrates with `wrkflw-executor` and `wrkflw-logging`

### Example

```rust
use std::path::PathBuf;
use wrkflw_executor::RuntimeType;
use wrkflw_ui::run_wrkflw_tui;

# tokio_test::block_on(async {
let path = PathBuf::from(".github/workflows");
run_wrkflw_tui(
    Some(&path),
    RuntimeType::Auto,
    /* verbose */ true,
    /* preserve_containers_on_failure */ false,
    /* show_action_messages */ false,
    /* theme */ None,
    /* platform_overrides */ Default::default(),
    /* kubernetes */ Default::default(),
).await?;
# Ok::<_, Box<dyn std::error::Error>>(())
# })?;
```

Most users should run the `wrkflw` binary and select TUI mode: `wrkflw tui`.
