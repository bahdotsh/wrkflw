use bollard::Docker;
use clap::{Parser, Subcommand};
use std::collections::HashMap;
use std::path::Path;
use std::path::PathBuf;

#[derive(Debug, Parser)]
#[command(
    name = "wrkflw",
    about = "GitHub & GitLab CI/CD validator and executor",
    version,
    long_about = "A CI/CD validator and executor that runs workflows locally.\n\nExamples:\n  wrkflw validate                             # Validate all workflows in .github/workflows\n  wrkflw run .github/workflows/build.yml      # Run a specific workflow\n  wrkflw run .gitlab-ci.yml                   # Run a GitLab CI pipeline\n  wrkflw --verbose run .github/workflows/build.yml  # Run with more output\n  wrkflw --debug run .github/workflows/build.yml    # Run with detailed debug information\n  wrkflw run --emulate .github/workflows/build.yml  # Use emulation mode instead of Docker"
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
        /// Path to workflow/pipeline file or directory (defaults to .github/workflows)
        path: Option<PathBuf>,

        /// Explicitly validate as GitLab CI/CD pipeline
        #[arg(long)]
        gitlab: bool,
    },

    /// Execute workflow or pipeline files locally
    Run {
        /// Path to workflow/pipeline file to execute
        path: PathBuf,

        /// Use emulation mode instead of Docker
        #[arg(short, long)]
        emulate: bool,

        /// Show 'Would execute GitHub action' messages in emulation mode
        #[arg(long, default_value_t = false)]
        show_action_messages: bool,

        /// Explicitly run as GitLab CI/CD pipeline
        #[arg(long)]
        gitlab: bool,
    },

    /// Open TUI interface to manage workflows
    Tui {
        /// Path to workflow file or directory (defaults to .github/workflows)
        path: Option<PathBuf>,

        /// Use emulation mode instead of Docker
        #[arg(short, long)]
        emulate: bool,

        /// Show 'Would execute GitHub action' messages in emulation mode
        #[arg(long, default_value_t = false)]
        show_action_messages: bool,
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
    List,
}

// Parser function for key-value pairs
fn parse_key_val(s: &str) -> Result<(String, String), String> {
    let pos = s
        .find('=')
        .ok_or_else(|| format!("invalid KEY=value: no `=` found in `{}`", s))?;

    Ok((s[..pos].to_string(), s[pos + 1..].to_string()))
}

