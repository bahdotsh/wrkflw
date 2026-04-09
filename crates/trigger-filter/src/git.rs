use crate::error::TriggerFilterError;
use tokio::process::Command;

/// Get changed files between the working tree and a base ref.
///
/// Combines:
/// - `git diff --name-only <base>` (staged + unstaged changes vs base)
/// - `git ls-files --others --exclude-standard` (untracked files)
pub async fn get_changed_files(base: &str) -> Result<Vec<String>, TriggerFilterError> {
    let mut files = Vec::new();

    // Get modified/deleted files relative to base
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

    let diff_str = String::from_utf8_lossy(&diff_output.stdout);
    for line in diff_str.lines() {
        let trimmed = line.trim();
        if !trimmed.is_empty() {
            files.push(trimmed.to_string());
        }
    }

    // Get untracked files
    let untracked_output = Command::new("git")
        .args(["ls-files", "--others", "--exclude-standard"])
        .output()
        .await
        .map_err(|e| TriggerFilterError::GitError(format!("Failed to run git ls-files: {}", e)))?;

    if untracked_output.status.success() {
        let untracked_str = String::from_utf8_lossy(&untracked_output.stdout);
        for line in untracked_str.lines() {
            let trimmed = line.trim();
            if !trimmed.is_empty() && !files.contains(&trimmed.to_string()) {
                files.push(trimmed.to_string());
            }
        }
    }

    Ok(files)
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

    let stdout = String::from_utf8_lossy(&output.stdout);
    Ok(stdout
        .lines()
        .map(|l| l.trim().to_string())
        .filter(|l| !l.is_empty())
        .collect())
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
