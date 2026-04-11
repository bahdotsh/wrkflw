use bollard::Docker;
use clap::{Parser, Subcommand, ValueEnum};
use std::collections::HashMap;
use std::path::Path;
use std::path::PathBuf;

#[derive(Debug, Clone, ValueEnum)]
enum RuntimeChoice {
    /// Use Docker containers for isolation
    Docker,
    /// Use Podman containers for isolation
    Podman,
    /// Use process emulation mode (no containers, UNSAFE)
    Emulation,
    /// Use secure emulation mode with sandboxing (recommended for untrusted code)
    SecureEmulation,
}

impl From<RuntimeChoice> for wrkflw_executor::RuntimeType {
    fn from(choice: RuntimeChoice) -> Self {
        match choice {
            RuntimeChoice::Docker => wrkflw_executor::RuntimeType::Docker,
            RuntimeChoice::Podman => wrkflw_executor::RuntimeType::Podman,
            RuntimeChoice::Emulation => wrkflw_executor::RuntimeType::Emulation,
            RuntimeChoice::SecureEmulation => wrkflw_executor::RuntimeType::SecureEmulation,
        }
    }
}

#[derive(Debug, Parser)]
#[command(
    name = "wrkflw",
    about = "GitHub & GitLab CI/CD validator and executor",
    version,
    long_about = "A CI/CD validator and executor that runs workflows locally.\n\nExamples:\n  wrkflw validate                             # Validate all workflows in .github/workflows\n  wrkflw run .github/workflows/build.yml      # Run a specific workflow\n  wrkflw run .gitlab-ci.yml                   # Run a GitLab CI pipeline\n  wrkflw --verbose run .github/workflows/build.yml  # Run with more output\n  wrkflw --debug run .github/workflows/build.yml    # Run with detailed debug information\n  wrkflw run --runtime emulation .github/workflows/build.yml  # Use emulation mode instead of containers\n  wrkflw run --runtime podman .github/workflows/build.yml     # Use Podman instead of Docker\n  wrkflw run --preserve-containers-on-failure .github/workflows/build.yml  # Keep failed containers for debugging"
)]
struct Wrkflw {
    #[command(subcommand)]
    command: Option<Commands>,

    /// Run in verbose mode with detailed output
    #[arg(short, long, global = true)]
    verbose: bool,

    /// Run in debug mode with extensive execution details
    #[arg(short, long, global = true)]
    debug: bool,
}

#[derive(Debug, Subcommand)]
enum Commands {
    /// Validate workflow or pipeline files
    Validate {
        /// Path(s) to workflow/pipeline file(s) or directory(ies) (defaults to .github/workflows if none provided)
        #[arg(value_name = "path", num_args = 0..)]
        paths: Vec<PathBuf>,

        /// Explicitly validate as GitLab CI/CD pipeline
        #[arg(long)]
        gitlab: bool,

        /// Set exit code to 1 on validation failure
        #[arg(long = "exit-code", default_value_t = true)]
        exit_code: bool,

        /// Don't set exit code to 1 on validation failure (overrides --exit-code)
        #[arg(long = "no-exit-code", conflicts_with = "exit_code")]
        no_exit_code: bool,
    },

    /// Execute workflow or pipeline files locally
    Run {
        /// Path to workflow/pipeline file to execute
        path: PathBuf,

        /// Container runtime to use (docker, podman, emulation, secure-emulation)
        #[arg(short, long, value_enum, default_value = "docker")]
        runtime: RuntimeChoice,

        /// Show 'Would execute GitHub action' messages in emulation mode
        #[arg(long, default_value_t = false)]
        show_action_messages: bool,

        /// Preserve Docker containers on failure for debugging (Docker mode only)
        #[arg(long)]
        preserve_containers_on_failure: bool,

        /// Explicitly run as GitLab CI/CD pipeline
        #[arg(long)]
        gitlab: bool,

        /// Run only a specific job by name
        #[arg(long)]
        job: Option<String>,

        /// Simulate a specific event type for trigger filtering (e.g., push, pull_request)
        #[arg(long)]
        event: Option<String>,

        /// Use git diff to determine changed files for trigger filtering
        #[arg(long)]
        diff: bool,

        /// Manually specify changed files (comma-separated) for trigger filtering
        #[arg(long, value_delimiter = ',')]
        changed_files: Option<Vec<String>>,

        /// Base ref for diff comparison.
        ///
        /// Omit to auto-detect: tries `origin/HEAD`, then `main`/`master`,
        /// then `HEAD~1`. Pass `HEAD` to compare working tree against the
        /// last commit (uncommitted changes only).
        #[arg(long)]
        diff_base: Option<String>,

        /// Head ref for diff comparison (default: working tree)
        #[arg(long)]
        diff_head: Option<String>,

        /// Target/base branch for pull_request events (e.g. main).
        /// GitHub Actions evaluates `branches:` filters on `pull_request`
        /// against the base branch — set this to simulate a PR locally.
        #[arg(long)]
        base_branch: Option<String>,

        /// Activity type for events that support it (e.g. `opened`,
        /// `synchronize` for pull_request). Required when simulating an
        /// event whose workflows use `types:` filters — without it, every
        /// such workflow is reported as skipped for "no activity type".
        #[arg(long)]
        activity_type: Option<String>,
    },