// Make this function public for testing? Or move to a utils/cleanup mod?
// Or call executor::cleanup and runtime::cleanup directly?
// Let's try calling them directly for now.
async fn cleanup_on_exit() {
    // Clean up Docker resources if available, but don't let it block indefinitely
    match tokio::time::timeout(std::time::Duration::from_secs(3), async {
        match Docker::connect_with_local_defaults() {
            Ok(docker) => {
                // Assuming cleanup_resources exists in executor crate
                executor::cleanup_resources(&docker).await;
            }
            Err(_) => {
                // Docker not available
                logging::info("Docker not available, skipping Docker cleanup");
            }
        }
    })
    .await
    {
        Ok(_) => logging::debug("Docker cleanup completed successfully"),
        Err(_) => {
            logging::warning("Docker cleanup timed out after 3 seconds, continuing with shutdown")
        }
    }

    // Always clean up emulation resources
    match tokio::time::timeout(
        std::time::Duration::from_secs(2),
        // Assuming cleanup_resources exists in runtime::emulation module
        runtime::emulation::cleanup_resources(),
    )
    .await
    {
        Ok(_) => logging::debug("Emulation cleanup completed successfully"),
        Err(_) => logging::warning("Emulation cleanup timed out, continuing with shutdown"),
    }

    logging::info("Resource cleanup completed");
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
        logging::error("Forced exit due to cleanup timeout");
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
    let cli = Wrkflw::parse();
    let verbose = cli.verbose;
    let debug = cli.debug;

    // Set log level based on command line flags
    if debug {
        logging::set_log_level(logging::LogLevel::Debug);
        logging::debug("Debug mode enabled - showing detailed logs");
    } else if verbose {
        logging::set_log_level(logging::LogLevel::Info);
        logging::info("Verbose mode enabled");
    } else {
        logging::set_log_level(logging::LogLevel::Warning);
    }

    // Setup a Ctrl+C handler that runs in the background
    tokio::spawn(handle_signals());

    match &cli.command {
        Some(Commands::Validate { path, gitlab }) => {
            // Determine the path to validate
            let validate_path = path
                .clone()
                .unwrap_or_else(|| PathBuf::from(".github/workflows"));

            // Check if the path exists
            if !validate_path.exists() {
                eprintln!("Error: Path does not exist: {}", validate_path.display());
                std::process::exit(1);
            }

            // Determine if we're validating a GitLab pipeline based on the --gitlab flag or file detection
            let force_gitlab = *gitlab;

            if validate_path.is_dir() {
                // Validate all workflow files in the directory
                let entries = std::fs::read_dir(&validate_path)
                    .expect("Failed to read directory")
                    .filter_map(|entry| entry.ok())
                    .filter(|entry| {
                        entry.path().is_file()
                            && entry
                                .path()
                                .extension()
                                .is_some_and(|ext| ext == "yml" || ext == "yaml")
                    })
                    .collect::<Vec<_>>();

                println!("Validating {} workflow file(s)...", entries.len());

                for entry in entries {
                    let path = entry.path();
                    let is_gitlab = force_gitlab || is_gitlab_pipeline(&path);

                    if is_gitlab {
                        validate_gitlab_pipeline(&path, verbose);
                    } else {
                        validate_github_workflow(&path, verbose);
                    }
                }
            } else {
                // Validate a single workflow file
                let is_gitlab = force_gitlab || is_gitlab_pipeline(&validate_path);

                if is_gitlab {
                    validate_gitlab_pipeline(&validate_path, verbose);
                } else {
                    validate_github_workflow(&validate_path, verbose);
                }
            }
        }
        Some(Commands::Run {
            path,
            emulate,
            show_action_messages: _,
            gitlab,
        }) => {
            // Determine the runtime type
            let runtime_type = if *emulate {
                executor::RuntimeType::Emulation
            } else {
                executor::RuntimeType::Docker
            };

            // Check if we're explicitly or implicitly running a GitLab pipeline
            let is_gitlab = *gitlab || is_gitlab_pipeline(path);
            let workflow_type = if is_gitlab {
                "GitLab CI pipeline"
            } else {
                "GitHub workflow"
            };

            logging::info(&format!("Running {} at: {}", workflow_type, path.display()));

            // Execute the workflow
            let result = executor::execute_workflow(path, runtime_type, verbose)
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
                if true {
                    // Always show job summary
                    println!("\nJob summary:");
                    for job in result.jobs {
                        println!(
                            "  {} {} ({})",
                            match job.status {
                                executor::JobStatus::Success => "✅",
                                executor::JobStatus::Failure => "❌",
                                executor::JobStatus::Skipped => "⏭️",
                            },
                            job.name,
                            match job.status {
                                executor::JobStatus::Success => "success",
                                executor::JobStatus::Failure => "failure",
                                executor::JobStatus::Skipped => "skipped",
                            }
                        );

                        // Always show steps, not just in debug mode
                        println!("  Steps:");
                        for step in job.steps {
                            let step_status = match step.status {
                                executor::StepStatus::Success => "✅",
                                executor::StepStatus::Failure => "❌",
                                executor::StepStatus::Skipped => "⏭️",
                            };

                            println!("    {} {}", step_status, step.name);

                            // If step failed and we're not in verbose mode, show condensed error info
                            if step.status == executor::StepStatus::Failure && !verbose {
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
            }

            // Cleanup is handled automatically via the signal handler
        }
        Some(Commands::TriggerGitlab { branch, variable }) => {
            // Convert optional Vec<(String, String)> to Option<HashMap<String, String>>
            let variables = variable
                .as_ref()
                .map(|v| v.iter().cloned().collect::<HashMap<String, String>>());

            // Trigger the pipeline
            if let Err(e) = gitlab::trigger_pipeline(branch.as_deref(), variables).await {
                eprintln!("Error triggering GitLab pipeline: {}", e);
                std::process::exit(1);
            }
        }
        Some(Commands::Tui {
            path,
            emulate,
            show_action_messages: _,
        }) => {
            // Set runtime type based on the emulate flag
            let runtime_type = if *emulate {
                executor::RuntimeType::Emulation
            } else {
                executor::RuntimeType::Docker
            };

            // Call the TUI implementation from the ui crate
            if let Err(e) = ui::run_wrkflw_tui(path.as_ref(), runtime_type, verbose).await {
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
            if let Err(e) = github::trigger_workflow(workflow, branch.as_deref(), inputs).await {
                eprintln!("Error triggering GitHub workflow: {}", e);
                std::process::exit(1);
            }
        }
        Some(Commands::List) => {
            list_workflows_and_pipelines(verbose);
        }
        None => {
            // Launch TUI by default when no command is provided
            let runtime_type = executor::RuntimeType::Docker;

            // Call the TUI implementation from the ui crate with default path
            if let Err(e) = ui::run_wrkflw_tui(None, runtime_type, verbose).await {
                eprintln!("Error running TUI: {}", e);
                std::process::exit(1);
            }
        }
    }
}

/// Validate a GitHub workflow file
fn validate_github_workflow(path: &Path, verbose: bool) {
    print!("Validating GitHub workflow file: {}... ", path.display());

    // Use the ui crate's validate_workflow function
    match ui::validate_workflow(path, verbose) {
        Ok(_) => {
            // The detailed validation output is already printed by the function
        }
        Err(e) => {
            eprintln!("Error validating workflow: {}", e);
        }
    }
}

/// Validate a GitLab CI/CD pipeline file
fn validate_gitlab_pipeline(path: &Path, verbose: bool) {
    print!("Validating GitLab CI pipeline file: {}... ", path.display());

    // Parse and validate the pipeline file
    match parser::gitlab::parse_pipeline(path) {
        Ok(pipeline) => {
            println!("✅ Valid syntax");

            // Additional structural validation
            let validation_result = validators::validate_gitlab_pipeline(&pipeline);

            if !validation_result.is_valid {
                println!("⚠️  Validation issues:");
                for issue in validation_result.issues {
                    println!("   - {}", issue);
                }
            } else if verbose {
                println!("✅ All validation checks passed");
            }
        }
        Err(e) => {
            println!("❌ Invalid");
            eprintln!("Validation failed: {}", e);
        }
    }
}

/// List available workflows and pipelines in the repository
fn list_workflows_and_pipelines(verbose: bool) {
    // Check for GitHub workflows
    let github_path = PathBuf::from(".github/workflows");
    if github_path.exists() && github_path.is_dir() {
        println!("GitHub Workflows:");

        let entries = std::fs::read_dir(&github_path)
            .expect("Failed to read directory")
            .filter_map(|entry| entry.ok())
            .filter(|entry| {
                entry.path().is_file()
                    && entry
                        .path()
                        .extension()
                        .is_some_and(|ext| ext == "yml" || ext == "yaml")
            })
            .collect::<Vec<_>>();

        if entries.is_empty() {
            println!("  No workflow files found in .github/workflows");
        } else {
            for entry in entries {
                println!("  - {}", entry.path().display());
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
