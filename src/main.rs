mod cleanup_test;
mod evaluator;
mod executor;
mod github;
mod logging;
mod matrix;
mod matrix_test;
mod models;
mod parser;
mod runtime;
mod ui;
mod utils;
mod validators;

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

// Make this function public for testing
pub async fn cleanup_on_exit() {
    // Clean up Docker resources if available, but don't let it block indefinitely
    match tokio::time::timeout(std::time::Duration::from_secs(3), async {
        match Docker::connect_with_local_defaults() {
            Ok(docker) => {
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

            // Run the validation
            ui::validate_workflow(&validate_path, verbose).unwrap_or_else(|e| {
                eprintln!("Error: {}", e);
                std::process::exit(1);
            });
        }

        Some(Commands::Run {
            path,
            emulate,
            show_action_messages: _,
        }) => {
            // Set runner mode based on flags
            let runtime_type = if *emulate {
                executor::RuntimeType::Emulation
            } else {
                executor::RuntimeType::Docker
            };

            // First validate the workflow file
            match parser::workflow::parse_workflow(path) {
                Ok(_) => logging::info("Validating workflow..."),
                Err(e) => {
                    logging::error(&format!("Workflow validation failed: {}", e));
                    std::process::exit(1);
                }
            }

            // Execute the workflow
            match executor::execute_workflow(path, runtime_type, verbose || debug).await {
                Ok(result) => {
                    // Display execution results
                    println!();
                    println!("Workflow execution results:");
                    println!();

                    // Check if we had any job failures
                    let mut any_job_failed = false;

                    // Summarize each job
                    for job in result.jobs {
                        match job.status {
                            executor::JobStatus::Success => {
                                println!("✅ Job succeeded: {}", job.name);
                            }
                            executor::JobStatus::Failure => {
                                any_job_failed = true;
                                println!("❌ Job failed: {}", job.name);
                            }
                            executor::JobStatus::Skipped => {
                                println!("⏭️ Job skipped: {}", job.name);
                            }
                        }
                        println!("-------------------------");

                        // Show individual steps
                        for step in job.steps {
                            let status_symbol = match step.status {
                                executor::StepStatus::Success => "✅",
                                executor::StepStatus::Failure => "❌",
                                executor::StepStatus::Skipped => "⏭️",
                            };
                            println!("  {} {}", status_symbol, step.name);

                            // Show output for any step if in verbose/debug mode or for failed steps always
                            if (verbose || debug) || step.status == executor::StepStatus::Failure {
                                // Show output if not empty and in verbose mode
                                if !step.output.trim().is_empty() {
                                    // If the output is very long, trim it
                                    let output_lines = step.output.lines().collect::<Vec<&str>>();
                                    
                                    // In verbose mode, show more output
                                    let max_lines = if verbose || debug { 
                                        std::cmp::min(15, output_lines.len()) 
                                    } else { 
                                        std::cmp::min(5, output_lines.len()) 
                                    };
                                    
                                    println!("    Output:");
                                    for line in output_lines.iter().take(max_lines) {
                                        println!("    {}", line);
                                    }
                                    
                                    if output_lines.len() > max_lines {
                                        println!("    ... ({} more lines, use --debug to see full output)", 
                                            output_lines.len() - max_lines);
                                    }
                                    println!();
                                }
                            }
                        }
                        println!();
                    }

                    if any_job_failed {
                        println!("❌ Workflow completed with errors!");
                        std::process::exit(1);
                    } else {
                        println!("✅ Workflow completed successfully!");
                    }
                }
                Err(e) => {
                    println!();
                    println!("❌ Workflow execution failed: {}", e);
                    std::process::exit(1);
                }
            };
        }

        Some(Commands::Tui {
            path,
            emulate,
            show_action_messages,
        }) => {
            // Open the TUI interface
            let runtime_type = if *emulate {
                executor::RuntimeType::Emulation
            } else {
                // Check if Docker is available, fall back to emulation if not
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
                    cleanup_on_exit().await;
                    std::process::exit(1);
                }
            }
        }

        Some(Commands::Trigger {
            workflow,
            branch,
            input,
        }) => {
            let inputs = input.as_ref().map(|kv_pairs| {
                kv_pairs
                    .iter()
                    .cloned()
                    .collect::<HashMap<String, String>>()
            });

            match github::trigger_workflow(workflow, branch.as_deref(), inputs.clone()).await {
                Ok(_) => {
                    // Success is already reported in the github module with detailed info
                }
                Err(e) => {
                    eprintln!("Error triggering workflow: {}", e);
                    std::process::exit(1);
                }
            }
        }

        Some(Commands::List) => match github::get_repo_info() {
            Ok(repo_info) => match github::list_workflows(&repo_info).await {
                Ok(workflows) => {
                    if workflows.is_empty() {
                        println!("No workflows found in the .github/workflows directory");
                    } else {
                        println!("Available workflows:");
                        for workflow in workflows {
                            println!("  {}", workflow);
                        }
                        println!("\nTrigger a workflow with: wrkflw trigger <workflow> [options]");
                    }
                }
                Err(e) => {
                    eprintln!("Error listing workflows: {}", e);
                    std::process::exit(1);
                }
            },
            Err(e) => {
                eprintln!("Error getting repository info: {}", e);
                std::process::exit(1);
            }
        },

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
                    cleanup_on_exit().await;
                    std::process::exit(1);
                }
            }
        }
    }

    // Final cleanup before program exit
    cleanup_on_exit().await;
}
