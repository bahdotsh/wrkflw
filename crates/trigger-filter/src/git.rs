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

/// Get the current branch name.
pub async fn get_current_branch(cwd: Option<&Path>) -> Result<String, TriggerFilterError> {
    let mut cmd = git_cmd(cwd);
    cmd.args(["rev-parse", "--abbrev-ref", "HEAD"]);
    let output = check_status(run_git(cmd, "git rev-parse").await?, "git rev-parse")?;
    Ok(String::from_utf8_lossy(&output.stdout).trim().to_string())
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
