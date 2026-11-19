//! Process-wide switches for where a local run may depart from a hosted
//! runner to save time. Both shortcuts are on by default; the CLI flags and
//! the environment variables below turn them off.
//!
//! Kept out of `ExecutionConfig` for the same reason as the run hooks: the
//! setting applies to every run in the process, and it would otherwise have
//! to be threaded through every config literal.

use std::sync::atomic::{AtomicBool, Ordering};

static FRESH_WORKSPACES: AtomicBool = AtomicBool::new(false);
static EVERY_MATRIX_LEG: AtomicBool = AtomicBool::new(false);

/// Env var that gives every job a new, empty workspace, as a hosted runner does.
pub const FRESH_WORKSPACE_ENV: &str = "WRKFLW_FRESH_WORKSPACE";
/// Env var that runs matrix legs even when they would repeat another leg.
pub const EVERY_MATRIX_LEG_ENV: &str = "WRKFLW_EVERY_MATRIX_LEG";
/// Env var that gives each `run:` step a container of its own instead of
/// the job's one.
pub const STEP_CONTAINERS_ENV: &str = "WRKFLW_STEP_CONTAINERS";
/// Env var that starts a Docker engine in the container of a job whose
/// steps run `docker`, which then runs privileged.
pub const DOCKER_IN_JOBS_ENV: &str = "WRKFLW_DOCKER_IN_JOBS";

/// Env var that gives jobs this machine's GitHub token as `github.token`
/// and `secrets.GITHUB_TOKEN`, where no secret provider has one.
pub const JOB_GITHUB_TOKEN_ENV: &str = "WRKFLW_JOB_GITHUB_TOKEN";

/// The token jobs get as `github.token` and `secrets.GITHUB_TOKEN` when
/// [`JOB_GITHUB_TOKEN_ENV`] asks for it: `GITHUB_TOKEN` or `GH_TOKEN` from
/// the environment, else what `gh auth token` prints. `None` when the
/// switch is off or there is no token to give.
///
/// Off by default, as this is the user's own token, which reaches further
/// than the one GitHub makes for a run and hands to its steps: the job
/// warns of it when it gets it (see [`warn_of_own_token`]).
pub(crate) async fn job_github_token() -> Option<String> {
    if !job_github_token_wanted() {
        return None;
    }
    crate::acceleration::run_job_token(|| async {
        // `gh` only runs when the environment has no token.
        let mut token = job_github_token_from(|name| std::env::var(name).ok(), || None);
        if token.is_none() {
            let from_gh = gh_auth_token().await;
            token = job_github_token_from(|_| None, || from_gh);
        }
        if token.is_none() {
            wrkflw_logging::warning(
                "WRKFLW_JOB_GITHUB_TOKEN is set, but there is no token to give jobs: \
                 set GITHUB_TOKEN, or sign in with `gh auth login`",
            );
        }
        token
    })
    .await
}

/// The token a clone of a private repository is retried with: the
/// `GITHUB_TOKEN` a secret provider gave the job (`provider`), else
/// `GITHUB_TOKEN` or `GH_TOKEN` from the environment, else what
/// `gh auth token` prints; found once per run.
///
/// Unlike [`job_github_token`] this needs no switch: the token goes to
/// `git` on this machine only, to read what the user can already read,
/// and never into a job.
pub(crate) async fn remote_clone_token(provider: Option<&str>) -> Option<String> {
    if let Some(token) = provider.map(str::trim).filter(|t| !t.is_empty()) {
        return Some(token.to_string());
    }
    crate::acceleration::run_clone_token(|| async {
        let token = job_github_token_from(|name| std::env::var(name).ok(), || None);
        match token {
            Some(token) => Some(token),
            None => {
                let from_gh = gh_auth_token().await;
                job_github_token_from(|_| None, || from_gh)
            }
        }
    })
    .await
}

/// Say, once per run, that its jobs get the user's own GitHub token.
pub(crate) fn warn_of_own_token() {
    if crate::acceleration::first_own_token_warning() {
        wrkflw_logging::warning(
            "Jobs get your own GitHub token, from this machine's environment or `gh`, as \
             github.token and secrets.GITHUB_TOKEN (WRKFLW_JOB_GITHUB_TOKEN): it can do \
             what you can, not just what a run's token can",
        );
    }
}

/// Whether the run asked for jobs to get a GitHub token, from a secret
/// provider or, failing that, this machine: see [`job_github_token`].
pub(crate) fn job_github_token_wanted() -> bool {
    env_flag(JOB_GITHUB_TOKEN_ENV)
}

/// How long `gh auth token` may take: it reads a file or the keychain,
/// and one waiting on a locked keychain mustn't hold up the run.
const GH_AUTH_TOKEN_TIMEOUT: std::time::Duration = std::time::Duration::from_secs(10);

