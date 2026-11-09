//! Small reads of the repository wrkflw runs in, shared by `gate` and
//! `compare`.

use std::process::Command;

/// One line of `git` output, or `None` when git fails or prints nothing.
pub(crate) fn line(args: &[&str]) -> Option<String> {
    // Some of these ask a remote. One that wants credentials fails instead
    // of stopping the command at a password prompt nobody may be watching.
    let output = Command::new("git")
        .args(args)
        .env("GIT_TERMINAL_PROMPT", "0")
        .stdin(std::process::Stdio::null())
        .output()
        .ok()?;
    let line = String::from_utf8_lossy(&output.stdout).trim().to_string();
    (output.status.success() && !line.is_empty()).then_some(line)
}

/// The branch checked out, or `None` on a detached HEAD.
pub(crate) fn current_branch() -> Option<String> {
    line(&["symbolic-ref", "--short", "-q", "HEAD"])
}

/// The branch pull requests go into: what `origin/HEAD` points at, else
/// `main` or `master` if one exists here or on `origin`.
pub(crate) fn default_branch() -> Option<String> {
    if let Some(remote_head) = line(&["symbolic-ref", "--short", "-q", "refs/remotes/origin/HEAD"])
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
            .any(|reference| line(&["rev-parse", "--verify", "-q", reference]).is_some())
        })
}
