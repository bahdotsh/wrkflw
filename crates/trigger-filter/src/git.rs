use crate::error::TriggerFilterError;
use tokio::process::Command;

// ---------------------------------------------------------------------------
// Shared helpers (operate on already-captured output)
// ---------------------------------------------------------------------------

fn parse_lines(output: &[u8]) -> Vec<String> {
    String::from_utf8_lossy(output)
        .lines()
        .map(|l| l.trim().to_string())
        .filter(|l| !l.is_empty())
        .collect()
}

fn collect_changed_files_from_outputs(
    diff_files: &[String],
    untracked_stdout: Option<&[u8]>,
) -> Vec<String> {
    let mut files = diff_files.to_vec();

    if let Some(untracked) = untracked_stdout {
        let seen: std::collections::HashSet<String> = files.iter().cloned().collect();
        for line in String::from_utf8_lossy(untracked).lines() {
            let trimmed = line.trim();
            if !trimmed.is_empty() && !seen.contains(trimmed) {
                files.push(trimmed.to_string());
            }
        }
    }

    files
}

// ===========================================================================
// Async API
// ===========================================================================

/// Get changed files between the working tree and a base ref.
///
/// Combines (in parallel):
/// - `git diff --name-only <base>` (unstaged changes vs base)
/// - `git diff --cached --name-only <base>` (staged changes vs base)
/// - `git ls-files --others --exclude-standard` (untracked files)
pub async fn get_changed_files(base: &str) -> Result<Vec<String>, TriggerFilterError> {
    let diff_fut = Command::new("git")
        .args(["diff", "--name-only", base])
        .output();

    let cached_fut = Command::new("git")
        .args(["diff", "--cached", "--name-only", base])
        .output();

    let untracked_fut = Command::new("git")
        .args(["ls-files", "--others", "--exclude-standard"])
        .output();

    let (diff_result, cached_result, untracked_result) =
        tokio::join!(diff_fut, cached_fut, untracked_fut);

    let diff_output = diff_result
        .map_err(|e| TriggerFilterError::GitError(format!("Failed to run git diff: {}", e)))?;

    if !diff_output.status.success() {
        let stderr = String::from_utf8_lossy(&diff_output.stderr);
        return Err(TriggerFilterError::GitError(format!(
            "git diff failed: {}",
            stderr.trim()
        )));
    }

    let cached_output = cached_result.map_err(|e| {
        TriggerFilterError::GitError(format!("Failed to run git diff --cached: {}", e))
    })?;

    // Merge unstaged and staged diffs
    let mut files = parse_lines(&diff_output.stdout);
    if cached_output.status.success() {
        let seen: std::collections::HashSet<String> = files.iter().cloned().collect();
        for line in parse_lines(&cached_output.stdout) {
            if !seen.contains(&line) {
                files.push(line);
            }
        }
    }

    let untracked_output = untracked_result
        .map_err(|e| TriggerFilterError::GitError(format!("Failed to run git ls-files: {}", e)))?;

    let untracked = if untracked_output.status.success() {
        Some(untracked_output.stdout.as_slice())
    } else {
        None
    };

    Ok(collect_changed_files_from_outputs(&files, untracked))
}

/// Get changed files between two refs.
pub async fn get_changed_files_between(
    base_ref: &str,
    head_ref: &str,
) -> Result<Vec<String>, TriggerFilterError> {
    let range = format!("{}..{}", base_ref, head_ref);
    let output = Command::new("git")
        .args(["diff", "--name-only", &range])
        .output()
        .await
        .map_err(|e| TriggerFilterError::GitError(format!("Failed to run git diff: {}", e)))?;

    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        return Err(TriggerFilterError::GitError(format!(
            "git diff failed: {}",
            stderr.trim()
        )));
    }

    Ok(parse_lines(&output.stdout))
}

/// Get the current branch name.
pub async fn get_current_branch() -> Result<String, TriggerFilterError> {
    let output = Command::new("git")
        .args(["rev-parse", "--abbrev-ref", "HEAD"])
        .output()
        .await
        .map_err(|e| {
            TriggerFilterError::GitError(format!("Failed to get current branch: {}", e))
        })?;

    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        return Err(TriggerFilterError::GitError(format!(
            "git rev-parse failed: {}",
            stderr.trim()
        )));
    }

    Ok(String::from_utf8_lossy(&output.stdout).trim().to_string())
}

/// Determine a sensible diff base for trigger evaluation.
///
/// Strategy:
/// 1. If there are uncommitted changes (vs HEAD), use "HEAD".
/// 2. Detect the remote default branch via `git symbolic-ref refs/remotes/origin/HEAD`.
/// 3. Fall back to trying `main`, then `master`.
/// 4. Falls back to "HEAD~1" if no merge-base is found.
pub async fn get_default_diff_base() -> String {
    // Check for uncommitted changes first
    if let Ok(output) = Command::new("git")
        .args(["status", "--porcelain"])
        .output()
        .await
    {
        let stdout = String::from_utf8_lossy(&output.stdout);
        if !stdout.trim().is_empty() {
            return "HEAD".to_string();
        }
    }

    // Build candidate list: detected default branch first, then common fallbacks
    let mut candidates: Vec<String> = Vec::new();

    // Try to detect the remote default branch
    if let Ok(output) = Command::new("git")
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
        if let Ok(output) = Command::new("git")
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

    // Try HEAD~1, but it fails on repos with only one commit (no parent)
    if let Ok(output) = Command::new("git")
        .args(["rev-parse", "--verify", "HEAD~1"])
        .output()
        .await
    {
        if output.status.success() {
            return "HEAD~1".to_string();
        }
    }

    // Ultimate fallback: the git empty tree SHA.
    // This compares against a completely empty tree, so all files appear as "changed".
    "4b825dc642cb6eb9a060e54bf899d69f82e4f2d1".to_string()
}

/// Get the current tag if HEAD is tagged, or None.
pub async fn get_current_tag() -> Result<Option<String>, TriggerFilterError> {
    let output = Command::new("git")
        .args(["describe", "--tags", "--exact-match", "HEAD"])
        .output()
        .await
        .map_err(|e| TriggerFilterError::GitError(format!("Failed to check tags: {}", e)))?;

    if output.status.success() {
        let tag = String::from_utf8_lossy(&output.stdout).trim().to_string();
        if tag.is_empty() {
            Ok(None)
        } else {
            Ok(Some(tag))
        }
    } else {
        // Not on a tag — this is normal, not an error
        Ok(None)
    }
}
