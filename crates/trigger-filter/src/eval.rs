use crate::model::{
    EventContext, EventFilter, GlobPattern, TriggerMatchResult, WorkflowTriggerConfig,
};
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
        // Check ref-axis filters (branches + tags) with GHA OR semantics
        // when both are configured. See [`ref_filters_pass`] for the full
        // rule — the short version is: a push event with BOTH `branches:`
        // and `tags:` set fires if *either* side matches the actual ref,
        // not both. The previous sequential-AND check rejected both
        // branch pushes (tag was None) and tag pushes (branch was None)
        // for such workflows — exactly the silent-skip mode the rest of
        // this crate exists to prevent. Pinned by
        // `push_with_branches_and_tags_is_or_not_and`.
        if !ref_filters_pass(filter, context) {
            continue;
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
        .map(|f| explain_filter_failure(f, context))
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

/// Pick the branch that GH Actions uses to evaluate `branches:` filters.
fn branch_for_filter(context: &EventContext) -> Option<&String> {
    match context.event_name.as_str() {
        "pull_request" | "pull_request_target" => context.base_branch.as_ref(),
        _ => context.branch.as_ref(),
    }
}

/// Evaluate the combined branches + tags filter axis against the context.
///
/// GitHub Actions semantics for `push`:
/// - Neither filter set → any ref matches.
/// - Only `branches:` set → branch pushes matching the pattern fire; tag
///   pushes are rejected.
/// - Only `tags:` set → tag pushes matching the pattern fire; branch
///   pushes are rejected.
/// - **Both `branches:` and `tags:` set → the workflow fires if the
///   branch push matches `branches:` OR the tag push matches `tags:`.**
///   This is the case the previous sequential-AND evaluator got wrong —
///   a push can't be on both a branch and a tag simultaneously, so
///   requiring both to match is uniformly unsatisfiable.
///
/// For `pull_request` / `pull_request_target` there is no `tags:` axis
/// in GHA; we still evaluate branches: (against the base branch, per
/// [`branch_for_filter`]) and anything in `tags:` will be checked
/// against the always-None PR tag — rejecting the filter, which matches
/// GHA's behavior (writing `tags:` under `pull_request:` is a user error
/// that never fires).
fn ref_filters_pass(filter: &EventFilter, context: &EventContext) -> bool {
    let has_branches = !filter.branches.is_empty() || !filter.branches_ignore.is_empty();
    let has_tags = !filter.tags.is_empty() || !filter.tags_ignore.is_empty();

    if !has_branches && !has_tags {
        return true;
    }

    let branch_ok = has_branches
        && branch_for_filter(context)
            .map(|b| ref_matcher::matches_ref(b, &filter.branches, &filter.branches_ignore))
            .unwrap_or(false);

    let tag_ok = has_tags
        && context
            .tag
            .as_ref()
            .map(|t| ref_matcher::matches_ref(t, &filter.tags, &filter.tags_ignore))
            .unwrap_or(false);

    // When only one axis is set, exactly one of the booleans can be
    // true; when both are set, GHA's rule is OR. Either way the right
    // aggregation is `branch_ok || tag_ok`.
    branch_ok || tag_ok
}

/// Combine include + ignore pattern sources into a single list, with
/// ignore entries prefixed by `!` so the diagnostic round-trips to the
/// surface YAML syntax the user wrote. Extracted so the branches and
/// tags paths in [`explain_filter_failure`] cannot drift apart.
fn combined_pattern_sources(includes: &[GlobPattern], ignores: &[GlobPattern]) -> Vec<String> {
    let mut out: Vec<String> = includes.iter().map(|p| p.source.clone()).collect();
    out.extend(ignores.iter().map(|p| format!("!{}", p.source)));
    out
}

/// Build a human-readable diagnostic explaining which sub-filter caused
/// `filter` to reject `context`.
fn explain_filter_failure(filter: &EventFilter, context: &EventContext) -> String {
    let mut parts = Vec::new();

    let has_branches = !filter.branches.is_empty() || !filter.branches_ignore.is_empty();
    let has_tags = !filter.tags.is_empty() || !filter.tags_ignore.is_empty();

    // When BOTH axes are set, GHA treats them as OR (see
    // [`ref_filters_pass`]). The diagnostic must reflect that — saying
    // "branch X did not match [...]" in isolation is technically true
    // but misleads the user into thinking the tags axis was not
    // considered. Render the combined failure as a single "neither
    // branch nor tag matched" line so the OR semantics are obvious.
    if has_branches && has_tags {
        let branch_sources = combined_pattern_sources(&filter.branches, &filter.branches_ignore);
        let tag_sources = combined_pattern_sources(&filter.tags, &filter.tags_ignore);
        let branch_part = match branch_for_filter(context) {
            Some(b) => format!("branch '{}' did not match {:?}", b, branch_sources),
            None => "no branch in context".to_string(),
        };
        let tag_part = match &context.tag {
            Some(t) => format!("tag '{}' did not match {:?}", t, tag_sources),
            None => "no tag in context".to_string(),
        };
        parts.push(format!(
            "neither ref axis matched ({}; {}) — GHA fires on either branches: OR tags:",
            branch_part, tag_part
        ));
    } else if has_branches {
        // Combined pattern list: a rejection driven by `branches-ignore:`
        // alone (or inline `!`-negations routed into `branches_ignore`)
        // must render the offending rule instead of
        // `branch 'main' did not match []`.
        let pattern_sources = combined_pattern_sources(&filter.branches, &filter.branches_ignore);
        match branch_for_filter(context) {
            Some(branch) => parts.push(format!(
                "branch '{}' did not match {:?}",
                branch, pattern_sources
            )),
            None => {
                let what = if matches!(
                    context.event_name.as_str(),
                    "pull_request" | "pull_request_target"
                ) {
                    "no base_branch in context (pull_request branches: filter requires the target branch — pass --base-branch)"
                } else {
                    "no branch in context (branch filter requires one)"
                };
                parts.push(what.to_string());
            }
        }
    } else if has_tags {
        // Same treatment as branches — see `combined_pattern_sources`.
        let pattern_sources = combined_pattern_sources(&filter.tags, &filter.tags_ignore);
        match &context.tag {
            Some(tag) => parts.push(format!("tag '{}' did not match {:?}", tag, pattern_sources)),
            None => parts.push("no tag in context (tag filter requires one)".to_string()),
        }
    }
    if !filter.types.is_empty() {
        match &context.activity_type {
            Some(activity) => {
                parts.push(format!("activity '{}' not in {:?}", activity, filter.types))
            }
            None => {
                parts.push("no activity type in context — pass --activity-type <name>".to_string())
            }
        }
    }
    if !filter.paths.is_empty() {
        let sources: Vec<&str> = filter.paths.iter().map(|p| p.source.as_str()).collect();
        // When the change set is empty the path filter cannot possibly
        // match. Distinguish two cases so the user is not blamed for a
        // flag they have already passed:
        //   - `changed_files_explicit == false`: the caller never ran a
        //     diff, so "pass --diff / --changed-files" is the correct
        //     fix.
        //   - `changed_files_explicit == true`: the diff WAS run and
        //     returned zero files. Telling the user to pass `--diff`
        //     here is actively wrong — the filter is working and
        //     there's simply nothing to match.
        // The old single-message form conflated these and sent users
        // on wild-goose chases after a flag they had already set.
        let detail = if context.changed_files.is_empty() {
            if context.changed_files_explicit {
                format!(
                    "paths: {:?} (diff produced no changed files — nothing to match \
                     against; this is not an error, just a no-op cycle)",
                    sources
                )
            } else {
                format!(
                    "paths: {:?} (no changed files in context — pass --diff or \
                     --changed-files)",
                    sources
                )
            }
        } else {
            format!("paths: {:?}", sources)
        };
        parts.push(detail);
    }
    if !filter.paths_ignore.is_empty() {
        let sources: Vec<&str> = filter
            .paths_ignore
            .iter()
            .map(|p| p.source.as_str())
            .collect();
        parts.push(format!("paths-ignore: {:?}", sources));
    }
    if parts.is_empty() {
        "no specific filters".to_string()
    } else {
        parts.join(", ")
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::{EventFilter, GlobPattern, MustDrainWarnings};
    use std::path::PathBuf;

    fn gp(s: &str) -> GlobPattern {
        GlobPattern::new(s).unwrap()
    }

    fn make_config(events: Vec<EventFilter>) -> WorkflowTriggerConfig {
        WorkflowTriggerConfig {
            workflow_path: PathBuf::from("test.yml"),
            workflow_name: "test".to_string(),
            events,
            warnings: MustDrainWarnings::new(),
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
            changed_files: vec!["src/main.rs".into()],
            ..Default::default()
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
            ..Default::default()
        };
        assert!(evaluate_trigger(&config, &ctx).matches);
    }

    #[test]
    fn branch_filter_matches() {
        let config = make_config(vec![EventFilter {
            event_name: "push".into(),
            branches: vec![gp("main"), gp("release/**")],
            ..Default::default()
        }]);
        let ctx = EventContext {
            event_name: "push".into(),
            branch: Some("main".into()),
            ..Default::default()
        };
        assert!(evaluate_trigger(&config, &ctx).matches);
    }

    #[test]
    fn branch_filter_no_match() {
        let config = make_config(vec![EventFilter {
            event_name: "push".into(),
            branches: vec![gp("main")],
            ..Default::default()
        }]);
        let ctx = EventContext {
            event_name: "push".into(),
            branch: Some("feature/foo".into()),
            ..Default::default()
        };
        assert!(!evaluate_trigger(&config, &ctx).matches);
    }

    #[test]
    fn path_filter_matches() {
        let config = make_config(vec![EventFilter {
            event_name: "push".into(),
            paths: vec![gp("src/**")],
            ..Default::default()
        }]);
        let ctx = EventContext {
            event_name: "push".into(),
            branch: Some("main".into()),
            changed_files: vec!["src/main.rs".into()],
            ..Default::default()
        };
        assert!(evaluate_trigger(&config, &ctx).matches);
    }

    #[test]
    fn path_filter_no_match() {
        let config = make_config(vec![EventFilter {
            event_name: "push".into(),
            paths: vec![gp("src/**")],
            ..Default::default()
        }]);
        let ctx = EventContext {
            event_name: "push".into(),
            branch: Some("main".into()),
            changed_files: vec!["docs/readme.md".into()],
            ..Default::default()
        };
        assert!(!evaluate_trigger(&config, &ctx).matches);
    }

    #[test]
    fn paths_ignore_match() {
        let config = make_config(vec![EventFilter {
            event_name: "push".into(),
            paths_ignore: vec![gp("docs/**"), gp("*.md")],
            ..Default::default()
        }]);
        // Only doc changes — should NOT trigger
        let ctx = EventContext {
            event_name: "push".into(),
            branch: Some("main".into()),
            changed_files: vec!["docs/guide.md".into()],
            ..Default::default()
        };
        assert!(!evaluate_trigger(&config, &ctx).matches);

        // Mix of doc and source changes — should trigger
        let ctx2 = EventContext {
            event_name: "push".into(),
            branch: Some("main".into()),
            changed_files: vec!["docs/guide.md".into(), "src/lib.rs".into()],
            ..Default::default()
        };
        assert!(evaluate_trigger(&config, &ctx2).matches);
    }

    #[test]
    fn combined_branch_and_path_filter() {
        let config = make_config(vec![EventFilter {
            event_name: "push".into(),
            branches: vec![gp("main")],
            paths: vec![gp("src/**")],
            ..Default::default()
        }]);

        let ctx = EventContext {
            event_name: "push".into(),
            branch: Some("main".into()),
            changed_files: vec!["src/main.rs".into()],
            ..Default::default()
        };
        assert!(evaluate_trigger(&config, &ctx).matches);

        let ctx2 = EventContext {
            event_name: "push".into(),
            branch: Some("develop".into()),
            changed_files: vec!["src/main.rs".into()],
            ..Default::default()
        };
        assert!(!evaluate_trigger(&config, &ctx2).matches);

        let ctx3 = EventContext {
            event_name: "push".into(),
            branch: Some("main".into()),
            changed_files: vec!["docs/readme.md".into()],
            ..Default::default()
        };
        assert!(!evaluate_trigger(&config, &ctx3).matches);
    }

    #[test]
    fn tag_filter() {
        let config = make_config(vec![EventFilter {
            event_name: "push".into(),
            tags: vec![gp("v*")],
            tags_ignore: vec![gp("v*-rc*")],
            ..Default::default()
        }]);

        let ctx = EventContext {
            event_name: "push".into(),
            tag: Some("v1.0.0".into()),
            ..Default::default()
        };
        assert!(evaluate_trigger(&config, &ctx).matches);

        let ctx2 = EventContext {
            event_name: "push".into(),
            tag: Some("v1.0.0-rc1".into()),
            ..Default::default()
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
            ..Default::default()
        };
        assert!(evaluate_trigger(&config, &ctx).matches);
    }

    #[test]
    fn manual_events_match_without_branch_tag_or_changed_files() {
        // Regression pin: `workflow_dispatch`, `schedule`, and
        // `repository_dispatch` have no `branches:`/`paths:`/`tags:`
        // filters in practice, so an empty EventContext (no branch, no
        // tag, no changed files) MUST match. A refactor that starts
        // defaulting `branches:` to `[default_branch]` or similar would
        // silently break this contract and the watcher would start
        // rejecting manual triggers with no diagnostic — the exact
        // silent-skip mode this crate is built to prevent.
        for event in ["workflow_dispatch", "schedule", "repository_dispatch"] {
            let config = make_config(vec![EventFilter {
                event_name: event.into(),
                ..Default::default()
            }]);
            let ctx = EventContext {
                event_name: event.into(),
                ..Default::default()
            };
            let result = evaluate_trigger(&config, &ctx);
            assert!(
                result.matches,
                "{} with empty context must match (no filters configured), got skip: {}",
                event, result.reason
            );
        }
    }

    #[test]
    fn branch_filter_fails_when_no_branch_in_context() {
        let config = make_config(vec![EventFilter {
            event_name: "push".into(),
            branches: vec![gp("main")],
            ..Default::default()
        }]);
        let ctx = EventContext {
            event_name: "push".into(),
            ..Default::default()
        };
        assert!(!evaluate_trigger(&config, &ctx).matches);
    }

    #[test]
    fn tag_filter_fails_when_no_tag_in_context() {
        let config = make_config(vec![EventFilter {
            event_name: "push".into(),
            tags: vec![gp("v*")],
            ..Default::default()
        }]);
        let ctx = EventContext {
            event_name: "push".into(),
            branch: Some("main".into()),
            ..Default::default()
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
            activity_type: Some("opened".into()),
            ..Default::default()
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
            activity_type: Some("closed".into()),
            ..Default::default()
        };
        assert!(!evaluate_trigger(&config, &ctx).matches);
    }

    #[test]
    fn types_filter_fails_when_no_activity_type_in_context() {
        let config = make_config(vec![EventFilter {
            event_name: "pull_request".into(),
            types: vec![gp("opened").source.clone()],
            ..Default::default()
        }]);
        let ctx = EventContext {
            event_name: "pull_request".into(),
            ..Default::default()
        };
        assert!(!evaluate_trigger(&config, &ctx).matches);
    }

    #[test]
    fn pull_request_branches_filter_matches_base_not_head() {
        // pull_request workflow listening only to PRs targeting `main`
        let config = make_config(vec![EventFilter {
            event_name: "pull_request".into(),
            branches: vec![gp("main")],
            ..Default::default()
        }]);

        // Source branch is feature/foo, base branch is main → should match
        let ctx = EventContext {
            event_name: "pull_request".into(),
            branch: Some("feature/foo".into()),
            base_branch: Some("main".into()),
            ..Default::default()
        };
        assert!(
            evaluate_trigger(&config, &ctx).matches,
            "PR feature/foo→main should match pull_request branches:[main]"
        );

        // Source branch is main, base branch is develop → should NOT match
        let ctx2 = EventContext {
            event_name: "pull_request".into(),
            branch: Some("main".into()),
            base_branch: Some("develop".into()),
            ..Default::default()
        };
        assert!(
            !evaluate_trigger(&config, &ctx2).matches,
            "PR main→develop should not match pull_request branches:[main]"
        );
    }

    #[test]
    fn pull_request_branches_filter_fails_without_base_branch() {
        let config = make_config(vec![EventFilter {
            event_name: "pull_request".into(),
            branches: vec![gp("main")],
            ..Default::default()
        }]);
        // No base_branch supplied → branch filter cannot succeed
        let ctx = EventContext {
            event_name: "pull_request".into(),
            branch: Some("main".into()),
            base_branch: None,
            ..Default::default()
        };
        let result = evaluate_trigger(&config, &ctx);
        assert!(!result.matches);
        assert!(result.reason.contains("base_branch"));
    }

    #[test]
    fn types_filter_failure_message_mentions_activity_type_flag() {
        // Regression: the diagnostic for "this workflow has a `types:`
        // filter and the context has no activity type" used to say
        // "no activity type in context (types filter requires one)" —
        // factually correct but no clue how to fix it. Surface the
        // exact CLI flag the user needs, mirroring the `paths:` branch.
        let config = make_config(vec![EventFilter {
            event_name: "pull_request".into(),
            types: vec!["opened".into()],
            ..Default::default()
        }]);
        let ctx = EventContext {
            event_name: "pull_request".into(),
            ..Default::default()
        };
        let result = evaluate_trigger(&config, &ctx);
        assert!(!result.matches);
        assert!(
            result.reason.contains("--activity-type"),
            "diagnostic must point users at the fix flag, got: {}",
            result.reason
        );
    }

    #[test]
    fn branches_ignore_only_rejection_includes_ignore_patterns_in_diagnostic() {
        // Regression: previously the branch-mismatch diagnostic only
        // iterated `filter.branches`, so a workflow with ONLY a
        // `branches-ignore: [main]` filter running on `main` rendered as
        // `branch 'main' did not match []` — factually correct but
        // totally opaque about which rule caused the rejection. The
        // fix combines include + `!`-prefixed ignore sources so the
        // user sees the actual rule that fired.
        let config = make_config(vec![EventFilter {
            event_name: "push".into(),
            branches_ignore: vec![gp("main")],
            ..Default::default()
        }]);
        let ctx = EventContext {
            event_name: "push".into(),
            branch: Some("main".into()),
            ..Default::default()
        };
        let result = evaluate_trigger(&config, &ctx);
        assert!(
            !result.matches,
            "main should be excluded by branches_ignore"
        );
        assert!(
            result.reason.contains("!main"),
            "diagnostic must include the ignore pattern that caused rejection, got: {}",
            result.reason
        );
    }

    #[test]
    fn tags_ignore_only_rejection_includes_ignore_patterns_in_diagnostic() {
        // Parallel regression for the tags branch of `explain_filter_failure`.
        let config = make_config(vec![EventFilter {
            event_name: "push".into(),
            tags_ignore: vec![gp("v*-rc*")],
            ..Default::default()
        }]);
        let ctx = EventContext {
            event_name: "push".into(),
            tag: Some("v1.0.0-rc1".into()),
            ..Default::default()
        };
        let result = evaluate_trigger(&config, &ctx);
        assert!(!result.matches);
        assert!(
            result.reason.contains("!v*-rc*"),
            "tag diagnostic must include the ignore pattern, got: {}",
            result.reason
        );
    }

    #[test]
    fn push_with_branches_and_tags_is_or_not_and() {
        // CRITICAL regression: GitHub Actions treats a push workflow
        // with BOTH `branches:` and `tags:` filters as OR, not AND.
        // Per the docs: "If a workflow includes both a branches filter
        // and a tags filter, the workflow will run when a push event
        // matches either the branches filter or the tags filter."
        //
        // The previous sequential check (branches first, continue on
        // miss; tags second, continue on miss) collapsed both sides:
        // a branch push had context.tag = None so the tags check
        // rejected it; a tag push had context.branch = None so the
        // branches check rejected it. A workflow like the one below
        // never fired on any real push. This test pins the OR
        // semantics so a future refactor cannot silently regress it.
        let config = make_config(vec![EventFilter {
            event_name: "push".into(),
            branches: vec![gp("main")],
            tags: vec![gp("v*")],
            ..Default::default()
        }]);

        // Branch push to main (no tag on HEAD): must fire.
        let branch_push = EventContext {
            event_name: "push".into(),
            branch: Some("main".into()),
            tag: None,
            ..Default::default()
        };
        assert!(
            evaluate_trigger(&config, &branch_push).matches,
            "push to main must fire under branches:[main] + tags:[v*]"
        );

        // Tag push to v1.0.0 (branch None because git reports detached
        // HEAD on a tag checkout): must also fire.
        let tag_push = EventContext {
            event_name: "push".into(),
            branch: None,
            tag: Some("v1.0.0".into()),
            ..Default::default()
        };
        assert!(
            evaluate_trigger(&config, &tag_push).matches,
            "push to tag v1.0.0 must fire under branches:[main] + tags:[v*]"
        );

        // Branch push to develop (not in branches, no tag): must NOT fire.
        let branch_miss = EventContext {
            event_name: "push".into(),
            branch: Some("develop".into()),
            tag: None,
            ..Default::default()
        };
        assert!(
            !evaluate_trigger(&config, &branch_miss).matches,
            "push to develop must not fire (neither ref axis matches)"
        );

        // Tag push to v1-rc1 explicitly excluded by tags-ignore via
        // inline `!`-negation: must NOT fire.
        let config_excluded = make_config(vec![EventFilter {
            event_name: "push".into(),
            branches: vec![gp("main")],
            tags: vec![gp("v*")],
            tags_ignore: vec![gp("v*-rc*")],
            ..Default::default()
        }]);
        let rc_push = EventContext {
            event_name: "push".into(),
            branch: None,
            tag: Some("v1.0.0-rc1".into()),
            ..Default::default()
        };
        assert!(
            !evaluate_trigger(&config_excluded, &rc_push).matches,
            "rc tag push must be excluded by tags-ignore even when branches: also set"
        );
    }

    #[test]
    fn push_with_branches_and_tags_failure_diagnostic_mentions_or() {
        // Pair with `push_with_branches_and_tags_is_or_not_and`: when
        // neither side matches, the diagnostic must reflect the OR
        // aggregation so the user understands both axes were checked.
        // Previously the failure line only mentioned the branches
        // rejection and the user had no idea tags were even considered.
        let config = make_config(vec![EventFilter {
            event_name: "push".into(),
            branches: vec![gp("main")],
            tags: vec![gp("v*")],
            ..Default::default()
        }]);
        let ctx = EventContext {
            event_name: "push".into(),
            branch: Some("develop".into()),
            tag: None,
            ..Default::default()
        };
        let result = evaluate_trigger(&config, &ctx);
        assert!(!result.matches);
        assert!(
            result.reason.contains("neither ref axis"),
            "diagnostic must acknowledge OR semantics, got: {}",
            result.reason
        );
        assert!(
            result.reason.contains("branches:") && result.reason.contains("tags:"),
            "diagnostic must mention both axes, got: {}",
            result.reason
        );
    }

    #[test]
    fn branches_only_rejects_tag_push() {
        // Regression guard: a workflow with `branches:` only (no tags)
        // must NOT fire on a tag push. The OR fix for the combo case
        // must not accidentally loosen the single-axis case.
        let config = make_config(vec![EventFilter {
            event_name: "push".into(),
            branches: vec![gp("main")],
            ..Default::default()
        }]);
        let tag_push = EventContext {
            event_name: "push".into(),
            branch: None,
            tag: Some("v1.0.0".into()),
            ..Default::default()
        };
        assert!(
            !evaluate_trigger(&config, &tag_push).matches,
            "branches-only workflow must not fire on tag push"
        );
    }

    #[test]
    fn tags_only_rejects_branch_push() {
        // Mirror of `branches_only_rejects_tag_push`.
        let config = make_config(vec![EventFilter {
            event_name: "push".into(),
            tags: vec![gp("v*")],
            ..Default::default()
        }]);
        let branch_push = EventContext {
            event_name: "push".into(),
            branch: Some("main".into()),
            tag: None,
            ..Default::default()
        };
        assert!(
            !evaluate_trigger(&config, &branch_push).matches,
            "tags-only workflow must not fire on branch push"
        );
    }

    #[test]
    fn pull_request_target_uses_base_branch_too() {
        let config = make_config(vec![EventFilter {
            event_name: "pull_request_target".into(),
            branches: vec![gp("main")],
            ..Default::default()
        }]);
        let ctx = EventContext {
            event_name: "pull_request_target".into(),
            branch: Some("feature/x".into()),
            base_branch: Some("main".into()),
            ..Default::default()
        };
        assert!(evaluate_trigger(&config, &ctx).matches);
    }
}
