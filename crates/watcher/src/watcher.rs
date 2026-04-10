use crate::debouncer::Debouncer;
use crate::error::WatchError;
use futures::stream::{self, StreamExt};
use notify::{Event, RecommendedWatcher, RecursiveMode, Watcher};
use std::collections::{HashMap, HashSet};
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::Duration;
use wrkflw_executor::ExecutionConfig;
use wrkflw_trigger_filter::WorkflowTriggerConfig;

/// Default cap on workflows executing concurrently in watch mode when the
/// caller does not supply an explicit limit.
pub const DEFAULT_MAX_CONCURRENT_EXECUTIONS: usize = 4;

/// A watch event containing the changed files and trigger evaluation results.
#[derive(Debug, Clone)]
pub struct WatchEvent {
    pub changed_files: Vec<String>,
    pub triggered_workflows: Vec<String>,
    pub skipped_workflows: Vec<String>,
}

/// Directories ignored by the filesystem watcher by default.
/// These are high-churn directories that almost never contain workflow-relevant
/// source files and would otherwise flood the event channel.
const DEFAULT_IGNORE_DIRS: &[&str] = &[
    ".git",
    "target",
    "node_modules",
    ".build",
    "build",
    "dist",
    "__pycache__",
    ".tox",
    ".mypy_cache",
    ".pytest_cache",
    ".venv",
    "venv",
];

/// Watches for filesystem changes and triggers workflow execution.
pub struct WorkflowWatcher {
    workflow_dir: PathBuf,
    repo_root: PathBuf,
    event_name: String,
    base_branch: Option<String>,
    debounce_duration: Duration,
    config_template: ExecutionConfig,
    verbose: bool,
    max_concurrent_executions: usize,
}

impl WorkflowWatcher {
    #[allow(clippy::too_many_arguments)]
    pub fn new(
        workflow_dir: PathBuf,
        repo_root: PathBuf,
        event_name: String,
        base_branch: Option<String>,
        debounce_duration: Duration,
        config: ExecutionConfig,
        verbose: bool,
        max_concurrent_executions: usize,
    ) -> Self {
        // 0 would deadlock buffer_unordered; clamp to at least 1.
        let max_concurrent_executions = max_concurrent_executions.max(1);
        Self {
            workflow_dir,
            repo_root,
            event_name,
            base_branch,
            debounce_duration,
            config_template: config,
            verbose,
            max_concurrent_executions,
        }
    }

    /// Collect workflow files from the configured directory.
    pub fn collect_workflow_files(&self) -> Result<Vec<PathBuf>, WatchError> {
        let dir = &self.workflow_dir;
        if dir.is_file() {
            return Ok(vec![dir.clone()]);
        }

        let mut files = Vec::new();
        for entry in std::fs::read_dir(dir)? {
            let entry = entry?;
            let path = entry.path();
            if let Some(ext) = path.extension().and_then(|e| e.to_str()) {
                if ext == "yml" || ext == "yaml" {
                    files.push(path);
                }
            }
        }

        if files.is_empty() {
            return Err(WatchError::NoWorkflows(dir.display().to_string()));
        }
        Ok(files)
    }

