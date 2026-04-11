//! Runtime configuration for the trigger-filter driver.
//!
//! This replaces the old pile of file-local `const`s sprinkled across
//! `git.rs`, `watcher.rs`, and the TUI. A single struct lets callers
//! override the knobs in one place instead of forking the crate:
//!
//! - `git_timeout` — hard cap on every `git` subprocess invocation.
//! - `git_state_ttl` — how long the watcher may reuse a cached
//!   `(branch, tag)` pair between `git checkout`s.
//! - `pattern_cache_size` — upper bound on the LRU of compiled
//!   trigger configs keyed by `(path, mtime)`. Zero disables caching.
//! - `default_event` — event name to synthesize when the caller does
//!   not pass one (CLI `--event` default, TUI diff-filter default, and
//!   the watcher's `WatcherConfig` fall-through).
//! - `strict_missing_context` — when true, building an `EventContext`
//!   with known-incomplete inputs (e.g. `--event` without a changed
//!   file set, or `pull_request` without a base branch) is a hard
//!   error instead of a warning-and-proceed.
//!
//! Construction is via `TriggerFilterConfig::default()` plus builder
//! setters, the same shape `WatcherConfig` already uses — this keeps
//! the CLI / TUI / watcher wiring uniform.

use std::time::Duration;

/// Hard upper bound on every git subprocess call, unless overridden.
///
/// Moved from `git.rs` so the knob is visible from every caller that
/// threads a `TriggerFilterConfig` through. The 10s default matches the
/// value the crate has shipped with, and exists to catch hung-process
/// failure modes (network filesystems, credential prompts, corrupt
/// repos) without letting them wedge the watch loop forever.
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
///     .with_git_timeout(Duration::from_secs(30))
///     .with_default_event("pull_request")
///     .with_strict_missing_context(true);
/// ```
#[derive(Debug, Clone)]
pub struct TriggerFilterConfig {
    pub git_timeout: Duration,
    pub git_state_ttl: Duration,
    pub pattern_cache_size: usize,
    pub default_event: String,
    pub strict_missing_context: bool,
}

impl Default for TriggerFilterConfig {
    fn default() -> Self {
        Self {
            git_timeout: DEFAULT_GIT_COMMAND_TIMEOUT,
            git_state_ttl: DEFAULT_GIT_STATE_TTL,
            pattern_cache_size: DEFAULT_PATTERN_CACHE_SIZE,
            default_event: DEFAULT_EVENT_NAME.to_string(),
            strict_missing_context: false,
        }
    }
}

impl TriggerFilterConfig {
    pub fn with_git_timeout(mut self, d: Duration) -> Self {
        self.git_timeout = d;
        self
    }

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

    pub fn with_strict_missing_context(mut self, strict: bool) -> Self {
        self.strict_missing_context = strict;
        self
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn default_matches_documented_constants() {
        let cfg = TriggerFilterConfig::default();
        assert_eq!(cfg.git_timeout, DEFAULT_GIT_COMMAND_TIMEOUT);
        assert_eq!(cfg.git_state_ttl, DEFAULT_GIT_STATE_TTL);
        assert_eq!(cfg.pattern_cache_size, DEFAULT_PATTERN_CACHE_SIZE);
        assert_eq!(cfg.default_event, DEFAULT_EVENT_NAME);
        assert!(!cfg.strict_missing_context);
    }

    #[test]
    fn builder_setters_override_defaults() {
        let cfg = TriggerFilterConfig::default()
            .with_git_timeout(Duration::from_secs(30))
            .with_git_state_ttl(Duration::from_secs(1))
            .with_pattern_cache_size(0)
            .with_default_event("pull_request")
            .with_strict_missing_context(true);
        assert_eq!(cfg.git_timeout, Duration::from_secs(30));
        assert_eq!(cfg.git_state_ttl, Duration::from_secs(1));
        assert_eq!(cfg.pattern_cache_size, 0);
        assert_eq!(cfg.default_event, "pull_request");
        assert!(cfg.strict_missing_context);
    }
}
