use bollard::Docker;
use clap::{Parser, Subcommand};
use std::collections::HashMap;
use std::path::PathBuf;

#[derive(Debug, Parser)]
#[command(
    name = "wrkflw",
    about = "GitHub Workflow validator and executor",
    version,
    long_about = "A GitHub Workflow validator and executor that runs workflows locally.\n\nExamples:\n  wrkflw validate                             # Validate all workflows in .github/workflows\n  wrkflw run .github/workflows/build.yml      # Run a specific workflow\n  wrkflw --verbose run .github/workflows/build.yml  # Run with more output\n  wrkflw --debug run .github/workflows/build.yml    # Run with detailed debug information\n  wrkflw run --emulate .github/workflows/build.yml  # Use emulation mode instead of Docker"
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
    /// Validate GitHub workflow files
    Validate {
        /// Path to workflow file or directory (defaults to .github/workflows)
        path: Option<PathBuf>,
    },

    /// Execute GitHub workflow files locally
    Run {
        /// Path to workflow file to execute
        path: PathBuf,

        /// Use emulation mode instead of Docker
        #[arg(short, long)]
        emulate: bool,

        /// Show 'Would execute GitHub action' messages in emulation mode
        #[arg(long, default_value_t = false)]
        show_action_messages: bool,
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

    /// List available workflows
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
        Some(Commands::Validate { path }) => {
            // Determine the path to validate
            let validate_path = path
                .clone()
                .unwrap_or_else(|| PathBuf::from(".github/workflows"));

            // Run the validation using ui crate
            ui::validate_workflow(&validate_path, verbose).unwrap_or_else(|e| {
                eprintln!("Error: {}", e);
                std::process::exit(1);
            });
        }

        Some(Commands::Run {
            path,
            emulate,
            show_action_messages: _, // Assuming this flag is handled within executor/runtime
        }) => {
            // Set runner mode based on flags
            let runtime_type = if *emulate {
                executor::RuntimeType::Emulation
            } else {
                executor::RuntimeType::Docker
            };

            // First validate the workflow file using parser crate
            match parser::workflow::parse_workflow(path) {
                Ok(_) => logging::info("Validating workflow..."),
                Err(e) => {
                    logging::error(&format!("Workflow validation failed: {}", e));
                    std::process::exit(1);
                }
            }

            // Execute the workflow using executor crate
            match executor::execute_workflow(path, runtime_type, verbose || debug).await {
                Ok(result) => {
                    // Print job results
                    for job in &result.jobs {
                        println!(
                            "\n{} Job {}: {}",
                            if job.status == executor::JobStatus::Success {
                                "✅"
                            } else {
                                "❌"
                            },
                            job.name,
                            if job.status == executor::JobStatus::Success {
                                "succeeded"
                            } else {
                                "failed"
                            }
                        );

                        // Print step results
                        for step in &job.steps {
                            println!(
                                "  {} {}",
                                if step.status == executor::StepStatus::Success {
                                    "✅"
                                } else {
                                    "❌"
                                },
                                step.name
                            );

                            if !step.output.trim().is_empty() {
                                // If the output is very long, trim it
                                let output_lines = step.output.lines().collect::<Vec<&str>>();

                                println!("    Output:");

                                // In verbose mode, show complete output
                                if verbose || debug {
                                    for line in &output_lines {
                                        println!("    {}", line);
                                    }
                                } else {
                                    // Show only the first few lines
                                    let max_lines = 5;
                                    for line in output_lines.iter().take(max_lines) {
                                        println!("    {}", line);
                                    }

                                    if output_lines.len() > max_lines {
                                        println!("    ... ({} more lines, use --verbose to see full output)",
                                            output_lines.len() - max_lines);
                                    }
                                }
                            }
                        }
                    }

                    // Print detailed failure information if available
                    if let Some(failure_details) = &result.failure_details {
                        println!("\n❌ Workflow execution failed!");
                        println!("{}", failure_details);
                        println!("\nTo fix these issues:");
                        println!("1. Check the formatting issues with: cargo fmt");
                        println!("2. Fix clippy warnings with: cargo clippy -- -D warnings");
                        println!("3. Run tests to ensure everything passes: cargo test");
                        std::process::exit(1);
                    } else {
                        println!("\n✅ Workflow completed successfully!");
                    }
                }
                Err(e) => {
                    logging::error(&format!("Workflow execution failed: {}", e));
                    std::process::exit(1);
                }
            }
        }

        Some(Commands::Tui {
            path,
            emulate,
            show_action_messages,
        }) => {
            // Open the TUI interface using ui crate
            let runtime_type = if *emulate {
                executor::RuntimeType::Emulation
            } else {
                // Check if Docker is available, fall back to emulation if not
                // Assuming executor::docker::is_available() exists
                if !executor::docker::is_available() {
                    println!("⚠️ Docker is not available. Using emulation mode instead.");
                    logging::warning("Docker is not available. Using emulation mode instead.");
                    executor::RuntimeType::Emulation
                } else {
                    executor::RuntimeType::Docker
                }
            };

            // Control hiding action messages based on the flag
            if !show_action_messages {
                std::env::set_var("WRKFLW_HIDE_ACTION_MESSAGES", "true");
            } else {
                std::env::set_var("WRKFLW_HIDE_ACTION_MESSAGES", "false");
            }

            match ui::run_wrkflw_tui(path.as_ref(), runtime_type, verbose).await {
                Ok(_) => {
                    // Clean up on successful exit
                    cleanup_on_exit().await;
                }
                Err(e) => {
                    eprintln!("Error: {}", e);
                    cleanup_on_exit().await; // Ensure cleanup even on error
                    std::process::exit(1);
                }
            }
        }

        Some(Commands::Trigger {
            workflow,
            branch,
            input,
        }) => {
            logging::info(&format!("Triggering workflow {} on GitHub", workflow));

            // Convert inputs to HashMap
            let input_map = input.as_ref().map(|i| {
                i.iter()
                    .map(|(k, v)| (k.clone(), v.clone()))
                    .collect::<HashMap<String, String>>()
            });

            // Use github crate
            match github::trigger_workflow(workflow, branch.as_deref(), input_map).await {
                Ok(_) => logging::info("Workflow triggered successfully"),
                Err(e) => {
                    eprintln!("Error triggering workflow: {}", e);
                    std::process::exit(1);
                }
            }
        }

        Some(Commands::TriggerGitlab { branch, variable }) => {
            logging::info("Triggering pipeline on GitLab");

            // Convert variables to HashMap
            let variable_map = variable.as_ref().map(|v| {
                v.iter()
                    .map(|(k, v)| (k.clone(), v.clone()))
                    .collect::<HashMap<String, String>>()
            });

            // Use gitlab crate
            match gitlab::trigger_pipeline(branch.as_deref(), variable_map).await {
                Ok(_) => logging::info("GitLab pipeline triggered successfully"),
                Err(e) => {
                    eprintln!("Error triggering GitLab pipeline: {}", e);
                    std::process::exit(1);
                }
            }
        }

        Some(Commands::List) => {
            logging::info("Listing available workflows");

            // Attempt to get GitHub repo info using github crate
            if let Ok(repo_info) = github::get_repo_info() {
                match github::list_workflows(&repo_info).await {
                    Ok(workflows) => {
                        if workflows.is_empty() {
                            println!("No GitHub workflows found in repository");
                        } else {
                            println!("GitHub workflows:");
                            for workflow in workflows {
                                println!("  {}", workflow);
                            }
                        }
                    }
                    Err(e) => {
                        eprintln!("Error listing GitHub workflows: {}", e);
                    }
                }
            } else {
                println!("Not a GitHub repository or unable to get repository information");
            }

            // Attempt to get GitLab repo info using gitlab crate
            if let Ok(repo_info) = gitlab::get_repo_info() {
                match gitlab::list_pipelines(&repo_info).await {
                    Ok(pipelines) => {
                        if pipelines.is_empty() {
                            println!("No GitLab pipelines found in repository");
                        } else {
                            println!("GitLab pipelines:");
                            for pipeline in pipelines {
                                println!("  {}", pipeline);
                            }
                        }
                    }
                    Err(e) => {
                        eprintln!("Error listing GitLab pipelines: {}", e);
                    }
                }
            } else {
                println!("Not a GitLab repository or unable to get repository information");
            }
        }

        None => {
            // Default to TUI interface if no subcommand
            // Check if Docker is available, fall back to emulation if not
            let runtime_type = if !executor::docker::is_available() {
                println!("⚠️ Docker is not available. Using emulation mode instead.");
                logging::warning("Docker is not available. Using emulation mode instead.");
                executor::RuntimeType::Emulation
            } else {
                executor::RuntimeType::Docker
            };

            // Set environment variable to hide action messages by default
            std::env::set_var("WRKFLW_HIDE_ACTION_MESSAGES", "true");

            match ui::run_wrkflw_tui(
                Some(&PathBuf::from(".github/workflows")),
                runtime_type,
                verbose,
            )
            .await
            {
                Ok(_) => {
                    // Clean up on successful exit
                    cleanup_on_exit().await;
                }
                Err(e) => {
                    eprintln!("Error: {}", e);
                    cleanup_on_exit().await; // Ensure cleanup even on error
                    std::process::exit(1);
                }
            }
        }
    }

    // Final cleanup before program exit (redundant if called on success/error/signal?)
    // Consider if this final call is necessary given the calls in Ok/Err/signal handlers.
    // It might be okay as a safety net, but ensure cleanup_on_exit is idempotent.
    // cleanup_on_exit().await; // Keep or remove based on idempotency review
}