    /// Start the watch loop. Calls `on_cycle_complete` after each
    /// debounced change set has been evaluated and executed.
    /// This blocks until an error occurs or the process is interrupted.
    ///
    /// **Important:** `on_cycle_complete` is invoked via `spawn_blocking` so it
    /// will not stall the async watch loop, but it should still return promptly
    /// to avoid exhausting the blocking thread pool under sustained churn.
    pub async fn run<F>(&self, on_cycle_complete: F) -> Result<(), WatchError>
    where
        F: Fn(WatchEvent) + Send + Sync + 'static,
    {
        let initial_workflow_files = self.collect_workflow_files()?;

        // Canonicalize the repo root once so incoming notify paths (which the
        // OS may deliver as canonicalized — e.g. macOS `/private/var` vs
        // `/var`, or a symlinked working copy) can be made root-relative
        // without silently failing every `strip_prefix`.
        let repo_root_canonical =
            std::fs::canonicalize(&self.repo_root).unwrap_or_else(|_| self.repo_root.clone());

        let debouncer = Arc::new(Debouncer::new(self.debounce_duration));
        let callback = Arc::new(on_cycle_complete);

        // Set up the notify watcher.
        //
        // The `watcher` binding is load-bearing: `RecommendedWatcher` stops
        // emitting events the moment it is dropped, so it MUST stay alive for
        // the entire duration of the watch loop below. Do not narrow this
        // scope or rebind it without preserving its lifetime.
        //
        // The callback pushes events directly into the shared debouncer set
        // (no intermediate bounded MPSC). This avoids silent drops under
        // burst load: a `HashSet::insert` on the debouncer's mutex is bounded
        // in cost, and the debouncer naturally deduplicates paths.
        let debouncer_for_callback = debouncer.clone();
        let repo_root_for_callback = repo_root_canonical.clone();
        let mut watcher = RecommendedWatcher::new(
            move |res: Result<Event, notify::Error>| {
                if let Ok(event) = res {
                    for path in event.paths {
                        if should_ignore_path(&path, &repo_root_for_callback) {
                            continue;
                        }
                        debouncer_for_callback.add_event(path);
                    }
                }
            },
            notify::Config::default(),
        )?;

        // Watch the repo root recursively
        watcher.watch(&self.repo_root, RecursiveMode::Recursive)?;

        wrkflw_logging::info(&format!(
            "Watching {} for changes (event={}, debounce={}ms)",
            self.repo_root.display(),
            self.event_name,
            self.debounce_duration.as_millis()
        ));

        let notify = debouncer.notifier();

        // Cache of compiled trigger configs keyed by workflow file path.
        // Invalidated only when a workflow file appears in the current
        // cycle's `changed_paths` set, so glob compilation doesn't repeat on
        // every file-save elsewhere in the repo.
        let mut trigger_cache: HashMap<PathBuf, WorkflowTriggerConfig> = HashMap::new();
        let mut workflow_files = initial_workflow_files;

        loop {
            // Only block on notification if no events are already pending.
            // This prevents losing events that accumulated during workflow execution.
            if !debouncer.has_pending() {
                notify.notified().await;
            }

            let changed_paths = debouncer.drain().await;
            if changed_paths.is_empty() {
                continue;
            }

            // Re-collect workflow files so newly added .yml files are picked up
            if let Ok(refreshed) = self.collect_workflow_files() {
                workflow_files = refreshed;
            }

            // Refresh the trigger cache: drop entries no longer in the
            // workflow list, and reparse new ones or any whose backing file
            // just changed.
            let active_set: HashSet<&PathBuf> = workflow_files.iter().collect();
            trigger_cache.retain(|k, _| active_set.contains(k));

            let changed_set: HashSet<&PathBuf> = changed_paths.iter().collect();
            let mut parse_failures = 0usize;
            for wf_path in &workflow_files {
                let needs_reparse =
                    !trigger_cache.contains_key(wf_path) || changed_set.contains(wf_path);
                if !needs_reparse {
                    continue;
                }
                match wrkflw_parser::workflow::parse_workflow(wf_path) {
                    Ok(wf) => {
                        match wrkflw_trigger_filter::parse_trigger_config(&wf, wf_path.clone()) {
                            Ok(cfg) => {
                                trigger_cache.insert(wf_path.clone(), cfg);
                            }
                            Err(e) => {
                                trigger_cache.remove(wf_path);
                                parse_failures += 1;
                                if self.verbose {
                                    wrkflw_logging::warning(&format!(
                                        "Failed to parse trigger config for {}: {}",
                                        wf_path.display(),
                                        e
                                    ));
                                }
                            }
                        }
                    }
                    Err(e) => {
                        trigger_cache.remove(wf_path);
                        parse_failures += 1;
                        if self.verbose {
                            wrkflw_logging::warning(&format!(
                                "Failed to parse {}: {}",
                                wf_path.display(),
                                e
                            ));
                        }
                    }
                }
            }

            // Build the borrowed view for evaluation.
            let configs_for_eval: Vec<&WorkflowTriggerConfig> = workflow_files
                .iter()
                .filter_map(|p| trigger_cache.get(p))
                .collect();

            if configs_for_eval.is_empty() && !workflow_files.is_empty() {
                wrkflw_logging::warning(&format!(
                    "No workflows are usable: all {} workflow file(s) failed to parse. \
                     Run with --verbose for details.",
                    workflow_files.len()
                ));
            } else if parse_failures > 0 && !self.verbose {
                wrkflw_logging::warning(&format!(
                    "{} workflow file(s) failed to parse and were skipped (use --verbose for details)",
                    parse_failures
                ));
            }

            // Convert paths to repo-relative, canonicalizing along the way so
            // that events delivered via symlinked paths or on macOS
            // (`/private/var` → `/var`) still line up with the canonical
            // repo root and don't drop out of the set.
            let changed_files: Vec<String> = changed_paths
                .iter()
                .filter_map(|p| {
                    let canonical = std::fs::canonicalize(p).unwrap_or_else(|_| p.clone());
                    canonical
                        .strip_prefix(&repo_root_canonical)
                        .ok()
                        .map(|rel| rel.to_string_lossy().to_string())
                })
                .collect();

            if changed_files.is_empty() {
                if self.verbose {
                    wrkflw_logging::warning(&format!(
                        "Ignored {} change event(s): none resolved under repo root {}",
                        changed_paths.len(),
                        repo_root_canonical.display()
                    ));
                }
                continue;
            }

            let event = self
                .evaluate_and_execute(&configs_for_eval, changed_files)
                .await;

            let cb = callback.clone();
            let _ = tokio::task::spawn_blocking(move || cb(event)).await;
        }
    }

