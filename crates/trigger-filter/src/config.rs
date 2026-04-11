//! Runtime configuration for the trigger-filter driver.
//!
//! This replaces the old pile of file-local `const`s sprinkled across
//! `git.rs`, `watcher.rs`, and the TUI. A single struct lets callers
//! override the knobs in one place instead of forking the crate:
//!
//! - `git_state_ttl` — how long the watcher may reuse a cached
//!   `(branch, tag)` pair between `git checkout`s.
//! - `pattern_cache_size` — upper bound on the LRU of compiled
//!   trigger configs keyed by `(path, mtime)`. Zero disables caching.
//! - `default_event` — event name to synthesize when the caller does
//!   not pass one (CLI `--event` default, TUI diff-filter default, and
//!   the watcher's `WatcherConfig` fall-through).
//!
//! Construction is via `TriggerFilterConfig::default()` plus builder
//! setters, the same shape `WatcherConfig` already uses — this keeps
//! the CLI / TUI / watcher wiring uniform.
//!
//! **On missing knobs.** This struct deliberately does NOT carry a
//! `git_timeout` or `strict_missing_context` field. Both existed as
//! builder-only plumbing in an earlier draft with no library-side
//! consumers — `git.rs` read a file-local const for the timeout, and
//! strict-mode was implemented entirely in the CLI. Shipping
//! non-functional config knobs is precisely the kind of drift the
//! rest of this crate is built to prevent, so they were removed
//! rather than half-wired. If a future caller needs to override the
//! git timeout, thread `Duration` through `run_git` first and add
//! the field at the same commit.

use std::time::Duration;

/// Hard upper bound on every git subprocess call.
///
/// The 10s default matches the value the crate has shipped with and
/// exists to catch hung-process failure modes (network filesystems,
/// credential prompts, corrupt repos) without letting them wedge the
/// watch loop forever. Currently hard-coded — see the module-level
/// docstring for the rationale on why there is no config knob yet.
pub const DEFAULT_GIT_COMMAND_TIMEOUT: Duration = Duration::from_secs(10);

/// Default TTL for the watcher's cached `(branch, tag)` pair.
///
/// Moved from `watcher.rs` so a single Config type owns the knob. See
/// the watcher crate for the rationale — short TTL avoids the
/// complexity of whitelisting `.git/HEAD` / `.git/refs/**` events past
/// the ignore filter while still bounding the worst-case staleness.
pub const DEFAULT_GIT_STATE_TTL: Duration = Duration::from_secs(3);

/// Default size of the LRU cache for compiled trigger configs.
///
/// 128 slots covers every monorepo we've profiled (`.github/workflows`
/// directories rarely exceed 50 files) with headroom for multiple
/// watcher instances. Set to zero via `with_pattern_cache_size(0)` to
/// disable caching entirely — useful in tests where you want every
/// `load_trigger_config` call to re-parse from disk.
pub const DEFAULT_PATTERN_CACHE_SIZE: usize = 128;

/// Default event name used when the caller does not supply one.
///
/// `push` matches GitHub Actions' own implicit default for bare
/// `on: push` shorthand and is the least surprising choice for CLI /
/// TUI users who toggle diff-filter against a local checkout.
pub const DEFAULT_EVENT_NAME: &str = "push";

/// Runtime configuration shared by the trigger-filter library and its
/// two main hosts, the CLI (`wrkflw run` / `wrkflw watch`) and the TUI.
///
/// See the module docs for the rationale behind each field. Use
/// `TriggerFilterConfig::default()` for the stock values and override
/// only the knobs you care about:
///
/// ```
/// use wrkflw_trigger_filter::TriggerFilterConfig;
/// use std::time::Duration;
///
/// let cfg = TriggerFilterConfig::default()
///     .with_git_state_ttl(Duration::from_secs(1))
///     .with_default_event("pull_request");
/// ```
#[derive(Debug, Clone)]
pub struct TriggerFilterConfig {
    pub git_state_ttl: Duration,
    pub pattern_cache_size: usize,
    pub default_event: String,
}

impl Default for TriggerFilterConfig {
    fn default() -> Self {
        Self {
            git_state_ttl: DEFAULT_GIT_STATE_TTL,
            pattern_cache_size: DEFAULT_PATTERN_CACHE_SIZE,
            default_event: DEFAULT_EVENT_NAME.to_string(),
        }
    }
}

impl TriggerFilterConfig {
    pub fn with_git_state_ttl(mut self, d: Duration) -> Self {
        self.git_state_ttl = d;
        self
    }

    pub fn with_pattern_cache_size(mut self, n: usize) -> Self {
        self.pattern_cache_size = n;
        self
    }

    pub fn with_default_event(mut self, event: impl Into<String>) -> Self {
        self.default_event = event.into();
        self
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn default_matches_documented_constants() {
        let cfg = TriggerFilterConfig::default();
        assert_eq!(cfg.git_state_ttl, DEFAULT_GIT_STATE_TTL);
        assert_eq!(cfg.pattern_cache_size, DEFAULT_PATTERN_CACHE_SIZE);
        assert_eq!(cfg.default_event, DEFAULT_EVENT_NAME);
    }

    #[test]
    fn builder_setters_override_defaults() {
        let cfg = TriggerFilterConfig::default()
            .with_git_state_ttl(Duration::from_secs(1))
            .with_pattern_cache_size(0)
            .with_default_event("pull_request");
        assert_eq!(cfg.git_state_ttl, Duration::from_secs(1));
        assert_eq!(cfg.pattern_cache_size, 0);
        assert_eq!(cfg.default_event, "pull_request");
    }
}
