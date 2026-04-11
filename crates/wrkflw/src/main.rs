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
            strict_filter,
            no_strict_filter,
        }) => {
            let strict_filter = effective_strict_filter(*strict_filter, *no_strict_filter);
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
                let decision = run_trigger_prefilter(PrefilterRequest {
                    workflow_path: path,
                    event: event.as_ref(),
                    diff: *diff,
                    changed_files: changed_files.as_ref(),
                    diff_base: diff_base.as_deref(),
                    diff_head: diff_head.as_ref(),
                    base_branch: base_branch.as_ref(),
                    activity_type: activity_type.as_ref(),
                    verbose,
                    strict_filter,
                })
                .await;
                match decision {
                    Ok(PrefilterDecision::Proceed) => {
                        // Match — fall through to the executor below.
                    }
                    Ok(PrefilterDecision::Skip { reason }) => {
                        use wrkflw_ui::cli_style;
                        println!(
                            "{}",
                            cli_style::dim(&format!("Workflow skipped: {}", reason))
                        );
                        std::process::exit(0);
                    }
                    Err(msg) => {
                        eprintln!("Error: {}", msg);
                        std::process::exit(1);
                    }
                }
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
            max_pending_events,
            ignore_dirs,
            strict_filter,
            no_strict_filter,
        }) => {
            let strict_filter = effective_strict_filter(*strict_filter, *no_strict_filter);
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

            // Hard-error on the load-bearing `pull_request + no base-branch`
            // combination under the default `--strict-filter`. Previously
            // this only produced a log warning that the user never saw in
            // non-interactive contexts, and the watcher then ran a
            // session-long stream of "0 triggered" results.
            if (event == "pull_request" || event == "pull_request_target") && base_branch.is_none()
            {
                if strict_filter {
                    eprintln!(
                        "Error: `wrkflw watch --event {}` requires --base-branch under \
                         --strict-filter. GitHub Actions evaluates `branches:` filters on \
                         pull_request events against the PR target branch, and without one \
                         every such workflow would be silently rejected. Pass \
                         --base-branch <name>, or use --no-strict-filter to proceed.",
                        event
                    );
                    std::process::exit(1);
                }
                wrkflw_logging::warning(
                    "Watching pull_request without --base-branch: any workflow with a \
                     `branches:` filter will be silently skipped because GitHub Actions \
                     evaluates that filter against the PR target branch. \
                     --no-strict-filter allowed this to proceed.",
                );
            }

            // Resolve `--max-pending-events`. The library's
            // `WatcherConfig::max_pending_events` field keeps its
            // existing `0 = use-library-default` convention (matches
            // how `TriggerFilterConfig::pattern_cache_size == 0`
            // disables caching — library-wide sentinel style). The
            // CLI, however, exposes an honest `Option<usize>` so
            // `--help` doesn't advertise a misleading `[default: 0]`.
            //
            // `Some(0)` is almost certainly an error on the user's
            // part (cap-everything-to-zero would drop every event
            // and make the watcher useless). Mirror the
            // `max_concurrency=0 → 1` clamp pattern in the library:
            // warn loudly and fall through to the library default.
            let max_pending_for_cfg: usize = match max_pending_events {
                Some(0) => {
                    wrkflw_logging::warning(
                        "--max-pending-events 0 is invalid (would cap the pending \
                         set at zero and drop every event); falling back to the \
                         library default.",
                    );
                    0 // 0 inside the library means "use DEFAULT_MAX_PENDING_EVENTS"
                }
                Some(n) => *n,
                None => 0,
            };

            let watcher_cfg = wrkflw_watcher::WatcherConfig::new(workflow_dir, repo_root, config)
                .with_event(event.clone())
                .with_base_branch(base_branch.clone())
                .with_activity_type(activity_type.clone())
                .with_debounce(debounce_duration)
                .with_verbose(verbose)
                .with_max_concurrency(*max_concurrency)
                .with_max_pending_events(max_pending_for_cfg)
                .with_extra_ignore_dirs(ignore_dirs.clone());
            let watcher = wrkflw_watcher::WorkflowWatcher::from_config(watcher_cfg);

            // Pre-flight: surface any real I/O error (missing dir,
            // permission denied) before the user sees a "watching..."
            // banner. An empty directory is NOT an error — the
            // watcher's internal rescan picks up `.yml` files the
            // moment they are created, which is the whole point of
            // watch mode. `collect_workflow_files_blocking` now
            // returns `Ok(Vec::new())` for an empty dir, so only
            // genuine failures propagate past this match.
            if let Err(e) = watcher.collect_workflow_files().await {
                eprintln!("Error: {}", e);
                std::process::exit(1);
            }

            // Install a graceful Ctrl+C handler: the default
            // top-of-`main` handler uses `process::exit(0)` after a
            // timed cleanup sweep, which bypasses the watcher's
            // normal drain (workflows mid-execution would be killed
            // by the OS without running the executor's teardown).
            //
            // Instead we own a `ShutdownSignal`, trigger it on
            // Ctrl+C, and let the watcher observe the signal at its
            // existing `tokio::select!` points. The global handler
            // at the top of `main` still runs — it kicks in after
            // the watch loop returns and performs the Docker /
            // emulation cleanup sweep — so Ctrl+C produces a clean
            // two-phase teardown instead of a hard exit.
            //
            // A race exists: if Ctrl+C fires while a workflow is
            // already executing, that workflow continues to
            // completion within the current cycle before `run()`
            // returns. We accept that bounded latency because the
            // executor holds container/tempdir handles that need
            // their normal cleanup — forcibly cancelling the
            // future would defeat the very cleanup we're trying to
            // preserve. `MAX_REASONABLE_CONCURRENCY` + the
            // user-specified `--max-concurrency` bound the
            // worst-case drain time.
            let shutdown = wrkflw_watcher::ShutdownSignal::new();
            let shutdown_for_signal = shutdown.clone();
            tokio::spawn(async move {
                if tokio::signal::ctrl_c().await.is_ok() {
                    wrkflw_logging::info(
                        "Ctrl+C received — draining current watch cycle gracefully. \
                         Press Ctrl+C again if the drain hangs.",
                    );
                    shutdown_for_signal.trigger();
                }
            });

            let watch_result = watcher
                .run(shutdown, |watch_event| {
                    println!(
                        "\n{}",
                        cli_style::section(&format!(
                            "Change detected ({} file(s) changed, {} triggered, {} skipped{})",
                            watch_event.changed_files.len(),
                            watch_event.triggered_workflows.len(),
                            watch_event.skipped_workflows.len(),
                            if watch_event.dropped_events > 0 {
                                format!(", {} dropped", watch_event.dropped_events)
                            } else {
                                String::new()
                            }
                        ))
                    );
                    // Surface degraded cycles loudly: if the watcher
                    // could not build a git event context, the trigger
                    // results are not authoritative and the user needs
                    // to know why before they assume "0 triggered".
                    if let Some(reason) = &watch_event.error {
                        eprintln!("  {} {}", cli_style::error("ERROR"), reason);
                    }
                    for warning in &watch_event.warnings {
                        eprintln!("  {} {}", cli_style::warning("WARN"), warning);
                    }
                    for wf in &watch_event.triggered_workflows {
                        println!("  {} {}", cli_style::success("TRIGGERED"), wf);
                    }
                    for wf in &watch_event.skipped_workflows {
                        println!("  {} {}", cli_style::dim("SKIPPED"), wf);
                    }
                })
                .await;

            if let Err(e) = watch_result {
                eprintln!("Watch error: {}", e);
                std::process::exit(1);
            }
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

/// Decision returned by [`run_trigger_prefilter`].
///
/// Previously the prefilter called `std::process::exit` from half a
/// dozen sites deep inside its body, which made the flag-matrix
/// untestable — a unit test would have to spawn a subprocess just to
/// observe the exit code. Returning a plain enum lets the orchestrator
/// own the decision and hand `main()` the responsibility of calling
/// `process::exit`. The side-effects that need to happen before the
/// decision (warning drains, verbose logging) still live in the
/// orchestrator body; only the exit is deferred.
#[derive(Debug)]
enum PrefilterDecision {
    /// The workflow's triggers matched the event context — main
    /// should proceed to execute the workflow.
    Proceed,
    /// The workflow's triggers did NOT match — main should print the
    /// reason (already formatted for the user) and exit 0.
    Skip { reason: String },
}

/// Resolve the effective `--strict-filter` / `--no-strict-filter`
/// bool toggle. `--no-strict-filter` wins over the default-true
/// `--strict-filter` via clap's `conflicts_with`, so the effective
/// value is `strict AND NOT no_strict`. Extracted so the two call
/// sites (`wrkflw run` and `wrkflw watch`) cannot drift apart — if
/// a third host grows the same flag pair, it gets the same
/// coalescing for free.
fn effective_strict_filter(strict: bool, no_strict: bool) -> bool {
    strict && !no_strict
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
    /// When true, known-incomplete filter contexts (missing changed
    /// files, missing base branch on a PR event) exit with a
    /// diagnostic instead of log-warning-and-proceeding. The review
    /// flagged the old warn-and-proceed as exactly the silent-skip
    /// mode the rest of this PR had been patching; strict mode is
    /// the default-on countermeasure.
    strict_filter: bool,
}

/// Build an event context from the user's CLI flags and decide
/// whether the workflow should run.
///
/// Returns:
/// - `Ok(PrefilterDecision::Proceed)` — triggers matched, main should
///   continue into the executor path.
/// - `Ok(PrefilterDecision::Skip { reason })` — triggers did not match,
///   main should print the reason and exit 0.
/// - `Err(msg)` — something went wrong building the context or parsing
///   the workflow, main should print the message and exit 1.
///
/// All `std::process::exit` calls have been lifted out of this
/// function so the decision logic is testable without spawning a
/// subprocess — the flag matrix (`--diff` vs `--diff-base` vs
/// `--changed-files`, strict vs non-strict, pull_request vs push) is
/// the sort of thing that benefits most from unit tests, and the old
/// shape made that impossible.
async fn run_trigger_prefilter(req: PrefilterRequest<'_>) -> Result<PrefilterDecision, String> {
    // `wrkflw run` expects a single workflow file. Catch directory paths up
    // front with a clear error; otherwise the user sees a confusing
    // "Error parsing workflow" from the YAML parser further down.
    if !req.workflow_path.is_file() {
        if req.workflow_path.is_dir() {
            return Err(format!(
                "--diff/--event/--changed-files require a single workflow file, not a directory.\n\
                 Hint: point at a specific .yml file, or use `wrkflw watch {}` for directory-wide watching.",
                req.workflow_path.display()
            ));
        } else {
            return Err(format!(
                "workflow file not found: {}",
                req.workflow_path.display()
            ));
        }
    }

    let event_name = req.event.cloned().unwrap_or_else(|| "push".to_string());

    // Root git operations at the git repo root when possible, so behavior
    // is consistent regardless of the directory the user ran `wrkflw`
    // from. Falls back to process CWD if we're not inside a repo.
    //
    // `find_repo_root_detailed` is a sync shell-out not covered by
    // `GIT_COMMAND_TIMEOUT`; wrap in `spawn_blocking` so a hung git
    // (credential prompt, stuck network mount) cannot freeze the reactor.
    //
    // We use the classified `_detailed` form so each failure mode
    // surfaces its own diagnostic. `NotInRepository` is a legitimate
    // soft failure (the user may have passed `--changed-files` without
    // needing any git helper) — fall back to `None` and let the
    // downstream git calls decide whether they need a repo. Every
    // other failure (git-not-installed, timeout, other) is loud and
    // fatal because the user has something actionable to fix.
    let repo_root: Option<PathBuf> =
        match tokio::task::spawn_blocking(wrkflw_trigger_filter::find_repo_root_detailed).await {
            Ok(Ok(p)) => Some(p),
            Ok(Err(wrkflw_trigger_filter::FindRepoRootError::NotInRepository)) => None,
            Ok(Err(e)) => return Err(e.to_string()),
            Err(join_err) => return Err(format!("find_repo_root task panicked: {}", join_err)),
        };
    let cwd_for_git: Option<&Path> = repo_root.as_deref();

    let mut event_context = build_event_context(&req, &event_name, cwd_for_git).await?;
    apply_base_branch(
        &mut event_context,
        &event_name,
        req.base_branch,
        req.strict_filter,
    )?;
    // Stamp `--activity-type` onto the context. `EventContext::activity_type`
    // is the field GitHub Actions matches its `types:` filter against —
    // without it, every workflow with `types: [opened, ...]` is silently
    // rejected for "no activity type in context", which is exactly the
    // silent-skip failure mode this PR is built to prevent.
    if let Some(activity) = req.activity_type {
        event_context.activity_type = Some(activity.clone());
    }

    // Surface any non-fatal warnings collected while building the
    // context (e.g. `git ls-files --others` failed, so untracked
    // files were dropped). The trigger-filter crate no longer logs
    // these itself — it collects them as data and hands them to
    // hosts via `EventContext::warnings`, so we own the rendering
    // policy here and can stay consistent with the rest of the CLI's
    // colorization.
    //
    // `take()` (rather than read-only iteration) is load-bearing:
    // `EventContext::warnings` is a `MustDrainWarnings` whose Drop
    // check fires in debug builds if a non-empty buffer is dropped
    // without being observed. Draining satisfies the contract and
    // guarantees the CLI path cannot silently reintroduce the
    // warning-loss failure mode the rest of this PR has been
    // plugging.
    for w in event_context.warnings.take() {
        wrkflw_logging::warning(&w);
    }

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
    let tf_config = wrkflw_trigger_filter::TriggerFilterConfig::default();
    let mut trigger_config = tokio::task::spawn_blocking(move || {
        // Route through the shared LRU cache so every wrkflw entry
        // point (CLI prefilter, TUI diff-filter, watcher hot loop)
        // contends over the same compiled-pattern store. Unifying
        // the three call sites was a review ask to prevent drift —
        // the same file never pays the YAML-parse cost twice.
        wrkflw_trigger_filter::load_trigger_config_cached(&workflow_path_owned, &tf_config)
    })
    .await
    .map_err(|e| format!("workflow parse task panicked: {}", e))?
    .map_err(|e| format!("parsing workflow: {}", e))?;
    // Drain parser-collected diagnostics (unknown event names, etc.)
    // — the library decouples from the log sink by design, so every
    // host must drain this field or reintroduce the silent-skip
    // failure mode. `take()` also satisfies the `MustDrainWarnings`
    // Drop-check contract that catches the regression in debug
    // builds.
    for w in trigger_config.warnings.take() {
        wrkflw_logging::warning(&w);
    }
    let match_result = wrkflw_trigger_filter::evaluate_trigger(&trigger_config, &event_context);

    if !match_result.matches {
        return Ok(PrefilterDecision::Skip {
            reason: match_result.reason,
        });
    }
    wrkflw_logging::info(&format!("Trigger matched: {}", match_result.reason));
    Ok(PrefilterDecision::Proceed)
}

/// Pick the right context-builder based on which flags the user supplied.
///
/// Returns a `Result<EventContext, String>` so the orchestrator owns the
/// `process::exit` policy — previously each branch called `exit` from
/// deep in the helper, which made the flag-matrix logic impossible to
/// unit-test without spawning a subprocess. The error string is ready
/// to be printed verbatim with an `Error:` prefix.
async fn build_event_context(
    req: &PrefilterRequest<'_>,
    event_name: &str,
    cwd_for_git: Option<&Path>,
) -> Result<wrkflw_trigger_filter::EventContext, String> {
    if let Some(files) = req.changed_files {
        // Validate every user-supplied entry before handing it to
        // the trigger-filter. Absolute paths, drive letters, and
        // `..` components break the repo-relative glob contract the
        // evaluator assumes; catching them up front produces a
        // "your flag was wrong" message instead of a session-long
        // "nothing matched" mystery.
        let normalized = wrkflw_trigger_filter::normalize_user_changed_files(files)
            .map_err(|e| format!("invalid --changed-files entry: {}", e))?;
        return wrkflw_trigger_filter::context_from_changed_files(
            event_name,
            normalized,
            cwd_for_git,
        )
        .await
        .map_err(|e| format!("failed to build event context: {}", e));
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
            wrkflw_trigger_filter::auto_detect_context_default_base(
                event_name,
                cwd_for_git,
                req.verbose,
            )
            .await
        }
        .map_err(|e| format!("failed to get git diff: {}", e));
    }

    // `--event` was passed alone (no `--diff`, no `--changed-files`).
    // Running with an empty changed-files set means every `paths:`
    // filter silently rejects — the exact silent-skip failure mode
    // the rest of this PR has been plugging. In strict mode (the
    // default) refuse to proceed so CI scripts fail loudly and the
    // operator has something actionable to fix.
    if req.strict_filter {
        return Err(
            "--event was supplied without --diff or --changed-files, so no changed files \
             are known and any workflow with a `paths:` filter would be silently skipped. \
             Pass --diff to auto-detect from git, --changed-files to supply them \
             explicitly, or --no-strict-filter to proceed anyway."
                .to_string(),
        );
    }
    wrkflw_logging::warning(
        "--event was supplied without --diff or --changed-files; \
         path filters will not match because no changed files are known. \
         --no-strict-filter allowed this to proceed.",
    );
    wrkflw_trigger_filter::context_from_changed_files(event_name, vec![], cwd_for_git)
        .await
        .map_err(|e| format!("failed to build event context: {}", e))
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
    strict_filter: bool,
) -> Result<(), String> {
    if let Some(base) = base_branch {
        ctx.base_branch = Some(base.clone());
        return Ok(());
    }
    if matches!(event_name, "pull_request" | "pull_request_target") {
        if strict_filter {
            return Err(format!(
                "simulating `{}` without --base-branch is rejected under --strict-filter: \
                 `branches:` filters on pull_request events evaluate against the PR target \
                 branch, and without one every such workflow is silently reported as not \
                 triggering. Pass --base-branch <name>, or use --no-strict-filter to proceed.",
                event_name
            ));
        }
        wrkflw_logging::warning(
            "Simulating pull_request without --base-branch: workflows that use \
             `branches:` to constrain the PR target branch will be reported as not triggering. \
             --no-strict-filter allowed this to proceed.",
        );
    }
    Ok(())
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
mod prefilter_tests {
    //! Unit coverage for the `run_trigger_prefilter` decision logic.
    //!
    //! These tests exist specifically because the previous
    //! `run_trigger_prefilter_or_exit` shape called `std::process::exit`
    //! from inside every failure branch, making the flag matrix
    //! impossible to exercise without spawning a subprocess. The
    //! refactor that returns `Result<PrefilterDecision, String>`
    //! lets us pin the "path is a directory", "path does not
    //! exist", and "workflow parses but does not match" branches
    //! here in-process.
    use super::*;

    #[tokio::test(flavor = "current_thread")]
    async fn directory_path_returns_err_with_watch_hint() {
        let tmp = tempfile::TempDir::new().expect("tempdir");
        let dir = tmp.path().to_path_buf();
        let empty_files: Option<Vec<String>> = None;
        let event = "push".to_string();
        let req = PrefilterRequest {
            workflow_path: &dir,
            event: Some(&event),
            diff: false,
            changed_files: empty_files.as_ref(),
            diff_base: None,
            diff_head: None,
            base_branch: None,
            activity_type: None,
            verbose: false,
            strict_filter: false,
        };
        let err = run_trigger_prefilter(req)
            .await
            .expect_err("directory path must produce an Err");
        assert!(
            err.contains("single workflow file"),
            "err must explain the single-file constraint, got: {}",
            err
        );
        assert!(
            err.contains("wrkflw watch"),
            "err must suggest `wrkflw watch` for directory-wide watching, got: {}",
            err
        );
    }

    #[tokio::test(flavor = "current_thread")]
    async fn missing_path_returns_err_with_not_found() {
        let tmp = tempfile::TempDir::new().expect("tempdir");
        let missing = tmp.path().join("does-not-exist.yml");
        let event = "push".to_string();
        let req = PrefilterRequest {
            workflow_path: &missing,
            event: Some(&event),
            diff: false,
            changed_files: None,
            diff_base: None,
            diff_head: None,
            base_branch: None,
            activity_type: None,
            verbose: false,
            strict_filter: false,
        };
        let err = run_trigger_prefilter(req)
            .await
            .expect_err("missing path must produce an Err");
        assert!(
            err.contains("not found"),
            "err must name the not-found case, got: {}",
            err
        );
    }

    /// Build a bare-bones git repo in `dir` with one committed file
    /// on branch `main`. Mirrors the `init_repo` helper in
    /// `crates/trigger-filter/src/git.rs` tests — duplicated here
    /// rather than lifted because this crate has no test-helpers
    /// module and a single-use helper doesn't justify one.
    fn init_repo_for_test(dir: &Path) -> bool {
        use std::process::Command as StdCommand;
        let status = StdCommand::new("git")
            .args(["-C", dir.to_str().unwrap(), "init", "--initial-branch=main"])
            .status();
        if !status.map(|s| s.success()).unwrap_or(false) {
            return false;
        }
        for (k, v) in [("user.email", "t@t.t"), ("user.name", "t")] {
            if StdCommand::new("git")
                .args(["-C", dir.to_str().unwrap(), "config", k, v])
                .status()
                .map(|s| !s.success())
                .unwrap_or(true)
            {
                return false;
            }
        }
        let path = dir.join("a.txt");
        if std::fs::write(&path, "1").is_err() {
            return false;
        }
        if StdCommand::new("git")
            .args(["-C", dir.to_str().unwrap(), "add", "a.txt"])
            .status()
            .map(|s| !s.success())
            .unwrap_or(true)
        {
            return false;
        }
        if StdCommand::new("git")
            .args([
                "-C",
                dir.to_str().unwrap(),
                "commit",
                "-m",
                "init",
                "--no-gpg-sign",
            ])
            .status()
            .map(|s| !s.success())
            .unwrap_or(true)
        {
            return false;
        }
        true
    }

    fn git_available() -> bool {
        use std::process::Command as StdCommand;
        StdCommand::new("git")
            .arg("--version")
            .status()
            .map(|s| s.success())
            .unwrap_or(false)
    }

    #[tokio::test(flavor = "current_thread")]
    async fn build_event_context_defaults_diff_base_to_head_when_only_diff_head_set() {
        // Regression pin for the `--diff-head` without `--diff-base`
        // branch at `build_event_context`'s `if let Some(head) =
        // req.diff_head` arm: the base end of the two-ref range
        // defaults to `"HEAD"` so the constructed range is
        // well-formed. Without a test this branch was reachable
        // from the CLI but never exercised in-process, and a
        // refactor that flipped the default to `"origin/HEAD"`
        // (or anything else) would silently break the flag matrix.
        //
        // We call `build_event_context` directly instead of
        // `run_trigger_prefilter` because the latter shells out to
        // `find_repo_root_detailed` against the process CWD — global
        // state that's not safe under cargo's parallel test runner.
        // The direct call takes a `cwd_for_git: Option<&Path>` which
        // we point at the tempdir repo, giving the test full
        // isolation.
        if !git_available() {
            return;
        }
        let tmp = tempfile::TempDir::new().expect("tempdir");
        let repo = tmp.path().to_path_buf();
        if !init_repo_for_test(&repo) {
            return;
        }

        // Write a minimal workflow so the prefilter has something to
        // point at if the test ever extends to parsing. Not strictly
        // needed for `build_event_context`, which never reads the
        // file, but keeps the setup close to a real CLI invocation.
        let wf = repo.join("ci.yml");
        std::fs::write(
            &wf,
            "name: ci\non: push\njobs:\n  b:\n    runs-on: ubuntu-latest\n    steps:\n      - run: echo hi\n",
        )
        .expect("write ci.yml");

        let event = "push".to_string();
        let head = "HEAD".to_string();
        let req = PrefilterRequest {
            workflow_path: &wf,
            event: Some(&event),
            diff: true,
            changed_files: None,
            diff_base: None,
            diff_head: Some(&head),
            base_branch: None,
            activity_type: None,
            verbose: false,
            strict_filter: false,
        };

        let ctx = build_event_context(&req, "push", Some(&repo)).await.expect(
            "build_event_context must succeed when --diff-head=HEAD and --diff-base is absent",
        );

        // The branch under test constructs a range `base..head` and
        // runs `git diff --name-only` on it. Base defaults to HEAD,
        // so the range is `HEAD..HEAD` — an empty diff against a
        // fresh repo. The key invariants:
        //   1. No error (the branch was reached and git ran cleanly).
        //   2. `changed_files_explicit == true` (caller asked for a
        //      two-ref diff, so an empty result is authoritative —
        //      the diagnostic layer must NOT suggest passing --diff).
        //   3. `changed_files.is_empty()` (HEAD..HEAD trivially empty).
        assert!(
            ctx.changed_files_explicit,
            "two-ref diff must mark changed_files as explicit"
        );
        assert!(
            ctx.changed_files.is_empty(),
            "HEAD..HEAD diff must be empty, got {:?}",
            ctx.changed_files
        );

        // Drain warnings to satisfy MustDrainWarnings (none expected,
        // but the contract is the same as every other host).
        let mut ctx = ctx;
        let _ = ctx.warnings.take();
    }

    #[tokio::test(flavor = "current_thread")]
    async fn skip_decision_returned_when_trigger_does_not_match() {
        // A push workflow gated on `paths: ['irrelevant/**']` with an
        // explicit empty --changed-files list must resolve to
        // `Skip`, not an error. This is the load-bearing "user got
        // a clean exit 0 because their edit did not touch the
        // filter's paths" scenario the executor path depends on.
        let tmp = tempfile::TempDir::new().expect("tempdir");
        let wf = tmp.path().join("ci.yml");
        std::fs::write(
            &wf,
            "name: ci\n\
             on:\n  push:\n    paths:\n      - 'irrelevant/**'\n\
             jobs:\n  build:\n    runs-on: ubuntu-latest\n    steps:\n      - run: echo hi\n",
        )
        .expect("write workflow");

        let event = "push".to_string();
        let changed: Vec<String> = vec!["src/main.rs".to_string()];
        let req = PrefilterRequest {
            workflow_path: &wf,
            event: Some(&event),
            diff: false,
            changed_files: Some(&changed),
            diff_base: None,
            diff_head: None,
            base_branch: None,
            activity_type: None,
            verbose: false,
            strict_filter: false,
        };
        let decision = run_trigger_prefilter(req)
            .await
            .expect("should not error on a valid workflow");
        match decision {
            PrefilterDecision::Skip { reason } => {
                assert!(
                    reason.contains("paths"),
                    "skip reason must mention the paths filter, got: {}",
                    reason
                );
            }
            PrefilterDecision::Proceed => {
                panic!("expected Skip for non-matching paths, got Proceed");
            }
        }
    }

    #[tokio::test(flavor = "current_thread")]
    async fn strict_filter_rejects_event_alone_without_diff_or_changed_files() {
        // Regression pin for the strict-filter default-on gate in
        // `build_event_context`: passing `--event push` with neither
        // `--diff` nor `--changed-files` means the caller could not
        // supply a change set, so every `paths:`-gated workflow would
        // be silently rejected at evaluation time. Under strict mode
        // (the default) this must be a hard error up front instead,
        // pointing the user at the three escape hatches.
        //
        // This is the load-bearing CLI behavior change the
        // BREAKING_CHANGES.md entry documents — keeping the rejection
        // behavior pinned here prevents a future refactor from
        // silently flipping it back to warn-and-proceed.
        let tmp = tempfile::TempDir::new().expect("tempdir");
        let wf = tmp.path().join("ci.yml");
        // Any parseable workflow works; `build_event_context` fails
        // before parsing.
        std::fs::write(
            &wf,
            "name: ci\n\
             on:\n  push:\n    paths:\n      - 'src/**'\n\
             jobs:\n  build:\n    runs-on: ubuntu-latest\n    steps:\n      - run: echo hi\n",
        )
        .expect("write workflow");

        let event = "push".to_string();
        let req = PrefilterRequest {
            workflow_path: &wf,
            event: Some(&event),
            diff: false,
            changed_files: None,
            diff_base: None,
            diff_head: None,
            base_branch: None,
            activity_type: None,
            verbose: false,
            strict_filter: true,
        };
        let err = run_trigger_prefilter(req)
            .await
            .expect_err("strict mode must reject --event without --diff/--changed-files");
        assert!(
            err.contains("--diff") && err.contains("--changed-files"),
            "error must point the user at the three escape hatches, got: {}",
            err
        );
        assert!(
            err.contains("--no-strict-filter"),
            "error must name the legacy opt-out, got: {}",
            err
        );
    }

    #[tokio::test(flavor = "current_thread")]
    async fn non_strict_filter_allows_event_alone_with_warning_and_empty_change_set() {
        // Mirror of the strict-mode test: with `--no-strict-filter`
        // the caller opts back into the legacy warn-and-proceed
        // behavior, and the prefilter must build a context with an
        // empty change set rather than erroring. We don't assert on
        // the log output (wrkflw_logging::warning goes to a global
        // sink), just that the path does not error and that a
        // workflow gated on paths: will resolve to Skip cleanly.
        let tmp = tempfile::TempDir::new().expect("tempdir");
        let wf = tmp.path().join("ci.yml");
        std::fs::write(
            &wf,
            "name: ci\n\
             on:\n  push:\n    paths:\n      - 'src/**'\n\
             jobs:\n  build:\n    runs-on: ubuntu-latest\n    steps:\n      - run: echo hi\n",
        )
        .expect("write workflow");

        let event = "push".to_string();
        let req = PrefilterRequest {
            workflow_path: &wf,
            event: Some(&event),
            diff: false,
            changed_files: None,
            diff_base: None,
            diff_head: None,
            base_branch: None,
            activity_type: None,
            verbose: false,
            strict_filter: false,
        };
        let decision = run_trigger_prefilter(req)
            .await
            .expect("non-strict mode must not error on --event alone");
        match decision {
            PrefilterDecision::Skip { reason } => {
                // Empty change set against `paths: ['src/**']` must
                // surface as a Skip whose reason mentions the paths
                // filter — not a Proceed (which would run the
                // workflow against a phantom empty change set).
                assert!(
                    reason.contains("paths"),
                    "non-strict empty change set must Skip on a paths-gated \
                     workflow, got reason: {}",
                    reason
                );
            }
            PrefilterDecision::Proceed => {
                panic!(
                    "non-strict mode with empty change set must Skip a \
                     paths-gated workflow, got Proceed"
                );
            }
        }
    }

    #[tokio::test(flavor = "current_thread")]
    async fn strict_filter_rejects_pull_request_without_base_branch() {
        // Regression pin for `apply_base_branch` under strict mode:
        // simulating pull_request or pull_request_target without
        // --base-branch is the same silent-skip shape as --event
        // alone — every `branches:` filter on the event is
        // deterministically rejected because GHA evaluates those
        // against the PR target. Strict mode must refuse to proceed
        // instead of warn-and-continue.
        let tmp = tempfile::TempDir::new().expect("tempdir");
        let wf = tmp.path().join("ci.yml");
        std::fs::write(
            &wf,
            "name: ci\n\
             on:\n  pull_request:\n    branches:\n      - main\n\
             jobs:\n  build:\n    runs-on: ubuntu-latest\n    steps:\n      - run: echo hi\n",
        )
        .expect("write workflow");

        // Pass `--changed-files` so `build_event_context` doesn't
        // reject on the "no change set" path — we want the error to
        // come from `apply_base_branch` specifically.
        let event = "pull_request".to_string();
        let changed: Vec<String> = vec!["src/main.rs".to_string()];
        let req = PrefilterRequest {
            workflow_path: &wf,
            event: Some(&event),
            diff: false,
            changed_files: Some(&changed),
            diff_base: None,
            diff_head: None,
            base_branch: None,
            activity_type: None,
            verbose: false,
            strict_filter: true,
        };
        let err = run_trigger_prefilter(req)
            .await
            .expect_err("strict mode must reject pull_request without --base-branch");
        assert!(
            err.contains("--base-branch"),
            "error must point the user at --base-branch, got: {}",
            err
        );
        assert!(
            err.contains("pull_request"),
            "error must name the offending event, got: {}",
            err
        );
    }
}
