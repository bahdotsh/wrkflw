use crate::config::DEFAULT_GIT_COMMAND_TIMEOUT;
use crate::error::TriggerFilterError;
use std::path::Path;
use std::time::Duration;
use tokio::process::Command;

/// Default upper bound used when the caller does not pass a per-call
/// override. See [`crate::config::DEFAULT_GIT_COMMAND_TIMEOUT`] for the
/// authoritative knob. We re-alias it locally so existing private
/// helpers can reference a stable name.
const GIT_COMMAND_TIMEOUT: Duration = DEFAULT_GIT_COMMAND_TIMEOUT;

/// Build a `git` command optionally rooted at a working directory via `-C`.
///
/// `kill_on_drop(true)` is load-bearing: [`run_git`] enforces a hard timeout
/// via `tokio::time::timeout`, but a timeout only drops the future — without
/// `kill_on_drop`, the underlying child process keeps running until it exits
/// on its own. The whole point of the timeout is to handle hung-process
/// failure modes (network filesystem stalls, credential prompts, corrupt
/// repos), so we MUST reap the child when we give up on it. Otherwise the
/// long-running watch loop accumulates one zombie per timed-out call.
///
/// `stdin(Stdio::null())` + `GIT_TERMINAL_PROMPT=0` are also load-bearing and
/// must never be removed. Without them:
///
/// 1. In raw-terminal TUI mode a backgrounded git subprocess inherits the
///    same TTY stdin as ratatui and can consume the user's keystrokes for
///    the full `GIT_COMMAND_TIMEOUT` window (up to 10 s) before the reaper
///    kicks in. This is visible on any repo whose remote requires auth.
/// 2. Git's askpass / credential helper can block indefinitely on a
///    `terminal.prompt`, which the timeout only partially mitigates —
///    during those 10 s git is actively reading from stdin, racing the UI
///    for every keystroke. Forcing `GIT_TERMINAL_PROMPT=0` turns the
///    prompt into an immediate auth-failure error instead.
///
/// The combination means every git subprocess is fully insulated from the
/// parent process's terminal state, regardless of caller (CLI, watcher, TUI).
fn git_cmd(cwd: Option<&Path>) -> Command {
    let mut cmd = Command::new("git");
    cmd.kill_on_drop(true)
        .stdin(std::process::Stdio::null())
        .env("GIT_TERMINAL_PROMPT", "0");
    if let Some(dir) = cwd {
        cmd.arg("-C").arg(dir.as_os_str());
    }
    cmd
}

/// Validate a git ref name to prevent argv injection.
///
/// We can't trust user-supplied refs to be safe to splat into a `git` argv —
/// a value like `--upload-pack=foo` would be parsed as an option. Reject any
/// ref starting with `-` or containing characters that aren't part of git's
/// documented revision syntax.
///
/// This validates a *single* ref. Range expressions (`a..b`, `a...b`) are
/// rejected — callers that need to take a range must take two refs and
/// validate each independently.
pub fn validate_ref_name(name: &str) -> Result<(), TriggerFilterError> {
    if name.is_empty() {
        return Err(TriggerFilterError::GitError(
            "git ref name must not be empty".to_string(),
        ));
    }
    if name.starts_with('-') {
        return Err(TriggerFilterError::GitError(format!(
            "git ref name '{}' must not start with '-' (refused as possible flag injection)",
            name
        )));
    }
    // `..` is git's range syntax. Accepting it here would let a caller
    // smuggle a range through an API that promises a single ref — and
    // when interpolated into another `format!("{}..{}", ...)` it
    // produces a malformed three-dot expression that surfaces as a
    // confusing `git diff` error. Reject it up front instead.
    if name.contains("..") {
        return Err(TriggerFilterError::GitError(format!(
            "git ref name '{}' must not contain '..' (range syntax is not a valid single ref)",
            name
        )));
    }
    // Allowlist covers:
    // - branch/tag names and sha1s
    // - path separators in refs: `/`, `.`
    // - revision suffixes: `~`, `^`
    // - reflog / upstream syntax: `@`, `{`, `}`
    //   (e.g. `HEAD@{1}`, `origin/main@{upstream}`, `@` as a synonym for HEAD)
    if !name.chars().all(|c| {
        c.is_ascii_alphanumeric()
            || matches!(c, '_' | '-' | '/' | '.' | '~' | '^' | '@' | '{' | '}')
    }) {
        return Err(TriggerFilterError::GitError(format!(
            "git ref name '{}' contains characters outside the allowed set",
            name
        )));
    }
    Ok(())
}

/// Run a prepared `git` command with a hard timeout. Maps timeout and spawn
/// failures into `TriggerFilterError::GitError` with a consistent message
/// shape so callers don't have to.
async fn run_git(
    mut cmd: Command,
    cmd_label: &str,
) -> Result<std::process::Output, TriggerFilterError> {
    let fut = cmd.output();
    match tokio::time::timeout(GIT_COMMAND_TIMEOUT, fut).await {
        Ok(Ok(output)) => Ok(output),
        Ok(Err(e)) => Err(TriggerFilterError::GitError(format!(
            "Failed to run {}: {}",
            cmd_label, e
        ))),
        Err(_) => Err(TriggerFilterError::GitError(format!(
            "{} timed out after {}s (git subprocess hung — check for network \
             filesystems, credential prompts, or corrupt repository state)",
            cmd_label,
            GIT_COMMAND_TIMEOUT.as_secs()
        ))),
    }
}

// ---------------------------------------------------------------------------
// Helpers operating on raw command output
// ---------------------------------------------------------------------------

