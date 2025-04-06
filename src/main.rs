mod evaluator;
mod executor;
mod logging;
mod models;
mod parser;
mod runtime;
mod ui;
mod utils;
mod validators;

use bollard::Docker;
use clap::{Parser, Subcommand};
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
    },

    /// Open TUI interface to manage workflows
    Tui {
        /// Path to workflow file or directory (defaults to .github/workflows)
        path: Option<PathBuf>,

        /// Use emulation mode instead of Docker
        #[arg(short, long)]
        emulate: bool,
    },
}

async fn cleanup_on_exit() {
    match Docker::connect_with_local_defaults() {
        Ok(docker) => {
            executor::cleanup_containers(&docker).await;
        }
        Err(_) => {
            // Docker not available, nothing to clean up
        }
    }
}

async fn handle_signals() {
    // Wait for Ctrl+C
    tokio::signal::ctrl_c()
        .await
        .expect("Failed to listen for ctrl+c event");

    println!("Received Ctrl+C, shutting down and cleaning up...");

    // Clean up containers
    cleanup_on_exit().await;

    // Exit with success status
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

        Some(Commands::Run { path, emulate }) => {
            // Run the workflow execution
            let runtime_type = if *emulate {
                executor::RuntimeType::Emulation
            } else {
                executor::RuntimeType::Docker
            };

            // Run in TUI mode with the specific workflow
            match ui::run_wrkflw_tui(Some(path), runtime_type, verbose).await {
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

        Some(Commands::Tui { path, emulate }) => {
            // Open the TUI interface
            let runtime_type = if *emulate {
                executor::RuntimeType::Emulation
            } else {
                executor::RuntimeType::Docker
            };

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

        None => {
            // Default to TUI interface if no subcommand
            match ui::run_wrkflw_tui(
                Some(&PathBuf::from(".github/workflows")),
                executor::RuntimeType::Docker,
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
