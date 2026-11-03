//! Credentials wrkflw itself reads from the environment.
//!
//! They're taken out of the process environment at startup, before any thread
//! exists, so nothing wrkflw starts inherits them. Emulation runs workflow
//! steps as children of this process, and a step that could read the signing
//! key could forge results for every machine that trusts the store.

use std::sync::OnceLock;

/// Signs and verifies shared results.
pub(crate) const SIGNING_KEY: &str = "WRKFLW_CACHE_SIGNING_KEY";
/// Bearer token for an `https://` remote.
pub(crate) const REMOTE_TOKEN: &str = "WRKFLW_REMOTE_CACHE_TOKEN";

const NAMES: [&str; 2] = [SIGNING_KEY, REMOTE_TOKEN];

static CAPTURED: OnceLock<Vec<(&'static str, String)>> = OnceLock::new();

/// Read the credentials and remove them from the environment. Call it first
/// in `main`, while the process has one thread: removing a variable races
/// with any other thread reading the environment.
pub(crate) fn capture() {
    CAPTURED.get_or_init(|| {
        let mut found = Vec::new();
        for name in NAMES {
            if let Some(value) = std::env::var(name).ok().filter(|v| !v.is_empty()) {
                found.push((name, value));
            }
            std::env::remove_var(name);
        }
        found
    });
}

/// The value `capture` found for `name`.
pub(crate) fn get(name: &str) -> Option<String> {
    CAPTURED
        .get()?
        .iter()
        .find(|(captured, _)| *captured == name)
        .map(|(_, value)| value.clone())
}

/// Hand the credentials to a child `wrkflw`, which captures them in turn.
pub(crate) fn pass_to(command: &mut std::process::Command) {
    for (name, value) in CAPTURED.get().into_iter().flatten() {
        command.env(name, value);
    }
}
