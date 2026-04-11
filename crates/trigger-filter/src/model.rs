use glob::{MatchOptions, Pattern, PatternError};
use std::path::PathBuf;

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
#[derive(Debug, Clone)]
#[non_exhaustive]
pub struct WorkflowTriggerConfig {
    pub workflow_path: PathBuf,
    pub workflow_name: String,
    pub events: Vec<EventFilter>,
}

/// Simulated event context for matching.
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
    /// cycle). Hosts should surface these to the user so "0 triggered"
    /// on a buggy context does not look identical to "0 triggered"
    /// because nothing matches — exactly the failure mode this crate
    /// has been iteratively patched to prevent.
    pub warnings: Vec<String>,
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