    /// Watch for file changes and re-run affected workflows
    Watch {
        /// Path to workflow file or directory (defaults to .github/workflows)
        path: Option<PathBuf>,

        /// Container runtime to use (docker, podman, emulation, secure-emulation)
        #[arg(short, long, value_enum, default_value = "docker")]
        runtime: RuntimeChoice,

        /// Debounce interval in milliseconds
        #[arg(long, default_value = "500")]
        debounce: u64,

        /// Event type to simulate (default: push)
        #[arg(long, default_value = "push")]
        event: String,

        /// Show 'Would execute GitHub action' messages in emulation mode
        #[arg(long, default_value_t = false)]
        show_action_messages: bool,

        /// Preserve Docker containers on failure for debugging (Docker mode only)
        #[arg(long)]
        preserve_containers_on_failure: bool,

        /// Maximum number of workflows that may execute concurrently per cycle
        #[arg(long, default_value_t = wrkflw_watcher::DEFAULT_MAX_CONCURRENT_EXECUTIONS)]
        max_concurrency: usize,

        /// Target/base branch for pull_request events (e.g. main).
        /// Required if you watch with `--event pull_request` and any workflow
        /// uses `branches:` to constrain the target branch.
        #[arg(long)]
        base_branch: Option<String>,

        /// Activity type for events that support it (e.g. `opened`,
        /// `synchronize` for pull_request). Required when watching an
        /// event whose workflows use `types:` filters — without it, every
        /// such workflow is silently rejected for "no activity type".
        #[arg(long)]
        activity_type: Option<String>,
    },

    /// Open TUI interface to manage workflows
    #[cfg(feature = "tui")]
    Tui {
        /// Path to workflow file or directory (defaults to .github/workflows)
        path: Option<PathBuf>,

        /// Container runtime to use (docker, podman, emulation, secure-emulation)
        #[arg(short, long, value_enum, default_value = "docker")]
        runtime: RuntimeChoice,

        /// Show 'Would execute GitHub action' messages in emulation mode
        #[arg(long, default_value_t = false)]
        show_action_messages: bool,

        /// Preserve Docker containers on failure for debugging (Docker mode only)
        #[arg(long)]
        preserve_containers_on_failure: bool,
    },

    /// Trigger a GitHub workflow remotely
    Trigger {
        /// Name of the workflow file (without .yml extension)
        workflow: String,

        /// Branch to run the workflow on
        #[arg(short, long)]
        branch: Option<String>,

        /// Key-value inputs for the workflow in format key=value
        #[arg(short, long, value_parser = parse_key_val)]
        input: Option<Vec<(String, String)>>,
    },

    /// Trigger a GitLab pipeline remotely
    TriggerGitlab {
        /// Branch to run the pipeline on
        #[arg(short, long)]
        branch: Option<String>,

        /// Key-value variables for the pipeline in format key=value
        #[arg(short = 'V', long, value_parser = parse_key_val)]
        variable: Option<Vec<(String, String)>>,
    },

    /// List available workflows and pipelines
    List {
        /// Show jobs within each workflow/pipeline
        #[arg(long)]
        jobs: bool,
    },
}

// Parser function for key-value pairs
fn parse_key_val(s: &str) -> Result<(String, String), String> {
    let pos = s
        .find('=')
        .ok_or_else(|| format!("invalid KEY=value: no `=` found in `{}`", s))?;

    Ok((s[..pos].to_string(), s[pos + 1..].to_string()))
}

// Make this function public for testing? Or move to a utils/cleanup mod?
// Or call wrkflw_executor::cleanup and wrkflw_runtime::cleanup directly?
// Let's try calling them directly for now.
async fn cleanup_on_exit() {
    // Clean up Docker resources if available, but don't let it block indefinitely
    match tokio::time::timeout(std::time::Duration::from_secs(3), async {
        match Docker::connect_with_local_defaults() {
            Ok(docker) => {
                // Assuming cleanup_resources exists in executor crate
                wrkflw_executor::cleanup_resources(&docker).await;
            }
            Err(_) => {
                // Docker not available
                wrkflw_logging::info("Docker not available, skipping Docker cleanup");
            }
        }
    })
    .await
    {
        Ok(_) => wrkflw_logging::debug("Docker cleanup completed successfully"),
        Err(_) => wrkflw_logging::warning(
            "Docker cleanup timed out after 3 seconds, continuing with shutdown",
        ),
    }

    // Always clean up emulation resources
    match tokio::time::timeout(
        std::time::Duration::from_secs(2),
        // Assuming cleanup_resources exists in wrkflw_runtime::emulation module
        wrkflw_runtime::emulation::cleanup_resources(),
    )
    .await
    {
        Ok(_) => wrkflw_logging::debug("Emulation cleanup completed successfully"),
        Err(_) => wrkflw_logging::warning("Emulation cleanup timed out, continuing with shutdown"),
    }

    wrkflw_logging::info("Resource cleanup completed");
}

async fn handle_signals() {
    // Set up a hard exit timer in case cleanup takes too long
    // This ensures the app always exits even if Docker operations are stuck
    let hard_exit_time = std::time::Duration::from_secs(10);

    // Wait for Ctrl+C
    match tokio::signal::ctrl_c().await {
        Ok(_) => {
            println!("Received Ctrl+C, shutting down and cleaning up...");
        }
        Err(e) => {
            // Log the error but continue with cleanup
            eprintln!("Warning: Failed to properly listen for ctrl+c event: {}", e);
            println!("Shutting down and cleaning up...");
        }
    }

    // Set up a watchdog thread that will force exit if cleanup takes too long
    // This is important because Docker operations can sometimes hang indefinitely
    let _ = std::thread::spawn(move || {
        std::thread::sleep(hard_exit_time);
        eprintln!(
            "Cleanup taking too long (over {} seconds), forcing exit...",
            hard_exit_time.as_secs()
        );
        wrkflw_logging::error("Forced exit due to cleanup timeout");
        std::process::exit(1);
    });

    // Clean up containers
    cleanup_on_exit().await;

    // Exit with success status - the force exit thread will be terminated automatically
    std::process::exit(0);
}

