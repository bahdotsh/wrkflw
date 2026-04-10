use crate::error::TriggerFilterError;
use std::path::Path;
use std::time::Duration;
use tokio::process::Command;

/// Hard upper bound on every git subprocess call. A git invocation that hasn't
/// completed within this window is almost certainly stuck on a network
/// filesystem, a hung credential prompt, or a corrupt repository — we'd rather
/// surface a clear error than wedge the watch loop forever.
const GIT_COMMAND_TIMEOUT: Duration = Duration::from_secs(10);

/// Build a `git` command optionally rooted at a working directory via `-C`.
fn git_cmd(cwd: Option<&Path>) -> Command {
    let mut cmd = Command::new("git");
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

fn parse_lines(output: &[u8]) -> Vec<String> {
    String::from_utf8_lossy(output)
        .lines()
        .map(|l| l.trim().to_string())
        .filter(|l| !l.is_empty())
        .collect()
}

fn merge_unique(mut into: Vec<String>, more: Vec<String>) -> Vec<String> {
    let seen: std::collections::HashSet<String> = into.iter().cloned().collect();
    for line in more {
        if !seen.contains(&line) {
            into.push(line);
        }
    }
    into
}

/// Get changed files between the working tree and a base ref.
///
/// `git diff <base>` already covers staged + unstaged + committed-on-branch
/// changes (the index is compared transitively through the working tree), so
/// one `git diff` plus an untracked-files probe is enough — no separate
/// `--cached` pass is needed.
///
/// `cwd` selects the git working directory; pass `None` to use the
/// process CWD. The watcher should always pass its repo root.
pub async fn get_changed_files(
    base: &str,
    cwd: Option<&Path>,
) -> Result<Vec<String>, TriggerFilterError> {
    validate_ref_name(base)?;

    let mut diff_cmd = git_cmd(cwd);
    diff_cmd.args(["diff", "--name-only", base]);
    let mut untracked_cmd = git_cmd(cwd);
    untracked_cmd.args(["ls-files", "--others", "--exclude-standard"]);

    let (diff_res, untracked_res) = tokio::join!(
        run_git(diff_cmd, "git diff"),
        run_git(untracked_cmd, "git ls-files"),
    );

    let diff_output = check_status(diff_res?, "git diff")?;
    let mut files = parse_lines(&diff_output.stdout);

    // Untracked files are a best-effort enrichment; don't fail the whole
    // call if `ls-files` errors (e.g. outside a repo).
    if let Ok(untracked_output) = untracked_res {
        if untracked_output.status.success() {
            files = merge_unique(files, parse_lines(&untracked_output.stdout));
        }
    }

    Ok(files)
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
pub async fn get_changed_files_between(
    base_ref: &str,
    head_ref: &str,
    cwd: Option<&Path>,
) -> Result<Vec<String>, TriggerFilterError> {
    validate_ref_name(base_ref)?;
    validate_ref_name(head_ref)?;

    let range = format!("{}..{}", base_ref, head_ref);
    let mut cmd = git_cmd(cwd);
    cmd.args(["diff", "--name-only", &range]);
    let output = check_status(run_git(cmd, "git diff").await?, "git diff")?;
    Ok(parse_lines(&output.stdout))
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
pub async fn get_default_diff_base(cwd: Option<&Path>) -> Result<String, TriggerFilterError> {
    // Check for uncommitted changes first
    let mut status_cmd = git_cmd(cwd);
    status_cmd.args(["status", "--porcelain"]);
    if let Ok(output) = run_git(status_cmd, "git status").await {
        let stdout = String::from_utf8_lossy(&output.stdout);
        if !stdout.trim().is_empty() {
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
            // symbolic-ref returns e.g. "origin/main" — strip the remote prefix
            let short = branch
                .strip_prefix("origin/")
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

/// Find the git repository root from the current working directory by
/// shelling out to `git rev-parse --show-toplevel`.
///
/// Returns `None` if `git` is unavailable, the call fails, or the
/// process is not inside a git working tree. This is the right anchor
/// for any consumer that wants to run subsequent git operations against
/// "the repo the user is in" — passing it as `cwd` to the other helpers
/// in this module makes their behavior independent of the process CWD.
///
/// Synchronous on purpose: callers tend to invoke this once at startup,
/// and the dependency on tokio for a single subprocess call would force
/// every consumer (including the synchronous TUI bootstrap) to create a
/// runtime just to discover a path.
pub fn find_repo_root() -> Option<std::path::PathBuf> {
    let output = std::process::Command::new("git")
        .args(["rev-parse", "--show-toplevel"])
        .output()
        .ok()?;

    if output.status.success() {
        let path = String::from_utf8_lossy(&output.stdout).trim().to_string();
        if path.is_empty() {
            None
        } else {
            Some(std::path::PathBuf::from(path))
        }
    } else {
        None
    }
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

        let base = get_default_diff_base(Some(&repo)).await.expect("diff base");
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

        let err = get_default_diff_base(Some(&repo)).await;
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
    fn validate_ref_accepts_reflog_and_upstream_syntax() {
        assert!(validate_ref_name("HEAD@{1}").is_ok());
        assert!(validate_ref_name("origin/main@{upstream}").is_ok());
        assert!(validate_ref_name("@").is_ok());
        assert!(validate_ref_name("@~1").is_ok());
    }
}
