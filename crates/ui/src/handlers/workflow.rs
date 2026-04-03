// Workflow handlers
use crate::cli_style;
use std::io;
use std::path::{Path, PathBuf};
use wrkflw_evaluator::evaluate_workflow_file;
use wrkflw_executor::{self, JobStatus, RuntimeType, StepStatus};
#[cfg(feature = "tui")]
use {
    crate::app::App,
    crate::models::{ExecutionResultMsg, WorkflowExecution, WorkflowStatus},
    chrono::Local,
    std::sync::mpsc,
    std::thread,
};

// Validate a workflow or directory containing workflows
pub fn validate_workflow(path: &Path, verbose: bool) -> io::Result<()> {
    let mut workflows = Vec::new();

    if path.is_dir() {
        let entries = std::fs::read_dir(path)?;

        for entry in entries {
            let entry = entry?;
            let entry_path = entry.path();

            if entry_path.is_file() && wrkflw_utils::is_workflow_file(&entry_path) {
                workflows.push(entry_path);
            }
        }
    } else if path.is_file() {
        workflows.push(PathBuf::from(path));
    } else {
        return Err(io::Error::new(
            io::ErrorKind::NotFound,
            format!("Path does not exist: {}", path.display()),
        ));
    }

    let mut valid_count = 0;
    let mut invalid_count = 0;

    println!(
        "{}",
        cli_style::info(&format!(
            "Validating {} workflow file(s)...",
            workflows.len()
        ))
    );

    for workflow_path in workflows {
        match evaluate_workflow_file(&workflow_path, verbose) {
            Ok(result) => {
                if result.is_valid {
                    println!(
                        "{}",
                        cli_style::success(&format!(
                            "Valid: {}",
                            cli_style::dim(&workflow_path.display().to_string())
                        ))
                    );
                    valid_count += 1;
                } else {
                    println!(
                        "{}",
                        cli_style::error(&format!("Invalid: {}", workflow_path.display()))
                    );
                    for (i, issue) in result.issues.iter().enumerate() {
                        println!("{}", cli_style::indent(&format!("{}. {}", i + 1, issue)));
                    }
                    invalid_count += 1;
                }
            }
            Err(e) => {
                println!(
                    "{}",
                    cli_style::error(&format!(
                        "Error processing {}: {}",
                        workflow_path.display(),
                        e
                    ))
                );
                invalid_count += 1;
            }
        }
    }

    use colored::Colorize;
    println!(
        "\n{}",
        format!("Summary: {} valid, {} invalid", valid_count, invalid_count).bold()
    );

    Ok(())
}