/// Determines if a file is a GitLab CI/CD pipeline based on its name and content
fn is_gitlab_pipeline(path: &Path) -> bool {
    // First check the file name
    if let Some(file_name) = path.file_name() {
        if let Some(file_name_str) = file_name.to_str() {
            if file_name_str == ".gitlab-ci.yml" || file_name_str.ends_with("gitlab-ci.yml") {
                return true;
            }
        }
    }

    // Check if file is in .gitlab/ci directory
    if let Some(parent) = path.parent() {
        if let Some(parent_str) = parent.to_str() {
            if parent_str.ends_with(".gitlab/ci")
                && path
                    .extension()
                    .is_some_and(|ext| ext == "yml" || ext == "yaml")
            {
                return true;
            }
        }
    }

    // If file exists, check the content
    if path.exists() {
        if let Ok(content) = std::fs::read_to_string(path) {
            // GitLab CI/CD pipelines typically have stages, before_script, after_script at the top level
            if content.contains("stages:")
                || content.contains("before_script:")
                || content.contains("after_script:")
            {
                // Check for GitHub Actions specific keys that would indicate it's not GitLab
                if !content.contains("on:")
                    && !content.contains("runs-on:")
                    && !content.contains("uses:")
                {
                    return true;
                }
            }
        }
    }

    false
}

