use bollard::Docker;
use futures::future;
use serde_yaml::Value;
use std::collections::HashMap;
use std::fs;
use std::path::Path;
use thiserror::Error;

use crate::executor::dependency;
use crate::executor::docker;
use crate::executor::environment;
use crate::executor::substitution;
use crate::logging;
use crate::matrix::{self, MatrixCombination};
use crate::parser::workflow::{parse_workflow, ActionInfo, Job, WorkflowDefinition};
use crate::runtime::container::ContainerRuntime;
use crate::runtime::emulation::handle_special_action;

#[allow(unused_variables, unused_assignments)]

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
                // Handle the Result returned by DockerRuntime::new()
                match docker::DockerRuntime::new() {
                    Ok(docker_runtime) => Ok(Box::new(docker_runtime)),
                    Err(e) => {
                        logging::error(&format!(
                            "Failed to initialize Docker runtime: {}, falling back to emulation mode",
                            e
                        ));
                        Ok(Box::new(crate::runtime::emulation::EmulationRuntime::new()))
                    }
                }
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

#[derive(Debug, Clone)]
pub struct StepResult {
    pub name: String,
    pub status: StepStatus,
    pub output: String,
}

#[derive(Debug, Clone, PartialEq)]
#[allow(dead_code)]
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
    // Execute jobs in parallel
    let futures = jobs
        .iter()
        .map(|job_name| execute_job_with_matrix(job_name, workflow, runtime, env_context, verbose));

    let result_arrays = future::join_all(futures).await;

    // Flatten the results from all jobs and their matrix combinations
    let mut results = Vec::new();
    for result_array in result_arrays {
        match result_array {
            Ok(job_results) => results.extend(job_results),
            Err(e) => return Err(e),
        }
    }

    Ok(results)
}

/// Execute a job, expanding matrix if present
async fn execute_job_with_matrix(
    job_name: &str,
    workflow: &WorkflowDefinition,
    runtime: &Box<dyn ContainerRuntime>,
    env_context: &HashMap<String, String>,
    verbose: bool,
) -> Result<Vec<JobResult>, ExecutionError> {
    // Get the job definition
    let job = workflow.jobs.get(job_name).ok_or_else(|| {
        ExecutionError::ExecutionError(format!("Job '{}' not found in workflow", job_name))
    })?;

    // Check if this is a matrix job
    if let Some(matrix_config) = &job.matrix {
        // Expand the matrix into combinations
        let combinations = matrix::expand_matrix(matrix_config).map_err(|e| {
            ExecutionError::ExecutionError(format!("Failed to expand matrix: {}", e))
        })?;

        if combinations.is_empty() {
            logging::info(&format!(
                "Matrix job '{}' has no valid combinations",
                job_name
            ));
            // Return empty result for jobs with no valid combinations
            return Ok(Vec::new());
        }

        logging::info(&format!(
            "Matrix job '{}' expanded to {} combinations",
            job_name,
            combinations.len()
        ));

        // Set maximum parallel jobs
        let max_parallel = matrix_config.max_parallel.unwrap_or_else(|| {
            // If not specified, use a reasonable default based on CPU cores
            std::cmp::max(1, num_cpus::get())
        });

        // Execute matrix combinations
        execute_matrix_combinations(
            job_name,
            job,
            &combinations,
            max_parallel,
            matrix_config.fail_fast.unwrap_or(true),
            workflow,
            runtime,
            env_context,
            verbose,
        )
        .await
    } else {
        // Regular job, no matrix
        let result = execute_job(job_name, workflow, runtime, env_context, verbose).await?;
        Ok(vec![result])
    }
}

