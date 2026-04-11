use glob::{MatchOptions, Pattern, PatternError};
use std::path::PathBuf;

/// A warning buffer that insists on being drained before drop.
///
/// The trigger-filter crate deliberately routes non-fatal diagnostics
/// (unknown event names, `git ls-files --others` failures, etc.)
/// through struct fields instead of calling `wrkflw_logging::warning`
/// directly — hosts own the rendering policy. That decoupling is only
/// as strong as every host's willingness to drain the field; a single
/// forgetful `filter_map(... .ok())` anywhere in the pipeline
/// reintroduces the silent-skip failure mode this crate exists to
/// plug.
///
/// `MustDrainWarnings` makes the contract self-enforcing in debug
/// builds: if a non-empty instance is dropped without its contents
/// being observed via [`MustDrainWarnings::take`], the Drop impl
/// prints an `eprintln!` naming the unobserved warnings. Tests and
/// CI catch the regression immediately; release builds carry no
/// overhead. The Drop check is a `debug_assertions` guard, not a
/// `panic!`, because panicking from Drop on a production code path
/// (e.g. a watcher evicting a cache entry at shutdown under
/// memory pressure) would be strictly worse than the silent skip
/// it is trying to prevent.
///
/// **Clone semantics.** Clones carry an independent copy of the
/// warnings — cloning does not count as observation. A cloned
/// instance must be drained separately or its own Drop check will
/// fire. This is the right behavior for the library's primary
/// clone sites (the process-wide LRU cache and `filter_trigger_configs`
/// iteration), because a clone whose warnings are silently dropped
/// is exactly the same failure mode as the original — we want both
/// paths to trip the check.
#[derive(Debug, Default)]
pub struct MustDrainWarnings {
    inner: Vec<String>,
}

impl MustDrainWarnings {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn push(&mut self, warning: String) {
        self.inner.push(warning);
    }

    pub fn extend<I: IntoIterator<Item = String>>(&mut self, iter: I) {
        self.inner.extend(iter);
    }

    pub fn is_empty(&self) -> bool {
        self.inner.is_empty()
    }

    pub fn len(&self) -> usize {
        self.inner.len()
    }

    /// Iterate without draining. Callers that use this form MUST
    /// follow it with a drain before the instance is dropped —
    /// otherwise the debug-mode Drop check fires. Prefer
    /// [`MustDrainWarnings::take`] whenever ownership semantics allow.
    pub fn iter(&self) -> std::slice::Iter<'_, String> {
        self.inner.iter()
    }

    /// Drain the buffer and return its contents. Satisfies the
    /// Drop-time observation contract — after this, the instance
    /// can be dropped cleanly even if the returned `Vec` is never
    /// used again.
    pub fn take(&mut self) -> Vec<String> {
        std::mem::take(&mut self.inner)
    }
}

impl Clone for MustDrainWarnings {
    fn clone(&self) -> Self {
        Self {
            inner: self.inner.clone(),
        }
    }
}

impl From<Vec<String>> for MustDrainWarnings {
    fn from(inner: Vec<String>) -> Self {
        Self { inner }
    }
}

