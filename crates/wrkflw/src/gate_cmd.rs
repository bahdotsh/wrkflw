//! `wrkflw gate`: run what a push would run, before pushing it.
//!
//! Each workflow runs in a child `wrkflw run`, because that command owns the
//! trigger prefilter and calls `process::exit` on every terminal outcome. The
//! child's exit code is the whole interface: 0 passed, `SKIPPED_EXIT` didn't
//! fire, and neither should stop the push.

use crate::{is_gitlab_pipeline, RuntimeChoice};
use std::path::{Path, PathBuf};
use std::process::Command;
use wrkflw_ui::cli_style;

/// Marks a hook as ours, so reinstalling can replace it and nothing else.
const HOOK_MARKER: &str = "# installed by wrkflw gate";

/// What the child exits with when its triggers don't match the event.
const SKIPPED_EXIT: i32 = 78;

pub(crate) struct GateCtx {
    pub(crate) path: Option<PathBuf>,
    pub(crate) event: String,
    pub(crate) base_branch: Option<String>,
    pub(crate) diff_base: Option<String>,
    pub(crate) runtime: RuntimeChoice,
    pub(crate) remote_cache: Option<String>,
    pub(crate) install_hook: bool,
}

/// `value` as one word of a POSIX shell script.
fn shell_quote(value: &str) -> String {
    format!("'{}'", value.replace('\'', "'\\''"))
}

/// The hook runs this binary if it's still there and `wrkflw` from `PATH` if
/// not. GUI git clients often run hooks with a shorter `PATH` than a terminal.
fn hook_script(program: &Path) -> String {
    format!(
        "#!/bin/sh\n{}\n# Skip it once with: git push --no-verify\nwrkflw={}\n[ -x \"$wrkflw\" ] || wrkflw=wrkflw\nexec \"$wrkflw\" gate\n",
        HOOK_MARKER,
        shell_quote(&program.display().to_string())
    )
}

/// Where git looks for `pre-push` here. Asking git covers worktrees and
/// `core.hooksPath`, where `.git/hooks` is the wrong place.
fn hook_path() -> Result<PathBuf, String> {
    let output = Command::new("git")
        .args(["rev-parse", "--git-path", "hooks/pre-push"])
        .output()
        .map_err(|e| format!("could not run git: {}", e))?;
    if !output.status.success() {
        return Err("not inside a git repository".to_string());
    }
    Ok(PathBuf::from(
        String::from_utf8_lossy(&output.stdout).trim(),
    ))
}

/// Write the hook. An existing hook that isn't ours is left alone: it may be
/// the only copy of someone's work, and they can chain to `wrkflw gate`.
fn install_hook_at(path: &Path, program: &Path) -> Result<(), String> {
    if let Ok(existing) = std::fs::read_to_string(path) {
        if !existing.contains(HOOK_MARKER) {
            return Err(format!(
                "{} already exists. Add `wrkflw gate` to it by hand",
                path.display()
            ));
        }
    } else if path.exists() {
        return Err(format!("{} exists and can't be read", path.display()));
    }
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent).map_err(|e| e.to_string())?;
    }
    std::fs::write(path, hook_script(program)).map_err(|e| e.to_string())?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(path, std::fs::Permissions::from_mode(0o755))
            .map_err(|e| e.to_string())?;
    }
    Ok(())
}

/// The workflow files under `path`, or under the detected directories.
fn workflow_files(path: Option<&Path>) -> Result<Vec<PathBuf>, String> {
    let roots = match path {
        Some(path) if path.is_file() => return Ok(vec![path.to_path_buf()]),
        Some(path) => vec![path.to_path_buf()],
        None => wrkflw_utils::default_workflow_dirs(),
    };
    let mut files = Vec::new();
    for root in roots {
        let entries = match std::fs::read_dir(&root) {
            Ok(entries) => entries,
            Err(e) if path.is_none() && e.kind() == std::io::ErrorKind::NotFound => continue,
            Err(e) => return Err(format!("cannot read {}: {}", root.display(), e)),
        };
        for entry in entries.flatten() {
            let file = entry.path();
            let is_yaml = file
                .extension()
                .is_some_and(|ext| ext == "yml" || ext == "yaml");
            if is_yaml && file.is_file() && !is_gitlab_pipeline(&file) {
                files.push(file);
            }
        }
    }
    files.sort();
    Ok(files)
}