/// Parse NUL-terminated output from `git ... -z`.
///
/// `git diff --name-only -z` and `git ls-files --others -z` emit each
/// path as a NUL-terminated record (optionally with a trailing NUL on
/// the last record). We deliberately do NOT use a newline-splitting
/// parser here — paths containing newlines would otherwise be silently
/// split into two entries, and git's default output *without* `-z`
/// additionally octal-escapes non-ASCII bytes, which would fail to
/// match any `paths:` glob written against the repo's true on-disk
/// form. Previously a `parse_lines` helper did exactly that and lost
/// both classes of path.
///
/// **Non-UTF-8 filenames.** On Unix, filesystem paths are not required
/// to be valid UTF-8, but the rest of the crate operates on `String`
/// glob arguments. Records that fail UTF-8 validation are collected in
/// the returned `lossy` list (with `String::from_utf8_lossy` applied so
/// they still round-trip visibly) AND counted in the `lossy_count`
/// return so callers can surface a diagnostic. Silently replacing
/// invalid bytes with U+FFFD — which the old implementation did — is
/// the same "silently drop a file from the change set" failure mode
/// the newline-split fix was put here to prevent: a workflow gated on
/// `paths:` would mysteriously not fire for a non-UTF-8 filename, and
/// no diagnostic would point at the cause. Callers should hoist the
/// warning into `EventContext::warnings` so the user sees the root
/// cause instead of a mystery non-match.
fn parse_nul_separated(output: &[u8]) -> NulParseResult {
    let mut files: Vec<String> = Vec::new();
    let mut lossy_names: Vec<String> = Vec::new();
    for bytes in output.split(|b| *b == 0).filter(|b| !b.is_empty()) {
        match std::str::from_utf8(bytes) {
            Ok(s) => files.push(s.to_string()),
            Err(_) => {
                // Retain a best-effort form so the user sees *which*
                // file was dropped; `from_utf8_lossy` replaces invalid
                // sequences with U+FFFD which is still visible and
                // grep-able, just not matchable by any real glob.
                let lossy = String::from_utf8_lossy(bytes).into_owned();
                lossy_names.push(lossy.clone());
                // Still push the lossy form into `files` so the rest
                // of the pipeline doesn't see a mysteriously shorter
                // list — the evaluator will simply fail to match it,
                // and the accompanying warning explains why.
                files.push(lossy);
            }
        }
    }
    NulParseResult { files, lossy_names }
}

/// Result of [`parse_nul_separated`].
///
/// `files` is the full list of paths in the order git emitted them.
/// `lossy_names` is the subset whose bytes failed UTF-8 validation and
/// were coerced via `from_utf8_lossy`. Callers MUST surface a warning
/// when `lossy_names` is non-empty — the lossy form will not match any
/// real glob pattern, and the silent drop is the same class of failure
/// the rest of this crate exists to plug.
#[derive(Debug, Default)]
struct NulParseResult {
    files: Vec<String>,
    lossy_names: Vec<String>,
}

fn merge_unique(mut into: Vec<String>, more: Vec<String>) -> Vec<String> {
    // `seen` MUST be updated as we go: otherwise duplicates within `more`
    // itself slip through and the function silently breaks its own
    // uniqueness contract. Use `HashSet::insert`'s "was new?" return so
    // each insertion is one lookup, not two.
    let mut seen: std::collections::HashSet<String> = into.iter().cloned().collect();
    for line in more {
        if seen.insert(line.clone()) {
            into.push(line);
        }
    }
    into
}

/// Get changed files between the working tree and a base ref.
///
/// `git diff <base>` compares the working tree against `<base>`, which
/// covers committed-on-branch changes plus any unstaged edits the user
/// has on top. We additionally probe `git ls-files --others
/// --exclude-standard` to surface untracked files, which `git diff`
/// alone misses.
///
/// Edge case worth knowing: a file that was modified, `git add`-ed, then
/// reverted in the working tree to match `<base>` will not show up here
/// (working tree == base), even though the index still has a divergent
/// version. We accept that — the trigger filter is a "what would run on
/// this checkout?" approximation, not a full index audit.
///
/// The same gap applies to staged deletions: if the user `git rm`-ed a
/// file but has not committed yet, the file is gone from the working
/// tree, present in the index, and present in `<base>`. `git diff <base>`
/// against the working tree reports it as deleted (the working tree no
/// longer has it), so this case IS covered. But a file that was deleted
/// in the index AND restored in the working tree is invisible — same
/// shape as the modified-then-reverted case above.
///
/// **Rename handling:** without `-M`/`--find-renames`, `--name-only`
/// reports a rename as a deletion of the old path plus an addition of
/// the new path, so both entries appear in the changed set. This
/// matches GitHub Actions' behavior on a PR or push diff, where
/// renames are also represented as a delete plus an add in the event
/// payload — `paths:` filters on either the old or the new location
/// correctly fire. Side effect: a `paths-ignore: ['docs/**']` filter
/// will NOT silence a rename from `docs/foo.md` to `src/foo.md`,
/// because `src/foo.md` is still in the changed set after the docs
/// entry is filtered out. This mirrors the hosted runner's semantics.
///
/// `cwd` selects the git working directory; pass `None` to use the
/// process CWD. The watcher should always pass its repo root.
pub async fn get_changed_files(
    base: &str,
    cwd: Option<&Path>,
) -> Result<Vec<String>, TriggerFilterError> {
    let (files, _warnings) = get_changed_files_with_warnings(base, cwd).await?;
    Ok(files)
}

