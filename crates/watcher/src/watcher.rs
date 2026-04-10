use crate::debouncer::Debouncer;
use crate::error::WatchError;
use futures::stream::{self, StreamExt};
use notify::{Event, RecommendedWatcher, RecursiveMode, Watcher};
use std::path::PathBuf;
use std::sync::Arc;
use std::time::Duration;
use tokio::sync::mpsc;
use wrkflw_executor::ExecutionConfig;

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
    debounce_duration: Duration,
    config_template: ExecutionConfig,
    verbose: bool,
    max_concurrent_executions: usize,
}

impl WorkflowWatcher {
    pub fn new(
        workflow_dir: PathBuf,
        repo_root: PathBuf,
        event_name: String,
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
        let workflow_files = self.collect_workflow_files()?;

        let (tx, mut rx) = mpsc::channel::<PathBuf>(256);
        let debouncer = Arc::new(Debouncer::new(self.debounce_duration));
        let callback = Arc::new(on_cycle_complete);

        // Set up the notify watcher.
        //
        // The `watcher` binding is load-bearing: `RecommendedWatcher` stops
        // emitting events the moment it is dropped, so it MUST stay alive for
        // the entire duration of the watch loop below. Do not narrow this
        // scope or rebind it without preserving its lifetime.
        let tx_clone = tx.clone();
        let mut watcher = RecommendedWatcher::new(
            move |res: Result<Event, notify::Error>| {
                if let Ok(event) = res {
                    for path in event.paths {
                        if should_ignore_path(&path) {
                            continue;
                        }
                        // Use try_send to avoid blocking the OS filesystem event
                        // thread when the channel is full.  Dropping an event is
                        // acceptable because the debouncer coalesces anyway.
                        let _ = tx_clone.try_send(path);
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

        let debouncer_clone = debouncer.clone();
        let notify = debouncer.notifier();

        // Spawn a task to receive filesystem events and feed the debouncer
        tokio::spawn(async move {
            while let Some(path) = rx.recv().await {
                debouncer_clone.add_event(path);
            }
        });

        // Main loop: wait for notification, debounce, evaluate triggers, execute
        let mut workflow_files = workflow_files;
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

            // Convert to relative paths
            let changed_files: Vec<String> = changed_paths
                .iter()
                .filter_map(|p| {
                    p.strip_prefix(&self.repo_root)
                        .ok()
                        .map(|rel| rel.to_string_lossy().to_string())
                })
                .collect();

            if changed_files.is_empty() {
                continue;
            }

            let event = self
                .evaluate_and_execute(&workflow_files, changed_files)
                .await;

            let cb = callback.clone();
            let _ = tokio::task::spawn_blocking(move || cb(event)).await;
        }
    }

    /// Evaluate triggers for all workflow files against the current git state,
    /// then execute the matching workflows with bounded concurrency.
    async fn evaluate_and_execute(
        &self,
        workflow_files: &[PathBuf],
        changed_files: Vec<String>,
    ) -> WatchEvent {
        let context = wrkflw_trigger_filter::context_from_changed_files(
            &self.event_name,
            changed_files.clone(),
        )
        .await
        .unwrap_or_else(|e| {
            wrkflw_logging::warning(&format!("Failed to build event context: {}", e));
            wrkflw_trigger_filter::EventContext {
                event_name: self.event_name.clone(),
                branch: None,
                tag: None,
                changed_files: changed_files.clone(),
                activity_type: None,
            }
        });

        // Parse all workflow files, logging failures
        let workflows: Vec<_> = workflow_files
            .iter()
            .filter_map(
                |wf_path| match wrkflw_parser::workflow::parse_workflow(wf_path) {
                    Ok(wf) => Some((wf_path.clone(), wf)),
                    Err(e) => {
                        if self.verbose {
                            wrkflw_logging::warning(&format!(
                                "Failed to parse {}: {}",
                                wf_path.display(),
                                e
                            ));
                        }
                        None
                    }
                },
            )
            .collect();

        let results = wrkflw_trigger_filter::filter_workflows(&workflows, &context);

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

/// Returns `true` if a path falls inside any of the default ignore directories.
fn should_ignore_path(path: &std::path::Path) -> bool {
    for component in path.components() {
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

    #[test]
    fn ignores_git_directory() {
        assert!(should_ignore_path(std::path::Path::new(
            "/repo/.git/objects/pack/abc"
        )));
    }

    #[test]
    fn ignores_target_directory() {
        assert!(should_ignore_path(std::path::Path::new(
            "/repo/target/debug/deps/foo"
        )));
    }

    #[test]
    fn ignores_node_modules() {
        assert!(should_ignore_path(std::path::Path::new(
            "/repo/node_modules/pkg/index.js"
        )));
    }

    #[test]
    fn does_not_ignore_src() {
        assert!(!should_ignore_path(std::path::Path::new(
            "/repo/src/main.rs"
        )));
    }

    #[test]
    fn does_not_ignore_workflow_files() {
        assert!(!should_ignore_path(std::path::Path::new(
            "/repo/.github/workflows/ci.yml"
        )));
    }

    #[test]
    fn ignores_pycache() {
        assert!(should_ignore_path(std::path::Path::new(
            "/repo/__pycache__/module.cpython-311.pyc"
        )));
    }
}