/// The child's arguments, apart from the runtime.
fn run_args(ctx: &GateCtx, file: &Path) -> Vec<String> {
    let mut args = vec![
        "run".to_string(),
        file.display().to_string(),
        "--event".to_string(),
        ctx.event.clone(),
        "--affected".to_string(),
        "--reuse-results".to_string(),
        "--job-container".to_string(),
        "--dep-cache".to_string(),
        "--intent".to_string(),
        "wrkflw gate".to_string(),
        "--skip-exit-code".to_string(),
        SKIPPED_EXIT.to_string(),
    ];
    if let Some(base) = &ctx.base_branch {
        args.extend(["--base-branch".to_string(), base.clone()]);
    }
    if let Some(base) = &ctx.diff_base {
        args.extend(["--diff-base".to_string(), base.clone()]);
    }
    if let Some(remote) = &ctx.remote_cache {
        args.extend(["--remote-cache".to_string(), remote.clone()]);
    }
    args
}

/// Run the gate. Exits 1 when a workflow fails or can't be run.
pub(crate) fn run(ctx: GateCtx) {
    let program = std::env::current_exe().unwrap_or_else(|e| {
        eprintln!("Error: cannot find this executable: {}", e);
        std::process::exit(1);
    });
    if ctx.install_hook {
        let installed = hook_path().and_then(|path| {
            install_hook_at(&path, &program)?;
            Ok(path)
        });
        match installed {
            Ok(path) => println!("Installed {}", path.display()),
            Err(e) => {
                eprintln!("Error: {}", e);
                std::process::exit(1);
            }
        }
        return;
    }
    let files = workflow_files(ctx.path.as_deref()).unwrap_or_else(|e| {
        eprintln!("Error: {}", e);
        std::process::exit(1);
    });
    let runtime = ctx.runtime.as_flag();

    // Every workflow runs even after a failure, so one push shows all of them.
    let mut failed = Vec::new();
    let mut skipped = 0;
    for file in &files {
        println!("{}", cli_style::dim(&format!("gate: {}", file.display())));
        let mut child = Command::new(&program);
        child
            .args(run_args(&ctx, file))
            .args(["--runtime", runtime]);
        crate::cli_secrets::pass_to(&mut child);
        let status = child.status();
        match status.map(|status| status.code()) {
            Ok(Some(0)) => {}
            Ok(Some(SKIPPED_EXIT)) => skipped += 1,
            _ => failed.push(file.display().to_string()),
        }
    }
    if failed.is_empty() {
        println!(
            "gate passed: {} workflow(s) ran, {} skipped",
            files.len() - skipped,
            skipped
        );
        return;
    }
    eprintln!("gate failed: {}", failed.join(", "));
    std::process::exit(1);
}

#[cfg(test)]
mod tests {
    use super::*;

    fn ctx() -> GateCtx {
        GateCtx {
            path: None,
            event: "push".to_string(),
            base_branch: None,
            diff_base: None,
            runtime: RuntimeChoice::Auto,
            remote_cache: None,
            install_hook: false,
        }
    }

    #[test]
    fn the_hook_installs_and_reinstalls() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("hooks/pre-push");
        install_hook_at(&path, Path::new("/opt/wrkflw")).unwrap();
        assert!(std::fs::read_to_string(&path)
            .unwrap()
            .contains("wrkflw gate"));
        install_hook_at(&path, Path::new("/opt/wrkflw")).unwrap();
    }

    #[test]
    fn the_hook_names_this_binary_and_quotes_it() {
        let script = hook_script(Path::new("/opt/it's here/wrkflw"));
        assert!(script.contains(r#"wrkflw='/opt/it'\''s here/wrkflw'"#));
        assert!(script.contains("exec \"$wrkflw\" gate"));
    }

    #[test]
    fn someone_elses_hook_is_left_alone() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("pre-push");
        std::fs::write(&path, "#!/bin/sh\nmake lint\n").unwrap();
        assert!(install_hook_at(&path, Path::new("/opt/wrkflw")).is_err());
        assert_eq!(
            std::fs::read_to_string(&path).unwrap(),
            "#!/bin/sh\nmake lint\n"
        );
    }

    #[test]
    fn the_child_gets_the_event_and_every_shortcut() {
        let args = run_args(
            &GateCtx {
                base_branch: Some("main".to_string()),
                ..ctx()
            },
            Path::new("ci.yml"),
        );
        for flag in [
            "--affected",
            "--reuse-results",
            "--job-container",
            "--dep-cache",
        ] {
            assert!(args.iter().any(|arg| arg == flag), "{}", flag);
        }
        assert!(args.windows(2).any(|w| w == ["--base-branch", "main"]));
        assert!(args
            .windows(2)
            .any(|w| w == ["--skip-exit-code", SKIPPED_EXIT.to_string().as_str()]));
    }

    #[test]
    fn only_workflow_files_are_collected() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(dir.path().join("a.yml"), "jobs: {}\n").unwrap();
        std::fs::write(dir.path().join("notes.md"), "x").unwrap();
        let files = workflow_files(Some(dir.path())).unwrap();
        assert_eq!(files, vec![dir.path().join("a.yml")]);
    }
}