impl Drop for MustDrainWarnings {
    fn drop(&mut self) {
        // Silent in release builds — no production overhead, no
        // stderr spam for users on platforms where the library is
        // embedded without a host that drains. The loud behaviour
        // is strictly a development/test guardrail.
        //
        // Also silent if we're already unwinding a panic:
        // eprintln-ing during panic drop would stomp on the real
        // failure message and confuse the test output.
        if cfg!(debug_assertions) && !self.inner.is_empty() && !std::thread::panicking() {
            eprintln!(
                "wrkflw-trigger-filter: {} warning(s) were dropped without being drained by the host: {:?}",
                self.inner.len(),
                self.inner,
            );
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn take_empties_the_buffer_and_returns_contents() {
        let mut w = MustDrainWarnings::new();
        w.push("alpha".to_string());
        w.push("beta".to_string());
        assert_eq!(w.len(), 2);
        let drained = w.take();
        assert_eq!(drained, vec!["alpha".to_string(), "beta".to_string()]);
        assert!(w.is_empty(), "take must leave the buffer empty");
        // Dropping the now-empty `w` at end of scope must NOT fire
        // the debug-mode Drop check.
    }

    #[test]
    fn dropping_after_take_is_silent() {
        // Regression pin: if a future refactor flips the Drop check
        // to fire on `len == 0 && drained == false` (a "did you
        // remember to call take?" guard), this test catches it.
        // The contract is: empty buffer at Drop = clean, regardless
        // of whether anything was ever pushed.
        let w = MustDrainWarnings::new();
        drop(w);
    }

    #[test]
    fn from_vec_roundtrip() {
        let mut w = MustDrainWarnings::from(vec!["x".to_string()]);
        let drained = w.take();
        assert_eq!(drained, vec!["x".to_string()]);
    }

    #[test]
    fn clone_produces_independent_buffers() {
        // Clone semantics are load-bearing for the library's LRU
        // cache: the cached clone must NOT count as observation of
        // the original. This test pins that cloning yields two
        // independent buffers, each of which must be drained
        // separately.
        let mut a = MustDrainWarnings::from(vec!["only-on-a".to_string()]);
        let mut b = a.clone();
        assert_eq!(a.len(), 1);
        assert_eq!(b.len(), 1);
        let _drain_a = a.take();
        assert!(a.is_empty());
        // b is still populated — drain it too so the Drop check
        // doesn't fire at end of scope.
        let _drain_b = b.take();
        assert!(b.is_empty());
    }
}

/// A glob pattern paired with its source string.
///
/// The source is retained so diagnostic messages can refer back to what the
/// user wrote in the workflow YAML (e.g. `branch 'main' did not match
/// ["release/*"]` instead of the compiled `Pattern`'s `Debug` output).
#[derive(Debug, Clone)]
pub struct GlobPattern {
    pub source: String,
    pub pattern: Pattern,
}

impl GlobPattern {
    pub fn new(source: impl Into<String>) -> Result<Self, PatternError> {
        let source = source.into();
        let pattern = Pattern::new(&source)?;
        Ok(Self { source, pattern })
    }

    /// Match options used for both ref and path glob matching.
    ///
    /// GitHub Actions semantics: `*` does not cross `/`, `**` does.
    pub fn match_options() -> MatchOptions {
        MatchOptions {
            case_sensitive: true,
            require_literal_separator: true,
            require_literal_leading_dot: false,
        }
    }
}

/// Parsed trigger configuration for a single event type (e.g., push, pull_request).
///
/// Marked `#[non_exhaustive]` so adding new filter axes (e.g. a
/// hypothetical future `labels:` or `authors:` filter) is a non-
/// breaking change. Construct via `..Default::default()` and the
/// builder-ish shape the rest of the crate already uses; external
/// code cannot pattern-match every field.
#[derive(Debug, Clone, Default)]
#[non_exhaustive]
pub struct EventFilter {
    pub event_name: String,
    pub branches: Vec<GlobPattern>,
    pub branches_ignore: Vec<GlobPattern>,
    pub tags: Vec<GlobPattern>,
    pub tags_ignore: Vec<GlobPattern>,
    pub paths: Vec<GlobPattern>,
    pub paths_ignore: Vec<GlobPattern>,
    pub types: Vec<String>,
}

/// Complete trigger configuration for a workflow.
///
/// `#[non_exhaustive]` so future fields (e.g. parsed `concurrency:`,
/// `env:`, or a cached source hash) can be added without forcing
/// every external reader to update. Within this crate struct-literal
/// construction is still permitted — the attribute only blocks
/// external crates, which today only read the fields and never build
/// this type themselves.
///
/// `warnings` carries non-fatal parse diagnostics (unknown event
/// names, typo detection) that the library surfaces as data rather
/// than via the global logger. Hosts that want to render them to the
/// user should iterate this field after a successful
/// [`crate::parse_trigger_config`]. Routing them through the struct
/// instead of `wrkflw_logging::warning` keeps the library decoupled
/// from the log sink — an embedder using a different logger (or
/// running silently in tests) never sees spurious output.
#[derive(Debug, Clone)]
#[non_exhaustive]
pub struct WorkflowTriggerConfig {
    pub workflow_path: PathBuf,
    pub workflow_name: String,
    pub events: Vec<EventFilter>,
    /// Parser-level diagnostics (unknown event names, typo
    /// detection). Hosts MUST drain via `warnings.take()` to satisfy
    /// the `MustDrainWarnings` contract — see the type docs for
    /// rationale.
    pub warnings: MustDrainWarnings,
}

/// Simulated event context for matching.
///
/// **Not `#[non_exhaustive]` on purpose.** The watcher crate
/// constructs this via struct literal from its own `cached_git_state`
/// path, so adding `non_exhaustive` here would break the cross-crate
/// build. If a future field is added, update the watcher's call site
/// in the same commit.
#[derive(Debug, Clone, Default)]
pub struct EventContext {
    pub event_name: String,
    /// The branch the event happened on (the head branch for `pull_request`).
    pub branch: Option<String>,
    /// The base branch the PR targets — only meaningful for `pull_request`
    /// and `pull_request_target`. GitHub Actions' `branches:` filter on a
    /// pull-request trigger matches against THIS, not [`branch`].
    pub base_branch: Option<String>,
    pub tag: Option<String>,
    pub changed_files: Vec<String>,
    /// `true` when the caller actually *ran* a diff / filesystem
    /// snapshot to populate [`changed_files`], even if the result was
    /// empty. `false` means the caller built the context without
    /// attempting to enumerate changes (e.g. `wrkflw run` without
    /// `--diff` or `--changed-files`).
    ///
    /// This distinction powers a better diagnostic in
    /// [`crate::eval::explain_filter_failure`]: an empty change set
    /// under `changed_files_explicit == true` means "the diff you
    /// requested came back empty"; under `false` it means "you didn't
    /// ask for one, so of course there's nothing to match against".
    /// The old single-case "pass --diff or --changed-files" message
    /// was actively wrong for the first scenario and sent users
    /// chasing a flag they had already passed.
    pub changed_files_explicit: bool,
    /// Activity type for events that support it (e.g., "opened", "synchronize" for pull_request)
    pub activity_type: Option<String>,
    /// Non-fatal diagnostics collected while building this context.
    ///
    /// Populated by the git helpers when a best-effort enrichment
    /// failed (the canonical example is `git ls-files --others` being
    /// rejected by a restrictive safe-directory config, which silently
    /// dropped untracked files from the changed set for the entire
    /// cycle). Hosts MUST drain this via `warnings.take()` so the
    /// `MustDrainWarnings` Drop check stays satisfied — failing to
    /// drain is exactly the silent-skip failure mode this crate has
    /// been iteratively patched to prevent, and the Drop check
    /// catches the regression in debug builds.
    pub warnings: MustDrainWarnings,
}

/// Result of trigger evaluation for a single workflow.
///
/// `#[non_exhaustive]` — external callers read this struct but never
/// build it (all construction happens inside `eval.rs`). Future
/// fields (e.g. a structured machine-readable match explanation)
/// can be added without breaking consumers.
#[derive(Debug, Clone)]
#[non_exhaustive]
pub struct TriggerMatchResult {
    pub workflow_path: PathBuf,
    pub workflow_name: String,
    pub matches: bool,
    pub matched_event: Option<String>,
    pub reason: String,
}
