pub mod error;
pub mod eval;
pub mod git;
pub mod model;
pub mod parser;
pub mod path_matcher;
pub mod ref_matcher;

pub use error::TriggerFilterError;
pub use eval::evaluate_trigger;
pub use model::{EventContext, EventFilter, TriggerMatchResult, WorkflowTriggerConfig};
pub use parser::parse_trigger_config;

use std::path::PathBuf;

/// Evaluate multiple workflows against an event context, returning match results for each.
pub fn filter_workflows(
    workflows: &[(PathBuf, wrkflw_parser::workflow::WorkflowDefinition)],
    context: &EventContext,
) -> Vec<TriggerMatchResult> {
    workflows
        .iter()
        .map(|(path, wf)| match parse_trigger_config(wf, path.clone()) {
            Ok(config) => evaluate_trigger(&config, context),
            Err(e) => TriggerMatchResult {
                workflow_path: path.clone(),
                workflow_name: wf.name.clone(),
                matches: false,
                matched_event: None,
                reason: format!("Failed to parse trigger config: {}", e),
            },
        })
        .collect()
}

/// Auto-detect event context from the current git state.
pub async fn auto_detect_context(
    event_name: &str,
    diff_base: &str,
) -> Result<EventContext, TriggerFilterError> {
    let branch = git::get_current_branch().await.ok();
    let tag = git::get_current_tag().await?;
    let changed_files = git::get_changed_files(diff_base).await?;

    Ok(EventContext {
        event_name: event_name.to_string(),
        branch,
        tag,
        changed_files,
        activity_type: None,
    })
}
