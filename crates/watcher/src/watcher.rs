use crate::debouncer::Debouncer;
use crate::error::WatchError;
use notify::{Event, RecommendedWatcher, RecursiveMode, Watcher};
use std::path::PathBuf;
use std::sync::Arc;
use std::time::Duration;
use tokio::sync::mpsc;
use wrkflw_executor::ExecutionConfig;

/// A watch event containing the changed files and trigger evaluation results.
#[derive(Debug, Clone)]
pub struct WatchEvent {
    pub changed_files: Vec<String>,
    pub triggered_workflows: Vec<String>,
    pub skipped_workflows: Vec<String>,
}

/// Watches for filesystem changes and triggers workflow execution.
pub struct WorkflowWatcher {
    workflow_dir: PathBuf,
    repo_root: PathBuf,
    event_name: String,
    debounce_duration: Duration,
    config_template: ExecutionConfig,
    verbose: bool,
}

impl WorkflowWatcher {
    pub fn new(
        workflow_dir: PathBuf,
        repo_root: PathBuf,
        event_name: String,
        debounce_duration: Duration,
        config: ExecutionConfig,
        verbose: bool,
    ) -> Self {
        Self {
            workflow_dir,
            repo_root,
            event_name,
            debounce_duration,
            config_template: config,
            verbose,
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

    /// Start the watch loop. Calls `on_event` for each coalesced change set.
    /// This blocks until an error occurs or the process is interrupted.
    pub async fn run<F>(&self, on_event: F) -> Result<(), WatchError>
    where
        F: Fn(WatchEvent) + Send + 'static,
    {
        let workflow_files = self.collect_workflow_files()?;

        let (tx, mut rx) = mpsc::channel::<PathBuf>(256);
        let debouncer = Arc::new(Debouncer::new(self.debounce_duration));

        // Set up the notify watcher
        let tx_clone = tx.clone();
        let mut watcher = RecommendedWatcher::new(
            move |res: Result<Event, notify::Error>| {
                if let Ok(event) = res {
                    for path in event.paths {
                        // Skip .git directory
                        let path_str = path.to_string_lossy();
                        if path_str.contains("/.git/") || path_str.contains("\\.git\\") {
                            continue;
                        }
                        let _ = tx_clone.blocking_send(path);
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
        loop {
            // Block until the debouncer signals that an event arrived
            notify.notified().await;

            let changed_paths = debouncer.drain().await;
            if changed_paths.is_empty() {
                continue;
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

            let branch = wrkflw_trigger_filter::git::get_current_branch().await.ok();
            let tag = wrkflw_trigger_filter::git::get_current_tag()
                .await
                .ok()
                .flatten();

            let context = wrkflw_trigger_filter::EventContext {
                event_name: self.event_name.clone(),
                branch,
                tag,
                changed_files: changed_files.clone(),
            };

            let mut triggered = Vec::new();
            let mut skipped = Vec::new();

            for wf_path in &workflow_files {
                let workflow = match wrkflw_parser::workflow::parse_workflow(wf_path) {
                    Ok(w) => w,
                    Err(e) => {
                        if self.verbose {
                            wrkflw_logging::warning(&format!(
                                "Failed to parse {}: {}",
                                wf_path.display(),
                                e
                            ));
                        }
                        continue;
                    }
                };

                let trigger_config =
                    match wrkflw_trigger_filter::parse_trigger_config(&workflow, wf_path.clone()) {
                        Ok(c) => c,
                        Err(e) => {
                            if self.verbose {
                                wrkflw_logging::warning(&format!(
                                    "Failed to parse triggers for {}: {}",
                                    wf_path.display(),
                                    e
                                ));
                            }
                            continue;
                        }
                    };

                let result = wrkflw_trigger_filter::evaluate_trigger(&trigger_config, &context);

                if result.matches {
                    triggered.push(wf_path.display().to_string());

                    let config = ExecutionConfig {
                        event_filter: None, // Already filtered
                        ..self.config_template.clone()
                    };

                    match wrkflw_executor::execute_workflow(wf_path, config).await {
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
                } else {
                    skipped.push(wf_path.display().to_string());
                }
            }

            on_event(WatchEvent {
                changed_files,
                triggered_workflows: triggered,
                skipped_workflows: skipped,
            });
        }
    }
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