#[tokio::main]
async fn main() {
    // Gracefully handle Broken pipe (EPIPE) when output is piped (e.g., to `head`)
    let default_panic_hook = std::panic::take_hook();
    std::panic::set_hook(Box::new(move |info| {
        let mut is_broken_pipe = false;
        if let Some(s) = info.payload().downcast_ref::<&str>() {
            if s.contains("Broken pipe") {
                is_broken_pipe = true;
            }
        }
        if let Some(s) = info.payload().downcast_ref::<String>() {
            if s.contains("Broken pipe") {
                is_broken_pipe = true;
            }
        }
        if is_broken_pipe {
            // Treat as a successful, short-circuited exit
            std::process::exit(0);
        }
        // Fallback to the default hook for all other panics
        default_panic_hook(info);
    }));

    let cli = Wrkflw::parse();
    let verbose = cli.verbose;
    let debug = cli.debug;

    // Set log level based on command line flags
    if debug {
        wrkflw_logging::set_log_level(wrkflw_logging::LogLevel::Debug);
        wrkflw_logging::debug("Debug mode enabled - showing detailed logs");
    } else if verbose {
        wrkflw_logging::set_log_level(wrkflw_logging::LogLevel::Info);
        wrkflw_logging::info("Verbose mode enabled");
    } else {
        wrkflw_logging::set_log_level(wrkflw_logging::LogLevel::Warning);
    }

    // Setup a Ctrl+C handler that runs in the background
    tokio::spawn(handle_signals());

    match &cli.command {
        Some(Commands::Validate {
            paths,
            gitlab,
            exit_code,
            no_exit_code,
        }) => {
            // Determine the paths to validate (default to .github/workflows when none provided)
            let validate_paths: Vec<PathBuf> = if paths.is_empty() {
                vec![PathBuf::from(".github/workflows")]
            } else {
                paths.clone()
            };

            // Determine if we're validating a GitLab pipeline based on the --gitlab flag or file detection
            let force_gitlab = *gitlab;
            let mut validation_failed = false;

            for validate_path in validate_paths {
                // Check if the path exists; if not, mark failure but continue
                if !validate_path.exists() {
                    eprintln!("Error: Path does not exist: {}", validate_path.display());
                    validation_failed = true;
                    continue;
                }

                if validate_path.is_dir() {
                    // Validate all workflow files in the directory
                    let rd = match std::fs::read_dir(&validate_path) {
                        Ok(rd) => rd,
                        Err(e) => {
                            eprintln!(
                                "Failed to read directory {}: {}",
                                validate_path.display(),
                                e
                            );
                            validation_failed = true;
                            continue;
                        }
                    };
                    let entries = rd
                        .filter_map(|entry| entry.ok())
                        .filter(|entry| {
                            entry.path().is_file()
                                && entry
                                    .path()
                                    .extension()
                                    .is_some_and(|ext| ext == "yml" || ext == "yaml")
                        })
                        .collect::<Vec<_>>();

                    println!(
                        "Validating {} workflow file(s) in {}...",
                        entries.len(),
                        validate_path.display()
                    );

                    for entry in entries {
                        let path = entry.path();
                        let is_gitlab = force_gitlab || is_gitlab_pipeline(&path);

                        let file_failed = if is_gitlab {
                            validate_gitlab_pipeline(&path, verbose)
                        } else {
                            validate_github_workflow(&path, verbose)
                        };

                        if file_failed {
                            validation_failed = true;
                        }
                    }
                } else {
                    // Validate a single workflow file
                    let is_gitlab = force_gitlab || is_gitlab_pipeline(&validate_path);

                    let file_failed = if is_gitlab {
                        validate_gitlab_pipeline(&validate_path, verbose)
                    } else {
                        validate_github_workflow(&validate_path, verbose)
                    };

                    if file_failed {
                        validation_failed = true;
                    }
                }
            }

            // Set exit code if validation failed and exit_code flag is true (and no_exit_code is false)
            if validation_failed && *exit_code && !*no_exit_code {
                std::process::exit(1);
            }
        }
        Some(Commands::Run {
            path,
            runtime,
            show_action_messages,
            preserve_containers_on_failure,
            gitlab,
            job,
            event,
            diff,
            changed_files,
            diff_base,
            diff_head,
            base_branch,
            activity_type,
        }) => {
            // Determine workflow type up front so the trigger prefilter
            // can short-circuit for GitLab pipelines with a clear error.
            // Previously the prefilter ran first and tried to parse the
            // file as a GitHub workflow, which surfaced a confusing
            // `Error parsing workflow: ...` from deep in the YAML parser.
            let is_gitlab = *gitlab || is_gitlab_pipeline(path);

            // Evaluate trigger filter at the call site before executing
            if *diff || event.is_some() || changed_files.is_some() {
                if is_gitlab {
                    eprintln!(
                        "Error: --diff, --event, and --changed-files are only \
                         supported for GitHub Actions workflows.\n\
                         {} appears to be a GitLab CI pipeline — trigger \
                         filtering is GitHub Actions-specific and cannot be \
                         evaluated against GitLab `rules:`/`only:`/`except:`.",
                        path.display()
                    );
                    std::process::exit(1);
                }
                run_trigger_prefilter_or_exit(PrefilterRequest {
                    workflow_path: path,
                    event: event.as_ref(),
                    diff: *diff,
                    changed_files: changed_files.as_ref(),
                    diff_base: diff_base.as_deref(),
                    diff_head: diff_head.as_ref(),
                    base_branch: base_branch.as_ref(),
                    activity_type: activity_type.as_ref(),
                    verbose,
                })
                .await;
            }

            // Create execution configuration
            let config = wrkflw_executor::ExecutionConfig {
                runtime_type: runtime.clone().into(),
                verbose,
                preserve_containers_on_failure: *preserve_containers_on_failure,
                secrets_config: None, // Use default secrets configuration
                show_action_messages: *show_action_messages,
                target_job: job.clone(),
            };
            let workflow_type = if is_gitlab {
                "GitLab CI pipeline"
            } else {
                "GitHub workflow"
            };

            wrkflw_logging::info(&format!("Running {} at: {}", workflow_type, path.display()));

            // Execute the workflow
            let result = wrkflw_executor::execute_workflow(path, config)
                .await
                .unwrap_or_else(|e| {
                    eprintln!("Error executing workflow: {}", e);
                    std::process::exit(1);
                });

            // Print execution summary
            use wrkflw_ui::cli_style;
            if result.failure_details.is_some() {
                eprintln!("{}", cli_style::error("Workflow execution failed:"));
                if let Some(details) = result.failure_details {
                    if verbose {
                        eprintln!("{}", details);
                    } else {
                        let simplified_error = details
                            .lines()
                            .filter(|line| {
                                line.contains(wrkflw_logging::symbols::FAILURE)
                                    || line.trim().starts_with("Error:")
                            })
                            .take(5)
                            .collect::<Vec<&str>>()
                            .join("\n");

                        eprintln!("{}", simplified_error);

                        if details.lines().count() > 5 {
                            eprintln!(
                                "\n{}",
                                cli_style::dim("Use --verbose flag to see full error details")
                            );
                        }
                    }
                }
                std::process::exit(1);
            } else {
                println!(
                    "{}",
                    cli_style::success("Workflow execution completed successfully!")
                );

                println!("{}", cli_style::section("Job summary"));
                for job in result.jobs {
                    match job.status {
                        wrkflw_executor::JobStatus::Success => {
                            println!("  {}", cli_style::job_success(&job.name))
                        }
                        wrkflw_executor::JobStatus::Failure => {
                            println!("  {}", cli_style::job_failure(&job.name))
                        }
                        wrkflw_executor::JobStatus::Skipped => {
                            println!("  {}", cli_style::job_skipped(&job.name))
                        }
                    }

                    for step in job.steps {
                        match step.status {
                            wrkflw_executor::StepStatus::Success => {
                                println!("{}", cli_style::step_success(&step.name))
                            }
                            wrkflw_executor::StepStatus::Failure => {
                                println!("{}", cli_style::step_failure(&step.name));

                                if !verbose {
                                    let error_lines = step
                                        .output
                                        .lines()
                                        .filter(|line| {
                                            line.contains("error:")
                                                || line.contains("Error:")
                                                || line.trim().starts_with("Exit code:")
                                                || line.contains("failed")
                                        })
                                        .take(3)
                                        .collect::<Vec<&str>>();

                                    if !error_lines.is_empty() {
                                        for line in error_lines {
                                            println!("{}", cli_style::indent(line.trim()));
                                        }

                                        if step.output.lines().count() > 3 {
                                            println!(
                                                "{}",
                                                cli_style::indent(
                                                    "(Use --verbose for full output)"
                                                )
                                            );
                                        }
                                    }
                                }
                            }
                            wrkflw_executor::StepStatus::Skipped => {
                                println!("{}", cli_style::step_skipped(&step.name))
                            }
                        }
                    }
                }
            }

            // Cleanup is handled automatically via the signal handler
        }
        Some(Commands::Watch {
            path,
            runtime,
            debounce,
            event,
            show_action_messages,
            preserve_containers_on_failure,
            max_concurrency,
            base_branch,
            activity_type,
        }) => {
            let workflow_dir = path
                .clone()
                .unwrap_or_else(|| PathBuf::from(".github/workflows"));
            if !workflow_dir.exists() {
                eprintln!(
                    "Error: workflow directory not found: {}",
                    workflow_dir.display()
                );
                std::process::exit(1);
            }

            // `find_repo_root_detailed` shells out to `git rev-parse`
            // synchronously and is NOT wrapped in the trigger-filter's
            // GIT_COMMAND_TIMEOUT, so a hung git (credential prompt,
            // stuck network mount) would block the reactor if we called
            // it directly. Move it onto the blocking pool to keep the
            // tokio runtime responsive.
            //
            // We use the `_detailed` variant so each failure mode
            // (missing binary / timeout / not-in-repo / other) renders
            // its own diagnostic. The legacy `Option`-returning wrapper
            // collapsed all four into "not inside a git repository",
            // which is actively wrong for the first three and sent
            // users down the wrong fix path.
            let repo_root =
                match tokio::task::spawn_blocking(wrkflw_trigger_filter::find_repo_root_detailed)
                    .await
                {
                    Ok(Ok(p)) => p,
                    Ok(Err(e)) => {
                        eprintln!("Error: {}", e);
                        std::process::exit(1);
                    }
                    Err(join_err) => {
                        eprintln!("Error: find_repo_root task panicked: {}", join_err);
                        std::process::exit(1);
                    }
                };

            let debounce_duration = std::time::Duration::from_millis(*debounce);

            let config = wrkflw_executor::ExecutionConfig {
                runtime_type: runtime.clone().into(),
                verbose,
                preserve_containers_on_failure: *preserve_containers_on_failure,
                secrets_config: None,
                show_action_messages: *show_action_messages,
                target_job: None,
            };

            use wrkflw_ui::cli_style;
            println!(
                "{}",
                cli_style::success(&format!(
                    "Watching for changes (event={}, debounce={}ms)... Press Ctrl+C to stop.",
                    event, debounce
                ))
            );

            // Warn loudly if the user is watching pull_request without a
            // base branch — branches: filters will reject every workflow.
            if (event == "pull_request" || event == "pull_request_target") && base_branch.is_none()
            {
                wrkflw_logging::warning(
                    "Watching pull_request without --base-branch: any workflow with a \
                     `branches:` filter will be silently skipped because GitHub Actions \
                     evaluates that filter against the PR target branch.",
                );
            }

            let watcher_cfg = wrkflw_watcher::WatcherConfig::new(workflow_dir, repo_root, config)
                .with_event(event.clone())
                .with_base_branch(base_branch.clone())
                .with_activity_type(activity_type.clone())
                .with_debounce(debounce_duration)
                .with_verbose(verbose)
                .with_max_concurrency(*max_concurrency);
            let watcher = wrkflw_watcher::WorkflowWatcher::from_config(watcher_cfg);

            // Validate workflow files exist before starting
            if let Err(e) = watcher.collect_workflow_files().await {
                eprintln!("Error: {}", e);
                std::process::exit(1);
            }

            watcher
                .run(|watch_event| {
                    println!(
                        "\n{}",
                        cli_style::section(&format!(
                            "Change detected ({} file(s) changed, {} triggered, {} skipped)",
                            watch_event.changed_files.len(),
                            watch_event.triggered_workflows.len(),
                            watch_event.skipped_workflows.len(),
                        ))
                    );
                    // Surface degraded cycles loudly: if the watcher
                    // could not build a git event context, the trigger
                    // results are not authoritative and the user needs
                    // to know why before they assume "0 triggered".
                    if let Some(reason) = &watch_event.error {
                        eprintln!("  {} {}", cli_style::error("ERROR"), reason);
                    }
                    for wf in &watch_event.triggered_workflows {
                        println!("  {} {}", cli_style::success("TRIGGERED"), wf);
                    }
                    for wf in &watch_event.skipped_workflows {
                        println!("  {} {}", cli_style::dim("SKIPPED"), wf);
                    }
                })
                .await
                .unwrap_or_else(|e| {
                    eprintln!("Watch error: {}", e);
                    std::process::exit(1);
                });
        }
        Some(Commands::TriggerGitlab { branch, variable }) => {
            // Convert optional Vec<(String, String)> to Option<HashMap<String, String>>
            let variables = variable
                .as_ref()
                .map(|v| v.iter().cloned().collect::<HashMap<String, String>>());

            // Trigger the pipeline
            if let Err(e) = wrkflw_gitlab::trigger_pipeline(branch.as_deref(), variables).await {
                eprintln!("Error triggering GitLab pipeline: {}", e);
                std::process::exit(1);
            }
        }
        #[cfg(feature = "tui")]
        Some(Commands::Tui {
            path,
            runtime,
            show_action_messages,
            preserve_containers_on_failure,
        }) => {
            // Set runtime type based on the runtime choice
            let runtime_type = runtime.clone().into();

            // Call the TUI implementation from the ui crate
            if let Err(e) = wrkflw_ui::run_wrkflw_tui(
                path.as_ref(),
                runtime_type,
                verbose,
                *preserve_containers_on_failure,
                *show_action_messages,
            )
            .await
            {
                eprintln!("Error running TUI: {}", e);
                std::process::exit(1);
            }
        }
        Some(Commands::Trigger {
            workflow,
            branch,
            input,
        }) => {
            // Convert optional Vec<(String, String)> to Option<HashMap<String, String>>
            let inputs = input
                .as_ref()
                .map(|i| i.iter().cloned().collect::<HashMap<String, String>>());

            // Trigger the workflow
            if let Err(e) =
                wrkflw_github::trigger_workflow(workflow, branch.as_deref(), inputs).await
            {
                eprintln!("Error triggering GitHub workflow: {}", e);
                std::process::exit(1);
            }
        }
        Some(Commands::List { jobs }) => {
            list_workflows_and_pipelines(verbose, *jobs);
        }
        None => {
            #[cfg(feature = "tui")]
            {
                // Launch TUI by default when no command is provided
                let runtime_type = wrkflw_executor::RuntimeType::Docker;

                // Call the TUI implementation from the ui crate with default path
                if let Err(e) =
                    wrkflw_ui::run_wrkflw_tui(None, runtime_type, verbose, false, false).await
                {
                    eprintln!("Error running TUI: {}", e);
                    std::process::exit(1);
                }
            }
            #[cfg(not(feature = "tui"))]
            {
                use clap::CommandFactory;
                Wrkflw::command().print_help().unwrap();
                println!();
            }
        }
    }
}