/// Execute a set of matrix combinations
async fn execute_matrix_combinations(
    job_name: &str,
    job_template: &Job,
    combinations: &[MatrixCombination],
    max_parallel: usize,
    fail_fast: bool,
    workflow: &WorkflowDefinition,
    runtime: &Box<dyn ContainerRuntime>,
    env_context: &HashMap<String, String>,
    verbose: bool,
) -> Result<Vec<JobResult>, ExecutionError> {
    let mut results = Vec::new();
    let mut any_failed = false;

    // Process combinations in chunks limited by max_parallel
    for chunk in combinations.chunks(max_parallel) {
        // Skip processing if fail-fast is enabled and a previous job failed
        if fail_fast && any_failed {
            // Add skipped results for remaining combinations
            for combination in chunk {
                let combination_name = matrix::format_combination_name(job_name, combination);
                results.push(JobResult {
                    name: combination_name,
                    status: JobStatus::Skipped,
                    steps: Vec::new(),
                    logs: "Job skipped due to previous matrix job failure".to_string(),
                });
            }
            continue;
        }

        // Process this chunk of combinations in parallel
        let chunk_futures = chunk.iter().map(|combination| {
            execute_matrix_job(
                job_name,
                job_template,
                combination,
                workflow,
                runtime,
                env_context,
                verbose,
            )
        });

        let chunk_results = future::join_all(chunk_futures).await;

        // Process results from this chunk
        for result in chunk_results {
            match result {
                Ok(job_result) => {
                    if job_result.status == JobStatus::Failure {
                        any_failed = true;
                    }
                    results.push(job_result);
                }
                Err(e) => {
                    // On error, mark as failed and continue if not fail-fast
                    any_failed = true;
                    logging::error(&format!("Matrix job failed: {}", e));

                    if fail_fast {
                        return Err(e);
                    }
                }
            }
        }
    }

    Ok(results)
}

/// Execute a single matrix job combination
async fn execute_matrix_job(
    job_name: &str,
    job_template: &Job,
    combination: &MatrixCombination,
    workflow: &WorkflowDefinition,
    runtime: &Box<dyn ContainerRuntime>,
    base_env_context: &HashMap<String, String>,
    verbose: bool,
) -> Result<JobResult, ExecutionError> {
    // Create the matrix-specific job name
    let matrix_job_name = matrix::format_combination_name(job_name, combination);

    logging::info(&format!("Executing matrix job: {}", matrix_job_name));

    // Clone the environment and add matrix-specific values
    let mut job_env = base_env_context.clone();
    environment::add_matrix_context(&mut job_env, combination);

    // Add job-level environment variables
    for (key, value) in &job_template.env {
        // TODO: Substitute matrix variable references in env values
        job_env.insert(key.clone(), value.clone());
    }

    // Execute the job steps
    let mut step_results = Vec::new();
    let mut job_logs = String::new();

    // Create a temporary directory for this job execution
    let job_dir = tempfile::tempdir().map_err(|e| {
        ExecutionError::ExecutionError(format!("Failed to create job directory: {}", e))
    })?;

    // Prepare the runner
    let runner_image = get_runner_image(&job_template.runs_on);
    prepare_runner_image(&runner_image, runtime, verbose).await?;

    // Copy project files to workspace
    let current_dir = std::env::current_dir().map_err(|e| {
        ExecutionError::ExecutionError(format!("Failed to get current directory: {}", e))
    })?;
    copy_directory_contents(&current_dir, job_dir.path())?;

    let job_success = if job_template.steps.is_empty() {
        logging::warning(&format!("Job '{}' has no steps", matrix_job_name));
        true
    } else {
        // Execute each step
        for (idx, step) in job_template.steps.iter().enumerate() {
            match execute_step(
                step,
                idx,
                &job_env,
                job_dir.path(),
                runtime,
                workflow,
                &job_template.runs_on, // Pass the job's runner
                verbose,
                &Some(combination.values.clone()),
            )
            .await
            {
                Ok(result) => {
                    job_logs.push_str(&format!("Step: {}\n", result.name));
                    job_logs.push_str(&format!("Status: {:?}\n", result.status));
                    job_logs.push_str(&result.output);
                    job_logs.push_str("\n\n");

                    step_results.push(result.clone());

                    if result.status != StepStatus::Success {
                        // Step failed, abort job
                        return Ok(JobResult {
                            name: matrix_job_name,
                            status: JobStatus::Failure,
                            steps: step_results,
                            logs: job_logs,
                        });
                    }
                }
                Err(e) => {
                    // Log the error and abort the job
                    job_logs.push_str(&format!("Step execution error: {}\n\n", e));
                    return Ok(JobResult {
                        name: matrix_job_name,
                        status: JobStatus::Failure,
                        steps: step_results,
                        logs: job_logs,
                    });
                }
            }
        }

        true
    };

    // Return job result
    Ok(JobResult {
        name: matrix_job_name,
        status: if job_success {
            JobStatus::Success
        } else {
            JobStatus::Failure
        },
        steps: step_results,
        logs: job_logs,
    })
}