// Execute a workflow through the CLI
pub async fn execute_workflow_cli(
    path: &Path,
    runtime_type: RuntimeType,
    verbose: bool,
    show_action_messages: bool,
) -> io::Result<()> {
    if !path.exists() {
        return Err(io::Error::new(
            io::ErrorKind::NotFound,
            format!("Workflow file does not exist: {}", path.display()),
        ));
    }

    println!("{}", cli_style::info("Validating workflow..."));
    match evaluate_workflow_file(path, false) {
        Ok(result) => {
            if !result.is_valid {
                println!(
                    "{}",
                    cli_style::error(&format!(
                        "Cannot execute invalid workflow: {}",
                        path.display()
                    ))
                );
                for (i, issue) in result.issues.iter().enumerate() {
                    println!("{}", cli_style::indent(&format!("{}. {}", i + 1, issue)));
                }
                return Err(io::Error::new(
                    io::ErrorKind::InvalidData,
                    "Workflow validation failed",
                ));
            }
        }
        Err(e) => {
            return Err(io::Error::other(format!(
                "Error validating workflow: {}",
                e
            )));
        }
    }

    // Check container runtime availability if container runtime is selected
    let runtime_type = match runtime_type {
        RuntimeType::Docker => {
            if !wrkflw_executor::docker::is_available() {
                println!(
                    "{}",
                    cli_style::warning("Docker is not available. Using emulation mode instead.")
                );
                wrkflw_logging::warning("Docker is not available. Using emulation mode instead.");
                RuntimeType::Emulation
            } else {
                RuntimeType::Docker
            }
        }
        RuntimeType::Podman => {
            if !wrkflw_executor::podman::is_available() {
                println!(
                    "{}",
                    cli_style::warning("Podman is not available. Using emulation mode instead.")
                );
                wrkflw_logging::warning("Podman is not available. Using emulation mode instead.");
                RuntimeType::Emulation
            } else {
                RuntimeType::Podman
            }
        }
        RuntimeType::SecureEmulation => RuntimeType::SecureEmulation,
        RuntimeType::Emulation => RuntimeType::Emulation,
    };

    println!(
        "{}",
        cli_style::key_value("Workflow", &path.display().to_string())
    );
    println!(
        "{}",
        cli_style::key_value("Runtime", &format!("{:?}", runtime_type))
    );

    // Log the start of the execution in debug mode with more details
    wrkflw_logging::debug(&format!(
        "Starting workflow execution: path={}, runtime={:?}, verbose={}",
        path.display(),
        runtime_type,
        verbose
    ));

    let config = wrkflw_executor::ExecutionConfig {
        runtime_type,
        verbose,
        preserve_containers_on_failure: false, // Default for this path
        secrets_config: None,                  // Use default secrets configuration
        show_action_messages,
        target_job: None,
    };

    match wrkflw_executor::execute_workflow(path, config).await {
        Ok(result) => {
            println!("{}", cli_style::section("Workflow execution results"));

            let mut any_job_failed = false;

            for job in &result.jobs {
                match job.status {
                    JobStatus::Success => println!("\n{}", cli_style::job_success(&job.name)),
                    JobStatus::Failure => {
                        println!("\n{}", cli_style::job_failure(&job.name));
                        any_job_failed = true;
                    }
                    JobStatus::Skipped => println!("\n{}", cli_style::job_skipped(&job.name)),
                }

                println!("{}", cli_style::separator());

                wrkflw_logging::debug(&format!("Job: {}, Status: {:?}", job.name, job.status));

                for step in job.steps.iter() {
                    match step.status {
                        StepStatus::Success => {
                            println!("{}", cli_style::step_success(&step.name));

                            let should_hide = std::env::var("WRKFLW_HIDE_ACTION_MESSAGES")
                                .map(|val| val == "true")
                                .unwrap_or(false)
                                && step.output.contains("Would execute GitHub action:");

                            if !should_hide
                                && !step.output.trim().is_empty()
                                && step.output.lines().count() <= 3
                            {
                                println!("{}", cli_style::indent(step.output.trim()));
                            }
                        }
                        StepStatus::Failure => {
                            println!("{}", cli_style::step_failure(&step.name));

                            if let Some(exit_code) = step
                                .output
                                .lines()
                                .find(|line| line.trim().starts_with("Exit code:"))
                                .map(|line| line.trim().to_string())
                            {
                                println!("{}", cli_style::indent(&exit_code));
                            }

                            if wrkflw_logging::get_log_level() <= wrkflw_logging::LogLevel::Debug {
                                if let Some(cmd_output) = step
                                    .output
                                    .lines()
                                    .skip_while(|l| !l.trim().starts_with("$"))
                                    .take(1)
                                    .next()
                                {
                                    println!(
                                        "{}",
                                        cli_style::indent(&format!(
                                            "Command: {}",
                                            cmd_output.trim()
                                        ))
                                    );
                                }
                            }

                            let output_lines: Vec<&str> = step
                                .output
                                .lines()
                                .filter(|line| !line.trim().starts_with("Exit code:"))
                                .collect();

                            if !output_lines.is_empty() {
                                println!("{}", cli_style::indent("Error output:"));
                                for line in output_lines.iter().take(10) {
                                    println!("{}", cli_style::indent(line.trim()));
                                }

                                if output_lines.len() > 10 {
                                    println!(
                                        "{}",
                                        cli_style::indent(&format!(
                                            "... (and {} more lines)",
                                            output_lines.len() - 10
                                        ))
                                    );
                                    println!(
                                        "{}",
                                        cli_style::indent("Use --debug to see full output")
                                    );
                                }
                            }
                        }
                        StepStatus::Skipped => {
                            println!("{}", cli_style::step_skipped(&step.name));
                        }
                    }

                    wrkflw_logging::debug(&format!(
                        "Step: {}, Status: {:?}, Output length: {} lines",
                        step.name,
                        step.status,
                        step.output.lines().count()
                    ));

                    if wrkflw_logging::get_log_level() == wrkflw_logging::LogLevel::Debug
                        && !step.output.trim().is_empty()
                    {
                        wrkflw_logging::debug(&format!(
                            "Step output for '{}': \n{}",
                            step.name, step.output
                        ));
                    }
                }
            }

            if any_job_failed {
                println!("\n{}", cli_style::error("Workflow completed with failures"));
                if wrkflw_logging::get_log_level() > wrkflw_logging::LogLevel::Debug {
                    println!(
                        "{}",
                        cli_style::indent("Run with --debug for more detailed output")
                    );
                }
            } else {
                println!(
                    "\n{}",
                    cli_style::success("Workflow completed successfully!")
                );
            }

            Ok(())
        }
        Err(e) => {
            println!(
                "{}",
                cli_style::error(&format!("Failed to execute workflow: {}", e))
            );
            wrkflw_logging::error(&format!("Failed to execute workflow: {}", e));
            Err(io::Error::other(e))
        }
    }
}