/// Bundled inputs for the `wrkflw run` trigger prefilter.
///
/// Grouping these into a single struct collapses the previous 8-argument
/// `run_trigger_prefilter_or_exit` into a more reviewable shape, and lets
/// the orchestrator pass the request through to its private helpers
/// (`build_event_context`, `apply_base_branch`) without dragging an
/// ever-growing positional list.
struct PrefilterRequest<'a> {
    workflow_path: &'a Path,
    event: Option<&'a String>,
    diff: bool,
    changed_files: Option<&'a Vec<String>>,
    /// `None` means the user did not pass `--diff-base` and we should fall
    /// back to `auto_detect_context_default_base` (origin/HEAD → main →
    /// master → HEAD~1). Previously this was a `&str` defaulting to
    /// `"HEAD"`, which made the smart detection unreachable from the CLI
    /// and silently restricted `--diff` to uncommitted-only changes.
    diff_base: Option<&'a str>,
    diff_head: Option<&'a String>,
    base_branch: Option<&'a String>,
    activity_type: Option<&'a String>,
    verbose: bool,
}

/// Build an event context from the user's CLI flags and short-circuit
/// the run if the workflow's triggers do not match.
///
/// Exits the process on failure or skip; returns normally only when the
/// workflow should run.
async fn run_trigger_prefilter_or_exit(req: PrefilterRequest<'_>) {
    // `wrkflw run` expects a single workflow file. Catch directory paths up
    // front with a clear error; otherwise the user sees a confusing
    // "Error parsing workflow" from the YAML parser further down.
    if !req.workflow_path.is_file() {
        if req.workflow_path.is_dir() {
            eprintln!(
                "Error: --diff/--event/--changed-files require a single workflow file, not a directory.\n\
                 Hint: point at a specific .yml file, or use `wrkflw watch {}` for directory-wide watching.",
                req.workflow_path.display()
            );
        } else {
            eprintln!(
                "Error: workflow file not found: {}",
                req.workflow_path.display()
            );
        }
        std::process::exit(1);
    }

    let event_name = req.event.cloned().unwrap_or_else(|| "push".to_string());

    // Root git operations at the git repo root when possible, so behavior
    // is consistent regardless of the directory the user ran `wrkflw`
    // from. Falls back to process CWD if we're not inside a repo.
    //
    // `find_repo_root` is a sync shell-out not covered by
    // `GIT_COMMAND_TIMEOUT`; wrap in `spawn_blocking` so a hung git
    // (credential prompt, stuck network mount) cannot freeze the reactor.
    let repo_root: Option<PathBuf> = tokio::task::spawn_blocking(wrkflw_watcher::find_repo_root)
        .await
        .ok()
        .flatten();
    let cwd_for_git: Option<&Path> = repo_root.as_deref();

    let mut event_context = build_event_context(&req, &event_name, cwd_for_git).await;
    apply_base_branch(&mut event_context, &event_name, req.base_branch);
    apply_activity_type(&mut event_context, req.activity_type);

    if req.verbose {
        wrkflw_logging::info(&format!(
            "Trigger filter: event={}, branch={:?}, base_branch={:?}, activity_type={:?}, changed_files={:?}",
            event_context.event_name,
            event_context.branch,
            event_context.base_branch,
            event_context.activity_type,
            event_context.changed_files
        ));
    }

    // Parse workflow and evaluate trigger before executing.
    //
    // `load_trigger_config` performs blocking file I/O + YAML parsing
    // (documented in `wrkflw_trigger_filter::lib.rs`). Move it onto a
    // blocking thread so we don't stall the tokio reactor. The latency
    // hit for a single file is small, but the contract should match
    // the watcher and TUI, both of which already do this — drifting
    // here is exactly how the silent-failure holes accumulated.
    let workflow_path_owned = req.workflow_path.to_path_buf();
    let trigger_config = tokio::task::spawn_blocking(move || {
        wrkflw_trigger_filter::load_trigger_config(&workflow_path_owned)
    })
    .await
    .unwrap_or_else(|e| {
        eprintln!("Error: workflow parse task panicked: {}", e);
        std::process::exit(1);
    })
    .unwrap_or_else(|e| {
        eprintln!("Error parsing workflow: {}", e);
        std::process::exit(1);
    });
    let match_result = wrkflw_trigger_filter::evaluate_trigger(&trigger_config, &event_context);

    if !match_result.matches {
        use wrkflw_ui::cli_style;
        println!(
            "{}",
            cli_style::dim(&format!("Workflow skipped: {}", match_result.reason))
        );
        std::process::exit(0);
    }
    wrkflw_logging::info(&format!("Trigger matched: {}", match_result.reason));
}

