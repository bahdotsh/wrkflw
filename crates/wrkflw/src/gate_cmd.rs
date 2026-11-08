//! `wrkflw gate`: run what CI would run for a push, before pushing it.
//!
//! Each workflow runs in a child `wrkflw run`, because that command owns the
//! trigger prefilter and calls `process::exit` on every terminal outcome. The
//! child's exit code is the whole interface: 0 passed, `SKIPPED_EXIT` didn't
//! fire, `NOTHING_CHECKED_EXIT` fired but every job was skipped, and none of
//! them should stop the push. With `--format json` each child
//! prints JSON too, and its `run_finished` goes into one `gate` object.
//!
//! Without `--event`, a workflow is checked as a push first. CI often runs on
//! pushes to the default branch only, and on pull requests into it, so on any
//! other branch a workflow a push doesn't fire is checked again as a pull
//! request into the default branch. Otherwise the gate would pass a feature
//! branch having checked nothing at all.

use crate::{is_gitlab_pipeline, OutputFormat, RuntimeChoice};
use serde_json::{json, Value};
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use wrkflw_executor::vm_memory;
use wrkflw_runs::SCHEMA_VERSION;
use wrkflw_ui::cli_style;

/// Marks a hook as ours, so reinstalling can replace it and nothing else.
const HOOK_MARKER: &str = "# installed by wrkflw gate";

/// What the child exits with when its triggers don't match the event.
const SKIPPED_EXIT: i32 = 78;

/// What the child exits with when it passed without running a job, every
/// one skipped by `if:`, `needs` or `gate: false`. It checked nothing, so
/// the gate treats it as it does a skip.
const NOTHING_CHECKED_EXIT: i32 = 79;

/// Whether a child's exit code says it checked nothing.
fn ran_nothing(code: Option<i32>) -> bool {
    matches!(code, Some(SKIPPED_EXIT | NOTHING_CHECKED_EXIT))
}