// Helper function to execute workflow trigger using curl
pub async fn execute_curl_trigger(
    workflow_name: &str,
    branch: Option<&str>,
) -> Result<(Vec<wrkflw_executor::JobResult>, ()), String> {
    // Get GitHub token
    let token = std::env::var("GITHUB_TOKEN").map_err(|_| {
        "GitHub token not found. Please set GITHUB_TOKEN environment variable".to_string()
    })?;

    // Debug log to check if GITHUB_TOKEN is set
    match std::env::var("GITHUB_TOKEN") {
        Ok(token) => wrkflw_logging::info(&format!("GITHUB_TOKEN is set: {}", &token[..5])), // Log first 5 characters for security
        Err(_) => wrkflw_logging::error("GITHUB_TOKEN is not set"),
    }

    // Get repository information
    let repo_info = wrkflw_github::get_repo_info()
        .map_err(|e| format!("Failed to get repository info: {}", e))?;

    // Determine branch to use
    let branch_ref = branch.unwrap_or(&repo_info.default_branch);

    // Extract just the workflow name from the path if it's a full path
    let workflow_name = if workflow_name.contains('/') {
        Path::new(workflow_name)
            .file_stem()
            .and_then(|s| s.to_str())
            .ok_or_else(|| "Invalid workflow name".to_string())?
    } else {
        workflow_name
    };

    wrkflw_logging::info(&format!("Using workflow name: {}", workflow_name));

    // Construct JSON payload
    let payload = serde_json::json!({
        "ref": branch_ref
    });

    // Construct API URL
    let url = format!(
        "https://api.github.com/repos/{}/{}/actions/workflows/{}.yml/dispatches",
        repo_info.owner, repo_info.repo, workflow_name
    );

    wrkflw_logging::info(&format!("Triggering workflow at URL: {}", url));

    // Create a reqwest client
    let client = reqwest::Client::new();

    // Send the request using reqwest
    let response = client
        .post(&url)
        .header("Authorization", format!("Bearer {}", token.trim()))
        .header("Accept", "application/vnd.github.v3+json")
        .header("Content-Type", "application/json")
        .header("User-Agent", "wrkflw-cli")
        .json(&payload)
        .send()
        .await
        .map_err(|e| format!("Failed to send request: {}", e))?;

    if !response.status().is_success() {
        let status = response.status().as_u16();
        let error_message = response
            .text()
            .await
            .unwrap_or_else(|_| format!("Unknown error (HTTP {})", status));

        return Err(format!("API error: {} - {}", status, error_message));
    }

    // Success message with URL to view the workflow
    let success_msg = format!(
        "Workflow triggered successfully. View it at: https://github.com/{}/{}/actions/workflows/{}.yml",
        repo_info.owner, repo_info.repo, workflow_name
    );

    // Create a job result structure
    let job_result = wrkflw_executor::JobResult {
        name: "GitHub Trigger".to_string(),
        status: wrkflw_executor::JobStatus::Success,
        steps: vec![wrkflw_executor::StepResult {
            name: "Remote Trigger".to_string(),
            status: wrkflw_executor::StepStatus::Success,
            output: success_msg,
        }],
        logs: "Workflow triggered remotely on GitHub".to_string(),
    };

    Ok((vec![job_result], ()))
}

