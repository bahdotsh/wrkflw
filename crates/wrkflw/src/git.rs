//! Small reads of the repository wrkflw runs in, shared by `gate` and
//! `compare`.

use std::process::Command;

/// One line of `git` output, or `None` when git fails or prints nothing.
pub(crate) fn line(args: &[&str]) -> Option<String> {
    let output = command(args).output().ok()?;
    let line = String::from_utf8_lossy(&output.stdout).trim().to_string();
    (output.status.success() && !line.is_empty()).then_some(line)
}

/// Whether `remote` has `reference`, or why that couldn't be found out,
/// as when the network or the credentials fail. Not having it and not
/// being able to ask are different answers.
pub(crate) fn remote_has(remote: &str, reference: &str) -> Result<bool, String> {
    // A remote that doesn't answer isn't waited for: ssh gives up on
    // connecting, and HTTPS on a transfer that stalls. Neither covers a
    // name lookup or a connection that hangs, nor ssh run as the user set
    // it up, so the whole call has a deadline too.
    let mut ask = command(&[
        "-c",
        "http.lowSpeedLimit=1",
        "-c",
        "http.lowSpeedTime=20",
        "ls-remote",
        "--exit-code",
        remote,
        reference,
    ]);
    let output = output_within(&mut ask, REMOTE_DEADLINE)
        .map_err(|e| format!("couldn't ask {remote} about {reference}: {e}"))?;
    match output.status.code() {
        Some(0) => Ok(true),
        // `--exit-code`: the remote answered and has no such ref.
        Some(2) => Ok(false),
        _ => {
            let stderr = String::from_utf8_lossy(&output.stderr);
            let why = stderr
                .lines()
                .map(str::trim)
                .rfind(|line| !line.is_empty())
                .unwrap_or("git ls-remote failed");
            Err(format!("couldn't ask {remote} about {reference}: {why}"))
        }
    }
}

/// How long a question to a remote may take in all.
const REMOTE_DEADLINE: std::time::Duration = std::time::Duration::from_secs(30);

/// Run `command` to its end, or kill it once `deadline` has passed. Its
/// output is a ref or two and an error line, which a pipe holds while it
/// runs.
fn output_within(
    command: &mut Command,
    deadline: std::time::Duration,
) -> Result<std::process::Output, String> {
    let mut child = command
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::piped())
        .spawn()
        .map_err(|e| format!("cannot run git: {e}"))?;
    let started = std::time::Instant::now();
    loop {
        match child.try_wait() {
            Ok(Some(_)) => return child.wait_with_output().map_err(|e| e.to_string()),
            Ok(None) if started.elapsed() >= deadline => {
                let _ = child.kill();
                let _ = child.wait();
                return Err(format!("no answer within {} s", deadline.as_secs()));
            }
            Ok(None) => std::thread::sleep(std::time::Duration::from_millis(50)),
            Err(e) => return Err(e.to_string()),
        }
    }
}

/// `git args`, set up never to wait for a person or a silent remote.
fn command(args: &[&str]) -> Command {
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
        command.env(
            "GIT_SSH_COMMAND",
            "ssh -o BatchMode=yes -o ConnectTimeout=20",
        );
    }
    command
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

    #[cfg(unix)]
    #[test]
    fn a_command_that_does_not_answer_is_given_up_on() {
        let started = std::time::Instant::now();
        let mut slow = Command::new("sleep");
        slow.arg("10");
        let err = output_within(&mut slow, std::time::Duration::from_millis(300)).unwrap_err();
        assert!(err.contains("no answer"), "{err}");
        assert!(started.elapsed() < std::time::Duration::from_secs(5));

        let mut quick = Command::new("sh");
        quick.args(["-c", "echo out; echo err >&2; exit 2"]);
        let output = output_within(&mut quick, std::time::Duration::from_secs(10)).unwrap();
        assert_eq!(output.status.code(), Some(2));
        assert_eq!(String::from_utf8_lossy(&output.stdout), "out\n");
        assert_eq!(String::from_utf8_lossy(&output.stderr), "err\n");
    }
}
