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
    version
)]
struct Wrkflw {
    #[command(subcommand)]
    command: Option<Commands>,

    /// Run in verbose mode with detailed output
    #[arg(short, long, global = true)]
    verbose: bool,
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
            show_action_messages,
        }) => {
            // Run the workflow execution
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

            // Run in CLI mode with the specific workflow
            match ui::execute_workflow_cli(path, runtime_type, verbose).await {
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