/// Variant of [`get_changed_files`] that additionally returns any
/// non-fatal warnings collected while enriching the changed set.
///
/// The primary caller is `auto_detect_context*` — the CLI and the
/// watcher want to surface e.g. a failed `git ls-files --others` so
/// "untracked files missing from the changed set" becomes visible to
/// the user, instead of a silent gap that manifests as a mysteriously
/// unfiring `paths:` filter. The old `get_changed_files` shape is
/// retained as a thin wrapper for callers that don't care.
///
/// The tuple convention (data, warnings) is deliberate: returning a
/// struct would drag every caller that has been happily using the
/// plain `Vec<String>` shape through a refactor, and the warning
/// stream is empty on the happy path so the extra allocation is free.
pub async fn get_changed_files_with_warnings(
    base: &str,
    cwd: Option<&Path>,
) -> Result<(Vec<String>, Vec<String>), TriggerFilterError> {
    validate_ref_name(base)?;

    // `-z` makes both commands emit NUL-terminated records, which is
    // mandatory for correct splitting of paths that contain newlines,
    // and also suppresses git's default octal-escaping of non-ASCII
    // bytes (equivalent to `core.quotepath=false`). Without `-z`, a
    // file like `some\nfile.rs` splits into two bogus entries and a
    // Unicode filename round-trips through `\302\244`-style escapes
    // that no `paths:` glob can match. Pair with `parse_nul_separated`
    // below — never `parse_lines` on `-z` output.
    let mut diff_cmd = git_cmd(cwd);
    diff_cmd.args(["diff", "--name-only", "-z", base]);
    let mut untracked_cmd = git_cmd(cwd);
    untracked_cmd.args(["ls-files", "--others", "--exclude-standard", "-z"]);

    let (diff_res, untracked_res) = tokio::join!(
        run_git(diff_cmd, "git diff"),
        run_git(untracked_cmd, "git ls-files"),
    );

    let diff_output = check_status(diff_res?, "git diff")?;
    let diff_parsed = parse_nul_separated(&diff_output.stdout);
    let mut files = diff_parsed.files;

    // Untracked files are a best-effort enrichment — don't fail the
    // whole call if `ls-files` errors (e.g. a safe-directory rejection,
    // transient permission glitch). Previously the failure was
    // swallowed in an `if let Ok(_) = ...` block with no trace; the
    // user saw an incomplete changed set and a `paths: ['new/**']`
    // filter that mysteriously refused to fire.
    //
    // We return the diagnostic as data rather than logging it here.
    // Hosts (watcher / CLI / TUI) receive the warnings through
    // `EventContext::warnings` and own the rendering policy — the
    // library used to double-emit via `wrkflw_logging::warning` AND
    // the return value, which coupled the crate to a global log
    // sink and forced every test that observed warnings to assert
    // against stdout.
    let mut warnings = Vec::new();

    // Surface any non-UTF-8 filenames from the diff output. These
    // are retained in `files` in lossy U+FFFD form so the rest of
    // the pipeline doesn't short the list, but they will not match
    // any real glob — tell the user so they don't chase a mystery
    // non-match.
    if !diff_parsed.lossy_names.is_empty() {
        warnings.push(format!(
            "{} changed file(s) have non-UTF-8 names and will not match any \
             `paths:` glob — the evaluator operates on String globs, and the \
             lossy U+FFFD form retained in the change set is not the on-disk \
             form git emitted. Affected (lossy-decoded): {:?}. To trigger \
             workflows on these files, rename them to UTF-8-clean paths.",
            diff_parsed.lossy_names.len(),
            diff_parsed.lossy_names,
        ));
    }

    match untracked_res {
        Ok(out) if out.status.success() => {
            let untracked_parsed = parse_nul_separated(&out.stdout);
            if !untracked_parsed.lossy_names.is_empty() {
                warnings.push(format!(
                    "{} untracked file(s) have non-UTF-8 names and will not \
                     match any `paths:` glob. Affected (lossy-decoded): {:?}. \
                     To trigger workflows on these files, rename them to \
                     UTF-8-clean paths.",
                    untracked_parsed.lossy_names.len(),
                    untracked_parsed.lossy_names,
                ));
            }
            files = merge_unique(files, untracked_parsed.files);
        }
        Ok(out) => {
            let stderr = String::from_utf8_lossy(&out.stderr).trim().to_string();
            warnings.push(format!(
                "git ls-files --others failed (exit {}): {} — untracked files will \
                 be missing from the changed set, so workflows gated on `paths:` for \
                 brand-new files may be incorrectly reported as not triggering",
                out.status
                    .code()
                    .map(|c| c.to_string())
                    .unwrap_or_else(|| "signal".to_string()),
                if stderr.is_empty() {
                    "<no stderr>"
                } else {
                    &stderr
                }
            ));
        }
        Err(e) => {
            warnings.push(format!(
                "git ls-files --others errored: {} — untracked files will be missing \
                 from the changed set",
                e
            ));
        }
    }

    Ok((files, warnings))
}

fn check_status(
    output: std::process::Output,
    cmd_label: &str,
) -> Result<std::process::Output, TriggerFilterError> {
    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        return Err(TriggerFilterError::GitError(format!(
            "{} failed: {}",
            cmd_label,
            stderr.trim()
        )));
    }
    Ok(output)
}

/// Get changed files between two refs.
///
/// Non-UTF-8 filenames are surfaced as a log warning from inside this
/// helper rather than returned as data. The two-ref form is lower-level
/// than [`get_changed_files_with_warnings`] and its consumers (the CLI
/// `--diff-head` path) do not carry a warning buffer through — logging
/// directly is the pragmatic choice for a single call site, and it
/// matches the policy the CLI uses for every other diagnostic.
pub async fn get_changed_files_between(
    base_ref: &str,
    head_ref: &str,
    cwd: Option<&Path>,
) -> Result<Vec<String>, TriggerFilterError> {
    validate_ref_name(base_ref)?;
    validate_ref_name(head_ref)?;

    let range = format!("{}..{}", base_ref, head_ref);
    let mut cmd = git_cmd(cwd);
    // `-z`: NUL-terminate records. See `get_changed_files_with_warnings`
    // for the full rationale — same issue applies to this two-ref form.
    cmd.args(["diff", "--name-only", "-z", &range]);
    let output = check_status(run_git(cmd, "git diff").await?, "git diff")?;
    let parsed = parse_nul_separated(&output.stdout);
    if !parsed.lossy_names.is_empty() {
        wrkflw_logging::warning(&format!(
            "{} changed file(s) in the {}..{} range have non-UTF-8 names and \
             will not match any `paths:` glob (lossy-decoded: {:?}).",
            parsed.lossy_names.len(),
            base_ref,
            head_ref,
            parsed.lossy_names,
        ));
    }
    Ok(parsed.files)
}

/// Get the current branch name, or `None` if HEAD is detached.
///
/// `git rev-parse --abbrev-ref HEAD` returns the literal string `"HEAD"`
/// when the repository is in detached-HEAD state (e.g. after checking out
/// a tag or commit SHA). Treating that as a branch name would cause
/// `branches:` filters to match the pseudo-ref `HEAD`, which is almost
/// never what the user intended. Surface detached HEAD as "no branch"
/// instead so callers can fall back to explicit `--base-branch`.
pub async fn get_current_branch(cwd: Option<&Path>) -> Result<Option<String>, TriggerFilterError> {
    let mut cmd = git_cmd(cwd);
    cmd.args(["rev-parse", "--abbrev-ref", "HEAD"]);
    let output = check_status(run_git(cmd, "git rev-parse").await?, "git rev-parse")?;
    let name = String::from_utf8_lossy(&output.stdout).trim().to_string();
    if name.is_empty() || name == "HEAD" {
        Ok(None)
    } else {
        Ok(Some(name))
    }
}

