use bollard::Docker;
use clap::{Parser, Subcommand, ValueEnum};
use std::collections::HashMap;
use std::path::Path;
use std::path::PathBuf;

mod prefilter;
mod run_workflow_cmd;
mod watch_cmd;

#[derive(Debug, Clone, ValueEnum)]
pub(crate) enum RuntimeChoice {
    /// Detect Docker first, then Podman, then fall back to emulation (default)
    Auto,
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
            RuntimeChoice::Auto => wrkflw_executor::RuntimeType::Auto,
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
        #[arg(short, long, value_enum, default_value = "auto")]
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

        /// Reject degraded filter contexts (missing base branch on
        /// `pull_request`, `--event` without changed-file input, etc.)
        /// with a hard error instead of a log warning.
        ///
        /// Defaults to `true` so the CLI fails loudly on a
        /// silently-under-filtered run — the opposite of the
        /// warn-and-proceed behavior that produced "why did my
        /// workflow not fire?" tickets. Pass `--no-strict-filter` to
        /// opt back into the legacy warning behavior for scripts that
        /// have already adapted to it.
        #[arg(long = "strict-filter", default_value_t = true)]
        strict_filter: bool,

        /// Opposite of `--strict-filter`; re-enables the legacy
        /// warn-and-proceed behavior for degraded contexts. Kept as
        /// a separate flag instead of `--no-strict-filter` so clap's
        /// `conflicts_with` makes the intent explicit at the call
        /// site.
        #[arg(long = "no-strict-filter", conflicts_with = "strict_filter")]
        no_strict_filter: bool,
    },

    /// Watch for file changes and re-run affected workflows.
    ///
    /// On Ctrl+C the watcher drains the current cycle gracefully:
    /// workflows already executing finish, the trigger-filter state
    /// is flushed, and the signal is passed through to the cleanup
    /// handler that reaps Docker containers and tempdirs. A hard
    /// exit only happens if the graceful drain is still running
    /// after ~10s — long enough for normal teardown, short enough
    /// that a hung subprocess cannot wedge the session.
    Watch {
        /// Path to workflow file or directory (defaults to .github/workflows)
        path: Option<PathBuf>,

        /// Container runtime to use (docker, podman, emulation, secure-emulation)
        #[arg(short, long, value_enum, default_value = "auto")]
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

        /// Upper bound on the debouncer's pending-event set. Events
        /// past this count during a churn burst are dropped and
        /// surfaced as a per-cycle warning so the user sees that
        /// something was missed. Omit the flag to use the debouncer's
        /// built-in default, which is sized for typical workloads.
        ///
        /// The flag is `Option<usize>` rather than `usize` with a
        /// sentinel `0 = default` value because `--max-pending-events 0`
        /// reads as "unbounded" to most users — the convention
        /// violation was flagged in review. `0` is now explicitly
        /// rejected at startup (warning + fall through to default)
        /// since a zero cap would drop every event and render the
        /// watcher useless.
        #[arg(long)]
        max_pending_events: Option<usize>,

        /// Extra directory names to ignore in addition to the built-in
        /// list (`.git`, `target`, `node_modules`, `.build`, `build`,
        /// `dist`, `__pycache__`, `.tox`, `.mypy_cache`, `.pytest_cache`,
        /// `.venv`, `venv`). Matched by directory-component name, not
        /// glob or path — a user file literally named `.terraform` is
        /// never silenced; only events whose parent path contains a
        /// `.terraform/` component are dropped. Pass multiple times
        /// or as a comma-separated list: `--ignore-dir .terraform
        /// --ignore-dir coverage` or `--ignore-dir .terraform,coverage`.
        #[arg(long = "ignore-dir", value_delimiter = ',')]
        ignore_dirs: Vec<String>,

        /// Reject degraded filter contexts (missing base branch on
        /// `pull_request`, unknown events, etc.) with a hard error
        /// instead of a log warning. Defaults to `true` so watch
        /// mode fails loudly on misconfiguration rather than running
        /// a session-long "0 triggered" stream.
        #[arg(long = "strict-filter", default_value_t = true)]
        strict_filter: bool,

        /// Opposite of `--strict-filter`; re-enables the legacy
        /// warn-and-proceed behavior for degraded contexts.
        #[arg(long = "no-strict-filter", conflicts_with = "strict_filter")]
        no_strict_filter: bool,
    },

    /// Open TUI interface to manage workflows
    #[cfg(feature = "tui")]
    Tui {
        /// Path to workflow file or directory (defaults to .github/workflows)
        path: Option<PathBuf>,

        /// Container runtime to use (docker, podman, emulation, secure-emulation)
        #[arg(short, long, value_enum, default_value = "auto")]
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
pub(crate) fn is_gitlab_pipeline(path: &Path) -> bool {
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
            strict_filter,
            no_strict_filter,
        }) => {
            run_workflow_cmd::run(run_workflow_cmd::RunCtx {
                path: path.clone(),
                runtime: runtime.clone(),
                show_action_messages: *show_action_messages,
                preserve_containers_on_failure: *preserve_containers_on_failure,
                gitlab: *gitlab,
                job: job.clone(),
                event: event.clone(),
                diff: *diff,
                changed_files: changed_files.clone(),
                diff_base: diff_base.clone(),
                diff_head: diff_head.clone(),
                base_branch: base_branch.clone(),
                activity_type: activity_type.clone(),
                strict_filter: *strict_filter,
                no_strict_filter: *no_strict_filter,
                verbose,
            })
            .await;
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
            max_pending_events,
            ignore_dirs,
            strict_filter,
            no_strict_filter,
        }) => {
            watch_cmd::run(watch_cmd::WatchCtx {
                path: path.clone(),
                runtime: runtime.clone(),
                debounce: *debounce,
                event: event.clone(),
                show_action_messages: *show_action_messages,
                preserve_containers_on_failure: *preserve_containers_on_failure,
                max_concurrency: *max_concurrency,
                base_branch: base_branch.clone(),
                activity_type: activity_type.clone(),
                max_pending_events: *max_pending_events,
                ignore_dirs: ignore_dirs.clone(),
                strict_filter: *strict_filter,
                no_strict_filter: *no_strict_filter,
                verbose,
            })
            .await;
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
                let runtime_type = wrkflw_executor::RuntimeType::Auto;

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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn runtime_choice_auto_maps_to_runtime_type_auto() {
        let rt: wrkflw_executor::RuntimeType = RuntimeChoice::Auto.into();
        assert_eq!(rt, wrkflw_executor::RuntimeType::Auto);
    }

    #[test]
    fn runtime_choice_all_variants_map_correctly() {
        use wrkflw_executor::RuntimeType;
        assert_eq!(RuntimeType::from(RuntimeChoice::Auto), RuntimeType::Auto);
        assert_eq!(
            RuntimeType::from(RuntimeChoice::Docker),
            RuntimeType::Docker
        );
        assert_eq!(
            RuntimeType::from(RuntimeChoice::Podman),
            RuntimeType::Podman
        );
        assert_eq!(
            RuntimeType::from(RuntimeChoice::Emulation),
            RuntimeType::Emulation
        );
        assert_eq!(
            RuntimeType::from(RuntimeChoice::SecureEmulation),
            RuntimeType::SecureEmulation
        );
    }
}