/// What `gh auth token` prints, or `None` when it fails, isn't installed or
/// takes longer than [`GH_AUTH_TOKEN_TIMEOUT`].
async fn gh_auth_token() -> Option<String> {
    let output = tokio::process::Command::new("gh")
        .args(["auth", "token"])
        .stdin(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .kill_on_drop(true)
        .output();
    let output = tokio::time::timeout(GH_AUTH_TOKEN_TIMEOUT, output)
        .await
        .ok()?
        .ok()?;
    output
        .status
        .success()
        .then(|| String::from_utf8_lossy(&output.stdout).into_owned())
}

/// [`job_github_token`] with `var` reading an environment variable and
/// `gh` asking the GitHub CLI for its token.
fn job_github_token_from(
    var: impl Fn(&str) -> Option<String>,
    gh: impl FnOnce() -> Option<String>,
) -> Option<String> {
    let usable = |token: String| {
        let token = token.trim().to_string();
        (!token.is_empty()).then_some(token)
    };
    ["GITHUB_TOKEN", "GH_TOKEN"]
        .into_iter()
        .find_map(|name| var(name).and_then(usable))
        .or_else(|| gh().and_then(usable))
}

/// Give every job a new, empty workspace instead of the one it kept from
/// its last run.
pub fn set_fresh_workspaces(on: bool) {
    FRESH_WORKSPACES.store(on, Ordering::Relaxed);
}

pub fn fresh_workspaces() -> bool {
    FRESH_WORKSPACES.load(Ordering::Relaxed) || env_flag(FRESH_WORKSPACE_ENV)
}

/// Run every matrix leg, including legs that would repeat another one.
pub fn set_every_matrix_leg(on: bool) {
    EVERY_MATRIX_LEG.store(on, Ordering::Relaxed);
}

pub fn every_matrix_leg() -> bool {
    EVERY_MATRIX_LEG.load(Ordering::Relaxed) || env_flag(EVERY_MATRIX_LEG_ENV)
}

/// Whether the switch in environment variable `name` is on, read as the
/// CLI reads it.
pub(crate) fn env_flag(name: &str) -> bool {
    std::env::var(name).is_ok_and(|v| switch_is_on(&v))
}

/// Whether the switch in environment variable `name` is on, `default` when
/// it isn't set: for a switch that is on until a user turns it off, read
/// as [`switch_is_on`] reads it.
pub(crate) fn env_switch(name: &str, default: bool) -> bool {
    std::env::var(name).map_or(default, |v| switch_is_on(&v))
}

/// Whether an environment switch is on, read exactly as the CLI's flags read
/// it (clap's `FalseyValueParser`): empty, `0`, `n`, `no`, `f`, `false` and
/// `off` in any case are off, and anything else is on, surrounding spaces
/// included, as clap doesn't trim. The CLI and the executor both read these
/// variables, so they must agree.
pub fn switch_is_on(value: &str) -> bool {
    let value = value.to_ascii_lowercase();
    !matches!(
        value.as_str(),
        "" | "0" | "n" | "no" | "f" | "false" | "off"
    )
}

#[cfg(test)]
mod tests {
    /// The job token is the environment's, `GITHUB_TOKEN` before `GH_TOKEN`,
    /// and the GitHub CLI's only when neither holds one.
    #[test]
    fn the_job_token_comes_from_the_environment_before_the_github_cli() {
        use super::job_github_token_from as token;
        let env = |pairs: &'static [(&'static str, &'static str)]| {
            move |name: &str| {
                pairs
                    .iter()
                    .find(|(n, _)| *n == name)
                    .map(|(_, v)| v.to_string())
            }
        };
        let gh = || Some("from-gh\n".to_string());
        assert_eq!(
            token(env(&[("GITHUB_TOKEN", " a "), ("GH_TOKEN", "b")]), gh).as_deref(),
            Some("a")
        );
        assert_eq!(
            token(env(&[("GITHUB_TOKEN", "  "), ("GH_TOKEN", "b")]), gh).as_deref(),
            Some("b")
        );
        assert_eq!(token(env(&[]), gh).as_deref(), Some("from-gh"));
        assert_eq!(token(env(&[]), || None), None);
        assert_eq!(token(env(&[]), || Some(" \n".to_string())), None);
    }

    #[test]
    fn switches_read_the_values_the_cli_reads() {
        for off in [
            "", "0", "n", "no", "No", "f", "false", "FALSE", "off", "Off",
        ] {
            assert!(!super::switch_is_on(off), "{off:?} turned a switch on");
        }
        for on in ["1", "y", "yes", "true", "on", "anything", " ", " 0", "no\n"] {
            assert!(super::switch_is_on(on), "{on:?} left a switch off");
        }
    }

    #[test]
    fn tests_never_keep_workspaces() {
        // `.cargo/config.toml` sets WRKFLW_FRESH_WORKSPACE for every test,
        // so jobs that tests run neither fill the user's cache directory nor
        // see files an earlier test run left behind.
        assert!(
            super::fresh_workspaces(),
            "set WRKFLW_FRESH_WORKSPACE=1 for tests (see .cargo/config.toml)"
        );
    }
}
