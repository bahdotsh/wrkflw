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
#[derive(Debug, Clone, Default)]
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
#[derive(Debug, Clone)]
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
    /// Activity type for events that support it (e.g., "opened", "synchronize" for pull_request)
    pub activity_type: Option<String>,
}

/// Result of trigger evaluation for a single workflow.
#[derive(Debug, Clone)]
pub struct TriggerMatchResult {
    pub workflow_path: PathBuf,
    pub workflow_name: String,
    pub matches: bool,
    pub matched_event: Option<String>,
    pub reason: String,
}