pub(crate) struct GateCtx {
    pub(crate) path: Option<PathBuf>,
    /// The event to check as, or `None` for a push with the pull request
    /// fallback.
    pub(crate) event: Option<String>,
    pub(crate) base_branch: Option<String>,
    pub(crate) diff_base: Option<String>,
    pub(crate) runtime: RuntimeChoice,
    pub(crate) remote_cache: Option<String>,
    pub(crate) install_hook: bool,
    pub(crate) format: OutputFormat,
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

/// The event a workflow is checked as.
#[derive(Debug, Clone, PartialEq)]
struct Event {
    name: String,
    base_branch: Option<String>,
}

impl Event {
    /// How reports name it: `push`, or `pull_request into main`.
    fn describe(&self) -> String {
        match &self.base_branch {
            Some(base) if self.name == "pull_request" || self.name == "pull_request_target" => {
                format!("{} into {}", self.name, base)
            }
            _ => self.name.clone(),
        }
    }
}

/// One line of `git` output, or `None` when git fails or prints nothing.
fn git_line(args: &[&str]) -> Option<String> {
    let output = Command::new("git").args(args).output().ok()?;
    let line = String::from_utf8_lossy(&output.stdout).trim().to_string();
    (output.status.success() && !line.is_empty()).then_some(line)
}

/// The branch checked out, or `None` on a detached HEAD.
fn current_branch() -> Option<String> {
    git_line(&["symbolic-ref", "--short", "-q", "HEAD"])
}

/// The branch pull requests go into: what `origin/HEAD` points at, else
/// `main` or `master` if one exists here or on `origin`.
fn default_branch() -> Option<String> {
    if let Some(remote_head) =
        git_line(&["symbolic-ref", "--short", "-q", "refs/remotes/origin/HEAD"])
    {
        return Some(
            remote_head
                .strip_prefix("origin/")
                .unwrap_or(&remote_head)
                .to_string(),
        );
    }
    ["main", "master"]
        .into_iter()
        .map(str::to_string)
        .find(|name| {
            [
                format!("refs/heads/{name}"),
                format!("refs/remotes/origin/{name}"),
            ]
            .iter()
            .any(|reference| git_line(&["rev-parse", "--verify", "-q", reference]).is_some())
        })
}

/// What each workflow is checked as: the first event, then the second for a
/// workflow the first doesn't fire.
///
/// With `--event`, only that. Without it, a push, then a pull request into
/// the default branch (or `--base-branch`), unless this is the default
/// branch, where a push is what CI sees.
fn events(ctx: &GateCtx, branch: Option<&str>, default: Option<&str>) -> (Event, Option<Event>) {
    if let Some(name) = &ctx.event {
        return (
            Event {
                name: name.clone(),
                base_branch: ctx.base_branch.clone(),
            },
            None,
        );
    }
    let push = Event {
        name: "push".to_string(),
        base_branch: None,
    };
    let target = ctx.base_branch.as_deref().or(default);
    let pull_request = target
        .filter(|target| branch != Some(*target))
        .map(|target| Event {
            name: "pull_request".to_string(),
            base_branch: Some(target.to_string()),
        });
    (push, pull_request)
}

/// The child's arguments, apart from the runtime.
fn run_args(ctx: &GateCtx, file: &Path, event: &Event) -> Vec<String> {
    let mut args = vec![
        "run".to_string(),
        file.display().to_string(),
        "--event".to_string(),
        event.name.clone(),
        "--affected".to_string(),
        "--reuse-results".to_string(),
        "--job-container".to_string(),
        "--dep-cache".to_string(),
        "--gate-mode".to_string(),
        "--intent".to_string(),
        "wrkflw gate".to_string(),
        "--skip-exit-code".to_string(),
        SKIPPED_EXIT.to_string(),
        "--nothing-checked-exit-code".to_string(),
        NOTHING_CHECKED_EXIT.to_string(),
    ];
    if let Some(base) = &event.base_branch {
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

/// One workflow's entry in the `gate` object, from the child's JSON output:
/// its `run_finished` without `type`, or the error it printed instead. A child that died
/// mid-run still names the run it started, so its record can be read.
fn workflow_report(file: &Path, code: Option<i32>, stdout: &str) -> Value {
    let lines: Vec<Value> = stdout
        .lines()
        .filter_map(|line| serde_json::from_str::<Value>(line).ok())
        .collect();
    let last_of = |kind: &str| lines.iter().rev().find(|line| line["type"] == kind);
    let mut entry = if let Some(run) = last_of("run_finished") {
        // All of it, so a field `run_finished` gains reaches the gate too.
        let mut entry = run.clone();
        if let Some(fields) = entry.as_object_mut() {
            fields.remove("type");
            fields.remove("schema_version");
        }
        if code != Some(0) && !ran_nothing(code) && run["status"] == "success" {
            entry["status"] = json!("error");
            entry["error"] = json!(format!("wrkflw exited with {:?}", code));
        }
        entry
    } else if let Some(error) = last_of("error") {
        json!({"status": "error", "error": error["message"]})
    } else {
        json!({
            "status": "error",
            "error": format!("wrkflw exited with {:?} before the run finished", code),
        })
    };
    if entry["run_id"].is_null() {
        if let Some(started) = last_of("run_started") {
            entry["run_id"] = started["run_id"].clone();
        }
    }
    entry["path"] = json!(file.display().to_string());
    entry
}

/// The gate's verdict, as a run's: `failed` when a workflow failed or
/// couldn't run, `nothing_checked` when no job ran anywhere, `partial` when
/// it passed but some job ran as another OS than `runs-on` asked for, and
/// `verified` otherwise.
fn gate_verdict(workflows: &[Value], failed: usize, unverified: usize) -> &'static str {
    let checked = workflows
        .iter()
        .any(|w| w["status"] == "success" && w["verdict"] != "nothing_checked");
    if failed > 0 {
        "failed"
    } else if !checked {
        "nothing_checked"
    } else if unverified > 0 || workflows.iter().any(|w| w["verdict"] == "partial") {
        "partial"
    } else {
        "verified"
    }
}

/// The `gate` object: every workflow's result, and whether the push may go.
///
/// `passed` is true only when something ran and nothing failed. A gate that
/// checked nothing still exits 0, since a push no CI runs for is no error,
/// but it doesn't say it passed.
fn gate_report(workflows: Vec<Value>, uncommitted: &[String]) -> Value {
    let failed: Vec<&Value> = workflows
        .iter()
        .filter(|w| w["status"] != "success" && w["status"] != "skipped")
        .map(|w| &w["path"])
        .collect();
    // As in text mode: a workflow that ran no job, because it didn't fire
    // or every job was skipped, counts as skipped, not as ran.
    let skipped = workflows.iter().filter(|w| checked_nothing(w)).count();
    let unverified: Vec<Value> = workflows
        .iter()
        .flat_map(|w| {
            w["unverified"].as_array().into_iter().flatten().map(|u| {
                let mut u = u.clone();
                u["workflow"] = w["path"].clone();
                u
            })
        })
        .collect();
    let verdict = gate_verdict(&workflows, failed.len(), unverified.len());
    json!({
        "schema_version": SCHEMA_VERSION,
        "type": "gate",
        "passed": failed.is_empty() && verdict != "nothing_checked",
        "verdict": verdict,
        "ran": workflows.len() - skipped,
        "skipped": skipped,
        "failed": failed,
        "unverified": unverified,
        "uncommitted_changes": uncommitted,
        "workflows": workflows,
    })
}

/// Whether a workflow's entry ran no job: it didn't fire, or every job was
/// skipped.
fn checked_nothing(entry: &Value) -> bool {
    entry["status"] == "skipped" || entry["verdict"] == "nothing_checked"
}

/// Why an entry [`checked_nothing`], for the combined skip reason.
fn why_nothing_ran(entry: &Value) -> String {
    match entry["skip_reason"].as_str() {
        Some(reason) => reason.to_string(),
        None if entry["verdict"] == "nothing_checked" => "every job was skipped".to_string(),
        None => "did not fire".to_string(),
    }
}

/// The warning for a gate that checked nothing.
fn nothing_checked_warning(first: &Event, second: Option<&Event>) -> String {
    let events = match second {
        Some(second) => format!("a {} or a {}", first.describe(), second.describe()),
        None => format!("a {}", first.describe()),
    };
    format!(
        "Warning: the gate checked nothing. No workflow runs a job for {events}, so this \
         push is unverified. Pass --event and --base-branch to check it as CI will see it"
    )
}

/// The memory of the runtime the children will use: `runtime` as passed to
/// them, with `auto` resolved as a run resolves it, Docker before Podman.
/// `None` for a runtime without a container VM, or one that didn't answer.
fn probe_memory(runtime: &str) -> Option<vm_memory::Memory> {
    if std::env::var_os(vm_memory::PROBED_ENV).is_some() {
        return None;
    }
    match runtime {
        "docker" | "podman" => vm_memory::probe(runtime),
        "auto" => vm_memory::probe("docker").or_else(|| vm_memory::probe("podman")),
        _ => None,
    }
}

/// What children get in [`vm_memory::PROBED_ENV`]: the probe, or `none`
/// when it found nothing. `None` leaves the variable alone: a runtime
/// without a VM never reads it, and a parent's value is passed on as is.
fn probed_env(runtime: &str, memory: Option<&vm_memory::Memory>) -> Option<String> {
    if std::env::var_os(vm_memory::PROBED_ENV).is_some()
        || !matches!(runtime, "docker" | "podman" | "auto")
    {
        return None;
    }
    Some(memory.map_or_else(|| "none".to_string(), vm_memory::Memory::to_env))
}

/// How the gate starts each child `wrkflw run`.
struct Launcher<'a> {
    program: &'a Path,
    runtime: &'a str,
    /// The runtime's memory, probed once by the gate, for
    /// [`vm_memory::PROBED_ENV`].
    probed: Option<&'a str>,
}

impl Launcher<'_> {
    /// Run the child that checks `file` as `event`, and return its entry for
    /// the `gate` object, with the event it was checked as. With `json`, the
    /// child prints JSON and its `run_finished` becomes the entry; without,
    /// its output reaches the terminal and the entry comes from its exit code.
    fn check(&self, ctx: &GateCtx, file: &Path, event: &Event, json: bool) -> Value {
        let mut child = self.command(ctx, file, event, json);
        let mut entry = if json {
            child.stdin(Stdio::null()).stderr(Stdio::inherit());
            match child.output() {
                Ok(output) => workflow_report(
                    file,
                    output.status.code(),
                    &String::from_utf8_lossy(&output.stdout),
                ),
                Err(e) => json!({
                    "path": file.display().to_string(),
                    "status": "error",
                    "error": format!("cannot run wrkflw: {}", e),
                }),
            }
        } else {
            println!(
                "{}",
                cli_style::dim(&format!("gate: {} ({})", file.display(), event.describe()))
            );
            text_entry(file, child.status().ok().and_then(|status| status.code()))
        };
        entry["event"] = json!(event.name);
        if let Some(base) = &event.base_branch {
            entry["base_branch"] = json!(base);
        }
        entry
    }