    /// Evaluate triggers for the given (already parsed) workflows against the
    /// current git state, then execute the matching workflows with bounded
    /// concurrency.
    async fn evaluate_and_execute(
        &self,
        configs: &[&WorkflowTriggerConfig],
        changed_files: Vec<String>,
    ) -> WatchEvent {
        let context = wrkflw_trigger_filter::context_from_changed_files(
            &self.event_name,
            changed_files.clone(),
            Some(&self.repo_root),
        )
        .await
        .map(|mut ctx| {
            ctx.base_branch = self.base_branch.clone();
            ctx
        })
        .unwrap_or_else(|e| {
            wrkflw_logging::warning(&format!("Failed to build event context: {}", e));
            wrkflw_trigger_filter::EventContext {
                event_name: self.event_name.clone(),
                branch: None,
                base_branch: self.base_branch.clone(),
                tag: None,
                changed_files: changed_files.clone(),
                activity_type: None,
            }
        });

        let results = wrkflw_trigger_filter::filter_trigger_configs(configs, &context);

        let mut triggered = Vec::new();
        let mut skipped = Vec::new();
        let mut exec_futures = Vec::new();

        for result in &results {
            if result.matches {
                triggered.push(result.workflow_path.display().to_string());

                let config = self.config_template.clone();
                let wf_path = result.workflow_path.clone();
                exec_futures.push(async move {
                    match wrkflw_executor::execute_workflow(&wf_path, config).await {
                        Ok(exec_result) => {
                            if exec_result.failure_details.is_some() {
                                wrkflw_logging::error(&format!(
                                    "Workflow {} failed",
                                    wf_path.display()
                                ));
                            } else {
                                wrkflw_logging::info(&format!(
                                    "Workflow {} succeeded",
                                    wf_path.display()
                                ));
                            }
                        }
                        Err(e) => {
                            wrkflw_logging::error(&format!(
                                "Workflow {} error: {}",
                                wf_path.display(),
                                e
                            ));
                        }
                    }
                });
            } else {
                skipped.push(result.workflow_path.display().to_string());
            }
        }

        // Execute triggered workflows with bounded concurrency
        stream::iter(exec_futures)
            .buffer_unordered(self.max_concurrent_executions)
            .collect::<Vec<()>>()
            .await;

        WatchEvent {
            changed_files,
            triggered_workflows: triggered,
            skipped_workflows: skipped,
        }
    }
}