/// Pick the right context-builder based on which flags the user supplied.
///
/// Exits the process on failure — the previous inline implementation also
/// exited from each branch, but extracting this makes the
/// `run_trigger_prefilter_or_exit` orchestrator easier to read and lets
/// each branch own its own error message without nesting.
async fn build_event_context(
    req: &PrefilterRequest<'_>,
    event_name: &str,
    cwd_for_git: Option<&Path>,
) -> wrkflw_trigger_filter::EventContext {
    if let Some(files) = req.changed_files {
        return wrkflw_trigger_filter::context_from_changed_files(
            event_name,
            files.clone(),
            cwd_for_git,
        )
        .await
        .unwrap_or_else(|e| {
            eprintln!("Error: failed to build event context: {}", e);
            std::process::exit(1);
        });
    }

    if req.diff {
        // Three branches:
        //   1. `--diff-head` set: explicit two-ref range. Honour
        //      `--diff-base` if given, default the base end of the range
        //      to `HEAD` so the range is well-formed.
        //   2. `--diff-base` set, no `--diff-head`: auto-detect against
        //      that base ref (working tree vs <base>).
        //   3. Neither: smart-detect via origin/HEAD → main → master →
        //      HEAD~1. This is the path the user gets from `--diff` alone,
        //      which previously was wired to "HEAD" and silently restricted
        //      the diff to uncommitted changes only.
        return if let Some(head) = req.diff_head {
            let base = req.diff_base.unwrap_or("HEAD");
            wrkflw_trigger_filter::context_from_diff_range(event_name, base, head, cwd_for_git)
                .await
        } else if let Some(base) = req.diff_base {
            wrkflw_trigger_filter::auto_detect_context(event_name, base, cwd_for_git).await
        } else {
            wrkflw_trigger_filter::auto_detect_context_default_base(event_name, cwd_for_git).await
        }
        .unwrap_or_else(|e| {
            eprintln!("Error: failed to get git diff: {}", e);
            std::process::exit(1);
        });
    }

    // --event was passed alone (no --diff, no --changed-files).
    // The context will have an empty changed-files set, which means
    // any workflow with a `paths:` filter will be silently skipped.
    // Warn so users do not get surprised by "nothing triggered".
    wrkflw_logging::warning(
        "--event was supplied without --diff or --changed-files; \
         path filters will not match because no changed files are known. \
         Use --diff to auto-detect from git, or --changed-files to specify them.",
    );
    wrkflw_trigger_filter::context_from_changed_files(event_name, vec![], cwd_for_git)
        .await
        .unwrap_or_else(|e| {
            eprintln!("Error: failed to build event context: {}", e);
            std::process::exit(1);
        })
}