    /// The child that checks `file` as `event`, printing JSON when `json`.
    fn command(&self, ctx: &GateCtx, file: &Path, event: &Event, json: bool) -> Command {
        let mut child = Command::new(self.program);
        if json {
            child.args(["--format", "json"]);
        }
        child
            .args(run_args(ctx, file, event))
            .args(["--runtime", self.runtime]);
        crate::cli_secrets::pass_to(&mut child);
        if let Some(value) = self.probed {
            // Used instead of probing the runtime again.
            child.env(vm_memory::PROBED_ENV, value);
        }
        child
    }
}

/// Run the gate. Exits 1 when a workflow fails or can't be run.
pub(crate) fn run(ctx: GateCtx) {
    let program = std::env::current_exe().unwrap_or_else(|e| {
        ctx.format
            .fail(&format!("cannot find this executable: {}", e))
    });
    if ctx.install_hook {
        let script = hook_script(&program, ctx.runtime.as_flag(), ctx.remote_cache.as_deref());
        let installed = hook_path().and_then(|path| {
            install_hook_at(&path, &script)?;
            Ok(path)
        });
        match installed {
            Ok(path) if ctx.format == OutputFormat::Json => {
                wrkflw_runs::emit_line(&json!({
                    "schema_version": SCHEMA_VERSION,
                    "type": "gate_hook",
                    "path": path,
                }));
            }
            Ok(path) => println!("Installed {}", path.display()),
            Err(e) => ctx.format.fail(&e),
        }
        return;
    }
    let files = workflow_files(ctx.path.as_deref()).unwrap_or_else(|e| ctx.format.fail(&e));
    let runtime = ctx.runtime.as_flag();
    let uncommitted = uncommitted_changes();
    if let Some(warning) = dirty_tree_warning(&uncommitted) {
        eprintln!("{}", cli_style::dim(&warning));
    }
    let (first, second) = events(
        &ctx,
        current_branch().as_deref(),
        default_branch().as_deref(),
    );
    // Every workflow, and on a feature branch every workflow twice, runs in
    // a child. Ask the runtime about memory once here and warn once, rather
    // than in each child.
    let memory = probe_memory(runtime);
    if let Some(warning) = memory.as_ref().and_then(vm_memory::Memory::warning) {
        eprintln!("{}", cli_style::warning(&warning));
    }
    let probed = probed_env(runtime, memory.as_ref());
    let launcher = Launcher {
        program: &program,
        runtime,
        probed: probed.as_deref(),
    };
    let json_mode = ctx.format == OutputFormat::Json;
    // Every workflow runs even after a failure, so one push shows all of them.
    let workflows: Vec<Value> = files
        .iter()
        .map(|file| {
            check_workflow(&first, second.as_ref(), json_mode, |event| {
                launcher.check(&ctx, file, event, json_mode)
            })
        })
        .collect();
    let report = gate_report(workflows, &uncommitted);
    let failed = report["verdict"] == "failed";
    if report["verdict"] == "nothing_checked" {
        eprintln!(
            "{}",
            cli_style::warning(&nothing_checked_warning(&first, second.as_ref()))
        );
    }
    if json_mode {
        wrkflw_runs::emit_line(&report);
    } else if failed {
        let paths: Vec<&str> = report["failed"]
            .as_array()
            .into_iter()
            .flatten()
            .filter_map(Value::as_str)
            .collect();
        eprintln!("gate failed: {}", paths.join(", "));
    } else if report["verdict"] != "nothing_checked" {
        println!(
            "gate passed: {} workflow(s) ran, {} skipped",
            report["ran"], report["skipped"]
        );
    }
    std::process::exit(i32::from(failed));
}