#[allow(unused_variables, unused_assignments)]
async fn execute_job(
    job_name: &str,
    workflow: &WorkflowDefinition,
    runtime: &Box<dyn ContainerRuntime>,
    env_context: &HashMap<String, String>,
    verbose: bool,
) -> Result<JobResult, ExecutionError> {
    // Get job definition
    let job = workflow.jobs.get(job_name).ok_or_else(|| {
        ExecutionError::ExecutionError(format!("Job '{}' not found in workflow", job_name))
    })?;

    // Clone context and add job-specific variables
    let mut job_env = env_context.clone();

    // Add job-level environment variables
    for (key, value) in &job.env {
        job_env.insert(key.clone(), value.clone());
    }

    // Execute job steps
    let mut step_results = Vec::new();
    let mut job_logs = String::new();

    // Create a temporary directory for this job execution
    let job_dir = tempfile::tempdir().map_err(|e| {
        ExecutionError::ExecutionError(format!("Failed to create job directory: {}", e))
    })?;

    // Try to get a Docker client if using Docker and services exist
    let docker_client = if !job.services.is_empty() {
        match Docker::connect_with_local_defaults() {
            Ok(client) => Some(client),
            Err(e) => {
                logging::error(&format!("Failed to connect to Docker: {}", e));
                None
            }
        }
    } else {
        None
    };

    // Create a Docker network for this job if we have services
    let network_id = if !job.services.is_empty() && docker_client.is_some() {
        let docker = match docker_client.as_ref() {
            Some(client) => client,
            None => {
                return Err(ExecutionError::RuntimeError(
                    "Docker client is required but not available".to_string(),
                ));
            }
        };
        match docker::create_job_network(docker).await {
            Ok(id) => {
                logging::info(&format!("Created network {} for job '{}'", id, job_name));
                Some(id)
            }
            Err(e) => {
                logging::error(&format!(
                    "Failed to create network for job '{}': {}",
                    job_name, e
                ));
                return Err(ExecutionError::RuntimeError(format!(
                    "Failed to create network: {}",
                    e
                )));
            }
        }
    } else {
        None
    };

    // Start service containers if any
    let mut service_containers = Vec::new();

    if !job.services.is_empty() {
        if docker_client.is_none() {
            logging::error("Services are only supported with Docker runtime");
            return Err(ExecutionError::RuntimeError(
                "Services require Docker runtime".to_string(),
            ));
        }

        logging::info(&format!(
            "Starting {} service containers for job '{}'",
            job.services.len(),
            job_name
        ));

        let docker = match docker_client.as_ref() {
            Some(client) => client,
            None => {
                return Err(ExecutionError::RuntimeError(
                    "Docker client is required but not available".to_string(),
                ));
            }
        };

        #[allow(unused_variables, unused_assignments)]
        for (service_name, service_config) in &job.services {
            logging::info(&format!(
                "Starting service '{}' with image '{}'",
                service_name, service_config.image
            ));

            // Prepare container configuration
            let container_name = format!("wrkflw-service-{}-{}", job_name, service_name);

            // Map ports if specified
            let mut port_bindings = HashMap::new();
            if let Some(ports) = &service_config.ports {
                for port_spec in ports {
                    // Parse port spec like "8080:80"
                    let parts: Vec<&str> = port_spec.split(':').collect();
                    if parts.len() == 2 {
                        let host_port = parts[0];
                        let container_port = parts[1];

                        let port_binding = bollard::models::PortBinding {
                            host_ip: Some("0.0.0.0".to_string()),
                            host_port: Some(host_port.to_string()),
                        };

                        let key = format!("{}/tcp", container_port);
                        port_bindings.insert(key, Some(vec![port_binding]));
                    }
                }
            }

            // Convert environment variables
            let env_vars: Vec<String> = service_config
                .env
                .iter()
                .map(|(k, v)| format!("{}={}", k, v))
                .collect();

            // Create container options
            let create_opts = bollard::container::CreateContainerOptions {
                name: container_name,
                platform: None,
            };

            // Host configuration
            let host_config = bollard::models::HostConfig {
                port_bindings: Some(port_bindings),
                network_mode: network_id.clone(),
                ..Default::default()
            };

            // Container configuration
            let config = bollard::container::Config {
                image: Some(service_config.image.clone()),
                env: Some(env_vars),
                host_config: Some(host_config),
                ..Default::default()
            };

            // Log the network connection
            if network_id.is_some() {
                logging::info(&format!(
                    "Service '{}' connected to network via host_config",
                    service_name
                ));
            }

            match docker.create_container(Some(create_opts), config).await {
                Ok(response) => {
                    let container_id = response.id;

                    // Track the container for cleanup
                    docker::track_container(&container_id);
                    service_containers.push(container_id.clone());

                    // Start the container
                    match docker.start_container::<String>(&container_id, None).await {
                        Ok(_) => {
                            logging::info(&format!("Started service container: {}", container_id));

                            // Add service address to environment
                            job_env.insert(
                                format!("{}_HOST", service_name.to_uppercase()),
                                service_name.clone(),
                            );

                            job_logs.push_str(&format!(
                                "Started service '{}' with container ID: {}\n",
                                service_name, container_id
                            ));
                        }
                        Err(e) => {
                            let error_msg = format!(
                                "Failed to start service container '{}': {}",
                                service_name, e
                            );
                            logging::error(&error_msg);

                            // Clean up the created container
                            let _ = docker.remove_container(&container_id, None).await;

                            // Clean up network if created
                            if let Some(net_id) = &network_id {
                                let _ = docker.remove_network(net_id).await;
                                docker::untrack_network(net_id);
                            }

                            return Err(ExecutionError::RuntimeError(error_msg));
                        }
                    }
                }
                Err(e) => {
                    let error_msg = format!(
                        "Failed to create service container '{}': {}",
                        service_name, e
                    );
                    logging::error(&error_msg);

                    // Clean up network if created
                    if let Some(net_id) = &network_id {
                        let _ = docker.remove_network(net_id).await;
                        docker::untrack_network(net_id);
                    }

                    return Err(ExecutionError::RuntimeError(error_msg));
                }
            }
        }

        // Give services a moment to start up
        tokio::time::sleep(tokio::time::Duration::from_secs(2)).await;
    }

    // Prepare the runner environment
    let runner_image = get_runner_image(&job.runs_on);
    prepare_runner_image(&runner_image, runtime, verbose).await?;

    // Copy project files to workspace
    let current_dir = std::env::current_dir().map_err(|e| {
        ExecutionError::ExecutionError(format!("Failed to get current directory: {}", e))
    })?;
    copy_directory_contents(&current_dir, job_dir.path())?;

    logging::info(&format!("Executing job: {}", job_name));

    let mut job_success = true;

    // Execute job steps
    for (idx, step) in job.steps.iter().enumerate() {
        let step_result = execute_step(
            step,
            idx,
            &job_env,
            job_dir.path(),
            runtime,
            workflow,
            &job.runs_on,
            verbose,
            &None,
        )
        .await;

        match step_result {
            Ok(result) => {
                // Check if step was successful
                if result.status == StepStatus::Failure {
                    job_success = false;
                }

                // Add step output to logs
                if !result.output.is_empty() {
                    job_logs.push_str(&format!(
                        "\n=== Output from step '{}' ===\n{}\n=== End output ===\n\n",
                        result.name, result.output
                    ));
                }

                step_results.push(result);
            }
            Err(e) => {
                job_success = false;
                job_logs.push_str(&format!("\n=== ERROR in step {} ===\n{}\n", idx + 1, e));

                // Record the error as a failed step
                step_results.push(StepResult {
                    name: step
                        .name
                        .clone()
                        .unwrap_or_else(|| format!("Step {}", idx + 1)),
                    status: StepStatus::Failure,
                    output: format!("Error: {}", e),
                });

                // Stop executing further steps
                break;
            }
        }
    }

    // Clean up service containers
    if !service_containers.is_empty() && docker_client.is_some() {
        let docker = match docker_client.as_ref() {
            Some(client) => client,
            None => {
                return Err(ExecutionError::RuntimeError(
                    "Docker client is required but not available".to_string(),
                ));
            }
        };

        for container_id in &service_containers {
            logging::info(&format!("Stopping service container: {}", container_id));

            let _ = docker.stop_container(container_id, None).await;
            let _ = docker.remove_container(container_id, None).await;

            // Untrack container since we've explicitly removed it
            docker::untrack_container(container_id);
        }
    }

    // Clean up network if created
    if let Some(net_id) = &network_id {
        if docker_client.is_some() {
            let docker = match docker_client.as_ref() {
                Some(client) => client,
                None => {
                    return Err(ExecutionError::RuntimeError(
                        "Docker client is required but not available".to_string(),
                    ));
                }
            };

            logging::info(&format!("Removing network: {}", net_id));
            if let Err(e) = docker.remove_network(net_id).await {
                logging::error(&format!("Failed to remove network {}: {}", net_id, e));
            }

            // Untrack network since we've explicitly removed it
            docker::untrack_network(net_id);
        }
    }

    Ok(JobResult {
        name: job_name.to_string(),
        status: if job_success {
            JobStatus::Success
        } else {
            JobStatus::Failure
        },
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
    matrix_combination: &Option<HashMap<String, Value>>,
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
                cmd.push(match owned_strings.last() {
                    Some(s) => s,
                    None => {
                        return Err(ExecutionError::ExecutionError(
                            "Expected at least one string in action arguments".to_string(),
                        ));
                    }
                });
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
        // Apply GitHub-style matrix variable substitution to the command
        let processed_run = substitution::process_step_run(run, matrix_combination);

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
            cmd_strings.push(processed_run);
            cmd.push(&cmd_strings[0]);
        } else if shell == "powershell" {
            cmd.push("pwsh");
            cmd.push("-Command");

            // Store the string and keep a reference to it
            cmd_strings.push(processed_run);
            cmd.push(&cmd_strings[0]);
        } else {
            cmd.push("sh");
            cmd.push("-c");

            // Store the string and keep a reference to it
            cmd_strings.push(processed_run);
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
        let file_name = match path.file_name() {
            Some(name) => name.to_string_lossy(),
            None => {
                return Err(ExecutionError::ExecutionError(
                    format!("Failed to get file name from path: {:?}", path)
                ));
            }
        };
        if file_name.starts_with(".") || file_name == "target" {
            continue;
        }

        let dest_path = match path.file_name() {
            Some(name) => to.join(name),
            None => {
                return Err(ExecutionError::ExecutionError(
                    format!("Failed to get file name from path: {:?}", path)
                ));
            }
        };

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
                    &None,
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