/// Stamp the user-supplied `--base-branch` onto the event context, or
/// warn if the event needs one but the user did not pass it.
///
/// Extracted out of the prefilter orchestrator so the warning behavior is
/// in one place — both the `pull_request` and `pull_request_target`
/// events evaluate `branches:` filters against the base branch, and the
/// failure mode is identical.
fn apply_base_branch(
    ctx: &mut wrkflw_trigger_filter::EventContext,
    event_name: &str,
    base_branch: Option<&String>,
) {
    if let Some(base) = base_branch {
        ctx.base_branch = Some(base.clone());
    } else if matches!(event_name, "pull_request" | "pull_request_target") {
        wrkflw_logging::warning(
            "Simulating pull_request without --base-branch: workflows that use \
             `branches:` to constrain the PR target branch will be reported as not triggering. \
             Pass --base-branch <name> to match against a target branch.",
        );
    }
}

/// Stamp `--activity-type` onto the event context.
///
/// `EventContext::activity_type` is the field GitHub Actions matches its
/// `types:` filter against. If a user simulates a `pull_request` whose
/// workflow has `types: [opened, synchronize]` but doesn't pass
/// `--activity-type`, every such workflow is silently rejected with
/// "no activity type in context" — exactly the silent-skip failure mode
/// the rest of this PR is built to prevent.
fn apply_activity_type(
    ctx: &mut wrkflw_trigger_filter::EventContext,
    activity_type: Option<&String>,
) {
    if let Some(activity) = activity_type {
        ctx.activity_type = Some(activity.clone());
    }
}

/// Validate a GitHub workflow file
/// Returns true if validation failed, false if it passed
fn validate_github_workflow(path: &Path, verbose: bool) -> bool {
    use wrkflw_ui::cli_style;
    print!("Validating GitHub workflow file: {}... ", path.display());

    match wrkflw_evaluator::evaluate_workflow_file(path, verbose) {
        Ok(result) => {
            if result.is_valid {
                println!("{}", cli_style::success("Valid"));
                if verbose {
                    println!("{}", cli_style::dim("  All validation checks passed"));
                }
            } else {
                println!("{}", cli_style::error("Invalid"));
                for (i, issue) in result.issues.iter().enumerate() {
                    println!("{}", cli_style::indent(&format!("{}. {}", i + 1, issue)));
                }
            }
            !result.is_valid
        }
        Err(e) => {
            println!("{}", cli_style::error("Error"));
            eprintln!("  {}", e);
            true
        }
    }
}