/// Determine a sensible diff base for trigger evaluation.
///
/// Strategy:
/// 1. If there are uncommitted changes (vs HEAD), use "HEAD".
/// 2. Detect the remote default branch via `git symbolic-ref refs/remotes/origin/HEAD`.
/// 3. Fall back to trying `main`, then `master`.
/// 4. Otherwise try `HEAD~1`.
///
/// Returns an error if none of these succeed — previously this fell back to
/// the empty-tree SHA, which silently made every tracked file appear as
/// changed and defeated the purpose of the filter. Callers should surface
/// the error so the user knows to pass `--diff-base` explicitly.
///
/// `verbose` gates the "diff base = HEAD" explanatory log. The CLI
/// passes its `--verbose` flag through so the message lands in the
/// user's terminal; long-lived hosts (TUI toggle, any future daemon)
/// pass `false` to avoid flooding the log pane on every hot-path call.
/// Previously this log was unconditional and the TUI user saw it on
/// every diff-filter toggle against a dirty tree.
pub async fn get_default_diff_base(
    cwd: Option<&Path>,
    verbose: bool,
) -> Result<String, TriggerFilterError> {
    // Check for uncommitted changes first. `run_git` returns `Ok` even
    // for non-zero exit (e.g. "not a git repository"), so we must check
    // `output.status.success()` before trusting stdout — otherwise a
    // corrupted repo silently claims a clean tree and we fall through
    // to remote-default-branch probing under a confusing pretext.
    let mut status_cmd = git_cmd(cwd);
    status_cmd.args(["status", "--porcelain"]);
    if let Ok(output) = run_git(status_cmd, "git status").await {
        if output.status.success() && !String::from_utf8_lossy(&output.stdout).trim().is_empty() {
            if verbose {
                // Surface the heuristic so users aren't surprised when
                // `--diff` against a dirty tree only considers uncommitted
                // changes. The typical confusion: "I have WIP edits and a
                // workflow with `paths: ['src/**']` is reported as not
                // triggering even though my committed branch obviously
                // changed src/". Pointing them at `--diff-base` lets them
                // override without having to read the source.
                wrkflw_logging::info(
                    "diff base = HEAD: working tree has uncommitted changes, so \
                     --diff is comparing the working tree against the last commit. \
                     Pass --diff-base <ref> (e.g. main, origin/main) to compare \
                     against a branch instead.",
                );
            }
            return Ok("HEAD".to_string());
        }
    }

    // Build candidate list: detected default branch first, then common fallbacks
    let mut candidates: Vec<String> = Vec::new();

    // Try to detect the remote default branch
    let mut sym_cmd = git_cmd(cwd);
    sym_cmd.args(["symbolic-ref", "refs/remotes/origin/HEAD", "--short"]);
    if let Ok(output) = run_git(sym_cmd, "git symbolic-ref").await {
        if output.status.success() {
            let branch = String::from_utf8_lossy(&output.stdout).trim().to_string();
            // symbolic-ref with `--short` returns e.g. "origin/main".
            // Without `--short` (older git, or if a different
            // implementation slips through) it returns
            // "refs/remotes/origin/main". Strip whichever form is
            // present so the candidate name is a bare branch.
            let short = branch
                .strip_prefix("refs/remotes/origin/")
                .or_else(|| branch.strip_prefix("origin/"))
                .unwrap_or(&branch)
                .to_string();
            if !short.is_empty() {
                candidates.push(short);
            }
        }
    }

    // Common fallbacks in case symbolic-ref is unavailable
    for fallback in &["main", "master"] {
        let s = fallback.to_string();
        if !candidates.contains(&s) {
            candidates.push(s);
        }
    }

    // Try merge-base with each candidate
    for base_branch in &candidates {
        let mut mb_cmd = git_cmd(cwd);
        mb_cmd.args(["merge-base", "HEAD", base_branch]);
        if let Ok(output) = run_git(mb_cmd, "git merge-base").await {
            if output.status.success() {
                let mb = String::from_utf8_lossy(&output.stdout).trim().to_string();
                if !mb.is_empty() {
                    return Ok(mb);
                }
            }
        }
    }

    // Try HEAD~1, which works on any repo with at least two commits
    let mut parent_cmd = git_cmd(cwd);
    parent_cmd.args(["rev-parse", "--verify", "HEAD~1"]);
    if let Ok(output) = run_git(parent_cmd, "git rev-parse").await {
        if output.status.success() {
            return Ok("HEAD~1".to_string());
        }
    }

    Err(TriggerFilterError::GitError(
        "could not detect a diff base (no uncommitted changes, no remote default branch, \
         no main/master branch, and HEAD has no parent). Pass --diff-base explicitly \
         to tell wrkflw what to compare against."
            .to_string(),
    ))
}

/// Hard upper bound on how long `find_repo_root` will wait for `git
/// rev-parse --show-toplevel` before giving up and killing the child.
/// Shorter than [`GIT_COMMAND_TIMEOUT`] because this call is on the
/// startup / toggle-hot-path for every consumer, and a hung git here
/// delays the very first user interaction.
const FIND_REPO_ROOT_TIMEOUT: Duration = Duration::from_secs(5);

/// Classified failure from [`find_repo_root_detailed`].
///
/// Separating the failure modes lets the CLI (and anything else that
/// surfaces the error to a human) produce a specific diagnostic instead
/// of the generic `"not inside a git repository"` message the old
/// `Option`-returning API forced everyone into. That message was a
/// misdiagnosis for three of the four failure branches: missing
/// binary, hung subprocess, and I/O error on `stdout` all rendered as
/// "not a git repo", leading users to run `git init` at the wrong level
/// and then wonder why the error persisted.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum FindRepoRootError {
    /// `git` could not be spawned at all — typically "command not found"
    /// or permission denied on the PATH entry.
    GitNotInstalled(String),
    /// The subprocess ran for longer than [`FIND_REPO_ROOT_TIMEOUT`].
    /// Usually a hung credential helper, a stuck network mount, or a
    /// corrupted `.git/` directory.
    Timeout,
    /// `git rev-parse` exited non-zero — the process is not inside any
    /// git working tree.
    NotInRepository,
    /// Catch-all for wait/read failures that don't fit the classifications
    /// above (e.g. the child process crashed, stdout pipe broke).
    Other(String),
}

