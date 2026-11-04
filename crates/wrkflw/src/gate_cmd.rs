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

/// The pre-push hook.
///
/// It runs this binary if it's still there and `wrkflw` from `PATH` if not,
/// as GUI git clients often run hooks with a shorter `PATH` than a terminal.
/// It keeps the `--runtime` and `--remote-cache` given at install time.
///
/// It checks the working tree, not the commits being pushed: uncommitted
/// edits and untracked files are part of what it runs. `gate` warns when the
/// tree is dirty.
///
/// git writes `<local ref> <local sha> <remote ref> <remote sha>` on stdin
/// for each ref pushed. The first remote sha that exists here is what the
/// remote already has, so it's the base for `--affected`. A new branch has
/// none and falls back to the usual detection. A push that only deletes refs
/// has nothing to check.
fn hook_script(program: &Path, runtime: &str, remote_cache: Option<&str>) -> String {
    let mut gate = format!("set -- gate --runtime {}\n", shell_quote(runtime));
    if let Some(remote) = remote_cache {
        gate.push_str(&format!(
            "set -- \"$@\" --remote-cache {}\n",
            shell_quote(remote)
        ));
    }
    format!(
        r#"#!/bin/sh
{marker}
# Skip it once with: git push --no-verify
# It checks the working tree, so uncommitted changes count as pushed.
wrkflw={program}
[ -x "$wrkflw" ] || wrkflw=wrkflw
refs=0
pushing=0
base=
while read -r local_ref local_sha remote_ref remote_sha; do
  refs=1
  case "$local_sha" in *[!0]*) ;; *) continue ;; esac
  pushing=1
  if [ -z "$base" ]; then
    case "$remote_sha" in
      *[!0]*) git cat-file -e "$remote_sha^{{commit}}" 2>/dev/null && base=$remote_sha ;;
    esac
  fi
done
[ "$refs" = 1 ] && [ "$pushing" = 0 ] && exit 0
{gate}[ -n "$base" ] && set -- "$@" --diff-base "$base"
exec "$wrkflw" "$@"
"#,
        marker = HOOK_MARKER,
        program = shell_quote(&program.display().to_string()),
        gate = gate,
    )
}

/// One `git rev-parse` answer, as an absolute path.
fn git_path_query(args: &[&str]) -> Option<PathBuf> {
    let output = Command::new("git")
        .arg("rev-parse")
        .args(args)
        .output()
        .ok()?;
    if !output.status.success() {
        return None;
    }
    let path = PathBuf::from(String::from_utf8_lossy(&output.stdout).trim());
    if path.as_os_str().is_empty() {
        return None;
    }
    Some(if path.is_absolute() {
        path
    } else {
        std::env::current_dir().ok()?.join(path)
    })
}

/// `path` with symlinks resolved as far as it exists, so a hook that hasn't
/// been written yet still compares against canonical roots.
fn resolve_existing(path: &Path) -> PathBuf {
    let mut missing = Vec::new();
    let mut current = path.to_path_buf();
    loop {
        if let Ok(real) = current.canonicalize() {
            return missing.iter().rev().fold(real, |acc, part| acc.join(part));
        }
        match (current.file_name(), current.parent()) {
            (Some(name), Some(parent)) => {
                missing.push(name.to_os_string());
                current = parent.to_path_buf();
            }
            _ => return path.to_path_buf(),
        }
    }
}

/// Whether `hook` belongs to this repository: inside its git directory, or
/// inside its work tree, as a hooks directory checked into it is.
fn hook_is_local(hook: &Path, roots: &[PathBuf]) -> bool {
    let hook = resolve_existing(hook);
    roots
        .iter()
        .any(|root| hook.starts_with(resolve_existing(root)))
}

/// Where git looks for `pre-push` here. Asking git covers worktrees and
/// `core.hooksPath`, where `.git/hooks` is the wrong place.
///
/// A `core.hooksPath` outside the repository, such as one set globally, is
/// shared with other repositories. A hook pinned to this binary and runtime
/// doesn't belong there, so that is an error.
fn hook_path() -> Result<PathBuf, String> {
    let path = git_path_query(&["--git-path", "hooks/pre-push"])
        .ok_or_else(|| "not inside a git repository".to_string())?;
    let roots: Vec<PathBuf> = [
        git_path_query(&["--git-common-dir"]),
        git_path_query(&["--show-toplevel"]),
    ]
    .into_iter()
    .flatten()
    .collect();
    if !hook_is_local(&path, &roots) {
        return Err(format!(
            "git would run {}, which is outside this repository, so it may be shared \
             with others (core.hooksPath is set). Add `wrkflw gate` to that hook by hand, \
             or unset core.hooksPath for this repository",
            path.display()
        ));
    }
    Ok(path)
}

