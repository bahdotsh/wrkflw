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
///
/// Fetches the current branch, tag, and changed files (vs `diff_base`) from git.
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

/// Build an event context using an explicit two-ref diff range.
///
/// Used by the CLI when both `--diff-base` and `--diff-head` are provided.
pub async fn context_from_diff_range(
    event_name: &str,
    base_ref: &str,
    head_ref: &str,
) -> Result<EventContext, TriggerFilterError> {
    let branch = git::get_current_branch().await.ok();
    let tag = git::get_current_tag().await?;
    let changed_files = git::get_changed_files_between(base_ref, head_ref).await?;

    Ok(EventContext {
        event_name: event_name.to_string(),
        branch,
        tag,
        changed_files,
        activity_type: None,
    })
}

/// Build an event context with pre-supplied changed files.
///
/// Useful when the caller already knows the changed files (e.g. from `--changed-files`
/// CLI flag or from filesystem watcher events).
pub async fn context_from_changed_files(
    event_name: &str,
    changed_files: Vec<String>,
) -> Result<EventContext, TriggerFilterError> {
    let branch = git::get_current_branch().await.ok();
    let tag = git::get_current_tag().await?;

    Ok(EventContext {
        event_name: event_name.to_string(),
        branch,
        tag,
        changed_files,
        activity_type: None,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::HashMap;

    fn make_workflow(on_yaml: &str) -> wrkflw_parser::workflow::WorkflowDefinition {
        wrkflw_parser::workflow::WorkflowDefinition {
            name: "test-workflow".to_string(),
            on: vec![],
            on_raw: serde_yaml::from_str(on_yaml).unwrap(),
            jobs: HashMap::new(),
            defaults: None,
        }
    }

    #[test]
    fn filter_workflows_matches_push_event() {
        let wf = make_workflow("push");
        let workflows = vec![(PathBuf::from("ci.yml"), wf)];
        let context = EventContext {
            event_name: "push".into(),
            branch: Some("main".into()),
            tag: None,
            changed_files: vec![],
            activity_type: None,
        };

        let results = filter_workflows(&workflows, &context);
        assert_eq!(results.len(), 1);
        assert!(results[0].matches);
    }

    #[test]
    fn filter_workflows_skips_non_matching_event() {
        let wf = make_workflow("push");
        let workflows = vec![(PathBuf::from("ci.yml"), wf)];
        let context = EventContext {
            event_name: "pull_request".into(),
            branch: Some("main".into()),
            tag: None,
            changed_files: vec![],
            activity_type: None,
        };

        let results = filter_workflows(&workflows, &context);
        assert_eq!(results.len(), 1);
        assert!(!results[0].matches);
    }

    #[test]
    fn filter_workflows_multiple_workflows() {
        let wf_push = make_workflow("push");
        let wf_pr = make_workflow("pull_request");
        let workflows = vec![
            (PathBuf::from("ci.yml"), wf_push),
            (PathBuf::from("pr.yml"), wf_pr),
        ];
        let context = EventContext {
            event_name: "push".into(),
            branch: Some("main".into()),
            tag: None,
            changed_files: vec![],
            activity_type: None,
        };

        let results = filter_workflows(&workflows, &context);
        assert_eq!(results.len(), 2);
        assert!(results[0].matches); // ci.yml matches push
        assert!(!results[1].matches); // pr.yml doesn't match push
    }

    #[test]
    fn filter_workflows_with_path_filter() {
        let wf = make_workflow(
            r#"
push:
  paths:
    - 'src/**'
"#,
        );
        let workflows = vec![(PathBuf::from("ci.yml"), wf)];

        // Changed file matches path filter
        let context = EventContext {
            event_name: "push".into(),
            branch: Some("main".into()),
            tag: None,
            changed_files: vec!["src/main.rs".into()],
            activity_type: None,
        };
        let results = filter_workflows(&workflows, &context);
        assert!(results[0].matches);

        // Changed file does NOT match path filter
        let context2 = EventContext {
            event_name: "push".into(),
            branch: Some("main".into()),
            tag: None,
            changed_files: vec!["docs/readme.md".into()],
            activity_type: None,
        };
        let results2 = filter_workflows(&workflows, &context2);
        assert!(!results2[0].matches);
    }
}
