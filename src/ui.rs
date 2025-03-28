use crate::evaluator::evaluate_workflow_file;
use crate::utils::is_workflow_file;
use colored::*;
use std::fs;
use std::path::{Path, PathBuf};
use std::process;

pub fn validate_target(path: &PathBuf, verbose: bool) {
    if !path.exists() {
        eprintln!(
            "{} {}",
            "Error:".bold().red(),
            format!("Path does not exist: {}", path.display()).red()
        );
        process::exit(1);
    }

    if path.is_dir() {
        evaluate_directory(path, verbose);
    } else {
        validate_single_file(path, verbose);
    }
}

fn validate_single_file(path: &PathBuf, verbose: bool) {
    if !is_workflow_file(path) {
        println!(
            "{} {}",
            "Warning:".bold().yellow(),
            format!(
                "File doesn't appear to be a workflow file: {}",
                path.display()
            )
            .yellow()
        );
        if !prompt_continue() {
            return;
        }
    }

    match evaluate_workflow_file(path, verbose) {
        Ok(result) => {
            if result.is_valid {
                println!(
                    "{} {}",
                    "✓".bold().green(),
                    format!("Valid workflow: {}", path.display()).green()
                );
            } else {
                println!(
                    "{} {}",
                    "✗".bold().red(),
                    format!("Invalid workflow: {}", path.display()).bold().red()
                );
                print_issues(&result.issues);
            }
        }
        Err(e) => {
            eprintln!(
                "{} {}: {}",
                "✗".bold().red(),
                format!("Error processing {}", path.display()).red(),
                e
            );
            process::exit(1);
        }
    }
}

pub fn evaluate_directory(dir_path: &PathBuf, verbose: bool) {
    println!(
        "\n{} {}",
        "Evaluating workflows in:".bold().blue(),
        dir_path.display().to_string().underline()
    );
    println!("{}", "=".repeat(60));

    let default_workflows_dir = Path::new(".github").join("workflows");
    let is_default_dir = dir_path == &default_workflows_dir || dir_path.ends_with("workflows");

    let entries = match fs::read_dir(dir_path) {
        Ok(entries) => entries,
        Err(e) => {
            eprintln!(
                "{} {}",
                "Error:".bold().red(),
                format!("Cannot read directory: {}", e).red()
            );
            process::exit(1);
        }
    };

    let mut files_to_validate = Vec::new();

    // Collect all workflow files first
    for entry in entries {
        if let Ok(entry) = entry {
            let path = entry.path();
            if path.is_file() && (is_workflow_file(&path) || !is_default_dir) {
                files_to_validate.push(path);
            }
        }
    }

    if files_to_validate.is_empty() {
        println!(
            "{} {}",
            "Notice:".bold().yellow(),
            "No workflow files found in this directory.".yellow()
        );
        return;
    }

    let mut valid_count = 0;
    let mut invalid_count = 0;

    for path in files_to_validate {
        match evaluate_workflow_file(&path, verbose) {
            Ok(result) => {
                if result.is_valid {
                    println!(
                        "{} {}",
                        "✓".bold().green(),
                        format!("Valid: {}", path.file_name().unwrap().to_string_lossy()).green()
                    );
                    valid_count += 1;
                } else {
                    println!(
                        "{} {}",
                        "✗".bold().red(),
                        format!("Invalid: {}", path.file_name().unwrap().to_string_lossy())
                            .bold()
                            .red()
                    );
                    print_issues(&result.issues);
                    invalid_count += 1;
                }
            }
            Err(e) => {
                eprintln!(
                    "{} {}: {}",
                    "✗".bold().red(),
                    format!("Error: {}", path.display()).red(),
                    e
                );
                invalid_count += 1;
            }
        }
        println!("{}", "-".repeat(60));
    }

    // Print summary
    print_summary(valid_count, invalid_count);
}

fn print_issues(issues: &[String]) {
    println!("{}", "  Issues:".bold());
    for (i, issue) in issues.iter().enumerate() {
        println!(
            "  {}. {}",
            (i + 1).to_string().bold().yellow(),
            issue.yellow()
        );
    }
}

fn print_summary(valid_count: usize, invalid_count: usize) {
    println!("\n{}", "Summary".bold().blue());
    println!("{}", "=".repeat(60));

    if valid_count > 0 {
        println!(
            "{} {}",
            "✓".bold().green(),
            format!("{} valid workflow file(s)", valid_count).green()
        );
    }

    if invalid_count > 0 {
        println!(
            "{} {}",
            "✗".bold().red(),
            format!("{} invalid workflow file(s)", invalid_count).red()
        );
    }

    if valid_count > 0 && invalid_count == 0 {
        println!("\n{}", "All workflows are valid! 🎉".bold().green());
    }
}

fn prompt_continue() -> bool {
    use std::io::{self, Write};

    print!("Continue with validation? [y/N]: ");
    io::stdout().flush().unwrap();

    let mut input = String::new();
    io::stdin().read_line(&mut input).unwrap();

    let input = input.trim().to_lowercase();
    input == "y" || input == "yes"
}
