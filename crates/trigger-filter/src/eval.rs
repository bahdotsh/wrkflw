use crate::model::{EventContext, TriggerMatchResult, WorkflowTriggerConfig};
use crate::path_matcher;
use crate::ref_matcher;

/// Evaluate whether a workflow should trigger given an event context.
pub fn evaluate_trigger(
    config: &WorkflowTriggerConfig,
    context: &EventContext,
) -> TriggerMatchResult {
    // Find event filters matching the context event name
    let matching_filters: Vec<_> = config
        .events
        .iter()
        .filter(|e| e.event_name == context.event_name)
        .collect();

    if matching_filters.is_empty() {
        return TriggerMatchResult {
            workflow_path: config.workflow_path.clone(),
            workflow_name: config.workflow_name.clone(),
            matches: false,
            matched_event: None,
            reason: format!(
                "Workflow does not listen to '{}' events (configured: {})",
                context.event_name,
                config
                    .events
                    .iter()
                    .map(|e| e.event_name.as_str())
                    .collect::<Vec<_>>()
                    .join(", ")
            ),
        };
    }

    for filter in &matching_filters {
        // Check branch filters (applies to push, pull_request, etc.)
        if !filter.branches.is_empty() || !filter.branches_ignore.is_empty() {
            match context.branch {
                Some(ref branch) => {
                    if !ref_matcher::matches_ref(branch, &filter.branches, &filter.branches_ignore)
                    {
                        continue;
                    }
                }
                None => continue, // No branch in context, branch filter cannot match
            }
        }

        // Check tag filters (applies to push with tags)
        if !filter.tags.is_empty() || !filter.tags_ignore.is_empty() {
            match context.tag {
                Some(ref tag) => {
                    if !ref_matcher::matches_ref(tag, &filter.tags, &filter.tags_ignore) {
                        continue;
                    }
                }
                None => continue, // No tag in context, tag filter cannot match
            }
        }

        // Check activity type filters (applies to pull_request, issues, etc.)
        if !filter.types.is_empty() {
            match context.activity_type {
                Some(ref activity) => {
                    if !filter.types.iter().any(|t| t == activity) {
                        continue;
                    }
                }
                None => continue, // No activity type in context, type filter cannot match
            }
        }

        // Check path filters
        if (!filter.paths.is_empty() || !filter.paths_ignore.is_empty())
            && !path_matcher::matches_paths(
                &context.changed_files,
                &filter.paths,
                &filter.paths_ignore,
            )
        {
            continue;
        }

        // All filters passed for this event
        return TriggerMatchResult {
            workflow_path: config.workflow_path.clone(),
            workflow_name: config.workflow_name.clone(),
            matches: true,
            matched_event: Some(filter.event_name.clone()),
            reason: format!("Matched '{}' event trigger", filter.event_name),
        };
    }

    // No filter combination matched — build a diagnostic reason
    let reasons: Vec<String> = matching_filters
        .iter()
        .map(|f| {
            let mut parts = Vec::new();
            if !f.branches.is_empty() || !f.branches_ignore.is_empty() {
                match &context.branch {
                    Some(branch) => parts.push(format!(
                        "branch '{}' did not match {:?}",
                        branch, f.branches
                    )),
                    None => {
                        parts.push("no branch in context (branch filter requires one)".to_string())
                    }
                }
            }
            if !f.tags.is_empty() || !f.tags_ignore.is_empty() {
                match &context.tag {
                    Some(tag) => parts.push(format!("tag '{}' did not match {:?}", tag, f.tags)),
                    None => parts.push("no tag in context (tag filter requires one)".to_string()),
                }
            }
            if !f.types.is_empty() {
                match &context.activity_type {
                    Some(activity) => {
                        parts.push(format!("activity '{}' not in {:?}", activity, f.types))
                    }
                    None => parts.push(
                        "no activity type in context (types filter requires one)".to_string(),
                    ),
                }
            }
            if !f.paths.is_empty() {
                parts.push(format!("paths: {:?}", f.paths));
            }
            if !f.paths_ignore.is_empty() {
                parts.push(format!("paths-ignore: {:?}", f.paths_ignore));
            }
            if parts.is_empty() {
                "no specific filters".to_string()
            } else {
                parts.join(", ")
            }
        })
        .collect();

    TriggerMatchResult {
        workflow_path: config.workflow_path.clone(),
        workflow_name: config.workflow_name.clone(),
        matches: false,
        matched_event: None,
        reason: format!(
            "Event '{}' matched but filters did not pass: {}",
            context.event_name,
            reasons.join("; ")
        ),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::EventFilter;
    use std::path::PathBuf;

    fn make_config(events: Vec<EventFilter>) -> WorkflowTriggerConfig {
        WorkflowTriggerConfig {
            workflow_path: PathBuf::from("test.yml"),
            workflow_name: "test".to_string(),
            events,
        }
    }

    #[test]
    fn no_matching_event() {
        let config = make_config(vec![EventFilter {
            event_name: "push".into(),
            ..Default::default()
        }]);
        let ctx = EventContext {
            event_name: "pull_request".into(),
            branch: Some("main".into()),
            tag: None,
            changed_files: vec!["src/main.rs".into()],
            activity_type: None,
        };
        let result = evaluate_trigger(&config, &ctx);
        assert!(!result.matches);
    }

    #[test]
    fn matching_event_no_filters() {
        let config = make_config(vec![EventFilter {
            event_name: "push".into(),
            ..Default::default()
        }]);
        let ctx = EventContext {
            event_name: "push".into(),
            branch: Some("main".into()),
            tag: None,
            changed_files: vec![],
            activity_type: None,
        };
        let result = evaluate_trigger(&config, &ctx);
        assert!(result.matches);
    }

    #[test]
    fn branch_filter_matches() {
        let config = make_config(vec![EventFilter {
            event_name: "push".into(),
            branches: vec!["main".into(), "release/**".into()],
            ..Default::default()
        }]);
        let ctx = EventContext {
            event_name: "push".into(),
            branch: Some("main".into()),
            tag: None,
            changed_files: vec![],
            activity_type: None,
        };
        assert!(evaluate_trigger(&config, &ctx).matches);
    }

    #[test]
    fn branch_filter_no_match() {
        let config = make_config(vec![EventFilter {
            event_name: "push".into(),
            branches: vec!["main".into()],
            ..Default::default()
        }]);
        let ctx = EventContext {
            event_name: "push".into(),
            branch: Some("feature/foo".into()),
            tag: None,
            changed_files: vec![],
            activity_type: None,
        };
        assert!(!evaluate_trigger(&config, &ctx).matches);
    }

    #[test]
    fn path_filter_matches() {
        let config = make_config(vec![EventFilter {
            event_name: "push".into(),
            paths: vec!["src/**".into()],
            ..Default::default()
        }]);
        let ctx = EventContext {
            event_name: "push".into(),
            branch: Some("main".into()),
            tag: None,
            changed_files: vec!["src/main.rs".into()],
            activity_type: None,
        };
        assert!(evaluate_trigger(&config, &ctx).matches);
    }

    #[test]
    fn path_filter_no_match() {
        let config = make_config(vec![EventFilter {
            event_name: "push".into(),
            paths: vec!["src/**".into()],
            ..Default::default()
        }]);
        let ctx = EventContext {
            event_name: "push".into(),
            branch: Some("main".into()),
            tag: None,
            changed_files: vec!["docs/readme.md".into()],
            activity_type: None,
        };
        assert!(!evaluate_trigger(&config, &ctx).matches);
    }

    #[test]
    fn paths_ignore_match() {
        let config = make_config(vec![EventFilter {
            event_name: "push".into(),
            paths_ignore: vec!["docs/**".into(), "*.md".into()],
            ..Default::default()
        }]);
        // Only doc changes — should NOT trigger
        let ctx = EventContext {
            event_name: "push".into(),
            branch: Some("main".into()),
            tag: None,
            changed_files: vec!["docs/guide.md".into()],
            activity_type: None,
        };
        assert!(!evaluate_trigger(&config, &ctx).matches);

        // Mix of doc and source changes — should trigger
        let ctx2 = EventContext {
            event_name: "push".into(),
            branch: Some("main".into()),
            tag: None,
            changed_files: vec!["docs/guide.md".into(), "src/lib.rs".into()],
            activity_type: None,
        };
        assert!(evaluate_trigger(&config, &ctx2).matches);
    }

    #[test]
    fn combined_branch_and_path_filter() {
        let config = make_config(vec![EventFilter {
            event_name: "push".into(),
            branches: vec!["main".into()],
            paths: vec!["src/**".into()],
            ..Default::default()
        }]);

        // Right branch, right path
        let ctx = EventContext {
            event_name: "push".into(),
            branch: Some("main".into()),
            tag: None,
            changed_files: vec!["src/main.rs".into()],
            activity_type: None,
        };
        assert!(evaluate_trigger(&config, &ctx).matches);

        // Wrong branch, right path
        let ctx2 = EventContext {
            event_name: "push".into(),
            branch: Some("develop".into()),
            tag: None,
            changed_files: vec!["src/main.rs".into()],
            activity_type: None,
        };
        assert!(!evaluate_trigger(&config, &ctx2).matches);

        // Right branch, wrong path
        let ctx3 = EventContext {
            event_name: "push".into(),
            branch: Some("main".into()),
            tag: None,
            changed_files: vec!["docs/readme.md".into()],
            activity_type: None,
        };
        assert!(!evaluate_trigger(&config, &ctx3).matches);
    }

    #[test]
    fn tag_filter() {
        let config = make_config(vec![EventFilter {
            event_name: "push".into(),
            tags: vec!["v*".into()],
            tags_ignore: vec!["v*-rc*".into()],
            ..Default::default()
        }]);

        let ctx = EventContext {
            event_name: "push".into(),
            branch: None,
            tag: Some("v1.0.0".into()),
            changed_files: vec![],
            activity_type: None,
        };
        assert!(evaluate_trigger(&config, &ctx).matches);

        let ctx2 = EventContext {
            event_name: "push".into(),
            branch: None,
            tag: Some("v1.0.0-rc1".into()),
            changed_files: vec![],
            activity_type: None,
        };
        assert!(!evaluate_trigger(&config, &ctx2).matches);
    }

    #[test]
    fn workflow_dispatch_always_matches() {
        let config = make_config(vec![EventFilter {
            event_name: "workflow_dispatch".into(),
            ..Default::default()
        }]);
        let ctx = EventContext {
            event_name: "workflow_dispatch".into(),
            branch: None,
            tag: None,
            changed_files: vec![],
            activity_type: None,
        };
        assert!(evaluate_trigger(&config, &ctx).matches);
    }

    #[test]
    fn branch_filter_fails_when_no_branch_in_context() {
        let config = make_config(vec![EventFilter {
            event_name: "push".into(),
            branches: vec!["main".into()],
            ..Default::default()
        }]);
        let ctx = EventContext {
            event_name: "push".into(),
            branch: None,
            tag: None,
            changed_files: vec![],
            activity_type: None,
        };
        assert!(!evaluate_trigger(&config, &ctx).matches);
    }

    #[test]
    fn tag_filter_fails_when_no_tag_in_context() {
        let config = make_config(vec![EventFilter {
            event_name: "push".into(),
            tags: vec!["v*".into()],
            ..Default::default()
        }]);
        let ctx = EventContext {
            event_name: "push".into(),
            branch: Some("main".into()),
            tag: None,
            changed_files: vec![],
            activity_type: None,
        };
        assert!(!evaluate_trigger(&config, &ctx).matches);
    }

    #[test]
    fn types_filter_matches() {
        let config = make_config(vec![EventFilter {
            event_name: "pull_request".into(),
            types: vec!["opened".into(), "synchronize".into()],
            ..Default::default()
        }]);
        let ctx = EventContext {
            event_name: "pull_request".into(),
            branch: None,
            tag: None,
            changed_files: vec![],
            activity_type: Some("opened".into()),
        };
        assert!(evaluate_trigger(&config, &ctx).matches);
    }

    #[test]
    fn types_filter_no_match() {
        let config = make_config(vec![EventFilter {
            event_name: "pull_request".into(),
            types: vec!["opened".into()],
            ..Default::default()
        }]);
        let ctx = EventContext {
            event_name: "pull_request".into(),
            branch: None,
            tag: None,
            changed_files: vec![],
            activity_type: Some("closed".into()),
        };
        assert!(!evaluate_trigger(&config, &ctx).matches);
    }

    #[test]
    fn types_filter_fails_when_no_activity_type_in_context() {
        let config = make_config(vec![EventFilter {
            event_name: "pull_request".into(),
            types: vec!["opened".into()],
            ..Default::default()
        }]);
        let ctx = EventContext {
            event_name: "pull_request".into(),
            branch: None,
            tag: None,
            changed_files: vec![],
            activity_type: None,
        };
        assert!(!evaluate_trigger(&config, &ctx).matches);
    }
}
