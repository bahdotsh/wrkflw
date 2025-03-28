mod evaluator;
mod models;
mod ui;
mod utils;
mod validators;

use clap::{Parser, Subcommand};
use std::path::PathBuf;

#[derive(Debug, Parser)]
#[command(name = "wrkflw", about = "GitHub Workflow validator", version)]
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
}

fn main() {
    let cli = Cli::parse();
    let verbose = cli.verbose;

    // Determine the path to validate
    let path = match &cli.command {
        Some(Commands::Validate { path }) => path
            .clone()
            .unwrap_or_else(|| PathBuf::from(".github/workflows")),
        None => PathBuf::from(".github/workflows"),
    };

    // Run the validation
    ui::validate_target(&path, verbose);
}