/// Returns `true` if a path falls inside any of the default ignore directories,
/// where "inside" means: a directory component (NOT the leaf filename) of the
/// path's `repo_root`-relative form matches one of the ignore names.
///
/// We deliberately skip the leaf component so a user file literally named
/// `target` (e.g. `scripts/target`) is not silently dropped — only paths that
/// have a `target/` (etc.) parent directory are filtered.
fn should_ignore_path(path: &Path, repo_root: &Path) -> bool {
    // Make the path repo-root-relative if possible. If it isn't under
    // repo_root we still apply the ignore set positionally rather than
    // returning false, so events from inside symlinks etc. still get filtered.
    let rel = path.strip_prefix(repo_root).unwrap_or(path);
    let components: Vec<_> = rel.components().collect();
    if components.is_empty() {
        return false;
    }
    // Iterate every component except the last (the leaf, which is presumed
    // to be a filename and shouldn't be matched against directory names).
    let last_idx = components.len() - 1;
    for (i, component) in components.iter().enumerate() {
        if i == last_idx {
            break;
        }
        if let std::path::Component::Normal(os) = component {
            if let Some(s) = os.to_str() {
                if DEFAULT_IGNORE_DIRS.contains(&s) {
                    return true;
                }
            }
        }
    }
    false
}

/// Find the git repository root from the current working directory.
pub fn find_repo_root() -> Option<PathBuf> {
    let output = std::process::Command::new("git")
        .args(["rev-parse", "--show-toplevel"])
        .output()
        .ok()?;

    if output.status.success() {
        let path = String::from_utf8_lossy(&output.stdout).trim().to_string();
        Some(PathBuf::from(path))
    } else {
        None
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn root() -> &'static Path {
        Path::new("/repo")
    }

    #[test]
    fn ignores_git_directory() {
        assert!(should_ignore_path(
            Path::new("/repo/.git/objects/pack/abc"),
            root()
        ));
    }

    #[test]
    fn ignores_target_directory() {
        assert!(should_ignore_path(
            Path::new("/repo/target/debug/deps/foo"),
            root()
        ));
    }

    #[test]
    fn ignores_node_modules() {
        assert!(should_ignore_path(
            Path::new("/repo/node_modules/pkg/index.js"),
            root()
        ));
    }

    #[test]
    fn does_not_ignore_src() {
        assert!(!should_ignore_path(Path::new("/repo/src/main.rs"), root()));
    }

    #[test]
    fn does_not_ignore_workflow_files() {
        assert!(!should_ignore_path(
            Path::new("/repo/.github/workflows/ci.yml"),
            root()
        ));
    }

    #[test]
    fn ignores_pycache() {
        assert!(should_ignore_path(
            Path::new("/repo/__pycache__/module.cpython-311.pyc"),
            root()
        ));
    }

    #[test]
    fn does_not_ignore_file_named_target() {
        // A file literally named `target` should not be filtered out;
        // only directories named `target/` count.
        assert!(!should_ignore_path(
            Path::new("/repo/scripts/target"),
            root()
        ));
    }

    #[test]
    fn does_not_ignore_file_named_build() {
        assert!(!should_ignore_path(Path::new("/repo/docs/build"), root()));
    }

    #[test]
    fn ignores_nested_target_subdirectory() {
        assert!(should_ignore_path(
            Path::new("/repo/crates/foo/target/debug/build/foo"),
            root()
        ));
    }
}
