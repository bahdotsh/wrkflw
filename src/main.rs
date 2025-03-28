mod evaluator;
mod models;
mod utils;
mod validators;

use clap::Parser;
use colored::*;
use evaluator::evaluate_workflow_file;
use std::path::PathBuf;
use std::process;
use utils::is_workflow_file;

#[derive(Debug, Parser)]
#[command(name = "wrkflw", about = "GitHub Workflow evaluator")]
struct Wrkflw {
    /// Path to the workflow file or directory containing workflow files
    path: PathBuf,

    /// Run in verbose mode with detailed output
    #[arg(short, long)]
    verbose: bool,
}

fn main() {
    let opt = Wrkflw::parse();

    let path = &opt.path;
    if !path.exists() {
        eprintln!("{}", "Error: Path does not exist".red());
        process::exit(1);
    }

    if path.is_dir() {
        evaluate_directory(path, opt.verbose);
    } else {
        match evaluate_workflow_file(path, opt.verbose) {
            Ok(result) => {
                if result.is_valid {
                    println!(
                        "{} {}",
                        "✓".green(),
                        format!("Workflow file is valid: {}", path.display()).green()
                    );
                } else {
                    println!(
                        "{} {}",
                        "✗".red(),
                        format!("Workflow file has issues: {}", path.display()).red()
                    );
                    for (i, issue) in result.issues.iter().enumerate() {
                        println!("  {}. {}", i + 1, issue);
                    }
                }
            }
            Err(e) => {
                eprintln!(
                    "{} {}: {}",
                    "✗".red(),
                    format!("Error processing file {}", path.display()).red(),
                    e
                );
                process::exit(1);
            }
        }
    }
}

fn evaluate_directory(dir_path: &PathBuf, verbose: bool) {
    use std::fs;

    let mut valid_count = 0;
    let mut invalid_count = 0;

    println!("Evaluating workflows in directory: {}", dir_path.display());

    let entries = match fs::read_dir(dir_path) {
        Ok(entries) => entries,
        Err(e) => {
            eprintln!("{}", format!("Error reading directory: {}", e).red());
            process::exit(1);
        }
    };

    for entry in entries {
        if let Ok(entry) = entry {
            let path = entry.path();
            if path.is_file() && is_workflow_file(&path) {
                match evaluate_workflow_file(&path, verbose) {
                    Ok(result) => {
                        if result.is_valid {
                            println!(
                                "{} {}",
                                "✓".green(),
                                format!("Valid: {}", path.display()).green()
                            );
                            valid_count += 1;
                        } else {
                            println!(
                                "{} {}",
                                "✗".red(),
                                format!("Invalid: {}", path.display()).red()
                            );
                            for (i, issue) in result.issues.iter().enumerate() {
                                println!("  {}. {}", i + 1, issue);
                            }
                            invalid_count += 1;
                        }
                    }
                    Err(e) => {
                        eprintln!(
                            "{} {}: {}",
                            "✗".red(),
                            format!("Error: {}", path.display()).red(),
                            e
                        );
                        invalid_count += 1;
                    }
                }
            }
        }
    }

    println!(
        "\nSummary: {} valid, {} invalid workflow files",
        valid_count, invalid_count
    );
}
