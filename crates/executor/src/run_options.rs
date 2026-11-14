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
