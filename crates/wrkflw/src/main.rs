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
    },

    /// Open TUI interface to manage workflows
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
        }) => {
            // Create execution configuration
            let config = wrkflw_executor::ExecutionConfig {
                runtime_type: runtime.clone().into(),
                verbose,
                preserve_containers_on_failure: *preserve_containers_on_failure,
                secrets_config: None, // Use default secrets configuration
                show_action_messages: *show_action_messages,
                target_job: job.clone(),
            };

            // Check if we're explicitly or implicitly running a GitLab pipeline
            let is_gitlab = *gitlab || is_gitlab_pipeline(path);
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
            if result.failure_details.is_some() {
                eprintln!("❌ Workflow execution failed:");
                if let Some(details) = result.failure_details {
                    if verbose {
                        // Show full error details in verbose mode
                        eprintln!("{}", details);
                    } else {
                        // Show simplified error info in non-verbose mode
                        let simplified_error = details
                            .lines()
                            .filter(|line| line.contains("❌") || line.trim().starts_with("Error:"))
                            .take(5) // Limit to the first 5 error lines
                            .collect::<Vec<&str>>()
                            .join("\n");

                        eprintln!("{}", simplified_error);

                        if details.lines().count() > 5 {
                            eprintln!("\nUse --verbose flag to see full error details");
                        }
                    }
                }
                std::process::exit(1);
            } else {
                println!("✅ Workflow execution completed successfully!");

                // Print a summary of executed jobs
                println!("\nJob summary:");
                for job in result.jobs {
                    println!(
                        "  {} {} ({})",
                        match job.status {
                            wrkflw_executor::JobStatus::Success => "✅",
                            wrkflw_executor::JobStatus::Failure => "❌",
                            wrkflw_executor::JobStatus::Skipped => "⏭️",
                        },
                        job.name,
                        match job.status {
                            wrkflw_executor::JobStatus::Success => "success",
                            wrkflw_executor::JobStatus::Failure => "failure",
                            wrkflw_executor::JobStatus::Skipped => "skipped",
                        }
                    );

                    // Always show steps, not just in debug mode
                    println!("  Steps:");
                    for step in job.steps {
                        let step_status = match step.status {
                            wrkflw_executor::StepStatus::Success => "✅",
                            wrkflw_executor::StepStatus::Failure => "❌",
                            wrkflw_executor::StepStatus::Skipped => "⏭️",
                        };

                        println!("    {} {}", step_status, step.name);

                        // If step failed and we're not in verbose mode, show condensed error info
                        if step.status == wrkflw_executor::StepStatus::Failure && !verbose {
                            // Extract error information from step output
                            let error_lines = step
                                .output
                                .lines()
                                .filter(|line| {
                                    line.contains("error:")
                                        || line.contains("Error:")
                                        || line.trim().starts_with("Exit code:")
                                        || line.contains("failed")
                                })
                                .take(3) // Limit to 3 most relevant error lines
                                .collect::<Vec<&str>>();

                            if !error_lines.is_empty() {
                                println!("      Error details:");
                                for line in error_lines {
                                    println!("      {}", line.trim());
                                }

                                if step.output.lines().count() > 3 {
                                    println!("      (Use --verbose for full output)");
                                }
                            }
                        }
                    }
                }
            }

            // Cleanup is handled automatically via the signal handler
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
    }
}

/// Validate a GitHub workflow file
/// Returns true if validation failed, false if it passed
fn validate_github_workflow(path: &Path, verbose: bool) -> bool {
    print!("Validating GitHub workflow file: {}... ", path.display());

    match wrkflw_evaluator::evaluate_workflow_file(path, verbose) {
        Ok(result) => {
            if result.is_valid {
                println!("✅ Valid");
                if verbose {
                    println!("  All validation checks passed");
                }
            } else {
                println!("❌ Invalid");
                for (i, issue) in result.issues.iter().enumerate() {
                    println!("   {}. {}", i + 1, issue);
                }
            }
            !result.is_valid
        }
        Err(e) => {
            println!("❌ Error");
            eprintln!("  {}", e);
            true // Parse errors count as validation failure
        }
    }
}

/// Validate a GitLab CI/CD pipeline file
/// Returns true if validation failed, false if it passed
fn validate_gitlab_pipeline(path: &Path, verbose: bool) -> bool {
    print!("Validating GitLab CI pipeline file: {}... ", path.display());

    // Parse and validate the pipeline file
    match wrkflw_parser::gitlab::parse_pipeline(path) {
        Ok(pipeline) => {
            println!("✅ Valid syntax");

            // Additional structural validation
            let validation_result = wrkflw_validators::validate_gitlab_pipeline(&pipeline);

            if !validation_result.is_valid {
                println!("⚠️  Validation issues:");
                for issue in validation_result.issues {
                    println!("   - {}", issue);
                }
                true // Validation failed
            } else {
                if verbose {
                    println!("✅ All validation checks passed");
                }
                false // Validation passed
            }
        }
        Err(e) => {
            println!("❌ Invalid");
            eprintln!("Validation failed: {}", e);
            true // Parse error counts as validation failure
        }
    }
}

/// List available workflows and pipelines in the repository
fn list_workflows_and_pipelines(verbose: bool, show_jobs: bool) {
    // Check for GitHub workflows
    let github_path = PathBuf::from(".github/workflows");
    if github_path.exists() && github_path.is_dir() {
        println!("GitHub Workflows:");

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
                    println!("  No workflow files found in .github/workflows");
                } else {
                    for entry in entries {
                        println!("  - {}", entry.path().display());
                        if show_jobs {
                            match wrkflw_parser::workflow::parse_workflow(&entry.path()) {
                                Ok(workflow) => {
                                    let mut job_names: Vec<&String> =
                                        workflow.jobs.keys().collect();
                                    job_names.sort();
                                    println!(
                                        "      Jobs: {}",
                                        job_names
                                            .iter()
                                            .map(|s| s.as_str())
                                            .collect::<Vec<_>>()
                                            .join(", ")
                                    );
                                }
                                Err(e) => {
                                    eprintln!("      Could not parse workflow: {}", e);
                                }
                            }
                        }
                    }
                }
            }
            Err(e) => {
                eprintln!(
                    "  Failed to read directory {}: {}",
                    github_path.display(),
                    e
                );
            }
        }
    } else {
        println!("GitHub Workflows: No .github/workflows directory found");
    }

    // Check for GitLab CI pipeline
    let gitlab_path = PathBuf::from(".gitlab-ci.yml");
    if gitlab_path.exists() && gitlab_path.is_file() {
        println!("GitLab CI Pipeline:");
        println!("  - {}", gitlab_path.display());
        if show_jobs {
            match wrkflw_parser::gitlab::parse_pipeline(Path::new(".gitlab-ci.yml")) {
                Ok(pipeline) => {
                    let mut job_names: Vec<&String> = pipeline.jobs.keys().collect();
                    job_names.sort();
                    println!(
                        "      Jobs: {}",
                        job_names
                            .iter()
                            .map(|s| s.as_str())
                            .collect::<Vec<_>>()
                            .join(", ")
                    );
                }
                Err(e) => {
                    eprintln!("      Could not parse pipeline: {}", e);
                }
            }
        }
    } else {
        println!("GitLab CI Pipeline: No .gitlab-ci.yml file found");
    }

    // Check for other GitLab CI pipeline files
    if verbose {
        println!("Searching for other GitLab CI pipeline files...");

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
            println!("Additional GitLab CI Pipeline files:");
            for entry in entries {
                println!("  - {}", entry.path().display());
            }
        }
    }
}
