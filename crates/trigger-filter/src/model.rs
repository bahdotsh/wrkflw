use std::path::PathBuf;

/// Parsed trigger configuration for a single event type (e.g., push, pull_request).
#[derive(Debug, Clone, Default)]
pub struct EventFilter {
    pub event_name: String,
    pub branches: Vec<String>,
    pub branches_ignore: Vec<String>,
    pub tags: Vec<String>,
    pub tags_ignore: Vec<String>,
    pub paths: Vec<String>,
    pub paths_ignore: Vec<String>,
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
#[derive(Debug, Clone)]
pub struct EventContext {
    pub event_name: String,
    pub branch: Option<String>,
    pub tag: Option<String>,
    pub changed_files: Vec<String>,
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