/// Check one workflow as `first`, and again as `second` when the first ran
/// no job, the one place the gate decides that. Both output formats come
/// through here, so they agree on what ran.
///
/// `check` runs the child and returns its entry. When both events checked
/// nothing, the entry's `skip_reason` gives both reasons.
fn check_workflow(
    first: &Event,
    second: Option<&Event>,
    json_mode: bool,
    mut check: impl FnMut(&Event) -> Value,
) -> Value {
    let entry = check(first);
    let Some(second) = second.filter(|_| checked_nothing(&entry)) else {
        return entry;
    };
    if !json_mode {
        println!(
            "{}",
            cli_style::dim(&format!(
                "gate: {} runs no job for a {}, checking it as a {}",
                entry["path"].as_str().unwrap_or_default(),
                first.describe(),
                second.describe()
            ))
        );
    }
    let first_reason = why_nothing_ran(&entry);
    let mut entry = check(second);
    if checked_nothing(&entry) {
        entry["skip_reason"] = json!(format!(
            "{}: {}; {}: {}",
            first.describe(),
            first_reason,
            second.describe(),
            why_nothing_ran(&entry),
        ));
    }
    entry
}

/// The entry for a child run in text mode, from its exit code alone: its
/// output went to the terminal, so the code is all there is to read.
fn text_entry(file: &Path, code: Option<i32>) -> Value {
    let mut entry = match code {
        Some(0) => json!({"status": "success"}),
        Some(SKIPPED_EXIT) => json!({"status": "skipped"}),
        Some(NOTHING_CHECKED_EXIT) => json!({"status": "success", "verdict": "nothing_checked"}),
        _ => json!({"status": "failure", "error": format!("wrkflw exited with {code:?}")}),
    };
    entry["path"] = json!(file.display().to_string());
    entry
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_gate_object_fails_on_a_failed_or_broken_workflow() {
        let passed = r#"{"type":"run_finished","status":"success","run_id":"a","jobs":[],"unverified":[{"job":"b (os: windows-latest)"}],"failure":null,"error":null,"skip_reason":null}"#;
        let failed = r#"{"type":"run_finished","status":"failure","run_id":"b","jobs":[],"unverified":[],"failure":{"failures":[]},"error":null,"skip_reason":null}"#;
        let skipped = r#"{"type":"run_finished","status":"skipped","run_id":"c","jobs":[],"unverified":[],"failure":null,"error":null,"skip_reason":"no match"}"#;
        let workflows = vec![
            workflow_report(
                Path::new("ok.yml"),
                Some(0),
                &format!("{{\"type\":\"job_started\"}}\n{passed}\n"),
            ),
            workflow_report(Path::new("bad.yml"), Some(1), failed),
            workflow_report(Path::new("off.yml"), Some(SKIPPED_EXIT), skipped),
            workflow_report(Path::new("crash.yml"), None, ""),
            workflow_report(
                Path::new("err.yml"),
                Some(1),
                r#"{"type":"error","message":"boom"}"#,
            ),
        ];
        let report = gate_report(workflows, &[]);
        assert_eq!(report["passed"], false);
        assert_eq!(report["verdict"], "failed");
        assert_eq!(report["ran"], 4);
        assert_eq!(report["skipped"], 1);
        assert_eq!(report["failed"], json!(["bad.yml", "crash.yml", "err.yml"]));
        assert_eq!(report["workflows"][4]["error"], "boom");
        assert_eq!(report["unverified"][0]["workflow"], "ok.yml");
        assert_eq!(report["workflows"][0]["run_id"], "a");
    }

    #[test]
    fn a_child_that_died_mid_run_still_names_its_run() {
        let stdout = concat!(
            r#"{"type":"run_started","run_id":"r1"}"#,
            "\n",
            r#"{"type":"step_finished","run_id":"r1","step":"build"}"#,
            "\n",
        );
        let entry = workflow_report(Path::new("x.yml"), None, stdout);
        assert_eq!(entry["status"], "error");
        assert_eq!(entry["run_id"], "r1");
        assert!(entry["error"]
            .as_str()
            .unwrap()
            .contains("before the run finished"));
    }

    #[test]
    fn the_result_is_read_from_run_finished_not_the_last_line() {
        let stdout = concat!(
            r#"{"type":"run_finished","status":"failure","run_id":"r2","jobs":[]}"#,
            "\n",
            r#"{"type":"note","message":"printed after the result"}"#,
            "\n",
        );
        let entry = workflow_report(Path::new("x.yml"), Some(1), stdout);
        assert_eq!(entry["status"], "failure");
        assert_eq!(entry["run_id"], "r2");
    }

    #[test]
    fn every_run_finished_field_reaches_the_gate() {
        let run = r#"{"schema_version":1,"type":"run_finished","status":"success","run_id":"a","jobs":[],"duration_ms":5,"exit_code":0}"#;
        let entry = workflow_report(Path::new("x.yml"), Some(0), run);
        assert_eq!(entry["duration_ms"], 5);
        assert_eq!(entry["exit_code"], 0);
        assert_eq!(entry["path"], "x.yml");
        assert!(entry.get("type").is_none());
        assert!(entry.get("schema_version").is_none());
    }

    #[test]
    fn a_success_with_a_failing_exit_code_is_not_a_pass() {
        let run = r#"{"type":"run_finished","status":"success","run_id":"a","jobs":[]}"#;
        let entry = workflow_report(Path::new("x.yml"), Some(101), run);
        assert_eq!(entry["status"], "error");
        assert_eq!(gate_report(vec![entry], &[])["passed"], false);
    }

    fn ctx() -> GateCtx {
        GateCtx {
            path: None,
            event: None,
            base_branch: None,
            diff_base: None,
            runtime: RuntimeChoice::Auto,
            remote_cache: None,
            install_hook: false,
            format: OutputFormat::Text,
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
        let event = Event {
            name: "pull_request".to_string(),
            base_branch: Some("main".to_string()),
        };
        let args = run_args(&ctx(), Path::new("ci.yml"), &event);
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
        assert!(args.windows(2).any(|w| w == ["--event", "pull_request"]));
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

    fn finished(status: &str, verdict: &str, unverified: &str) -> String {
        format!(
            r#"{{"type":"run_finished","status":"{status}","verdict":"{verdict}","run_id":"a","jobs":[],"unverified":{unverified}}}"#
        )
    }

    #[test]
    fn a_gate_that_ran_nothing_does_not_pass() {
        let skipped = workflow_report(
            Path::new("ci.yml"),
            Some(SKIPPED_EXIT),
            &finished("skipped", "skipped", "[]"),
        );
        let report = gate_report(vec![skipped], &[]);
        assert_eq!(report["verdict"], "nothing_checked");
        assert_eq!(report["passed"], false);
        assert_eq!(report["ran"], 0);

        let all_jobs_skipped = workflow_report(
            Path::new("ci.yml"),
            Some(0),
            &finished("success", "nothing_checked", "[]"),
        );
        let report = gate_report(vec![all_jobs_skipped], &[]);
        assert_eq!(report["verdict"], "nothing_checked");
        assert_eq!(report["passed"], false);
        assert_eq!(report["ran"], 0, "{report}");
        assert_eq!(report["skipped"], 1, "{report}");
    }

    #[test]
    fn a_pass_with_jobs_run_as_another_os_is_partial() {
        let partial = workflow_report(
            Path::new("ci.yml"),
            Some(0),
            &finished(
                "success",
                "partial",
                r#"[{"job":"b (os: windows-latest)"}]"#,
            ),
        );
        let report = gate_report(vec![partial], &[]);
        assert_eq!(report["verdict"], "partial");
        assert_eq!(report["passed"], true);

        let verified = workflow_report(
            Path::new("ci.yml"),
            Some(0),
            &finished("success", "verified", "[]"),
        );
        assert_eq!(gate_report(vec![verified], &[])["verdict"], "verified");
    }

    #[test]
    fn a_feature_branch_falls_back_to_a_pull_request_into_the_default_branch() {
        let (first, second) = events(&ctx(), Some("feat/x"), Some("main"));
        assert_eq!(first.name, "push");
        let second = second.unwrap();
        assert_eq!(second.name, "pull_request");
        assert_eq!(second.base_branch.as_deref(), Some("main"));
        assert_eq!(second.describe(), "pull_request into main");

        let (_, detached) = events(&ctx(), None, Some("main"));
        assert!(
            detached.is_some(),
            "a detached HEAD is not the default branch"
        );

        let (_, on_default) = events(&ctx(), Some("main"), Some("main"));
        assert_eq!(on_default, None);

        let (_, unknown) = events(&ctx(), Some("feat/x"), None);
        assert_eq!(unknown, None);

        let into_develop = GateCtx {
            base_branch: Some("develop".to_string()),
            ..ctx()
        };
        let (_, second) = events(&into_develop, Some("feat/x"), Some("main"));
        assert_eq!(second.unwrap().base_branch.as_deref(), Some("develop"));
    }

    #[test]
    fn an_explicit_event_is_the_only_one_checked() {
        let explicit = GateCtx {
            event: Some("pull_request".to_string()),
            base_branch: Some("main".to_string()),
            ..ctx()
        };
        let (first, second) = events(&explicit, Some("feat/x"), Some("main"));
        assert_eq!(first.name, "pull_request");
        assert_eq!(first.base_branch.as_deref(), Some("main"));
        assert_eq!(second, None);
    }
}
