use std::collections::HashMap;
use std::path::Path;
use thiserror::Error;

use crate::executor::dependency;
use crate::executor::docker;
use crate::executor::environment;
use crate::parser::workflow::{parse_workflow, ActionInfo, WorkflowDefinition};
use crate::runtime::container::ContainerRuntime;
use crate::runtime::emulation::handle_special_action;

/// Execute a GitHub Actions workflow file locally
pub async fn execute_workflow(
    workflow_path: &Path,
    runtime_type: RuntimeType,
    verbose: bool,
) -> Result<ExecutionResult, ExecutionError> {
    // 1. Parse workflow file
    let workflow = parse_workflow(workflow_path)?;

    // 2. Resolve job dependencies and create execution plan
    let execution_plan = dependency::resolve_dependencies(&workflow)?;

    // 3. Initialize appropriate runtime
    let runtime = initialize_runtime(runtime_type)?;

    // 4. Set up GitHub-like environment
    let env_context = environment::create_github_context(&workflow);

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
                eprintln!("Docker not available, falling back to emulation mode");
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

    #[error("Dependency error: {0}")]
    DependencyError(String),

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
        println!("Executing job: {}", job_name);
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
    job_runs_on: &str, // Add this parameter to get the runner
    verbose: bool,
) -> Result<StepResult, ExecutionError> {
    let step_name = step
        .name
        .clone()
        .unwrap_or_else(|| format!("Step {}", step_idx + 1));

    if verbose {
        println!("  Executing step: {}", step_name);
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
            // Other actions - original code for handling non-checkout actions
            let image = prepare_action(&action_info, runtime).await?;

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
                if let Err(e) = handle_special_action(uses, &step.with).await {
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
        println!("📝 Executing command: {}", run);

        let shell_default = "bash".to_string();
        let shell = step_env.get("SHELL").unwrap_or(&shell_default);
        println!("📝 Using shell: {}", shell);

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

async fn prepare_nix_container(
    runtime: &Box<dyn ContainerRuntime>,
    verbose: bool,
) -> Result<String, ExecutionError> {
    if verbose {
        println!("🔧 Preparing specialized container for Nix workflow");
    }

    // Create a container that has Nix pre-installed
    // We'll use a multi-step approach to create a Nix-enabled container

    // Step 1: Create a temporary Dockerfile for a Nix-enabled container
    let temp_dir = tempfile::tempdir()
        .map_err(|e| ExecutionError::ExecutionError(format!("Failed to create temp dir: {}", e)))?;

    let dockerfile_path = temp_dir.path().join("Dockerfile");
    let dockerfile_content = r#"FROM ubuntu:latest
RUN apt-get update && apt-get install -y curl xz-utils sudo
RUN adduser --disabled-password --gecos '' nix && \
    echo "nix ALL=(ALL) NOPASSWD:ALL" > /etc/sudoers.d/nix && \
    mkdir -p /nix && chown nix:nix /nix

USER nix
RUN curl -L https://nixos.org/nix/install | sh
ENV PATH="/nix/var/nix/profiles/default/bin:${PATH}"
ENV NIX_PATH="nixpkgs=/nix/var/nix/profiles/per-user/nix/channels/nixpkgs"

# Run nix once to verify it works
RUN nix --version

WORKDIR /github/workspace
"#;

    std::fs::write(&dockerfile_path, dockerfile_content).map_err(|e| {
        ExecutionError::ExecutionError(format!("Failed to write Dockerfile: {}", e))
    })?;

    // Step 2: Build the custom image
    let nix_image_tag = format!("wrkflw-nix-{}", uuid::Uuid::new_v4());

    if verbose {
        println!("🔧 Building custom Nix-enabled image: {}", nix_image_tag);
    }

    runtime
        .build_image(&dockerfile_path, &nix_image_tag)
        .await
        .map_err(|e| ExecutionError::RuntimeError(format!("Failed to build Nix image: {}", e)))?;

    if verbose {
        println!("✅ Successfully built Nix-enabled container image");
    }

    Ok(nix_image_tag)
}