impl std::fmt::Display for FindRepoRootError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::GitNotInstalled(e) => write!(
                f,
                "`git` could not be executed: {}. Install git and ensure it is on PATH.",
                e
            ),
            Self::Timeout => write!(
                f,
                "`git rev-parse --show-toplevel` timed out after {}s. \
                 This usually means a hung credential prompt, a stuck network \
                 filesystem, or a corrupted .git/ directory — investigate with \
                 `GIT_TRACE=1 git rev-parse --show-toplevel`.",
                FIND_REPO_ROOT_TIMEOUT.as_secs()
            ),
            Self::NotInRepository => {
                write!(f, "not inside a git repository (run `git init` first)")
            }
            Self::Other(e) => write!(f, "git subprocess failed: {}", e),
        }
    }
}

impl std::error::Error for FindRepoRootError {}

/// Find the git repository root from the current working directory by
/// shelling out to `git rev-parse --show-toplevel`. Returns a classified
/// error on failure so callers can render the diagnostic the user needs.
///
/// Same synchronous + timeout-protected mechanics as [`find_repo_root`]
/// below — this is the implementation, and `find_repo_root` is a thin
/// `Option` adapter for legacy call sites that don't distinguish failure
/// modes.
///
/// `stdin(Stdio::null())` + `GIT_TERMINAL_PROMPT=0` mirror `git_cmd` above —
/// see its rationale for why the parent's TTY must never leak into a git
/// subprocess.
pub fn find_repo_root_detailed() -> Result<std::path::PathBuf, FindRepoRootError> {
    use std::io::Read;
    use std::process::{Command as StdCommand, Stdio};
    use std::time::Instant;

    let mut child = StdCommand::new("git")
        .args(["rev-parse", "--show-toplevel"])
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .env("GIT_TERMINAL_PROMPT", "0")
        .spawn()
        .map_err(|e| FindRepoRootError::GitNotInstalled(e.to_string()))?;

    let deadline = Instant::now() + FIND_REPO_ROOT_TIMEOUT;
    let exit_status = loop {
        match child.try_wait() {
            Ok(Some(status)) => break status,
            Ok(None) => {
                if Instant::now() >= deadline {
                    // Kill + reap the child so we don't leave a zombie.
                    // Errors on kill/wait are intentionally swallowed —
                    // we're already in the failure branch and there's
                    // nothing more useful we can do.
                    let _ = child.kill();
                    let _ = child.wait();
                    return Err(FindRepoRootError::Timeout);
                }
                std::thread::sleep(Duration::from_millis(25));
            }
            Err(e) => {
                let _ = child.kill();
                let _ = child.wait();
                return Err(FindRepoRootError::Other(format!("wait failed: {}", e)));
            }
        }
    };

    if !exit_status.success() {
        return Err(FindRepoRootError::NotInRepository);
    }

    // Child has exited; drain stdout. `wait_with_output` is not
    // available here because we've already waited — read from the
    // piped handle directly instead.
    let mut buf = String::new();
    child
        .stdout
        .take()
        .ok_or_else(|| FindRepoRootError::Other("stdout pipe closed".to_string()))?
        .read_to_string(&mut buf)
        .map_err(|e| FindRepoRootError::Other(format!("read stdout: {}", e)))?;
    let path = buf.trim().to_string();
    if path.is_empty() {
        Err(FindRepoRootError::NotInRepository)
    } else {
        Ok(std::path::PathBuf::from(path))
    }
}

/// Best-effort mtime of the repository's `.git/HEAD` file.
///
/// The watcher uses this to cheaply detect a `git checkout` happening
/// mid-TTL: if the current HEAD mtime differs from the one captured at
/// cache-population time, the cached `(branch, tag)` pair is stale even
/// though the TTL window has not elapsed. `None` means either the file
/// could not be stat'd (not a repo, linked worktree with an exotic
/// layout, transient filesystem error) or the platform does not
/// expose a `modified()` time — callers must treat `None` as "do not
/// know, skip the short-circuit" to avoid the false-positive path
/// where a stale cache is re-served forever because every call
/// observed `None`.
pub fn head_mtime(cwd: Option<&Path>) -> Option<std::time::SystemTime> {
    // Resolve the real HEAD file location before stat-ing it. Layout
    // cases we have to handle:
    //
    //   1. Plain repo: `<cwd>/.git` is a directory and HEAD lives at
    //      `<cwd>/.git/HEAD`. Cheap metadata read, 99% case.
    //   2. Linked worktree: `<cwd>/.git` is a regular FILE whose
    //      contents are `gitdir: /abs/path/to/.git/worktrees/<name>`.
    //      The real HEAD lives at that `gitdir` + `/HEAD`. Stat-ing
    //      `<cwd>/.git/HEAD` would return `NotFound` and the watcher
    //      would serve its cached `(branch, tag)` forever on any
    //      worktree-based workflow.
    //   3. Submodule / custom gitdir: same shape as (2) but gitdir
    //      may be a relative path, which we resolve relative to the
    //      `.git` file's parent.
    //
    // Keep this synchronous: the watcher calls it from a tight
    // cache-check loop and cannot afford to fan out to
    // `spawn_blocking` every cycle.
    let base = cwd.map(Path::to_path_buf).unwrap_or_else(|| {
        std::env::current_dir().unwrap_or_else(|_| std::path::PathBuf::from("."))
    });
    let dotgit = base.join(".git");

    // Resolve the effective gitdir. When `.git` is a directory, the
    // gitdir IS `.git`; when it's a file, parse the `gitdir: ...`
    // pointer and resolve it (relative paths are resolved against the
    // parent of the `.git` file, matching git's own rules).
    let gitdir = match std::fs::metadata(&dotgit) {
        Ok(meta) if meta.is_dir() => dotgit.clone(),
        Ok(meta) if meta.is_file() => {
            let contents = std::fs::read_to_string(&dotgit).ok()?;
            let target = contents
                .lines()
                .find_map(|l| l.strip_prefix("gitdir:"))
                .map(str::trim)?;
            let target_path = std::path::Path::new(target);
            if target_path.is_absolute() {
                target_path.to_path_buf()
            } else {
                // Resolve against the parent of the `.git` file — the
                // same anchor git itself uses for relative gitdir
                // pointers.
                base.join(target_path)
            }
        }
        _ => return None,
    };

    let head_path = gitdir.join("HEAD");
    let meta = std::fs::metadata(&head_path).ok()?;
    meta.modified().ok()
}

