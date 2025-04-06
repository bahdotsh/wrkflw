use std::collections::HashMap;
use std::fs;
use std::path::Path;
use thiserror::Error;

use crate::executor::dependency;
use crate::executor::docker;
use crate::executor::environment;
use crate::logging;
use crate::parser::workflow::{parse_workflow, ActionInfo, WorkflowDefinition};
use crate::runtime::container::ContainerRuntime;
use crate::runtime::emulation::handle_special_action;

/// Execute a GitHub Actions workflow file locally
pub async fn execute_workflow(
    workflow_path: &Path,
    runtime_type: RuntimeType,
    verbose: bool,
) -> Result<ExecutionResult, ExecutionError> {
    logging::info(&format!("Executing workflow: {}", workflow_path.display()));
    logging::info(&format!("Runtime: {:?}", runtime_type));

    // 1. Parse workflow file
    let workflow = parse_workflow(workflow_path)?;

    // 2. Resolve job dependencies and create execution plan
    let execution_plan = dependency::resolve_dependencies(&workflow)?;

    // 3. Initialize appropriate runtime
    let runtime = initialize_runtime(runtime_type)?;

    // Create a temporary workspace directory
    let workspace_dir = tempfile::tempdir().map_err(|e| {
        ExecutionError::ExecutionError(format!("Failed to create workspace: {}", e))
    })?;

    // 4. Set up GitHub-like environment
    let env_context = environment::create_github_context(&workflow, workspace_dir.path());

    // Setup GitHub environment files
    environment::setup_github_environment_files(workspace_dir.path()).map_err(|e| {
        ExecutionError::ExecutionError(format!("Failed to setup GitHub env files: {}", e))
    })?;

    // 5. Execute jobs according to the plan
    let mut results = Vec::new();
    for job_batch in execution_plan {
        // Execute jobs in parallel if they don't depend on each other
        let job_results =
            execute_job_batch(&job_batch, &workflow, &runtime, &env_context, verbose).await?;
        results.extend(job_results);
    }

    Ok(ExecutionResult { jobs: results })
}

// Determine if Docker is available or fall back to emulation
fn initialize_runtime(
    runtime_type: RuntimeType,
) -> Result<Box<dyn ContainerRuntime>, ExecutionError> {
    match runtime_type {
        RuntimeType::Docker => {
            if docker::is_available() {
                Ok(Box::new(docker::DockerRuntime::new()))
            } else {
                logging::error(&format!(
                    "Docker not available, falling back to emulation mode"
                ));
                Ok(Box::new(crate::runtime::emulation::EmulationRuntime::new()))
            }
        }
        RuntimeType::Emulation => Ok(Box::new(crate::runtime::emulation::EmulationRuntime::new())),
    }
}

#[derive(Debug, Clone, PartialEq)]
pub enum RuntimeType {
    Docker,
    Emulation,
}

pub struct ExecutionResult {
    pub jobs: Vec<JobResult>,
}

pub struct JobResult {
    pub name: String,
    pub status: JobStatus,
    pub steps: Vec<StepResult>,
    pub logs: String,
}

#[derive(Debug, Clone, PartialEq)]
#[allow(dead_code)]
pub enum JobStatus {
    Success,
    Failure,
    Skipped,
}

pub struct StepResult {
    pub name: String,
    pub status: StepStatus,
    pub output: String,
}

#[derive(Debug, Clone, PartialEq)]
pub enum StepStatus {
    Success,
    Failure,
    Skipped,
}

#[derive(Error, Debug)]
pub enum ExecutionError {
    #[error("Parse error: {0}")]
    ParseError(String),

    #[error("Runtime error: {0}")]
    RuntimeError(String),

    #[error("Execution error: {0}")]
    ExecutionError(String),