// Extract common workflow execution logic to avoid duplication
#[cfg(feature = "tui")]
pub fn start_next_workflow_execution(
    app: &mut App,
    tx_clone: &mpsc::Sender<ExecutionResultMsg>,
    verbose: bool,
) {
    if let Some((next_idx, target_job)) = app.get_next_workflow_to_execute() {
        app.current_execution = Some(next_idx);
        let tx_clone_inner = tx_clone.clone();
        let workflow_path = app.workflows[next_idx].path.clone();

        // Log whether verbose mode is enabled
        if verbose {
            app.logs
                .push("Verbose mode: Step outputs will be displayed in full".to_string());
            wrkflw_logging::info("Verbose mode: Step outputs will be displayed in full");
        } else {
            app.logs.push(
                "Standard mode: Only step status will be shown (use --verbose for full output)"
                    .to_string(),
            );
            wrkflw_logging::info(
                "Standard mode: Only step status will be shown (use --verbose for full output)",
            );
        }

        // Check container runtime availability again if container runtime is selected
        let runtime_type = match app.runtime_type {
            RuntimeType::Docker => {
                // Use safe FD redirection to check Docker availability
                let is_docker_available = match wrkflw_utils::fd::with_stderr_to_null(
                    wrkflw_executor::docker::is_available,
                ) {
                    Ok(result) => result,
                    Err(_) => {
                        wrkflw_logging::debug(
                            "Failed to redirect stderr when checking Docker availability.",
                        );
                        false
                    }
                };

                if !is_docker_available {
                    app.logs
                        .push("Docker is not available. Using emulation mode instead.".to_string());
                    wrkflw_logging::warning(
                        "Docker is not available. Using emulation mode instead.",
                    );
                    RuntimeType::Emulation
                } else {
                    RuntimeType::Docker
                }
            }
            RuntimeType::Podman => {
                // Use safe FD redirection to check Podman availability
                let is_podman_available = match wrkflw_utils::fd::with_stderr_to_null(
                    wrkflw_executor::podman::is_available,
                ) {
                    Ok(result) => result,
                    Err(_) => {
                        wrkflw_logging::debug(
                            "Failed to redirect stderr when checking Podman availability.",
                        );
                        false
                    }
                };

                if !is_podman_available {
                    app.logs
                        .push("Podman is not available. Using emulation mode instead.".to_string());
                    wrkflw_logging::warning(
                        "Podman is not available. Using emulation mode instead.",
                    );
                    RuntimeType::Emulation
                } else {
                    RuntimeType::Podman
                }
            }
            RuntimeType::SecureEmulation => RuntimeType::SecureEmulation,
            RuntimeType::Emulation => RuntimeType::Emulation,
        };

        let validation_mode = app.validation_mode;
        let preserve_containers_on_failure = app.preserve_containers_on_failure;
        let show_action_messages = app.show_action_messages;

        // Update workflow status and add execution details
        app.workflows[next_idx].status = WorkflowStatus::Running;

        // Initialize execution details if not already done
        if app.workflows[next_idx].execution_details.is_none() {
            app.workflows[next_idx].execution_details = Some(WorkflowExecution {
                jobs: Vec::new(),
                start_time: Local::now(),
                end_time: None,
                logs: Vec::new(),
                progress: 0.0,
            });
        }

        thread::spawn(move || {
            let rt = match tokio::runtime::Runtime::new() {
                Ok(runtime) => runtime,
                Err(e) => {
                    let _ = tx_clone_inner.send((
                        next_idx,
                        Err(format!("Failed to create Tokio runtime: {}", e)),
                    ));
                    return;
                }
            };

            let result = rt.block_on(async {
                if validation_mode {
                    // Perform validation instead of execution
                    match evaluate_workflow_file(&workflow_path, verbose) {
                        Ok(validation_result) => {
                            // Create execution result based on validation
                            let status = if validation_result.is_valid {
                                wrkflw_executor::JobStatus::Success
                            } else {
                                wrkflw_executor::JobStatus::Failure
                            };

                            // Create a synthetic job result for validation
                            let jobs = vec![wrkflw_executor::JobResult {
                                name: "Validation".to_string(),
                                status,
                                steps: vec![wrkflw_executor::StepResult {
                                    name: "Validator".to_string(),
                                    status: if validation_result.is_valid {
                                        wrkflw_executor::StepStatus::Success
                                    } else {
                                        wrkflw_executor::StepStatus::Failure
                                    },
                                    output: validation_result.issues.join("\n"),
                                }],
                                logs: format!(
                                    "Validation result: {}",
                                    if validation_result.is_valid {
                                        "PASSED"
                                    } else {
                                        "FAILED"
                                    }
                                ),
                            }];

                            Ok((jobs, ()))
                        }
                        Err(e) => Err(e.to_string()),
                    }
                } else {
                    // Use safe FD redirection for execution
                    let config = wrkflw_executor::ExecutionConfig {
                        runtime_type,
                        verbose,
                        preserve_containers_on_failure,
                        secrets_config: None, // Use default secrets configuration
                        show_action_messages,
                        target_job,
                    };

                    let execution_result = wrkflw_utils::fd::with_stderr_to_null(|| {
                        futures::executor::block_on(async {
                            wrkflw_executor::execute_workflow(&workflow_path, config).await
                        })
                    })
                    .map_err(|e| format!("Failed to redirect stderr during execution: {}", e))?;

                    match execution_result {
                        Ok(execution_result) => {
                            // Send back the job results in a wrapped result
                            Ok((execution_result.jobs, ()))
                        }
                        Err(e) => Err(e.to_string()),
                    }
                }
            });

            // Only send if we get a valid result
            if let Err(e) = tx_clone_inner.send((next_idx, result)) {
                wrkflw_logging::error(&format!("Error sending execution result: {}", e));
            }
        });
    } else {
        app.running = false;
        let timestamp = Local::now().format("%H:%M:%S").to_string();
        app.logs
            .push(format!("[{}] All workflows completed execution", timestamp));
        wrkflw_logging::info("All workflows completed execution");
    }
}