/// Get the current tag if HEAD is tagged, or None.
pub async fn get_current_tag(cwd: Option<&Path>) -> Result<Option<String>, TriggerFilterError> {
    let mut cmd = git_cmd(cwd);
    cmd.args(["describe", "--tags", "--exact-match", "HEAD"]);
    match run_git(cmd, "git describe").await {
        Ok(output) if output.status.success() => {
            let tag = String::from_utf8_lossy(&output.stdout).trim().to_string();
            if tag.is_empty() {
                Ok(None)
            } else {
                Ok(Some(tag))
            }
        }
        Ok(_) => Ok(None), // not on a tag is normal
        Err(e) => Err(e),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::PathBuf;
    use std::process::Command as StdCommand;
    use tempfile::TempDir;

    /// Initialize a bare-bones git repo in `dir` with a single committed
    /// file and one branch `main`. Returns the path so the caller can
    /// operate on it.
    fn init_repo(dir: &Path) {
        // Git >= 2.28 supports --initial-branch; older needs fallback.
        let status = StdCommand::new("git")
            .args(["-C", dir.to_str().unwrap(), "init", "--initial-branch=main"])
            .status();
        if !status.map(|s| s.success()).unwrap_or(false) {
            StdCommand::new("git")
                .args(["-C", dir.to_str().unwrap(), "init"])
                .status()
                .expect("git init");
            StdCommand::new("git")
                .args(["-C", dir.to_str().unwrap(), "checkout", "-b", "main"])
                .status()
                .expect("git checkout -b main");
        }
        // Configure identity to allow commits (sandboxed CI may have no global config)
        for (k, v) in [("user.email", "t@t.t"), ("user.name", "t")] {
            StdCommand::new("git")
                .args(["-C", dir.to_str().unwrap(), "config", k, v])
                .status()
                .expect("git config");
        }
    }

    fn commit_file(dir: &Path, name: &str, content: &str) {
        let path = dir.join(name);
        std::fs::write(&path, content).expect("write");
        StdCommand::new("git")
            .args(["-C", dir.to_str().unwrap(), "add", name])
            .status()
            .expect("git add");
        StdCommand::new("git")
            .args([
                "-C",
                dir.to_str().unwrap(),
                "commit",
                "-m",
                "msg",
                "--no-gpg-sign",
            ])
            .status()
            .expect("git commit");
    }

    fn git_available() -> bool {
        StdCommand::new("git")
            .arg("--version")
            .status()
            .map(|s| s.success())
            .unwrap_or(false)
    }

    #[tokio::test]
    async fn get_changed_files_reports_modified_and_untracked() {
        if !git_available() {
            return;
        }
        let tmp = TempDir::new().expect("tempdir");
        let repo: PathBuf = tmp.path().to_path_buf();
        init_repo(&repo);
        commit_file(&repo, "tracked.txt", "a");

        // Modify tracked file, add an untracked file
        std::fs::write(repo.join("tracked.txt"), "b").unwrap();
        std::fs::write(repo.join("new.txt"), "x").unwrap();

        let files = get_changed_files("HEAD", Some(&repo))
            .await
            .expect("get_changed_files");
        assert!(files.iter().any(|f| f == "tracked.txt"));
        assert!(files.iter().any(|f| f == "new.txt"));
    }

    #[tokio::test]
    async fn get_changed_files_reports_deleted_file() {
        if !git_available() {
            return;
        }
        let tmp = TempDir::new().expect("tempdir");
        let repo: PathBuf = tmp.path().to_path_buf();
        init_repo(&repo);
        commit_file(&repo, "doomed.txt", "a");

        std::fs::remove_file(repo.join("doomed.txt")).unwrap();

        let files = get_changed_files("HEAD", Some(&repo))
            .await
            .expect("get_changed_files");
        assert!(
            files.iter().any(|f| f == "doomed.txt"),
            "deleted files must appear in changed set, got {:?}",
            files
        );
    }

    #[tokio::test]
    async fn get_default_diff_base_returns_head_on_dirty_tree() {
        if !git_available() {
            return;
        }
        let tmp = TempDir::new().expect("tempdir");
        let repo: PathBuf = tmp.path().to_path_buf();
        init_repo(&repo);
        commit_file(&repo, "a.txt", "1");
        // Make tree dirty
        std::fs::write(repo.join("a.txt"), "2").unwrap();

        let base = get_default_diff_base(Some(&repo), false)
            .await
            .expect("diff base");
        assert_eq!(base, "HEAD");
    }

    #[tokio::test]
    async fn get_default_diff_base_errors_when_no_base_available() {
        // A repo whose only branch is neither `main` nor `master`, with no
        // remote and a single root commit, has no valid diff base:
        //   - no uncommitted changes
        //   - no remote default branch (no origin)
        //   - no main/master fallback
        //   - no HEAD~1 (root commit)
        // The function must error rather than silently fall back to an
        // empty-tree SHA as it used to.
        if !git_available() {
            return;
        }
        let tmp = TempDir::new().expect("tempdir");
        let repo: PathBuf = tmp.path().to_path_buf();
        // Initialize with a custom branch so there's no main/master.
        let status = StdCommand::new("git")
            .args([
                "-C",
                repo.to_str().unwrap(),
                "init",
                "--initial-branch=weirdname",
            ])
            .status();
        if !status.map(|s| s.success()).unwrap_or(false) {
            // Older git fallback — skip the test if we can't force a
            // non-main initial branch.
            return;
        }
        for (k, v) in [("user.email", "t@t.t"), ("user.name", "t")] {
            StdCommand::new("git")
                .args(["-C", repo.to_str().unwrap(), "config", k, v])
                .status()
                .expect("git config");
        }
        commit_file(&repo, "a.txt", "1");

        let err = get_default_diff_base(Some(&repo), false).await;
        assert!(
            err.is_err(),
            "expected error when no diff base is available, got {:?}",
            err
        );
    }

    #[tokio::test]
    async fn get_current_branch_returns_none_on_detached_head() {
        if !git_available() {
            return;
        }
        let tmp = TempDir::new().expect("tempdir");
        let repo: PathBuf = tmp.path().to_path_buf();
        init_repo(&repo);
        commit_file(&repo, "a.txt", "1");
        commit_file(&repo, "b.txt", "2");
        // Detach HEAD
        StdCommand::new("git")
            .args(["-C", repo.to_str().unwrap(), "checkout", "HEAD~1"])
            .status()
            .expect("checkout");

        let result = get_current_branch(Some(&repo)).await.expect("branch");
        assert_eq!(
            result, None,
            "detached HEAD must be reported as None, got {:?}",
            result
        );
    }

    #[tokio::test]
    async fn get_current_branch_returns_name_on_normal_checkout() {
        if !git_available() {
            return;
        }
        let tmp = TempDir::new().expect("tempdir");
        let repo: PathBuf = tmp.path().to_path_buf();
        init_repo(&repo);
        commit_file(&repo, "a.txt", "1");

        let result = get_current_branch(Some(&repo)).await.expect("branch");
        assert_eq!(result, Some("main".to_string()));
    }

    #[test]
    fn validate_ref_accepts_normal_branches() {
        assert!(validate_ref_name("main").is_ok());
        assert!(validate_ref_name("feature/foo").is_ok());
        assert!(validate_ref_name("release-1.2.3").is_ok());
        assert!(validate_ref_name("HEAD").is_ok());
        assert!(validate_ref_name("HEAD~1").is_ok());
        assert!(validate_ref_name("HEAD^").is_ok());
    }

    #[test]
    fn validate_ref_rejects_flag_injection() {
        assert!(validate_ref_name("--upload-pack=foo").is_err());
        assert!(validate_ref_name("-fbad").is_err());
    }

    #[test]
    fn validate_ref_rejects_empty() {
        assert!(validate_ref_name("").is_err());
    }

    #[test]
    fn validate_ref_rejects_range_syntax() {
        // `..` is git's range expression — not a valid single ref, and
        // smuggling it through here turns `{base}..{head}` interpolation
        // into a malformed three-dot mess.
        assert!(validate_ref_name("HEAD..foo").is_err());
        assert!(validate_ref_name("..").is_err());
        assert!(validate_ref_name("main..feature").is_err());
        // A single dot remains valid (e.g. `release-1.2.3`).
        assert!(validate_ref_name("release-1.2.3").is_ok());
    }

    #[test]
    fn validate_ref_rejects_shell_metachars() {
        assert!(validate_ref_name("main; rm -rf /").is_err());
        assert!(validate_ref_name("main`whoami`").is_err());
        assert!(validate_ref_name("main$(id)").is_err());
    }

    #[test]
    fn merge_unique_dedupes_within_more() {
        // Regression: previously `seen` was built once from `into` and
        // never updated, so duplicates within `more` itself were both
        // appended. The function name promises uniqueness — exercise the
        // case where `into` is empty and `more` carries duplicates.
        let merged = merge_unique(Vec::new(), vec!["a".into(), "b".into(), "a".into()]);
        assert_eq!(
            merged,
            vec!["a".to_string(), "b".to_string()],
            "merge_unique must deduplicate `more` against itself"
        );
    }

    #[test]
    fn merge_unique_skips_entries_already_in_into() {
        let merged = merge_unique(
            vec!["a".into(), "b".into()],
            vec!["b".into(), "c".into(), "a".into()],
        );
        assert_eq!(
            merged,
            vec!["a".to_string(), "b".to_string(), "c".to_string()]
        );
    }

    #[test]
    fn validate_ref_accepts_reflog_and_upstream_syntax() {
        assert!(validate_ref_name("HEAD@{1}").is_ok());
        assert!(validate_ref_name("origin/main@{upstream}").is_ok());
        assert!(validate_ref_name("@").is_ok());
        assert!(validate_ref_name("@~1").is_ok());
    }

    #[test]
    fn parse_nul_separated_splits_on_nul_and_preserves_newlines() {
        // Regression pin: `git diff --name-only -z` emits records
        // separated by NUL bytes precisely because path names can
        // legitimately contain newlines. The old newline-splitting
        // parser would turn `foo\nbar.txt` into two fake entries and
        // silently lose the real file from the changed set.
        let raw: Vec<u8> = b"a.txt\0foo\nbar.txt\0c.txt\0".to_vec();
        let parsed = parse_nul_separated(&raw);
        assert_eq!(
            parsed.files,
            vec![
                "a.txt".to_string(),
                "foo\nbar.txt".to_string(),
                "c.txt".to_string()
            ],
            "NUL splitter must preserve newlines inside a record"
        );
        assert!(
            parsed.lossy_names.is_empty(),
            "clean UTF-8 must not produce lossy_names entries"
        );
    }

    #[test]
    fn parse_nul_separated_drops_empty_trailing_record() {
        // `git diff -z` usually (but not always) emits a trailing NUL
        // after the last record. The parser must not treat that
        // trailing NUL as an empty filename — otherwise `.contains`
        // or set membership against a path of `""` produces silent
        // false negatives.
        let raw = b"a.txt\0b.txt\0".to_vec();
        let parsed = parse_nul_separated(&raw);
        assert_eq!(parsed.files, vec!["a.txt".to_string(), "b.txt".to_string()]);
        assert!(parsed.lossy_names.is_empty());
    }

    #[test]
    fn parse_nul_separated_handles_empty_output() {
        let parsed = parse_nul_separated(&[]);
        assert!(parsed.files.is_empty());
        assert!(parsed.lossy_names.is_empty());
    }

    #[test]
    fn parse_nul_separated_records_non_utf8_filenames() {
        // Regression pin: the old implementation called
        // `from_utf8_lossy(...).into_owned()` unconditionally, which
        // replaced invalid bytes with U+FFFD and silently made the
        // file unmatchable by any real glob — the same silent-skip
        // mode the rest of this module fights. The parser must now
        // collect invalid records in `lossy_names` so callers can
        // surface a warning and the user sees *which* file was
        // dropped from the change set.
        //
        // 0xC3 0x28 is a two-byte sequence with a valid lead byte
        // (0xC3 starts a 2-byte sequence) followed by an invalid
        // continuation byte (0x28 is ASCII, not 10xxxxxx), so it
        // fails `str::from_utf8` but is a shape git can legitimately
        // emit on Unix.
        let raw: Vec<u8> = b"clean.txt\0bad\xC3\x28name.rs\0also_clean.rs\0".to_vec();
        let parsed = parse_nul_separated(&raw);
        // All three records must still appear in `files` so the
        // change-set cardinality is honest — we don't drop the bad
        // one, we just flag it so the host can explain the non-match.
        assert_eq!(parsed.files.len(), 3);
        assert_eq!(parsed.files[0], "clean.txt");
        assert_eq!(parsed.files[2], "also_clean.rs");
        // Exactly one record must be flagged as lossy, and it must
        // be the invalid one (not the clean ones).
        assert_eq!(parsed.lossy_names.len(), 1);
        assert!(
            parsed.lossy_names[0].contains("name.rs"),
            "lossy form should retain the tail bytes, got {:?}",
            parsed.lossy_names
        );
        assert!(
            !parsed.lossy_names.iter().any(|s| s == "clean.txt"),
            "clean records must not appear in lossy_names"
        );
    }

    #[tokio::test]
    async fn get_changed_files_handles_filename_with_newline() {
        // End-to-end coverage for the `-z` switch: create a real file
        // whose name contains a newline, commit to a baseline, modify
        // it, and assert the changed set contains the file as a
        // single entry. Without `-z`, this test would either fail to
        // locate the file at all (newline splits the record) or see
        // two bogus half-entries.
        if !git_available() {
            return;
        }
        // Skip on Windows where `\n` is not a legal filename byte —
        // the feature is Unix-only anyway.
        if cfg!(windows) {
            return;
        }
        let tmp = TempDir::new().expect("tempdir");
        let repo: PathBuf = tmp.path().to_path_buf();
        init_repo(&repo);
        // Commit a placeholder so HEAD exists.
        commit_file(&repo, "seed.txt", "seed");

        let weird_name = "weird\nfile.txt";
        std::fs::write(repo.join(weird_name), "a").expect("write");
        StdCommand::new("git")
            .args(["-C", repo.to_str().unwrap(), "add", "--", weird_name])
            .status()
            .expect("git add");
        StdCommand::new("git")
            .args([
                "-C",
                repo.to_str().unwrap(),
                "commit",
                "-m",
                "add weird",
                "--no-gpg-sign",
            ])
            .status()
            .expect("git commit");

        // Modify the tracked weird-named file; diff vs HEAD~1 must
        // report it as exactly one changed entry.
        std::fs::write(repo.join(weird_name), "b").expect("write");

        let files = get_changed_files("HEAD~1", Some(&repo))
            .await
            .expect("get_changed_files");
        assert!(
            files.iter().any(|f| f == weird_name),
            "file with newline in name must appear as a single entry, got {:?}",
            files
        );
        // And it must NOT appear split into the two pseudo-entries
        // the old newline-splitting parser would have produced.
        assert!(
            !files.iter().any(|f| f == "weird"),
            "newline in name must not cause a bogus split, got {:?}",
            files
        );
    }

    #[test]
    fn head_mtime_follows_linked_worktree_gitdir_pointer() {
        // Regression pin: linked worktrees have `.git` as a regular
        // file whose contents are `gitdir: /abs/path/to/real/HEAD`.
        // The old implementation stat'd `.git/HEAD` directly and
        // returned None on worktrees, which meant the watcher's
        // git-state cache served stale `(branch, tag)` pairs until
        // the bare TTL expired. We simulate the layout by creating a
        // fake worktree root whose `.git` is a file pointing at a
        // real gitdir in a sibling temp dir. `head_mtime` must
        // resolve the pointer and return the mtime of the real HEAD.
        let tmp = TempDir::new().expect("tempdir");
        let worktree = tmp.path().join("worktree");
        let real_gitdir = tmp.path().join("realgitdir");
        std::fs::create_dir_all(&worktree).unwrap();
        std::fs::create_dir_all(&real_gitdir).unwrap();
        // Create a HEAD file inside the real gitdir.
        let real_head = real_gitdir.join("HEAD");
        std::fs::write(&real_head, "ref: refs/heads/main\n").unwrap();
        // Create the worktree's `.git` pointer file. Use an absolute
        // gitdir path so the resolution doesn't depend on CWD.
        let pointer = format!("gitdir: {}\n", real_gitdir.display());
        std::fs::write(worktree.join(".git"), pointer).unwrap();

        let mtime = head_mtime(Some(&worktree));
        assert!(
            mtime.is_some(),
            "head_mtime must follow the worktree .git gitdir pointer"
        );
        // Rewrite real HEAD to bump its mtime, then verify the
        // function sees the change (proves it's really reading the
        // pointed-at file and not some stale path).
        std::thread::sleep(std::time::Duration::from_millis(20));
        std::fs::write(&real_head, "ref: refs/heads/other\n").unwrap();
        let mtime_after = head_mtime(Some(&worktree));
        assert!(mtime_after.is_some());
        assert!(
            mtime_after.unwrap() >= mtime.unwrap(),
            "rewriting the real HEAD must be visible through the worktree pointer"
        );
    }

    #[test]
    fn head_mtime_returns_none_when_no_dotgit() {
        let tmp = TempDir::new().expect("tempdir");
        assert!(
            head_mtime(Some(tmp.path())).is_none(),
            "a directory with no .git entry at all must return None, not a spurious mtime"
        );
    }
}
