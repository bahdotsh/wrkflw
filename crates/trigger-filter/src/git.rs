use crate::error::TriggerFilterError;
use std::process::Command as StdCommand;
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
    diff_stdout: &[u8],
    untracked_stdout: Option<&[u8]>,
) -> Vec<String> {
    let mut files = parse_lines(diff_stdout);

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
/// Combines:
/// - `git diff --name-only <base>` (staged + unstaged changes vs base)
/// - `git ls-files --others --exclude-standard` (untracked files)
pub async fn get_changed_files(base: &str) -> Result<Vec<String>, TriggerFilterError> {
    let diff_output = Command::new("git")
        .args(["diff", "--name-only", base])
        .output()
        .await
        .map_err(|e| TriggerFilterError::GitError(format!("Failed to run git diff: {}", e)))?;

    if !diff_output.status.success() {
        let stderr = String::from_utf8_lossy(&diff_output.stderr);
        return Err(TriggerFilterError::GitError(format!(
            "git diff failed: {}",
            stderr.trim()
        )));
    }

    let untracked_output = Command::new("git")
        .args(["ls-files", "--others", "--exclude-standard"])
        .output()
        .await
        .map_err(|e| TriggerFilterError::GitError(format!("Failed to run git ls-files: {}", e)))?;

    let untracked = if untracked_output.status.success() {
        Some(untracked_output.stdout.as_slice())
    } else {
        None
    };

    Ok(collect_changed_files_from_outputs(
        &diff_output.stdout,
        untracked,
    ))
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

// ===========================================================================
// Synchronous API — for use in blocking contexts (e.g. background threads)
// ===========================================================================

/// Synchronous version of [`get_changed_files`].
pub fn get_changed_files_sync(base: &str) -> Result<Vec<String>, TriggerFilterError> {
    let diff_output = StdCommand::new("git")
        .args(["diff", "--name-only", base])
        .output()
        .map_err(|e| TriggerFilterError::GitError(format!("Failed to run git diff: {}", e)))?;

    if !diff_output.status.success() {
        let stderr = String::from_utf8_lossy(&diff_output.stderr);
        return Err(TriggerFilterError::GitError(format!(
            "git diff failed: {}",
            stderr.trim()
        )));
    }

    let untracked_output = StdCommand::new("git")
        .args(["ls-files", "--others", "--exclude-standard"])
        .output()
        .map_err(|e| TriggerFilterError::GitError(format!("Failed to run git ls-files: {}", e)))?;

    let untracked = if untracked_output.status.success() {
        Some(untracked_output.stdout.as_slice())
    } else {
        None
    };

    Ok(collect_changed_files_from_outputs(
        &diff_output.stdout,
        untracked,
    ))
}

/// Synchronous version of [`get_current_branch`].
pub fn get_current_branch_sync() -> Result<String, TriggerFilterError> {
    let output = StdCommand::new("git")
        .args(["rev-parse", "--abbrev-ref", "HEAD"])
        .output()
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

/// Synchronous version of [`get_current_tag`].
pub fn get_current_tag_sync() -> Result<Option<String>, TriggerFilterError> {
    let output = StdCommand::new("git")
        .args(["describe", "--tags", "--exact-match", "HEAD"])
        .output()
        .map_err(|e| TriggerFilterError::GitError(format!("Failed to check tags: {}", e)))?;

    if output.status.success() {
        let tag = String::from_utf8_lossy(&output.stdout).trim().to_string();
        if tag.is_empty() {
            Ok(None)
        } else {
            Ok(Some(tag))
        }
    } else {
        Ok(None)
    }
}
