//! Small reads of the repository wrkflw runs in, shared by `gate` and
//! `compare`.

use std::process::Command;

/// One line of `git` output, or `None` when git fails or prints nothing.
pub(crate) fn line(args: &[&str]) -> Option<String> {
    // Some of these ask a remote. One that wants credentials fails instead
    // of stopping the command at a password prompt nobody may be watching.
    let mut command = Command::new("git");
    command
        .args(args)
        .env("GIT_TERMINAL_PROMPT", "0")
        .stdin(std::process::Stdio::null());
    // GIT_TERMINAL_PROMPT covers HTTPS. ssh asks for a key's passphrase or
    // a new host key on the terminal itself, so it is told not to ask,
    // unless the user chose how git runs ssh.
    if batch_ssh_wanted(
        |name| std::env::var_os(name).is_some(),
        ssh_command_configured,
    ) {
        command.env("GIT_SSH_COMMAND", "ssh -o BatchMode=yes");
    }
    let output = command.output().ok()?;
    let line = String::from_utf8_lossy(&output.stdout).trim().to_string();
    (output.status.success() && !line.is_empty()).then_some(line)
}

/// Whether git should be given an ssh that never prompts: only when neither
/// the environment nor git's config already says how to run ssh.
fn batch_ssh_wanted(env_set: impl Fn(&str) -> bool, configured: impl Fn() -> bool) -> bool {
    !env_set("GIT_SSH_COMMAND") && !env_set("GIT_SSH") && !configured()
}

fn ssh_command_configured() -> bool {
    static CONFIGURED: std::sync::OnceLock<bool> = std::sync::OnceLock::new();
    *CONFIGURED.get_or_init(|| {
        Command::new("git")
            .args(["config", "--get", "core.sshCommand"])
            .stdin(std::process::Stdio::null())
            .output()
            .is_ok_and(|output| output.status.success() && !output.stdout.trim_ascii().is_empty())
    })
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ssh_is_told_not_to_prompt_unless_the_user_chose_how_it_runs() {
        assert!(batch_ssh_wanted(|_| false, || false));
        assert!(!batch_ssh_wanted(
            |name| name == "GIT_SSH_COMMAND",
            || false
        ));
        assert!(!batch_ssh_wanted(|name| name == "GIT_SSH", || false));
        assert!(!batch_ssh_wanted(|_| false, || true));
    }
}