/// Validate a GitLab CI/CD pipeline file
/// Returns true if validation failed, false if it passed
fn validate_gitlab_pipeline(path: &Path, verbose: bool) -> bool {
    use wrkflw_ui::cli_style;
    print!("Validating GitLab CI pipeline file: {}... ", path.display());

    match wrkflw_parser::gitlab::parse_pipeline(path) {
        Ok(pipeline) => {
            println!("{}", cli_style::success("Valid syntax"));

            let validation_result = wrkflw_validators::validate_gitlab_pipeline(&pipeline);

            if !validation_result.is_valid {
                println!("{}", cli_style::warning("Validation issues:"));
                for issue in validation_result.issues {
                    println!("{}", cli_style::indent(&format!("- {}", issue)));
                }
                true
            } else {
                if verbose {
                    println!("{}", cli_style::success("All validation checks passed"));
                }
                false // Validation passed
            }
        }
        Err(e) => {
            println!("{}", cli_style::error("Invalid"));
            eprintln!("Validation failed: {}", e);
            true
        }
    }
}

/// List available workflows and pipelines in the repository
fn list_workflows_and_pipelines(verbose: bool, show_jobs: bool) {
    use colored::Colorize;
    use wrkflw_ui::cli_style;

    // Check for GitHub workflows
    let github_path = PathBuf::from(".github/workflows");
    if github_path.exists() && github_path.is_dir() {
        println!("{}", "GitHub Workflows".bold().cyan());

        match std::fs::read_dir(&github_path) {
            Ok(rd) => {
                let entries: Vec<_> = rd
                    .filter_map(|entry| entry.ok())
                    .filter(|entry| {
                        entry.path().is_file()
                            && entry
                                .path()
                                .extension()
                                .is_some_and(|ext| ext == "yml" || ext == "yaml")
                    })
                    .collect();

                if entries.is_empty() {
                    println!(
                        "{}",
                        cli_style::dim("  No workflow files found in .github/workflows")
                    );
                } else {
                    for (i, entry) in entries.iter().enumerate() {
                        let is_last = i == entries.len() - 1;
                        let connector = if is_last {
                            "\u{2514}\u{2500}\u{2500}"
                        } else {
                            "\u{251C}\u{2500}\u{2500}"
                        };
                        println!("{} {}", connector.dimmed(), entry.path().display());
                        if show_jobs {
                            let prefix = if is_last { "    " } else { "\u{2502}   " };
                            match wrkflw_parser::workflow::parse_workflow(&entry.path()) {
                                Ok(workflow) => {
                                    let mut job_names: Vec<&String> =
                                        workflow.jobs.keys().collect();
                                    job_names.sort();
                                    println!(
                                        "{}{}",
                                        prefix.dimmed(),
                                        format!(
                                            "Jobs: {}",
                                            job_names
                                                .iter()
                                                .map(|s| s.as_str())
                                                .collect::<Vec<_>>()
                                                .join(", ")
                                        )
                                        .dimmed()
                                    );
                                }
                                Err(e) => {
                                    eprintln!("{}Could not parse workflow: {}", prefix, e);
                                }
                            }
                        }
                    }
                }
            }
            Err(e) => {
                eprintln!(
                    "{}",
                    cli_style::error(&format!(
                        "Failed to read directory {}: {}",
                        github_path.display(),
                        e
                    ))
                );
            }
        }
    } else {
        println!(
            "{}",
            cli_style::dim("GitHub Workflows: No .github/workflows directory found")
        );
    }

    // Check for GitLab CI pipeline
    let gitlab_path = PathBuf::from(".gitlab-ci.yml");
    if gitlab_path.exists() && gitlab_path.is_file() {
        println!("\n{}", "GitLab CI Pipeline".bold().cyan());
        println!(
            "{} {}",
            "\u{2514}\u{2500}\u{2500}".dimmed(),
            gitlab_path.display()
        );
        if show_jobs {
            match wrkflw_parser::gitlab::parse_pipeline(Path::new(".gitlab-ci.yml")) {
                Ok(pipeline) => {
                    let mut job_names: Vec<&String> = pipeline.jobs.keys().collect();
                    job_names.sort();
                    println!(
                        "    {}",
                        format!(
                            "Jobs: {}",
                            job_names
                                .iter()
                                .map(|s| s.as_str())
                                .collect::<Vec<_>>()
                                .join(", ")
                        )
                        .dimmed()
                    );
                }
                Err(e) => {
                    eprintln!("    Could not parse pipeline: {}", e);
                }
            }
        }
    } else {
        println!(
            "{}",
            cli_style::dim("GitLab CI Pipeline: No .gitlab-ci.yml file found")
        );
    }

    // Check for other GitLab CI pipeline files
    if verbose {
        println!(
            "\n{}",
            cli_style::info("Searching for other GitLab CI pipeline files...")
        );

        let entries = walkdir::WalkDir::new(".")
            .follow_links(true)
            .into_iter()
            .filter_map(|entry| entry.ok())
            .filter(|entry| {
                entry.path().is_file()
                    && entry
                        .file_name()
                        .to_string_lossy()
                        .ends_with("gitlab-ci.yml")
                    && entry.path() != gitlab_path
            })
            .collect::<Vec<_>>();

        if !entries.is_empty() {
            println!("{}", "Additional GitLab CI Pipeline files:".bold());
            for entry in entries {
                println!(
                    "{} {}",
                    "\u{2514}\u{2500}\u{2500}".dimmed(),
                    entry.path().display()
                );
            }
        }
    }
}