    #[error("IO error: {0}")]
    IoError(#[from] std::io::Error),
}

// Convert errors from other modules
impl From<String> for ExecutionError {
    fn from(err: String) -> Self {
        ExecutionError::ParseError(err)
    }
}

// Add Action preparation functions
async fn prepare_action(
    action: &ActionInfo,
    runtime: &Box<dyn ContainerRuntime>,
) -> Result<String, ExecutionError> {
    if action.is_docker {
        // Docker action: pull the image
        let image = action.repository.trim_start_matches("docker://");

        runtime.pull_image(image).await.map_err(|e| {
            ExecutionError::RuntimeError(format!("Failed to pull Docker image: {}", e))
        })?;

        return Ok(image.to_string());
    }

    if action.is_local {
        // Local action: build from local directory
        let action_dir = Path::new(&action.repository);

        if !action_dir.exists() {
            return Err(ExecutionError::ExecutionError(format!(
                "Local action directory not found: {}",
                action_dir.display()
            )));
        }

        let dockerfile = action_dir.join("Dockerfile");
        if dockerfile.exists() {
            // It's a Docker action, build it
            let tag = format!("wrkflw-local-action:{}", uuid::Uuid::new_v4());

            runtime.build_image(&dockerfile, &tag).await.map_err(|e| {
                ExecutionError::RuntimeError(format!("Failed to build image: {}", e))
            })?;

            return Ok(tag);
        } else {
            // It's a JavaScript or composite action
            // For simplicity, we'll use node to run it (this would need more work for full support)
            return Ok("node:16-buster-slim".to_string());
        }
    }

    // GitHub action: use standard runner image
    // In a real implementation, you'd need to clone the repo at the specified version
    Ok("node:16-buster-slim".to_string())
}

async fn execute_job_batch(
    jobs: &[String],
    workflow: &WorkflowDefinition,
    runtime: &Box<dyn ContainerRuntime>,
    env_context: &HashMap<String, String>,
    verbose: bool,
) -> Result<Vec<JobResult>, ExecutionError> {
    use futures::future;

    // Execute jobs in parallel
    let futures = jobs
        .iter()
        .map(|job_name| execute_job(job_name, workflow, runtime, env_context, verbose));

    let results = future::join_all(futures).await;

    // Collect and check for errors
    let mut job_results = Vec::new();
    for result in results {
        match result {
            Ok(job_result) => job_results.push(job_result),
            Err(e) => return Err(e),
        }
    }

    Ok(job_results)
}

async fn execute_job(
    job_name: &str,
    workflow: &WorkflowDefinition,
    runtime: &Box<dyn ContainerRuntime>,
    env_context: &HashMap<String, String>,
    verbose: bool,
) -> Result<JobResult, ExecutionError> {
    if verbose {
        logging::info(&format!("Executing job: {}", job_name));
    }

    let job = workflow.jobs.get(job_name).ok_or_else(|| {
        ExecutionError::ExecutionError(format!("Job '{}' not found in workflow", job_name))
    })?;

    // Setup job environment
    let mut job_env = env_context.clone();

    // Add job-level environment variables
    for (key, value) in &job.env {
        job_env.insert(key.clone(), value.clone());
    }

    // Create a temporary directory for job execution
    let job_dir = tempfile::tempdir()
        .map_err(|e| ExecutionError::ExecutionError(format!("Failed to create temp dir: {}", e)))?;

    // Get the runner image
    let runner_image = get_runner_image(&job.runs_on);

    // Prepare the runner image (pull it)
    prepare_runner_image(&runner_image, runtime, verbose).await?;

    // Execute steps sequentially
    let mut step_results = Vec::new();
    let mut job_status = JobStatus::Success;
    let mut job_logs = String::new();

    for (step_idx, step) in job.steps.iter().enumerate() {
        if job_status == JobStatus::Failure {
            // Skip remaining steps if job has failed
            let step_name = step
                .name
                .clone()
                .unwrap_or_else(|| format!("Step {}", step_idx + 1));
            step_results.push(StepResult {
                name: step_name,
                status: StepStatus::Skipped,
                output: "Skipped due to previous step failure".to_string(),
            });
            continue;
        }

        let step_result = execute_step(
            step,
            step_idx,
            &job_env,
            job_dir.path(),
            runtime,
            workflow,
            &job.runs_on, // Pass the job's runner
            verbose,
        )
        .await;

        match step_result {
            Ok(result) => {
                job_logs.push_str(&format!("\n## Step: {}\n{}\n", result.name, result.output));

                if result.status == StepStatus::Failure {
                    job_status = JobStatus::Failure;
                }

                step_results.push(result);
            }
            Err(e) => {
                let step_name = step
                    .name
                    .clone()
                    .unwrap_or_else(|| format!("Step {}", step_idx + 1));
                let error_msg = format!("Step execution error: {}", e);

                job_logs.push_str(&format!("\n## Step: {}\nERROR: {}\n", step_name, error_msg));

                step_results.push(StepResult {
                    name: step_name,
                    status: StepStatus::Failure,
                    output: error_msg,
                });

                job_status = JobStatus::Failure;
            }
        }
    }

    Ok(JobResult {
        name: job_name.to_string(),
        status: job_status,
        steps: step_results,
        logs: job_logs,
    })
}

async fn execute_step(
    step: &crate::parser::workflow::Step,
    step_idx: usize,
    job_env: &HashMap<String, String>,
    working_dir: &Path,
    runtime: &Box<dyn ContainerRuntime>,
    workflow: &WorkflowDefinition,
    job_runs_on: &str,
    verbose: bool,
) -> Result<StepResult, ExecutionError> {
    let step_name = step
        .name
        .clone()
        .unwrap_or_else(|| format!("Step {}", step_idx + 1));

    if verbose {
        logging::info(&format!("  Executing step: {}", step_name));
    }

    // Prepare step environment
    let mut step_env = job_env.clone();

    // Add step-level environment variables
    for (key, value) in &step.env {
        step_env.insert(key.clone(), value.clone());
    }

    // Execute the step based on its type
    if let Some(uses) = &step.uses {
        // Action step
        let action_info = workflow.resolve_action(uses);

        // Check if this is the checkout action
        if uses.starts_with("actions/checkout") {
            // Get the current directory (assumes this is where your project is)
            let current_dir = std::env::current_dir().map_err(|e| {
                ExecutionError::ExecutionError(format!("Failed to get current dir: {}", e))
            })?;

            // Copy the project files to the workspace
            copy_directory_contents(&current_dir, working_dir)?;

            // Add info for logs
            let output = format!("Emulated checkout: Copied current directory to workspace");

            if verbose {
                println!("  Emulated actions/checkout: copied project files to workspace");
            }

            Ok(StepResult {
                name: step_name,
                status: StepStatus::Success,
                output,
            })
        } else {
            // Get action info
            let image = prepare_action(&action_info, runtime).await?;

            // Special handling for composite actions
            if image == "composite" && action_info.is_local {
                // Handle composite action
                let action_path = Path::new(&action_info.repository);
                return execute_composite_action(
                    step,
                    action_path,
                    &step_env,
                    working_dir,
                    runtime,
                    job_runs_on,
                    verbose,
                )
                .await;
            }

            // Regular Docker or JavaScript action processing
            // ... (rest of the existing code for handling regular actions)
            // Build command for Docker action
            let mut cmd = Vec::new();
            let mut owned_strings = Vec::new(); // Keep strings alive until after we use cmd

            if action_info.is_docker {
                // Docker actions just run the container
                cmd.push("sh");
                cmd.push("-c");
                cmd.push("echo 'Executing Docker action'");
            } else if action_info.is_local {
                // For local actions, we need more complex logic based on action type
                let action_dir = Path::new(&action_info.repository);
                let action_yaml = action_dir.join("action.yml");

                if action_yaml.exists() {
                    // Parse the action.yml to determine action type
                    // This is simplified - real implementation would be more complex
                    cmd.push("sh");
                    cmd.push("-c");
                    cmd.push("node /action/index.js");
                } else {
                    cmd.push("sh");
                    cmd.push("-c");
                    cmd.push("echo 'Local action without action.yml'");
                }
            } else {
                // For GitHub actions, check if we have special handling
                if let Err(e) = handle_special_action(uses).await {
                    // Log error but continue
                    println!("   Warning: Special action handling failed: {}", e);
                }
                // GitHub actions - would need to clone repo and setup action
                cmd.push("sh");
                cmd.push("-c");

                // Store the string and keep a reference to it
                let echo_cmd = format!("echo 'Would execute GitHub action: {}'", uses);
                owned_strings.push(echo_cmd);
                cmd.push(owned_strings.last().unwrap());
            }

            // Convert 'with' parameters to environment variables
            if let Some(with_params) = &step.with {
                for (key, value) in with_params {
                    step_env.insert(format!("INPUT_{}", key.to_uppercase()), value.clone());
                }
            }

            // Convert environment HashMap to Vec<(&str, &str)> for container runtime
            let env_vars: Vec<(&str, &str)> = step_env
                .iter()
                .map(|(k, v)| (k.as_str(), v.as_str()))
                .collect();

            // Map volumes
            let volumes: Vec<(&Path, &Path)> = vec![(working_dir, Path::new("/github/workspace"))];

            let output = runtime
                .run_container(
                    &image,
                    &cmd.iter().map(|s| *s).collect::<Vec<&str>>(),
                    &env_vars,
                    Path::new("/github/workspace"),
                    &volumes,
                )
                .await
                .map_err(|e| ExecutionError::RuntimeError(format!("{}", e)))?;

            if output.exit_code == 0 {
                Ok(StepResult {
                    name: step_name,
                    status: StepStatus::Success,
                    output: format!("{}\n{}", output.stdout, output.stderr),
                })
            } else {
                Ok(StepResult {
                    name: step_name,
                    status: StepStatus::Failure,
                    output: format!(
                        "Exit code: {}\n{}\n{}",
                        output.exit_code, output.stdout, output.stderr
                    ),
                })
            }
        }
    } else if let Some(run) = &step.run {
        // Print the command we're trying to run

        let shell_default = "bash".to_string();
        let shell = step_env.get("SHELL").unwrap_or(&shell_default);

        // Store command in a vector to ensure it stays alive
        let mut cmd_strings = Vec::new();
        let mut cmd: Vec<&str> = Vec::new();

        if shell == "bash" {
            cmd.push("bash");
            cmd.push("-e");
            cmd.push("-c");

            // Store the string and keep a reference to it
            cmd_strings.push(run.clone());
            cmd.push(&cmd_strings[0]);
        } else if shell == "powershell" {
            cmd.push("pwsh");
            cmd.push("-Command");

            // Store the string and keep a reference to it
            cmd_strings.push(run.clone());
            cmd.push(&cmd_strings[0]);
        } else {
            cmd.push("sh");
            cmd.push("-c");

            // Store the string and keep a reference to it
            cmd_strings.push(run.clone());
            cmd.push(&cmd_strings[0]);
        }

        // Convert environment HashMap to Vec<(&str, &str)> for container runtime
        let env_vars: Vec<(&str, &str)> = step_env
            .iter()
            .map(|(k, v)| (k.as_str(), v.as_str()))
            .collect();

        let image = if job_runs_on.contains("ubuntu") {
            // Try with a simple Ubuntu image
            "ubuntu:latest".to_string()
        } else {
            get_runner_image(job_runs_on)
        };

        runtime
            .pull_image(&image)
            .await
            .map_err(|e| ExecutionError::RuntimeError(format!("Failed to pull image: {}", e)))?;
        // Map volumes
        let volumes: Vec<(&Path, &Path)> = vec![(working_dir, Path::new("/github/workspace"))];

        let output = runtime
            .run_container(
                &image,
                &cmd,
                &env_vars,
                Path::new("/github/workspace"),
                &volumes,
            )
            .await
            .map_err(|e| ExecutionError::RuntimeError(format!("{}", e)))?;

        if output.exit_code == 0 {
            Ok(StepResult {
                name: step_name,
                status: StepStatus::Success,
                output: format!("{}\n{}", output.stdout, output.stderr),
            })
        } else {
            Ok(StepResult {
                name: step_name,
                status: StepStatus::Failure,
                output: format!(
                    "Exit code: {}\n{}\n{}",
                    output.exit_code, output.stdout, output.stderr
                ),
            })
        }
    } else {
        // Neither 'uses' nor 'run' - this is an error
        Err(ExecutionError::ExecutionError(format!(
            "Step '{}' has neither 'uses' nor 'run' directive",
            step_name
        )))
    }
}

fn copy_directory_contents(from: &Path, to: &Path) -> Result<(), ExecutionError> {
    for entry in std::fs::read_dir(from)
        .map_err(|e| ExecutionError::ExecutionError(format!("Failed to read directory: {}", e)))?
    {
        let entry = entry
            .map_err(|e| ExecutionError::ExecutionError(format!("Failed to read entry: {}", e)))?;
        let path = entry.path();

        // Skip hidden files/dirs and target directory for efficiency
        let file_name = path.file_name().unwrap().to_string_lossy();
        if file_name.starts_with(".") || file_name == "target" {
            continue;
        }

        let dest_path = to.join(path.file_name().unwrap());

        if path.is_dir() {
            std::fs::create_dir_all(&dest_path).map_err(|e| {
                ExecutionError::ExecutionError(format!("Failed to create dir: {}", e))
            })?;

            // Recursively copy subdirectories
            copy_directory_contents(&path, &dest_path)?;
        } else {
            std::fs::copy(&path, &dest_path).map_err(|e| {
                ExecutionError::ExecutionError(format!("Failed to copy file: {}", e))
            })?;
        }
    }

    Ok(())
}

fn get_runner_image(runs_on: &str) -> String {
    // Map GitHub runners to Docker images
    match runs_on.trim() {
        // ubuntu runners - micro images (minimal size)
        "ubuntu-latest" => "node:16-buster-slim",
        "ubuntu-22.04" => "node:16-bullseye-slim",
        "ubuntu-20.04" => "node:16-buster-slim",
        "ubuntu-18.04" => "node:16-buster-slim",

        // ubuntu runners - medium images (with more tools)
        "ubuntu-latest-medium" => "catthehacker/ubuntu:act-latest",
        "ubuntu-22.04-medium" => "catthehacker/ubuntu:act-22.04",
        "ubuntu-20.04-medium" => "catthehacker/ubuntu:act-20.04",
        "ubuntu-18.04-medium" => "catthehacker/ubuntu:act-18.04",

        // ubuntu runners - large images (with most tools)
        "ubuntu-latest-large" => "catthehacker/ubuntu:full-latest",
        "ubuntu-22.04-large" => "catthehacker/ubuntu:full-22.04",
        "ubuntu-20.04-large" => "catthehacker/ubuntu:full-20.04",
        "ubuntu-18.04-large" => "catthehacker/ubuntu:full-18.04",

        // Default case for other runners or custom strings
        _ => "ubuntu:latest", // Default to Ubuntu for everything
    }
    .to_string()
}

async fn prepare_runner_image(
    image: &str,
    runtime: &Box<dyn ContainerRuntime>,
    verbose: bool,
) -> Result<(), ExecutionError> {
    if verbose {
        println!("  Preparing runner image: {}", image);
    }

    // Pull the image
    runtime
        .pull_image(image)
        .await
        .map_err(|e| ExecutionError::RuntimeError(format!("Failed to pull runner image: {}", e)))?;

    if verbose {
        println!("  Image {} ready", image);
    }

    Ok(())
}

async fn execute_composite_action(
    step: &crate::parser::workflow::Step,
    action_path: &Path,
    job_env: &HashMap<String, String>,
    working_dir: &Path,
    runtime: &Box<dyn ContainerRuntime>,
    job_runs_on: &str,
    verbose: bool,
) -> Result<StepResult, ExecutionError> {
    // Find the action definition file
    let action_yaml = action_path.join("action.yml");
    let action_yaml_alt = action_path.join("action.yaml");

    let action_file = if action_yaml.exists() {
        action_yaml
    } else if action_yaml_alt.exists() {
        action_yaml_alt
    } else {
        return Err(ExecutionError::ExecutionError(format!(
            "No action.yml or action.yaml found in {}",
            action_path.display()
        )));
    };

    // Parse the composite action definition
    let action_content = fs::read_to_string(&action_file).map_err(|e| {
        ExecutionError::ExecutionError(format!("Failed to read action file: {}", e))
    })?;

    let action_def: serde_yaml::Value = serde_yaml::from_str(&action_content)
        .map_err(|e| ExecutionError::ExecutionError(format!("Invalid action YAML: {}", e)))?;

    // Check if it's a composite action
    match action_def.get("runs").and_then(|v| v.get("using")) {
        Some(serde_yaml::Value::String(using)) if using == "composite" => {
            // Get the steps
            let steps = match action_def.get("runs").and_then(|v| v.get("steps")) {
                Some(serde_yaml::Value::Sequence(steps)) => steps,
                _ => {
                    return Err(ExecutionError::ExecutionError(
                        "Composite action is missing steps".to_string(),
                    ))
                }
            };

            // Process inputs from the calling step's 'with' parameters
            let mut action_env = job_env.clone();
            if let Some(inputs_def) = action_def.get("inputs") {
                if let Some(inputs_map) = inputs_def.as_mapping() {
                    for (input_name, input_def) in inputs_map {
                        if let Some(input_name_str) = input_name.as_str() {
                            // Get default value if available
                            let default_value = input_def
                                .get("default")
                                .and_then(|v| v.as_str())
                                .unwrap_or("");

                            // Check if the input was provided in the 'with' section
                            let input_value = step
                                .with
                                .as_ref()
                                .and_then(|with| with.get(input_name_str))
                                .unwrap_or(&default_value.to_string())
                                .clone();

                            // Add to environment as INPUT_X
                            action_env.insert(
                                format!("INPUT_{}", input_name_str.to_uppercase()),
                                input_value,
                            );
                        }
                    }
                }
            }

            // Execute each step
            let mut step_outputs = Vec::new();
            for (idx, step_def) in steps.iter().enumerate() {
                // Convert the YAML step to our Step struct
                let composite_step = match convert_yaml_to_step(step_def) {
                    Ok(step) => step,
                    Err(e) => {
                        return Err(ExecutionError::ExecutionError(format!(
                            "Failed to process composite action step {}: {}",
                            idx + 1,
                            e
                        )))
                    }
                };

                // Execute the step - using Box::pin to handle async recursion
                let step_result = Box::pin(execute_step(
                    &composite_step,
                    idx,
                    &action_env,
                    working_dir,
                    runtime,
                    &crate::parser::workflow::WorkflowDefinition {
                        name: "Composite Action".to_string(),
                        on: vec![],
                        on_raw: serde_yaml::Value::Null,
                        jobs: HashMap::new(),
                    },
                    job_runs_on,
                    verbose,
                ))
                .await?;

                // Add output to results
                step_outputs.push(format!("Step {}: {}", idx + 1, step_result.output));

                // Short-circuit on failure if needed
                if step_result.status == StepStatus::Failure {
                    return Ok(StepResult {
                        name: step
                            .name
                            .clone()
                            .unwrap_or_else(|| "Composite Action".to_string()),
                        status: StepStatus::Failure,
                        output: format!("Composite action failed:\n{}", step_outputs.join("\n")),
                    });
                }
            }

            // All steps completed successfully
            Ok(StepResult {
                name: step
                    .name
                    .clone()
                    .unwrap_or_else(|| "Composite Action".to_string()),
                status: StepStatus::Success,
                output: format!("Composite action completed:\n{}", step_outputs.join("\n")),
            })
        }
        _ => Err(ExecutionError::ExecutionError(
            "Action is not a composite action or has invalid format".to_string(),
        )),
    }
}

// Helper function to convert YAML step to our Step struct
fn convert_yaml_to_step(
    step_yaml: &serde_yaml::Value,
) -> Result<crate::parser::workflow::Step, String> {
    // Extract step properties
    let name = step_yaml
        .get("name")
        .and_then(|v| v.as_str())
        .map(|s| s.to_string());

    let uses = step_yaml
        .get("uses")
        .and_then(|v| v.as_str())
        .map(|s| s.to_string());

    let run = step_yaml
        .get("run")
        .and_then(|v| v.as_str())
        .map(|s| s.to_string());

    let shell = step_yaml
        .get("shell")
        .and_then(|v| v.as_str())
        .map(|s| s.to_string());

    let with = step_yaml.get("with").and_then(|v| v.as_mapping()).map(|m| {
        let mut with_map = HashMap::new();
        for (k, v) in m {
            if let (Some(key), Some(value)) = (k.as_str(), v.as_str()) {
                with_map.insert(key.to_string(), value.to_string());
            }
        }
        with_map
    });

    let env = step_yaml
        .get("env")
        .and_then(|v| v.as_mapping())
        .map(|m| {
            let mut env_map = HashMap::new();
            for (k, v) in m {
                if let (Some(key), Some(value)) = (k.as_str(), v.as_str()) {
                    env_map.insert(key.to_string(), value.to_string());
                }
            }
            env_map
        })
        .unwrap_or_default();

    // For composite steps with shell, construct a run step
    let final_run = if shell.is_some() && run.is_some() {
        run
    } else {
        run
    };

    Ok(crate::parser::workflow::Step {
        name,
        uses,
        run: final_run,
        with: with,
        env,
    })
}
