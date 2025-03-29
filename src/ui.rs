use crate::evaluator::evaluate_workflow_file;
use crate::executor::{self, ExecutionResult, JobStatus, RuntimeType, StepStatus};
use crate::utils::is_workflow_file;
use colored::*;
use std::fs;
use std::path::{Path, PathBuf};
use std::process;

// Validate target file or directory
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

// New function for workflow execution
pub async fn execute_workflow(path: &PathBuf, runtime_type: RuntimeType, verbose: bool) {
    if !path.exists() {
        eprintln!(
            "{} {}",
            "Error:".bold().red(),
            format!("Workflow file does not exist: {}", path.display()).red()
        );
        process::exit(1);
    }

    // Check if workflow uses Nix
    let uses_nix = match std::fs::read_to_string(path) {
        Ok(content) => {
            content.contains("nix ")
                || content.contains("install-nix-action")
                || content.to_lowercase().contains("name: nix")
        }
        Err(_) => false,
    };

    if uses_nix {
        // Check if Nix is installed
        let nix_installed = std::process::Command::new("which")
            .arg("nix")
            .output()
            .map(|output| output.status.success())
            .unwrap_or(false);

        if !nix_installed {
            println!(
                "{}",
                "⚠️ This workflow requires Nix, but Nix is not installed on your system."
                    .yellow()
                    .bold()
            );
            println!(
                "{}",
                "   Please install Nix first: https://nixos.org/download.html".yellow()
            );

            if let RuntimeType::Docker = runtime_type {
                println!("{}", "   When using Docker mode, we'll try to create a container with Nix installed.".yellow());
            } else {
                println!("{}", "   Since you're using emulation mode, Nix commands will fail without Nix installed.".yellow());
                println!(
                    "{}",
                    "   Consider switching to Docker mode after installing Nix:".yellow()
                );
                println!(
                    "{}",
                    format!("   cargo run -- run {}", path.display()).yellow()
                );
            }

            println!("{}", "-".repeat(60));
        } else {
            println!(
                "{}",
                "✅ Nix is installed, ready to execute Nix workflow".green()
            );
            println!("{}", "-".repeat(60));
        }
    }
    // First validate the workflow
    match evaluate_workflow_file(path, false) {
        Ok(result) => {
            if !result.is_valid {
                println!(
                    "{} {}",
                    "✗".bold().red(),
                    format!("Cannot execute invalid workflow: {}", path.display())
                        .bold()
                        .red()
                );
                print_issues(&result.issues);
                println!("\nPlease fix these issues before running the workflow.");
                process::exit(1);
            }
        }
        Err(e) => {
            eprintln!(
                "{} {}: {}",
                "✗".bold().red(),
                format!("Error validating workflow {}", path.display()).red(),
                e
            );
            process::exit(1);
        }
    }

    println!(
        "\n{} {}",
        "Executing workflow:".bold().blue(),
        path.display().to_string().underline()
    );
    println!("{}", "=".repeat(60));

    // Indicate which runtime is being used
    match runtime_type {
        RuntimeType::Docker => {
            println!("{} {}", "Runtime:".bold(), "Docker".cyan());
        }
        RuntimeType::Emulation => {
            println!(
                "{} {}",
                "Runtime:".bold(),
                "Emulation (no Docker required)".yellow()
            );
        }
    }
    println!("{}", "-".repeat(60));

    match executor::execute_workflow(path, runtime_type, verbose).await {
        Ok(result) => {
            print_execution_results(&result);

            // Check if overall execution succeeded
            let failures = result
                .jobs
                .iter()
                .filter(|job| job.status == JobStatus::Failure)
                .count();

            if failures > 0 {
                println!("\n{}", "Workflow completed with failures".bold().red());
                process::exit(1);
            } else {
                println!("\n{}", "Workflow completed successfully! 🎉".bold().green());
            }
        }
        Err(e) => {
            eprintln!(
                "{} {}",
                "Error:".bold().red(),
                format!("Failed to execute workflow: {}", e).red()
            );
            process::exit(1);
        }
    }
}

