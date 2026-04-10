pub mod error;
pub mod eval;
pub mod git;
pub mod model;
pub mod parser;
pub mod path_matcher;
pub mod ref_matcher;

pub use error::TriggerFilterError;
pub use eval::evaluate_trigger;
pub use git::find_repo_root;
pub use model::{
    EventContext, EventFilter, GlobPattern, TriggerMatchResult, WorkflowTriggerConfig,
};
pub use parser::parse_trigger_config;

use std::path::{Path, PathBuf};

/// Read a workflow file from disk and parse its trigger configuration in
/// one step. Centralizes the "read + parse + compile globs" pipeline so
/// that `watcher`, the TUI, and the CLI all fail identically on the same
/// broken file.
///
/// This performs blocking file I/O via [`wrkflw_parser::workflow::parse_workflow`].
/// Call from a blocking context (or wrap in `spawn_blocking`) if invoked
/// from an async task that must not stall the reactor.
pub fn load_trigger_config(
    workflow_path: &Path,
) -> Result<WorkflowTriggerConfig, TriggerFilterError> {
    let workflow = wrkflw_parser::workflow::parse_workflow(workflow_path)
        .map_err(|e| TriggerFilterError::ParseError(e.to_string()))?;
    parse_trigger_config(&workflow, workflow_path.to_path_buf())
}

/// Bulk-load trigger configs for many workflow files, partitioning the
/// result into successes and per-file failure pairs.
///
/// This is the shape both the watcher and the TUI need: parse failures
/// must be surfaced to the user (rather than `filter_map(... .ok())`d
/// into invisibility), so any caller that wants to render "N failed to
/// parse" diagnostics gets the offending paths and reasons in one call.
///
/// Same blocking-I/O caveat as [`load_trigger_config`] — wrap in
/// `spawn_blocking` from async contexts.
pub fn load_trigger_configs(
    paths: &[PathBuf],
) -> (Vec<WorkflowTriggerConfig>, Vec<(PathBuf, String)>) {
    let mut configs = Vec::with_capacity(paths.len());
    let mut failures: Vec<(PathBuf, String)> = Vec::new();
    for path in paths {
        match load_trigger_config(path) {
            Ok(cfg) => configs.push(cfg),
            Err(e) => failures.push((path.clone(), e.to_string())),
        }
    }
    (configs, failures)
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
/// **Branch handling:** detached HEAD returns `Ok` with `branch: None` (the
/// underlying [`git::get_current_branch`] surfaces detached HEAD as `Ok(None)`,
/// not an error). A *real* git error — e.g. permission denied on `.git/HEAD`,
/// corrupt repo — propagates as `Err`. Previously this code collapsed both
/// cases to `branch: None`, which masked real failures.
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
        branch: branch_res?,
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
        branch: branch_res?,
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
        branch: branch_res?,
        base_branch: None,
        tag: tag_res?,
        changed_files,
        activity_type: None,
    })
}

// Note: the tests for the deleted `filter_workflows` (which parsed +
// evaluated in one shot) used to live here. They have been removed
// alongside the function — equivalent coverage already exists in
// `eval.rs` (match/no-match across event types and filter combos) and
// `parser.rs::invalid_glob_pattern_surfaces_as_parse_error` (the
// "broken glob surfaces a parse error" contract). The cached path
// `filter_trigger_configs` is exercised by every consumer in the
// workspace.

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::TempDir;

    #[test]
    fn load_trigger_configs_partitions_successes_and_failures() {
        // Mixed batch: one valid workflow, one with broken YAML, one
        // with a malformed glob. The bulk loader must return the
        // successful config and a per-file failure entry for each
        // broken file — the silent-drop pattern that
        // `.filter_map(... .ok())` produced was exactly the failure
        // mode that drove the original "lying about which workflows
        // would run" incident.
        let tmp = TempDir::new().expect("tempdir");
        let root = tmp.path();

        let good = root.join("good.yml");
        std::fs::write(
            &good,
            "name: good\non: push\njobs:\n  build:\n    runs-on: ubuntu-latest\n    steps:\n      - run: echo hi\n",
        )
        .unwrap();

        let bad_yaml = root.join("bad_yaml.yml");
        std::fs::write(&bad_yaml, "name: bad\non: [unterminated\n").unwrap();

        let bad_glob = root.join("bad_glob.yml");
        std::fs::write(
            &bad_glob,
            "name: bad_glob\non:\n  push:\n    paths:\n      - '[unclosed'\njobs:\n  build:\n    runs-on: ubuntu-latest\n    steps:\n      - run: echo hi\n",
        )
        .unwrap();

        let paths = vec![good.clone(), bad_yaml.clone(), bad_glob.clone()];
        let (configs, failures) = load_trigger_configs(&paths);

        assert_eq!(configs.len(), 1, "exactly one workflow should parse");
        assert_eq!(configs[0].workflow_path, good);

        assert_eq!(failures.len(), 2, "both broken files must surface");
        let failed_paths: Vec<&PathBuf> = failures.iter().map(|(p, _)| p).collect();
        assert!(failed_paths.contains(&&bad_yaml));
        assert!(failed_paths.contains(&&bad_glob));
    }
}