/// Write the hook. An existing hook that isn't ours is left alone: it may be
/// the only copy of someone's work, and they can chain to `wrkflw gate`.
fn install_hook_at(path: &Path, script: &str) -> Result<(), String> {
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
    std::fs::write(path, script).map_err(|e| e.to_string())?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(path, std::fs::Permissions::from_mode(0o755))
            .map_err(|e| e.to_string())?;
    }
    Ok(())
}

/// Changes in the working tree that aren't committed, as `git status` lists
/// them. Empty when the tree is clean or git can't say.
fn uncommitted_changes() -> Vec<String> {
    let Ok(output) = Command::new("git")
        .args(["status", "--porcelain", "--untracked-files=normal"])
        .output()
    else {
        return Vec::new();
    };
    if !output.status.success() {
        return Vec::new();
    }
    String::from_utf8_lossy(&output.stdout)
        .lines()
        .filter_map(|line| line.get(3..))
        .map(str::to_string)
        .collect()
}

/// The warning for a dirty tree, which the gate checks in place of what a
/// push would send.
fn dirty_tree_warning(changes: &[String]) -> Option<String> {
    const SHOWN: usize = 5;
    if changes.is_empty() {
        return None;
    }
    let mut listed = changes[..changes.len().min(SHOWN)].join(", ");
    if changes.len() > SHOWN {
        listed.push_str(&format!(" and {} more", changes.len() - SHOWN));
    }
    Some(format!(
        "Warning: the gate checks the working tree, which has uncommitted changes ({}). \
         A push sends only commits, so the result may differ from CI. Commit or stash them \
         to check exactly what you push",
        listed
    ))
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
        "--gate-mode".to_string(),
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
        let script = hook_script(&program, ctx.runtime.as_flag(), ctx.remote_cache.as_deref());
        let installed = hook_path().and_then(|path| {
            install_hook_at(&path, &script)?;
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
    if let Some(warning) = dirty_tree_warning(&uncommitted_changes()) {
        eprintln!("{}", cli_style::dim(&warning));
    }

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
        let script = hook_script(Path::new("/opt/wrkflw"), "auto", None);
        install_hook_at(&path, &script).unwrap();
        assert!(std::fs::read_to_string(&path)
            .unwrap()
            .contains("wrkflw gate"));
        install_hook_at(&path, &script).unwrap();
    }

    #[test]
    fn the_hook_names_this_binary_and_quotes_it() {
        let script = hook_script(Path::new("/opt/it's here/wrkflw"), "auto", None);
        assert!(script.contains(r#"wrkflw='/opt/it'\''s here/wrkflw'"#));
        assert!(script.contains("exec \"$wrkflw\" \"$@\""));
    }

    #[test]
    fn the_hook_keeps_the_runtime_and_remote_it_was_installed_with() {
        let script = hook_script(Path::new("/w"), "podman", Some("https://c/x y"));
        assert!(script.contains("set -- gate --runtime 'podman'"));
        assert!(script.contains("--remote-cache 'https://c/x y'"));
        let plain = hook_script(Path::new("/w"), "auto", None);
        assert!(!plain.contains("--remote-cache"));
    }

    /// Run the hook's stdin handling with a stand-in `wrkflw` that prints
    /// its arguments, inside a repository with one commit.
    #[cfg(unix)]
    fn hook_args(stdin: impl Fn(&str) -> String) -> (String, String) {
        use std::io::Write;
        use std::os::unix::fs::PermissionsExt;
        let dir = tempfile::tempdir().unwrap();
        let git = |args: &[&str]| {
            let output = Command::new("git")
                .current_dir(dir.path())
                .env("GIT_CONFIG_GLOBAL", "/dev/null")
                .env("GIT_CONFIG_NOSYSTEM", "1")
                .args(["-c", "user.name=t", "-c", "user.email=t@example.com"])
                .args(args)
                .output()
                .unwrap();
            assert!(output.status.success());
            String::from_utf8(output.stdout).unwrap().trim().to_string()
        };
        git(&["init", "-q"]);
        git(&["commit", "-q", "--allow-empty", "-m", "base"]);
        let head = git(&["rev-parse", "HEAD"]);
        let fake = dir.path().join("fake-wrkflw");
        std::fs::write(&fake, "#!/bin/sh\necho \"$@\"\n").unwrap();
        std::fs::set_permissions(&fake, std::fs::Permissions::from_mode(0o755)).unwrap();
        let hook = dir.path().join("hook");
        std::fs::write(&hook, hook_script(&fake, "docker", None)).unwrap();
        let mut child = Command::new("sh")
            .arg(&hook)
            .current_dir(dir.path())
            .stdin(std::process::Stdio::piped())
            .stdout(std::process::Stdio::piped())
            .spawn()
            .unwrap();
        child
            .stdin
            .take()
            .unwrap()
            .write_all(stdin(&head).as_bytes())
            .unwrap();
        let output = child.wait_with_output().unwrap();
        assert!(output.status.success());
        let printed = String::from_utf8(output.stdout).unwrap().trim().to_string();
        (printed, head)
    }

    #[cfg(unix)]
    #[test]
    fn the_hook_compares_against_what_the_remote_has() {
        let zero = "0".repeat(40);
        let local = "1".repeat(40);
        let (pushed, head) =
            hook_args(|head| format!("refs/heads/a {} refs/heads/a {}\n", local, head));
        assert_eq!(
            pushed,
            format!("gate --runtime docker --diff-base {}", head)
        );

        let (new_branch, _) =
            hook_args(|_| format!("refs/heads/a {} refs/heads/a {}\n", local, zero));
        assert_eq!(new_branch, "gate --runtime docker");

        let missing = "2".repeat(40);
        let (unknown, _) =
            hook_args(|_| format!("refs/heads/a {} refs/heads/a {}\n", local, missing));
        assert_eq!(unknown, "gate --runtime docker");

        let (delete, _) = hook_args(|head| format!("(delete) {} refs/heads/a {}\n", zero, head));
        assert_eq!(delete, "");

        let (by_hand, _) = hook_args(|_| String::new());
        assert_eq!(by_hand, "gate --runtime docker");
    }

    #[test]
    fn a_hook_outside_the_repository_is_not_ours_to_write() {
        let repo = tempfile::tempdir().unwrap();
        let shared = tempfile::tempdir().unwrap();
        let git_dir = repo.path().join(".git");
        std::fs::create_dir_all(git_dir.join("hooks")).unwrap();
        let roots = vec![git_dir.clone(), repo.path().to_path_buf()];
        assert!(hook_is_local(&git_dir.join("hooks/pre-push"), &roots));
        assert!(hook_is_local(&repo.path().join(".husky/pre-push"), &roots));
        assert!(!hook_is_local(&shared.path().join("pre-push"), &roots));
        assert!(!hook_is_local(
            &repo.path().join("../elsewhere/pre-push"),
            &roots
        ));
    }

    #[test]
    fn someone_elses_hook_is_left_alone() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("pre-push");
        std::fs::write(&path, "#!/bin/sh\nmake lint\n").unwrap();
        let script = hook_script(Path::new("/opt/wrkflw"), "auto", None);
        assert!(install_hook_at(&path, &script).is_err());
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
            "--gate-mode",
        ] {
            assert!(args.iter().any(|arg| arg == flag), "{}", flag);
        }
        assert!(args.windows(2).any(|w| w == ["--base-branch", "main"]));
        assert!(args
            .windows(2)
            .any(|w| w == ["--skip-exit-code", SKIPPED_EXIT.to_string().as_str()]));
    }

    #[test]
    fn a_dirty_tree_is_named_and_a_clean_one_is_not() {
        assert_eq!(dirty_tree_warning(&[]), None);
        let few = dirty_tree_warning(&["src/a.rs".to_string()]).unwrap();
        assert!(few.contains("src/a.rs"), "{}", few);
        let many: Vec<String> = (0..7).map(|i| format!("f{}", i)).collect();
        let warning = dirty_tree_warning(&many).unwrap();
        assert!(warning.contains("f4 and 2 more"), "{}", warning);
        assert!(!warning.contains("f5"), "{}", warning);
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
