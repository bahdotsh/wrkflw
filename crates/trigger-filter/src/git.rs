use crate::error::TriggerFilterError;
use std::path::Path;
use tokio::process::Command;

/// Git's empty-tree object SHA. Diffing against it shows every tracked file
/// as added — used as a last-resort fallback for repositories where no other
/// reasonable diff base can be detected (e.g. a fresh repo with one commit).
const GIT_EMPTY_TREE_SHA: &str = "4b825dc642cb6eb9a060e54bf899d69f82e4f2d1";

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
/// When `base == "HEAD"`, we run two diffs (unstaged and staged) plus an
/// untracked-files probe. For any other base, `git diff <base>` already
/// covers staged + unstaged + committed-on-branch changes, so the `--cached`
/// pass is redundant and we omit it.
///
/// `cwd` selects the git working directory; pass `None` to use the
/// process CWD. The watcher should always pass its repo root.
pub async fn get_changed_files(
    base: &str,
    cwd: Option<&Path>,
) -> Result<Vec<String>, TriggerFilterError> {
    validate_ref_name(base)?;

    let untracked_fut = git_cmd(cwd)
        .args(["ls-files", "--others", "--exclude-standard"])
        .output();

    if base == "HEAD" {
        let diff_fut = git_cmd(cwd).args(["diff", "--name-only", base]).output();
        let cached_fut = git_cmd(cwd)
            .args(["diff", "--cached", "--name-only", base])
            .output();

        let (diff_result, cached_result, untracked_result) =
            tokio::join!(diff_fut, cached_fut, untracked_fut);

        let diff_output = check_status(diff_result, "git diff")?;
        let mut files = parse_lines(&diff_output.stdout);

        if let Ok(cached_output) = cached_result {
            if cached_output.status.success() {
                files = merge_unique(files, parse_lines(&cached_output.stdout));
            }
        }

        if let Ok(untracked_output) = untracked_result {
            if untracked_output.status.success() {
                files = merge_unique(files, parse_lines(&untracked_output.stdout));
            }
        }

        Ok(files)
    } else {
        let diff_fut = git_cmd(cwd).args(["diff", "--name-only", base]).output();
        let (diff_result, untracked_result) = tokio::join!(diff_fut, untracked_fut);

        let diff_output = check_status(diff_result, "git diff")?;
        let mut files = parse_lines(&diff_output.stdout);

        if let Ok(untracked_output) = untracked_result {
            if untracked_output.status.success() {
                files = merge_unique(files, parse_lines(&untracked_output.stdout));
            }
        }

        Ok(files)
    }
}

fn check_status(
    result: Result<std::process::Output, std::io::Error>,
    cmd_label: &str,
) -> Result<std::process::Output, TriggerFilterError> {
    let output = result
        .map_err(|e| TriggerFilterError::GitError(format!("Failed to run {}: {}", cmd_label, e)))?;
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
    let result = git_cmd(cwd)
        .args(["diff", "--name-only", &range])
        .output()
        .await;
    let output = check_status(result, "git diff")?;
    Ok(parse_lines(&output.stdout))
}

/// Get the current branch name.
pub async fn get_current_branch(cwd: Option<&Path>) -> Result<String, TriggerFilterError> {
    let result = git_cmd(cwd)
        .args(["rev-parse", "--abbrev-ref", "HEAD"])
        .output()
        .await;
    let output = check_status(result, "git rev-parse")?;
    Ok(String::from_utf8_lossy(&output.stdout).trim().to_string())
}

/// Determine a sensible diff base for trigger evaluation.
///
/// Strategy:
/// 1. If there are uncommitted changes (vs HEAD), use "HEAD".
/// 2. Detect the remote default branch via `git symbolic-ref refs/remotes/origin/HEAD`.
/// 3. Fall back to trying `main`, then `master`.
/// 4. Otherwise try `HEAD~1`.
/// 5. Last resort: the empty-tree SHA, which makes every tracked file appear
///    as changed. A warning is logged because this is rarely what the user wants.
pub async fn get_default_diff_base(cwd: Option<&Path>) -> String {
    // Check for uncommitted changes first
    if let Ok(output) = git_cmd(cwd).args(["status", "--porcelain"]).output().await {
        let stdout = String::from_utf8_lossy(&output.stdout);
        if !stdout.trim().is_empty() {
            return "HEAD".to_string();
        }
    }

    // Build candidate list: detected default branch first, then common fallbacks
    let mut candidates: Vec<String> = Vec::new();

    // Try to detect the remote default branch
    if let Ok(output) = git_cmd(cwd)
        .args(["symbolic-ref", "refs/remotes/origin/HEAD", "--short"])
        .output()
        .await
    {
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
        if let Ok(output) = git_cmd(cwd)
            .args(["merge-base", "HEAD", base_branch])
            .output()
            .await
        {
            if output.status.success() {
                let mb = String::from_utf8_lossy(&output.stdout).trim().to_string();
                if !mb.is_empty() {
                    return mb;
                }
            }
        }
    }

    // Try HEAD~1, which works on any repo with at least two commits
    if let Ok(output) = git_cmd(cwd)
        .args(["rev-parse", "--verify", "HEAD~1"])
        .output()
        .await
    {
        if output.status.success() {
            return "HEAD~1".to_string();
        }
    }

    // Ultimate fallback: empty-tree SHA. This compares against an empty tree,
    // so every tracked file appears as added. Warn loudly because that's
    // almost certainly not what the user wants.
    wrkflw_logging::warning(
        "Could not detect a sensible diff base (no remote default, no HEAD~1). \
         Falling back to the empty tree, which treats every file as changed. \
         Pass --diff-base explicitly to override.",
    );
    GIT_EMPTY_TREE_SHA.to_string()
}

/// Get the current tag if HEAD is tagged, or None.
pub async fn get_current_tag(cwd: Option<&Path>) -> Result<Option<String>, TriggerFilterError> {
    let result = git_cmd(cwd)
        .args(["describe", "--tags", "--exact-match", "HEAD"])
        .output()
        .await;
    match result {
        Ok(output) if output.status.success() => {
            let tag = String::from_utf8_lossy(&output.stdout).trim().to_string();
            if tag.is_empty() {
                Ok(None)
            } else {
                Ok(Some(tag))
            }
        }
        Ok(_) => Ok(None), // not on a tag is normal
        Err(e) => Err(TriggerFilterError::GitError(format!(
            "Failed to check tags: {}",
            e
        ))),
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
