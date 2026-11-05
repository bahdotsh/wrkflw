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

fn env_flag(name: &str) -> bool {
    std::env::var(name).is_ok_and(|v| !matches!(v.trim(), "" | "0" | "false"))
}

#[cfg(test)]
mod tests {
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
