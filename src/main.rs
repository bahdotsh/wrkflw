mod evaluator;
mod executor;
mod logging;
mod models;
mod parser;
mod runtime;
mod ui;
mod utils;
mod validators;

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

#[tokio::main]
async fn main() {
    let cli = Wrkflw::parse();
    let verbose = cli.verbose;

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
            ui::run_wrkflw_tui(Some(path), runtime_type, verbose).await;
        }

        Some(Commands::Tui { path, emulate }) => {
            // Open the TUI interface
            let runtime_type = if *emulate {
                executor::RuntimeType::Emulation
            } else {
                executor::RuntimeType::Docker
            };

            ui::run_wrkflw_tui(path.as_ref(), runtime_type, verbose).await;
        }

        None => {
            // Default to TUI interface if no subcommand
            ui::run_wrkflw_tui(
                Some(&PathBuf::from(".github/workflows")),
                executor::RuntimeType::Docker,
                verbose,
            )
            .await;
        }
    }
}
