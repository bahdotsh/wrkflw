mod evaluator;
mod executor;
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
struct Cli {
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
}

#[tokio::main]
async fn main() {
    let cli = Cli::parse();
    let verbose = cli.verbose;

    match &cli.command {
        Some(Commands::Validate { path }) => {
            // Determine the path to validate
            let validate_path = path
                .clone()
                .unwrap_or_else(|| PathBuf::from(".github/workflows"));

            // Run the validation
            ui::validate_target(&validate_path, verbose);
        }

        Some(Commands::Run { path, emulate }) => {
            // Run the workflow execution
            let runtime_type = if *emulate {
                executor::RuntimeType::Emulation
            } else {
                executor::RuntimeType::Docker
            };

            ui::execute_workflow(path, runtime_type, verbose).await;
        }

        None => {
            // Default to validation if no subcommand
            ui::validate_target(&PathBuf::from(".github/workflows"), verbose);
        }
    }
}