fn print_execution_results(result: &ExecutionResult) {
    for job in &result.jobs {
        match job.status {
            JobStatus::Success => {
                println!(
                    "\n{} {}",
                    "✓".bold().green(),
                    format!("Job succeeded: {}", job.name).green()
                );
            }
            JobStatus::Failure => {
                println!(
                    "\n{} {}",
                    "✗".bold().red(),
                    format!("Job failed: {}", job.name).bold().red()
                );
            }
            JobStatus::Skipped => {
                println!(
                    "\n{} {}",
                    "⚠".bold().yellow(),
                    format!("Job skipped: {}", job.name).yellow()
                );
            }
        }

        println!("{}", "-".repeat(60));

        for (_i, step) in job.steps.iter().enumerate() {
            let step_name = step.name.clone();

            match step.status {
                StepStatus::Success => {
                    println!("  {} {}", "✓".green(), step_name.green());

                    if !step.output.trim().is_empty() && step.output.lines().count() <= 3 {
                        // For short outputs, show directly
                        println!("    {}", step.output.trim().green());
                    } else if !step.output.trim().is_empty() {
                        // For longer outputs, just indicate output is available
                        println!("    {}", "[Output available]".green());
                    }
                }
                StepStatus::Failure => {
                    println!("  {} {}", "✗".red(), step_name.red().bold());

                    // For failures, always show output
                    let error_output = step
                        .output
                        .lines()
                        .collect::<Vec<&str>>()
                        .iter()
                        .map(|line| format!("    {}", line))
                        .collect::<Vec<String>>()
                        .join("\n");

                    println!("{}", error_output.red());
                }
                StepStatus::Skipped => {
                    println!("  {} {}", "⚠".yellow(), step_name.yellow());
                }
            }
        }

        // For verbose logging or specific job outputs
        if !job.logs.trim().is_empty() && job.status == JobStatus::Failure {
            println!("\n  {}", "Full job logs:".bold());
            println!("  {}", "-".repeat(30));
            println!("  {}", job.logs.trim());
            println!("  {}", "-".repeat(30));
        }
    }

    println!("\n{}", "Summary".bold().blue());
    println!("{}", "=".repeat(60));

    let success_count = result
        .jobs
        .iter()
        .filter(|job| job.status == JobStatus::Success)
        .count();
    let failure_count = result
        .jobs
        .iter()
        .filter(|job| job.status == JobStatus::Failure)
        .count();
    let skipped_count = result
        .jobs
        .iter()
        .filter(|job| job.status == JobStatus::Skipped)
        .count();

    if success_count > 0 {
        println!(
            "{} {}",
            "✓".bold().green(),
            format!("{} job(s) succeeded", success_count).green()
        );
    }

    if failure_count > 0 {
        println!(
            "{} {}",
            "✗".bold().red(),
            format!("{} job(s) failed", failure_count).red()
        );
    }

    if skipped_count > 0 {
        println!(
            "{} {}",
            "⚠".bold().yellow(),
            format!("{} job(s) skipped", skipped_count).yellow()
        );
    }
}

// Display detailed logs for a specific job
pub fn view_job_logs(job_name: &str, logs: &str) {
    println!("\n{} {}", "Job Logs:".bold().blue(), job_name.underline());
    println!("{}", "=".repeat(60));

    // Split logs by step section markers and print with formatting
    let sections: Vec<&str> = logs.split("\n## Step:").collect();

    if sections.is_empty() {
        println!("{}", logs);
    } else {
        for (i, section) in sections.iter().enumerate() {
            if i == 0 && section.trim().is_empty() {
                continue;
            }

            if i == 0 {
                // This is pre-step output
                println!("{}", section);
            } else {
                // This is a step section
                let lines: Vec<&str> = section.lines().collect();
                if !lines.is_empty() {
                    println!("  {}", lines[0].bold()); // Step name

                    for line in &lines[1..] {
                        println!("    {}", line);
                    }

                    println!();
                }
            }
        }
    }

    println!("{}", "=".repeat(60));
}

// Function to prompt for user input with a message
pub fn prompt_for_input(message: &str) -> String {
    use std::io::{self, Write};

    print!("{}: ", message);
    io::stdout().flush().unwrap();

    let mut input = String::new();
    io::stdin().read_line(&mut input).unwrap();

    input.trim().to_string()
}

// Function to show progress for long-running operations
pub fn show_progress(message: &str, total: usize) -> ProgressBar {
    println!("{} {}", "⏳".bold(), message);
    ProgressBar::new(total)
}

// Simple progress bar implementation
pub struct ProgressBar {
    total: usize,
    current: usize,
}

impl ProgressBar {
    pub fn new(total: usize) -> Self {
        ProgressBar { total, current: 0 }
    }

    pub fn increment(&mut self, amount: usize) {
        self.current += amount;
        self.current = self.current.min(self.total);
        self.display();
    }

    pub fn display(&self) {
        use std::io::{self, Write};

        let width = 30;
        let progress = (self.current as f32 / self.total as f32 * width as f32) as usize;
        let bar = "█".repeat(progress) + &"░".repeat(width - progress);
        let percentage = (self.current as f32 / self.total as f32 * 100.0) as usize;

        print!(
            "\r[{}] {}% ({}/{})",
            bar, percentage, self.current, self.total
        );
        io::stdout().flush().unwrap();

        if self.current == self.total {
            println!();
        }
    }

    pub fn complete(&mut self) {
        self.current = self.total;
        self.display();
    }
}
