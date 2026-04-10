pub mod error;
pub mod eval;
pub mod git;
pub mod model;
pub mod parser;
pub mod path_matcher;
pub mod ref_matcher;

pub use error::TriggerFilterError;
pub use eval::evaluate_trigger;
pub use model::{
    EventContext, EventFilter, GlobPattern, TriggerMatchResult, WorkflowTriggerConfig,
};
pub use parser::parse_trigger_config;

use std::path::{Path, PathBuf};

/// Evaluate multiple workflows against an event context, returning match results for each.
///
/// `WorkflowDefinition` is borrowed (not cloned) because it isn't `Clone`.
/// The watcher hits this hot loop on every cycle, so it caches parsed
/// workflows and passes references in.
///
/// This parses and compiles trigger globs on every call. For hot loops
/// (e.g. the filesystem watcher), prefer [`filter_trigger_configs`], which
/// takes pre-parsed [`WorkflowTriggerConfig`]s so that glob compilation can
/// be cached across cycles.
pub fn filter_workflows(
    workflows: &[(PathBuf, &wrkflw_parser::workflow::WorkflowDefinition)],
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

/// Evaluate multiple pre-parsed trigger configs against an event context.
///
/// Callers are expected to cache the [`WorkflowTriggerConfig`] values and
/// invalidate them only when the underlying workflow file changes. This
/// avoids re-running `parse_trigger_config` — and thus re-compiling every
/// glob pattern — on every cycle.
pub fn filter_trigger_configs(
    configs: &[&WorkflowTriggerConfig],
    context: &EventContext,
) -> Vec<TriggerMatchResult> {
    configs
        .iter()
        .map(|config| evaluate_trigger(config, context))
        .collect()
}

/// Auto-detect event context from the current git state.
///
/// Fetches the current branch, tag, and changed files (vs `diff_base`) from git.
///
/// `cwd` selects the working directory git operates in; pass `None` to use the
/// process CWD. Long-running consumers (e.g. the watcher) should always pass
/// their explicit repo root so they don't accidentally query the wrong repo.
///
/// **Note:** for `pull_request`/`pull_request_target` events, this does NOT
/// populate `base_branch` — there's no way to infer the PR target from a
/// local checkout. Callers should pass it explicitly via the higher-level
/// API or the `--base-branch` CLI flag.
pub async fn auto_detect_context(
    event_name: &str,
    diff_base: &str,
    cwd: Option<&Path>,
) -> Result<EventContext, TriggerFilterError> {
    // Run the three independent git queries concurrently.
    let (branch_res, tag_res, changed_res) = tokio::join!(
        git::get_current_branch(cwd),
        git::get_current_tag(cwd),
        git::get_changed_files(diff_base, cwd),
    );

    Ok(EventContext {
        event_name: event_name.to_string(),
        branch: branch_res.ok(),
        base_branch: None,
        tag: tag_res?,
        changed_files: changed_res?,
        activity_type: None,
    })
}

/// Like [`auto_detect_context`] but also resolves the diff base via
/// [`git::get_default_diff_base`] when the caller has no preference.
///
/// Fails with a `GitError` if no reasonable diff base can be detected — the
/// caller should surface that so the user can pass `--diff-base` explicitly
/// instead of silently getting a filter that matches every workflow.
pub async fn auto_detect_context_default_base(
    event_name: &str,
    cwd: Option<&Path>,
) -> Result<EventContext, TriggerFilterError> {
    let diff_base = git::get_default_diff_base(cwd).await?;
    auto_detect_context(event_name, &diff_base, cwd).await
}

/// Build an event context using an explicit two-ref diff range.
///
/// Used by the CLI when both `--diff-base` and `--diff-head` are provided.
pub async fn context_from_diff_range(
    event_name: &str,
    base_ref: &str,
    head_ref: &str,
    cwd: Option<&Path>,
) -> Result<EventContext, TriggerFilterError> {
    let (branch_res, tag_res, changed_res) = tokio::join!(
        git::get_current_branch(cwd),
        git::get_current_tag(cwd),
        git::get_changed_files_between(base_ref, head_ref, cwd),
    );

    Ok(EventContext {
        event_name: event_name.to_string(),
        branch: branch_res.ok(),
        base_branch: None,
        tag: tag_res?,
        changed_files: changed_res?,
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
    cwd: Option<&Path>,
) -> Result<EventContext, TriggerFilterError> {
    let (branch_res, tag_res) =
        tokio::join!(git::get_current_branch(cwd), git::get_current_tag(cwd),);

    Ok(EventContext {
        event_name: event_name.to_string(),
        branch: branch_res.ok(),
        base_branch: None,
        tag: tag_res?,
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

    fn borrow(
        v: &[(PathBuf, wrkflw_parser::workflow::WorkflowDefinition)],
    ) -> Vec<(PathBuf, &wrkflw_parser::workflow::WorkflowDefinition)> {
        v.iter().map(|(p, w)| (p.clone(), w)).collect()
    }

    #[test]
    fn filter_workflows_matches_push_event() {
        let wf = make_workflow("push");
        let owned = vec![(PathBuf::from("ci.yml"), wf)];
        let context = EventContext {
            event_name: "push".into(),
            branch: Some("main".into()),
            ..Default::default()
        };

        let results = filter_workflows(&borrow(&owned), &context);
        assert_eq!(results.len(), 1);
        assert!(results[0].matches);
    }

    #[test]
    fn filter_workflows_skips_non_matching_event() {
        let wf = make_workflow("push");
        let owned = vec![(PathBuf::from("ci.yml"), wf)];
        let context = EventContext {
            event_name: "pull_request".into(),
            branch: Some("main".into()),
            ..Default::default()
        };

        let results = filter_workflows(&borrow(&owned), &context);
        assert_eq!(results.len(), 1);
        assert!(!results[0].matches);
    }

    #[test]
    fn filter_workflows_multiple_workflows() {
        let wf_push = make_workflow("push");
        let wf_pr = make_workflow("pull_request");
        let owned = vec![
            (PathBuf::from("ci.yml"), wf_push),
            (PathBuf::from("pr.yml"), wf_pr),
        ];
        let context = EventContext {
            event_name: "push".into(),
            branch: Some("main".into()),
            ..Default::default()
        };

        let results = filter_workflows(&borrow(&owned), &context);
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
        let owned = vec![(PathBuf::from("ci.yml"), wf)];

        // Changed file matches path filter
        let context = EventContext {
            event_name: "push".into(),
            branch: Some("main".into()),
            changed_files: vec!["src/main.rs".into()],
            ..Default::default()
        };
        let results = filter_workflows(&borrow(&owned), &context);
        assert!(results[0].matches);

        // Changed file does NOT match path filter
        let context2 = EventContext {
            event_name: "push".into(),
            branch: Some("main".into()),
            changed_files: vec!["docs/readme.md".into()],
            ..Default::default()
        };
        let results2 = filter_workflows(&borrow(&owned), &context2);
        assert!(!results2[0].matches);
    }

    #[test]
    fn filter_workflows_surfaces_invalid_glob_as_failure_reason() {
        // A workflow with an invalid glob should be reported as not-matching
        // with a clear "Failed to parse" reason — not silently dropped.
        let wf = make_workflow(
            r#"
push:
  paths:
    - '[unclosed'
"#,
        );
        let owned = vec![(PathBuf::from("bad.yml"), wf)];
        let ctx = EventContext {
            event_name: "push".into(),
            branch: Some("main".into()),
            changed_files: vec!["src/main.rs".into()],
            ..Default::default()
        };
        let results = filter_workflows(&borrow(&owned), &ctx);
        assert_eq!(results.len(), 1);
        assert!(!results[0].matches);
        assert!(results[0].reason.contains("Failed to parse trigger config"));
        assert!(results[0].reason.contains("[unclosed"));
    }
}
