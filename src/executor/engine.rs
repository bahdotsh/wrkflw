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
    let runtime = initialize_runtime(runtime_type.clone())?;

    // Create a temporary workspace directory
    let workspace_dir = tempfile::tempdir()
        .map_err(|e| ExecutionError::Execution(format!("Failed to create workspace: {}", e)))?;

    // 4. Set up GitHub-like environment
    let mut env_context = environment::create_github_context(&workflow, workspace_dir.path());

    // Add runtime mode to environment
    env_context.insert(
        "WRKFLW_RUNTIME_MODE".to_string(),
        if runtime_type == RuntimeType::Emulation {
            "emulation".to_string()
        } else {
            "docker".to_string()
        },
    );

    // Add flag to hide GitHub action messages when in emulation mode
    env_context.insert(
        "WRKFLW_HIDE_ACTION_MESSAGES".to_string(),
        "true".to_string(),
    );

    // Setup GitHub environment files
    environment::setup_github_environment_files(workspace_dir.path()).map_err(|e| {
        ExecutionError::Execution(format!("Failed to setup GitHub env files: {}", e))
    })?;

    // 5. Execute jobs according to the plan
    let mut results = Vec::new();
    for job_batch in execution_plan {
        // Execute jobs in parallel if they don't depend on each other
        let job_results = execute_job_batch(
            &job_batch,
            &workflow,
            runtime.as_ref(),
            &env_context,
            verbose,
        )
        .await?;
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
                logging::error("Docker not available, falling back to emulation mode");
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
    Parse(String),

    #[error("Runtime error: {0}")]
    Runtime(String),

    #[error("Execution error: {0}")]
    Execution(String),

    #[error("IO error: {0}")]
    Io(#[from] std::io::Error),
}

// Convert errors from other modules
impl From<String> for ExecutionError {
    fn from(err: String) -> Self {
        ExecutionError::Parse(err)
    }
}

// Add Action preparation functions
async fn prepare_action(
    action: &ActionInfo,
    runtime: &dyn ContainerRuntime,
) -> Result<String, ExecutionError> {
    if action.is_docker {
        // Docker action: pull the image
        let image = action.repository.trim_start_matches("docker://");

        runtime
            .pull_image(image)
            .await
            .map_err(|e| ExecutionError::Runtime(format!("Failed to pull Docker image: {}", e)))?;

        return Ok(image.to_string());
    }

    if action.is_local {
        // Local action: build from local directory
        let action_dir = Path::new(&action.repository);

        if !action_dir.exists() {
            return Err(ExecutionError::Execution(format!(
                "Local action directory not found: {}",
                action_dir.display()
            )));
        }

        let dockerfile = action_dir.join("Dockerfile");
        if dockerfile.exists() {
            // It's a Docker action, build it
            let tag = format!("wrkflw-local-action:{}", uuid::Uuid::new_v4());

            runtime
                .build_image(&dockerfile, &tag)
                .await
                .map_err(|e| ExecutionError::Runtime(format!("Failed to build image: {}", e)))?;

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
    runtime: &dyn ContainerRuntime,
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

// Before execute_job_with_matrix implementation, add this struct
struct JobExecutionContext<'a> {
    job_name: &'a str,
    workflow: &'a WorkflowDefinition,
    runtime: &'a dyn ContainerRuntime,
    env_context: &'a HashMap<String, String>,
    verbose: bool,
}

/// Execute a job, expanding matrix if present
async fn execute_job_with_matrix(
    job_name: &str,
    workflow: &WorkflowDefinition,
    runtime: &dyn ContainerRuntime,
    env_context: &HashMap<String, String>,
    verbose: bool,
) -> Result<Vec<JobResult>, ExecutionError> {
    // Get the job definition
    let job = workflow.jobs.get(job_name).ok_or_else(|| {
        ExecutionError::Execution(format!("Job '{}' not found in workflow", job_name))
    })?;

    // Check if this is a matrix job
    if let Some(matrix_config) = &job.matrix {
        // Expand the matrix into combinations
        let combinations = matrix::expand_matrix(matrix_config)
            .map_err(|e| ExecutionError::Execution(format!("Failed to expand matrix: {}", e)))?;

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
        execute_matrix_combinations(MatrixExecutionContext {
            job_name,
            job_template: job,
            combinations: &combinations,
            max_parallel,
            fail_fast: matrix_config.fail_fast.unwrap_or(true),
            workflow,
            runtime,
            env_context,
            verbose,
        })
        .await
    } else {
        // Regular job, no matrix
        let ctx = JobExecutionContext {
            job_name,
            workflow,
            runtime,
            env_context,
            verbose,
        };
        let result = execute_job(ctx).await?;
        Ok(vec![result])
    }
}

#[allow(unused_variables, unused_assignments)]
async fn execute_job(ctx: JobExecutionContext<'_>) -> Result<JobResult, ExecutionError> {
    // Get job definition
    let job = ctx.workflow.jobs.get(ctx.job_name).ok_or_else(|| {
        ExecutionError::Execution(format!("Job '{}' not found in workflow", ctx.job_name))
    })?;

    // Clone context and add job-specific variables
    let mut job_env = ctx.env_context.clone();

    // Add job-level environment variables
    for (key, value) in &job.env {
        job_env.insert(key.clone(), value.clone());
    }

    // Execute job steps
    let mut step_results = Vec::new();
    let mut job_logs = String::new();

    // Create a temporary directory for this job execution
    let job_dir = tempfile::tempdir()
        .map_err(|e| ExecutionError::Execution(format!("Failed to create job directory: {}", e)))?;

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
                return Err(ExecutionError::Runtime(
                    "Docker client is required but not available".to_string(),
                ));
            }
        };
        match docker::create_job_network(docker).await {
            Ok(id) => {
                logging::info(&format!(
                    "Created network {} for job '{}'",
                    id, ctx.job_name
                ));
                Some(id)
            }
            Err(e) => {
                logging::error(&format!(
                    "Failed to create network for job '{}': {}",
                    ctx.job_name, e
                ));
                return Err(ExecutionError::Runtime(format!(
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
            return Err(ExecutionError::Runtime(
                "Services require Docker runtime".to_string(),
            ));
        }

        logging::info(&format!(
            "Starting {} service containers for job '{}'",
            job.services.len(),
            ctx.job_name
        ));

        let docker = match docker_client.as_ref() {
            Some(client) => client,
            None => {
                return Err(ExecutionError::Runtime(
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
            let container_name = format!("wrkflw-service-{}-{}", ctx.job_name, service_name);

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

                            return Err(ExecutionError::Runtime(error_msg));
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

                    return Err(ExecutionError::Runtime(error_msg));
                }
            }
        }

        // Give services a moment to start up
        tokio::time::sleep(tokio::time::Duration::from_secs(2)).await;
    }

    // Prepare the runner environment
    let runner_image = get_runner_image(&job.runs_on);
    prepare_runner_image(&runner_image, ctx.runtime, ctx.verbose).await?;

    // Copy project files to workspace
    let current_dir = std::env::current_dir().map_err(|e| {
        ExecutionError::Execution(format!("Failed to get current directory: {}", e))
    })?;
    copy_directory_contents(&current_dir, job_dir.path())?;

    logging::info(&format!("Executing job: {}", ctx.job_name));

    let mut job_success = true;

    // Execute job steps
    for (idx, step) in job.steps.iter().enumerate() {
        let step_result = execute_step(StepExecutionContext {
            step,
            step_idx: idx,
            job_env: &job_env,
            working_dir: job_dir.path(),
            runtime: ctx.runtime,
            workflow: ctx.workflow,
            job_runs_on: &job.runs_on,
            verbose: ctx.verbose,
            matrix_combination: &None,
        })
        .await;

        match step_result {
            Ok(result) => {
                // Check if step was successful
                if result.status == StepStatus::Failure {
                    job_success = false;
                }

                // Add step output to logs only in verbose mode or if there's an error
                if ctx.verbose || result.status == StepStatus::Failure {
                    job_logs.push_str(&format!(
                        "\n=== Output from step '{}' ===\n{}\n=== End output ===\n\n",
                        result.name, result.output
                    ));
                } else {
                    // In non-verbose mode, just record that the step ran but don't include output
                    job_logs.push_str(&format!("Step '{}' completed with status: {:?}\n", 
                        result.name, result.status));
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
                return Err(ExecutionError::Runtime(
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
                    return Err(ExecutionError::Runtime(
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
        name: ctx.job_name.to_string(),
        status: if job_success {
            JobStatus::Success
        } else {
            JobStatus::Failure
        },
        steps: step_results,
        logs: job_logs,
    })
}

// Before the execute_matrix_combinations function, add this struct
struct MatrixExecutionContext<'a> {
    job_name: &'a str,
    job_template: &'a Job,
    combinations: &'a [MatrixCombination],
    max_parallel: usize,
    fail_fast: bool,
    workflow: &'a WorkflowDefinition,
    runtime: &'a dyn ContainerRuntime,
    env_context: &'a HashMap<String, String>,
    verbose: bool,
}

/// Execute a set of matrix combinations
async fn execute_matrix_combinations(
    ctx: MatrixExecutionContext<'_>,
) -> Result<Vec<JobResult>, ExecutionError> {
    let mut results = Vec::new();
    let mut any_failed = false;

    // Process combinations in chunks limited by max_parallel
    for chunk in ctx.combinations.chunks(ctx.max_parallel) {
        // Skip processing if fail-fast is enabled and a previous job failed
        if ctx.fail_fast && any_failed {
            // Add skipped results for remaining combinations
            for combination in chunk {
                let combination_name = matrix::format_combination_name(ctx.job_name, combination);
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
                ctx.job_name,
                ctx.job_template,
                combination,
                ctx.workflow,
                ctx.runtime,
                ctx.env_context,
                ctx.verbose,
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

                    if ctx.fail_fast {
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
    runtime: &dyn ContainerRuntime,
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
    let job_dir = tempfile::tempdir()
        .map_err(|e| ExecutionError::Execution(format!("Failed to create job directory: {}", e)))?;

    // Prepare the runner
    let runner_image = get_runner_image(&job_template.runs_on);
    prepare_runner_image(&runner_image, runtime, verbose).await?;

    // Copy project files to workspace
    let current_dir = std::env::current_dir().map_err(|e| {
        ExecutionError::Execution(format!("Failed to get current directory: {}", e))
    })?;
    copy_directory_contents(&current_dir, job_dir.path())?;

    let job_success = if job_template.steps.is_empty() {
        logging::warning(&format!("Job '{}' has no steps", matrix_job_name));
        true
    } else {
        // Execute each step
        for (idx, step) in job_template.steps.iter().enumerate() {
            match execute_step(StepExecutionContext {
                step,
                step_idx: idx,
                job_env: &job_env,
                working_dir: job_dir.path(),
                runtime,
                workflow,
                job_runs_on: &job_template.runs_on,
                verbose,
                matrix_combination: &Some(combination.values.clone()),
            })
            .await
            {
                Ok(result) => {
                    job_logs.push_str(&format!("Step: {}\n", result.name));
                    job_logs.push_str(&format!("Status: {:?}\n", result.status));
                    
                    // Only include step output in verbose mode or if there's an error
                    if verbose || result.status == StepStatus::Failure {
                        job_logs.push_str(&result.output);
                        job_logs.push_str("\n\n");
                    } else {
                        job_logs.push('\n');
                    }

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

// Before the execute_step function, add this struct
struct StepExecutionContext<'a> {
    step: &'a crate::parser::workflow::Step,
    step_idx: usize,
    job_env: &'a HashMap<String, String>,
    working_dir: &'a Path,
    runtime: &'a dyn ContainerRuntime,
    workflow: &'a WorkflowDefinition,
    job_runs_on: &'a str,
    verbose: bool,
    matrix_combination: &'a Option<HashMap<String, Value>>,
}

async fn execute_step(ctx: StepExecutionContext<'_>) -> Result<StepResult, ExecutionError> {
    let step_name = ctx
        .step
        .name
        .clone()
        .unwrap_or_else(|| format!("Step {}", ctx.step_idx + 1));

    if ctx.verbose {
        logging::info(&format!("  Executing step: {}", step_name));
    }

    // Prepare step environment
    let mut step_env = ctx.job_env.clone();

    // Add step-level environment variables
    for (key, value) in &ctx.step.env {
        step_env.insert(key.clone(), value.clone());
    }

    // Execute the step based on its type
    if let Some(uses) = &ctx.step.uses {
        // Action step
        let action_info = ctx.workflow.resolve_action(uses);

        // Check if this is the checkout action
        if uses.starts_with("actions/checkout") {
            // Get the current directory (assumes this is where your project is)
            let current_dir = std::env::current_dir().map_err(|e| {
                ExecutionError::Execution(format!("Failed to get current dir: {}", e))
            })?;

            // Copy the project files to the workspace
            copy_directory_contents(&current_dir, ctx.working_dir)?;

            // Add info for logs
            let output = "Emulated checkout: Copied current directory to workspace".to_string();

            if ctx.verbose {
                println!("  Emulated actions/checkout: copied project files to workspace");
            }

            Ok(StepResult {
                name: step_name,
                status: StepStatus::Success,
                output,
            })
        } else {
            // Get action info
            let image = prepare_action(&action_info, ctx.runtime).await?;

            // Special handling for composite actions
            if image == "composite" && action_info.is_local {
                // Handle composite action
                let action_path = Path::new(&action_info.repository);
                return execute_composite_action(
                    ctx.step,
                    action_path,
                    &step_env,
                    ctx.working_dir,
                    ctx.runtime,
                    ctx.job_runs_on,
                    ctx.verbose,
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

                // Check if we should hide GitHub action messages
                let hide_action_value = ctx
                    .job_env
                    .get("WRKFLW_HIDE_ACTION_MESSAGES")
                    .cloned()
                    .unwrap_or_else(|| "not set".to_string());

                logging::debug(&format!(
                    "WRKFLW_HIDE_ACTION_MESSAGES value: {}",
                    hide_action_value
                ));

                let hide_messages = hide_action_value == "true";
                logging::debug(&format!("Should hide messages: {}", hide_messages));

                // Only log a message to the console if we're showing action messages
                if !hide_messages {
                    // For Emulation mode, log a message about what action would be executed
                    println!("   ⚙️ Would execute GitHub action: {}", uses);
                }

                // GitHub actions - would need to clone repo and setup action
                cmd.push("sh");
                cmd.push("-c");

                // Store the string and keep a reference to it
                let echo_cmd = if hide_messages {
                    // Empty string so there's no output
                    "".to_string()
                } else {
                    format!("echo 'Would execute GitHub action: {}'", uses)
                };

                owned_strings.push(echo_cmd);
                cmd.push(match owned_strings.last() {
                    Some(s) => s,
                    None => {
                        return Err(ExecutionError::Execution(
                            "Expected at least one string in action arguments".to_string(),
                        ));
                    }
                });
            }

            // Convert 'with' parameters to environment variables
            if let Some(with_params) = &ctx.step.with {
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
            let volumes: Vec<(&Path, &Path)> =
                vec![(ctx.working_dir, Path::new("/github/workspace"))];

            let output = ctx
                .runtime
                .run_container(
                    &image,
                    &cmd.to_vec(),
                    &env_vars,
                    Path::new("/github/workspace"),
                    &volumes,
                )
                .await
                .map_err(|e| ExecutionError::Runtime(format!("{}", e)))?;

            // Check if this was called from 'run' branch - don't try to hide these outputs
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
    } else if let Some(run) = &ctx.step.run {
        // Apply GitHub-style matrix variable substitution to the command
        let processed_run = substitution::process_step_run(run, ctx.matrix_combination);

        // Determine the shell to use based on the runner platform
        let mut cmd_strings = Vec::new();
        let mut cmd: Vec<&str> = Vec::new();

        // Detect platform from the runner string
        let platform = if ctx.job_runs_on.to_lowercase().contains("windows") {
            "windows"
        } else if ctx.job_runs_on.to_lowercase().contains("macos") {
            "macos"
        } else {
            "linux"
        };

        // Get the shell from environment or use platform-appropriate default
        let shell_env = step_env.get("SHELL").map(|s| s.as_str());

        // We'll use the runtime mode for this step - no reliable way to detect at runtime
        // so we trust the environment variables
        let runtime_mode = ctx
            .job_env
            .get("WRKFLW_RUNTIME_MODE")
            .map(|s| s.as_str())
            .unwrap_or("default");
        let is_emulation = runtime_mode == "emulation";

        match platform {
            "windows" => {
                // Use PowerShell for Windows runners
                cmd.push("pwsh");
                cmd.push("-Command");
                cmd_strings.push(processed_run);
                cmd.push(&cmd_strings[0]);

                // Add Windows-specific environment variables if not present
                if !step_env.contains_key("TEMP") {
                    step_env.insert("TEMP".to_string(), "C:\\Windows\\Temp".to_string());
                }
                if !step_env.contains_key("TMP") {
                    step_env.insert("TMP".to_string(), "C:\\Windows\\Temp".to_string());
                }
            }
            "macos" => {
                // Use bash for macOS runners
                cmd.push("bash");
                cmd.push("-e");
                cmd.push("-c");
                cmd_strings.push(processed_run);
                cmd.push(&cmd_strings[0]);

                // Add macOS-specific environment variables if not present
                if !step_env.contains_key("TMPDIR") {
                    step_env.insert("TMPDIR".to_string(), "/tmp".to_string());
                }

                // Set HOME directory differently based on runtime mode
                if !step_env.contains_key("HOME") {
                    if is_emulation {
                        // In emulation mode, use the real user's home directory
                        if let Ok(home) = std::env::var("HOME") {
                            step_env.insert("HOME".to_string(), home);
                        } else {
                            // Fallback to something that should be writable
                            step_env.insert("HOME".to_string(), ".".to_string());
                        }
                    } else {
                        // In Docker mode, use /root
                        step_env.insert("HOME".to_string(), "/root".to_string());
                    }
                }

                // Add RUNNER_OS to help scripts identify the platform
                if !step_env.contains_key("RUNNER_OS") {
                    step_env.insert("RUNNER_OS".to_string(), "macOS".to_string());
                }
            }
            _ => {
                // linux and any other platform
                match shell_env {
                    Some("powershell") => {
                        cmd.push("pwsh");
                        cmd.push("-Command");
                        cmd_strings.push(processed_run);
                        cmd.push(&cmd_strings[0]);
                    }
                    Some(shell) if shell.ends_with("bash") => {
                        cmd.push("bash");
                        cmd.push("-e");
                        cmd.push("-c");
                        cmd_strings.push(processed_run);
                        cmd.push(&cmd_strings[0]);
                    }
                    _ => {
                        // Default to sh
                        cmd.push("sh");
                        cmd.push("-c");
                        cmd_strings.push(processed_run);
                        cmd.push(&cmd_strings[0]);
                    }
                }
            }
        }

        // Convert environment HashMap to Vec<(&str, &str)> for container runtime
        let env_vars: Vec<(&str, &str)> = step_env
            .iter()
            .map(|(k, v)| (k.as_str(), v.as_str()))
            .collect();

        // Use the appropriate runner image based on platform
        let image = get_runner_image(ctx.job_runs_on);

        ctx.runtime
            .pull_image(&image)
            .await
            .map_err(|e| ExecutionError::Runtime(format!("Failed to pull image: {}", e)))?;

        // Map volumes with platform-specific paths
        let container_workspace_path = Path::new("/github/workspace"); // Use consistent path for all platforms

        let volumes: Vec<(&Path, &Path)> = vec![(ctx.working_dir, container_workspace_path)];

        let output = ctx
            .runtime
            .run_container(&image, &cmd, &env_vars, container_workspace_path, &volumes)
            .await
            .map_err(|e| ExecutionError::Runtime(format!("{}", e)))?;

        // Check if this was called from 'run' branch - don't try to hide these outputs
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
        Err(ExecutionError::Execution(format!(
            "Step '{}' has neither 'uses' nor 'run' directive",
            step_name
        )))
    }
}

fn copy_directory_contents(from: &Path, to: &Path) -> Result<(), ExecutionError> {
    for entry in std::fs::read_dir(from)
        .map_err(|e| ExecutionError::Execution(format!("Failed to read directory: {}", e)))?
    {
        let entry =
            entry.map_err(|e| ExecutionError::Execution(format!("Failed to read entry: {}", e)))?;
        let path = entry.path();

        // Skip hidden files/dirs and target directory for efficiency
        let file_name = match path.file_name() {
            Some(name) => name.to_string_lossy(),
            None => {
                return Err(ExecutionError::Execution(format!(
                    "Failed to get file name from path: {:?}",
                    path
                )));
            }
        };
        if file_name.starts_with(".") || file_name == "target" {
            continue;
        }

        let dest_path = match path.file_name() {
            Some(name) => to.join(name),
            None => {
                return Err(ExecutionError::Execution(format!(
                    "Failed to get file name from path: {:?}",
                    path
                )));
            }
        };

        if path.is_dir() {
            std::fs::create_dir_all(&dest_path)
                .map_err(|e| ExecutionError::Execution(format!("Failed to create dir: {}", e)))?;

            // Recursively copy subdirectories
            copy_directory_contents(&path, &dest_path)?;
        } else {
            std::fs::copy(&path, &dest_path)
                .map_err(|e| ExecutionError::Execution(format!("Failed to copy file: {}", e)))?;
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

        // macOS runners - use existing images for macOS compatibility layer
        "macos-latest" => "catthehacker/ubuntu:act-latest", // Use Ubuntu with macOS compatibility
        "macos-12" => "catthehacker/ubuntu:act-latest",     // Monterey equivalent
        "macos-11" => "catthehacker/ubuntu:act-latest",     // Big Sur equivalent
        "macos-10.15" => "catthehacker/ubuntu:act-latest",  // Catalina equivalent

        // Windows runners - using servercore-based images
        "windows-latest" => "mcr.microsoft.com/windows/servercore:ltsc2022",
        "windows-2022" => "mcr.microsoft.com/windows/servercore:ltsc2022",
        "windows-2019" => "mcr.microsoft.com/windows/servercore:ltsc2019",

        // Default case for other runners or custom strings
        _ => {
            // Check for platform prefixes and provide appropriate images
            let runs_on_lower = runs_on.trim().to_lowercase();
            if runs_on_lower.starts_with("macos") {
                "catthehacker/ubuntu:act-latest" // Use Ubuntu with macOS compatibility
            } else if runs_on_lower.starts_with("windows") {
                "mcr.microsoft.com/windows/servercore:ltsc2022" // Default Windows image
            } else {
                "ubuntu:latest" // Default to Ubuntu for everything else
            }
        }
    }
    .to_string()
}

async fn prepare_runner_image(
    image: &str,
    runtime: &dyn ContainerRuntime,
    verbose: bool,
) -> Result<(), ExecutionError> {
    if verbose {
        println!("  Preparing runner image: {}", image);
    }

    // Check if this is a platform-specific image
    let is_windows_image =
        image.contains("windows") || image.contains("servercore") || image.contains("nanoserver");
    let is_macos_emu =
        image.contains("act-") && (image.contains("catthehacker") || image.contains("nektos"));

    // Display appropriate warnings for non-Linux runners
    if is_windows_image {
        logging::warning("Windows runners in Docker mode have limited compatibility");
        logging::info("Some Windows-specific features may not work correctly");
    } else if is_macos_emu {
        logging::warning(
            "macOS emulation active - running on Linux with macOS compatibility layer",
        );
        logging::info("Using an Ubuntu-based runner with macOS compatibility");
    }

    // Pull the image
    match runtime.pull_image(image).await {
        Ok(_) => {
            if verbose {
                println!("  Image {} ready", image);
            }
            Ok(())
        }
        Err(e) => {
            // For platform-specific images, provide more helpful error messages
            if is_windows_image {
                logging::error("Failed to pull Windows runner image. Docker may not support Windows containers on your system.");
                logging::info("Try using emulation mode (-e) or switch to a Linux-based runner like 'ubuntu-latest'");
            } else if is_macos_emu {
                logging::error("Failed to pull macOS compatibility image.");
                logging::info("Try using emulation mode (-e) or switch to a Linux-based runner like 'ubuntu-latest'");
            }

            Err(ExecutionError::Runtime(format!(
                "Failed to pull runner image: {}",
                e
            )))
        }
    }
}

async fn execute_composite_action(
    step: &crate::parser::workflow::Step,
    action_path: &Path,
    job_env: &HashMap<String, String>,
    working_dir: &Path,
    runtime: &dyn ContainerRuntime,
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
        return Err(ExecutionError::Execution(format!(
            "No action.yml or action.yaml found in {}",
            action_path.display()
        )));
    };

    // Parse the composite action definition
    let action_content = fs::read_to_string(&action_file)
        .map_err(|e| ExecutionError::Execution(format!("Failed to read action file: {}", e)))?;

    let action_def: serde_yaml::Value = serde_yaml::from_str(&action_content)
        .map_err(|e| ExecutionError::Execution(format!("Invalid action YAML: {}", e)))?;

    // Check if it's a composite action
    match action_def.get("runs").and_then(|v| v.get("using")) {
        Some(serde_yaml::Value::String(using)) if using == "composite" => {
            // Get the steps
            let steps = match action_def.get("runs").and_then(|v| v.get("steps")) {
                Some(serde_yaml::Value::Sequence(steps)) => steps,
                _ => {
                    return Err(ExecutionError::Execution(
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
                        return Err(ExecutionError::Execution(format!(
                            "Failed to process composite action step {}: {}",
                            idx + 1,
                            e
                        )))
                    }
                };

                // Execute the step - using Box::pin to handle async recursion
                let step_result = Box::pin(execute_step(StepExecutionContext {
                    step: &composite_step,
                    step_idx: idx,
                    job_env: &action_env,
                    working_dir,
                    runtime,
                    workflow: &crate::parser::workflow::WorkflowDefinition {
                        name: "Composite Action".to_string(),
                        on: vec![],
                        on_raw: serde_yaml::Value::Null,
                        jobs: HashMap::new(),
                    },
                    job_runs_on,
                    verbose,
                    matrix_combination: &None,
                }))
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
        _ => Err(ExecutionError::Execution(
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
    let final_run = run;

    // Extract continue_on_error
    let continue_on_error = step_yaml
        .get("continue-on-error")
        .and_then(|v| v.as_bool());

    Ok(crate::parser::workflow::Step {
        name,
        uses,
        run: final_run,
        with,
        env,
        continue_on_error,
    })
}
