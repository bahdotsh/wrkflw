#[allow(unused_imports)]
use bollard::Docker;
use futures::future;
use serde_yaml::Value;
use std::collections::HashMap;
use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;
use thiserror::Error;

use ignore::{gitignore::GitignoreBuilder, Match};

use crate::action_resolver;
use crate::dependency;
use crate::docker;
use crate::environment;
use crate::podman;
use wrkflw_logging;
use wrkflw_matrix::MatrixCombination;
use wrkflw_models::gitlab::Pipeline;
use wrkflw_parser::gitlab::{self, parse_pipeline};
use wrkflw_parser::workflow::{
    self, parse_workflow, ActionInfo, Job, JobContainer, Step, WorkflowDefinition,
};
use wrkflw_runtime::container::ContainerRuntime;
use wrkflw_runtime::emulation;
use wrkflw_secrets::{SecretConfig, SecretManager, SecretMasker, SecretSubstitution};

#[allow(unused_variables, unused_assignments)]
/// Execute a GitHub Actions workflow file locally
pub async fn execute_workflow(
    workflow_path: &Path,
    config: ExecutionConfig,
) -> Result<ExecutionResult, ExecutionError> {
    wrkflw_logging::info(&format!("Executing workflow: {}", workflow_path.display()));
    wrkflw_logging::info(&format!("Runtime: {:?}", config.runtime_type));

    // Determine if this is a GitLab CI/CD pipeline or GitHub Actions workflow
    let is_gitlab = is_gitlab_pipeline(workflow_path);

    if is_gitlab {
        execute_gitlab_pipeline(workflow_path, config.clone()).await
    } else {
        execute_github_workflow(workflow_path, config.clone()).await
    }
}

/// Determine if a file is a GitLab CI/CD pipeline
fn is_gitlab_pipeline(path: &Path) -> bool {
    // Check the file name
    if let Some(file_name) = path.file_name() {
        if let Some(file_name_str) = file_name.to_str() {
            return file_name_str == ".gitlab-ci.yml" || file_name_str.ends_with("gitlab-ci.yml");
        }
    }

    // If file name check fails, try to read and determine by content
    if let Ok(content) = fs::read_to_string(path) {
        // GitLab CI/CD pipelines typically have stages, before_script, after_script at the top level
        if content.contains("stages:")
            || content.contains("before_script:")
            || content.contains("after_script:")
        {
            // Check for GitHub Actions specific keys that would indicate it's not GitLab
            if !content.contains("on:")
                && !content.contains("runs-on:")
                && !content.contains("uses:")
            {
                return true;
            }
        }
    }

    false
}

/// Execute a GitHub Actions workflow file locally
async fn execute_github_workflow(
    workflow_path: &Path,
    config: ExecutionConfig,
) -> Result<ExecutionResult, ExecutionError> {
    // 1. Parse workflow file
    let workflow = parse_workflow(workflow_path)?;

    // 2. Resolve job dependencies and create execution plan
    let execution_plan = dependency::resolve_dependencies(&workflow)?;

    // 3. Initialize appropriate runtime
    let runtime = initialize_runtime(
        config.runtime_type.clone(),
        config.preserve_containers_on_failure,
    )?;

    // Create a temporary workspace directory
    let workspace_dir = tempfile::tempdir()
        .map_err(|e| ExecutionError::Execution(format!("Failed to create workspace: {}", e)))?;

    // 4. Set up GitHub-like environment
    let mut env_context = environment::create_github_context(&workflow, workspace_dir.path());

    // Add runtime mode to environment
    env_context.insert(
        "WRKFLW_RUNTIME_MODE".to_string(),
        match config.runtime_type {
            RuntimeType::Emulation => "emulation".to_string(),
            RuntimeType::SecureEmulation => "secure_emulation".to_string(),
            RuntimeType::Docker => "docker".to_string(),
            RuntimeType::Podman => "podman".to_string(),
        },
    );

    // show=true means hide=false (inverted for the env var)
    env_context.insert(
        "WRKFLW_HIDE_ACTION_MESSAGES".to_string(),
        if config.show_action_messages {
            "false"
        } else {
            "true"
        }
        .to_string(),
    );

    // Setup GitHub environment files
    environment::setup_github_environment_files(workspace_dir.path()).map_err(|e| {
        ExecutionError::Execution(format!("Failed to setup GitHub env files: {}", e))
    })?;

    // 5. Initialize secrets management
    let secret_manager = if let Some(secrets_config) = &config.secrets_config {
        Some(
            SecretManager::new(secrets_config.clone())
                .await
                .map_err(|e| {
                    ExecutionError::Execution(format!("Failed to initialize secret manager: {}", e))
                })?,
        )
    } else {
        Some(SecretManager::default().await.map_err(|e| {
            ExecutionError::Execution(format!(
                "Failed to initialize default secret manager: {}",
                e
            ))
        })?)
    };

    let secret_masker = SecretMasker::new();

    // 6. Execute jobs according to the plan
    let mut results = Vec::new();
    let mut has_failures = false;
    let mut failure_details = String::new();

    for job_batch in execution_plan {
        // Execute jobs in parallel if they don't depend on each other
        let job_results = execute_job_batch(
            &job_batch,
            &workflow,
            runtime.as_ref(),
            &env_context,
            config.verbose,
            secret_manager.as_ref(),
            Some(&secret_masker),
        )
        .await?;

        // Check for job failures and collect details
        for job_result in &job_results {
            if job_result.status == JobStatus::Failure {
                has_failures = true;
                failure_details.push_str(&format!("\n❌ Job failed: {}\n", job_result.name));

                // Add step details for failed jobs
                for step in &job_result.steps {
                    if step.status == StepStatus::Failure {
                        failure_details.push_str(&format!("  ❌ {}: {}\n", step.name, step.output));
                    }
                }
            }
        }

        results.extend(job_results);
    }

    // If there were failures, add detailed failure information to the result
    if has_failures {
        wrkflw_logging::error(&format!("Workflow execution failed:{}", failure_details));
    }

    Ok(ExecutionResult {
        jobs: results,
        failure_details: if has_failures {
            Some(failure_details)
        } else {
            None
        },
    })
}

/// Execute a GitLab CI/CD pipeline locally
async fn execute_gitlab_pipeline(
    pipeline_path: &Path,
    config: ExecutionConfig,
) -> Result<ExecutionResult, ExecutionError> {
    wrkflw_logging::info("Executing GitLab CI/CD pipeline");

    // 1. Parse the GitLab pipeline file
    let pipeline = parse_pipeline(pipeline_path)
        .map_err(|e| ExecutionError::Parse(format!("Failed to parse GitLab pipeline: {}", e)))?;

    // 2. Convert the GitLab pipeline to a format compatible with the workflow executor
    let workflow = gitlab::convert_to_workflow_format(&pipeline);

    // 3. Resolve job dependencies based on stages
    let execution_plan = resolve_gitlab_dependencies(&pipeline, &workflow)?;

    // 4. Initialize appropriate runtime
    let runtime = initialize_runtime(
        config.runtime_type.clone(),
        config.preserve_containers_on_failure,
    )?;

    // Create a temporary workspace directory
    let workspace_dir = tempfile::tempdir()
        .map_err(|e| ExecutionError::Execution(format!("Failed to create workspace: {}", e)))?;

    // 5. Set up GitLab-like environment
    let mut env_context = create_gitlab_context(&pipeline, workspace_dir.path());

    // Add runtime mode to environment
    env_context.insert(
        "WRKFLW_RUNTIME_MODE".to_string(),
        match config.runtime_type {
            RuntimeType::Emulation => "emulation".to_string(),
            RuntimeType::SecureEmulation => "secure_emulation".to_string(),
            RuntimeType::Docker => "docker".to_string(),
            RuntimeType::Podman => "podman".to_string(),
        },
    );

    // Setup environment files
    environment::setup_github_environment_files(workspace_dir.path()).map_err(|e| {
        ExecutionError::Execution(format!("Failed to setup environment files: {}", e))
    })?;

    // 6. Initialize secrets management
    let secret_manager = if let Some(secrets_config) = &config.secrets_config {
        Some(
            SecretManager::new(secrets_config.clone())
                .await
                .map_err(|e| {
                    ExecutionError::Execution(format!("Failed to initialize secret manager: {}", e))
                })?,
        )
    } else {
        Some(SecretManager::default().await.map_err(|e| {
            ExecutionError::Execution(format!(
                "Failed to initialize default secret manager: {}",
                e
            ))
        })?)
    };

    let secret_masker = SecretMasker::new();

    // 7. Execute jobs according to the plan
    let mut results = Vec::new();
    let mut has_failures = false;
    let mut failure_details = String::new();

    for job_batch in execution_plan {
        // Execute jobs in parallel if they don't depend on each other
        let job_results = execute_job_batch(
            &job_batch,
            &workflow,
            runtime.as_ref(),
            &env_context,
            config.verbose,
            secret_manager.as_ref(),
            Some(&secret_masker),
        )
        .await?;

        // Check for job failures and collect details
        for job_result in &job_results {
            if job_result.status == JobStatus::Failure {
                has_failures = true;
                failure_details.push_str(&format!("\n❌ Job failed: {}\n", job_result.name));

                // Add step details for failed jobs
                for step in &job_result.steps {
                    if step.status == StepStatus::Failure {
                        failure_details.push_str(&format!("  ❌ {}: {}\n", step.name, step.output));
                    }
                }
            }
        }

        results.extend(job_results);
    }

    // If there were failures, add detailed failure information to the result
    if has_failures {
        wrkflw_logging::error(&format!("Pipeline execution failed:{}", failure_details));
    }

    Ok(ExecutionResult {
        jobs: results,
        failure_details: if has_failures {
            Some(failure_details)
        } else {
            None
        },
    })
}

/// Create an environment context for GitLab CI/CD pipeline execution
fn create_gitlab_context(pipeline: &Pipeline, workspace_dir: &Path) -> HashMap<String, String> {
    let mut env_context = HashMap::new();

    // Add GitLab CI/CD environment variables
    env_context.insert("CI".to_string(), "true".to_string());
    env_context.insert("GITLAB_CI".to_string(), "true".to_string());

    // Add custom environment variable to indicate use in wrkflw
    env_context.insert("WRKFLW_CI".to_string(), "true".to_string());

    // Add workspace directory
    env_context.insert(
        "CI_PROJECT_DIR".to_string(),
        workspace_dir.to_string_lossy().to_string(),
    );

    // Also add the workspace as the GitHub workspace for compatibility with emulation runtime
    env_context.insert(
        "GITHUB_WORKSPACE".to_string(),
        workspace_dir.to_string_lossy().to_string(),
    );

    // Add global variables from the pipeline
    if let Some(variables) = &pipeline.variables {
        for (key, value) in variables {
            env_context.insert(key.clone(), value.clone());
        }
    }

    env_context
}

/// Resolve GitLab CI/CD pipeline dependencies
fn resolve_gitlab_dependencies(
    pipeline: &Pipeline,
    workflow: &WorkflowDefinition,
) -> Result<Vec<Vec<String>>, ExecutionError> {
    // For GitLab CI/CD pipelines, jobs within the same stage can run in parallel,
    // but jobs in different stages run sequentially

    // Get stages from the pipeline or create a default one
    let stages = match &pipeline.stages {
        Some(defined_stages) => defined_stages.clone(),
        None => vec![
            "build".to_string(),
            "test".to_string(),
            "deploy".to_string(),
        ],
    };

    // Create an execution plan based on stages
    let mut execution_plan = Vec::new();

    // For each stage, collect the jobs that belong to it
    for stage in stages {
        let mut stage_jobs = Vec::new();

        for (job_name, job) in &pipeline.jobs {
            // Skip template jobs
            if let Some(true) = job.template {
                continue;
            }

            // Get the job's stage, or assume "test" if not specified
            let default_stage = "test".to_string();
            let job_stage = job.stage.as_ref().unwrap_or(&default_stage);

            // If the job belongs to the current stage, add it to the batch
            if job_stage == &stage {
                stage_jobs.push(job_name.clone());
            }
        }

        if !stage_jobs.is_empty() {
            execution_plan.push(stage_jobs);
        }
    }

    // Also create a batch for jobs without a stage
    let mut stageless_jobs = Vec::new();

    for (job_name, job) in &pipeline.jobs {
        // Skip template jobs
        if let Some(true) = job.template {
            continue;
        }

        if job.stage.is_none() {
            stageless_jobs.push(job_name.clone());
        }
    }

    if !stageless_jobs.is_empty() {
        execution_plan.push(stageless_jobs);
    }

    Ok(execution_plan)
}

// Determine if Docker/Podman is available or fall back to emulation
fn initialize_runtime(
    runtime_type: RuntimeType,
    preserve_containers_on_failure: bool,
) -> Result<Box<dyn ContainerRuntime>, ExecutionError> {
    match runtime_type {
        RuntimeType::Docker => {
            if docker::is_available() {
                // Handle the Result returned by DockerRuntime::new()
                match docker::DockerRuntime::new_with_config(preserve_containers_on_failure) {
                    Ok(docker_runtime) => Ok(Box::new(docker_runtime)),
                    Err(e) => {
                        wrkflw_logging::error(&format!(
                            "Failed to initialize Docker runtime: {}, falling back to emulation mode",
                            e
                        ));
                        Ok(Box::new(emulation::EmulationRuntime::new()))
                    }
                }
            } else {
                wrkflw_logging::error("Docker not available, falling back to emulation mode");
                Ok(Box::new(emulation::EmulationRuntime::new()))
            }
        }
        RuntimeType::Podman => {
            if podman::is_available() {
                // Handle the Result returned by PodmanRuntime::new()
                match podman::PodmanRuntime::new_with_config(preserve_containers_on_failure) {
                    Ok(podman_runtime) => Ok(Box::new(podman_runtime)),
                    Err(e) => {
                        wrkflw_logging::error(&format!(
                            "Failed to initialize Podman runtime: {}, falling back to emulation mode",
                            e
                        ));
                        Ok(Box::new(emulation::EmulationRuntime::new()))
                    }
                }
            } else {
                wrkflw_logging::error("Podman not available, falling back to emulation mode");
                Ok(Box::new(emulation::EmulationRuntime::new()))
            }
        }
        RuntimeType::Emulation => Ok(Box::new(emulation::EmulationRuntime::new())),
        RuntimeType::SecureEmulation => Ok(Box::new(
            wrkflw_runtime::secure_emulation::SecureEmulationRuntime::new(),
        )),
    }
}

#[derive(Debug, Clone, PartialEq)]
pub enum RuntimeType {
    Docker,
    Podman,
    Emulation,
    SecureEmulation,
}

#[derive(Debug, Clone)]
pub struct ExecutionConfig {
    pub runtime_type: RuntimeType,
    pub verbose: bool,
    pub preserve_containers_on_failure: bool,
    pub secrets_config: Option<SecretConfig>,
    pub show_action_messages: bool,
}

pub struct ExecutionResult {
    pub jobs: Vec<JobResult>,
    pub failure_details: Option<String>,
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

/// The result of preparing an action — either a Docker image to run or a composite action.
enum PreparedAction {
    /// A Docker image name to pull and run the action in.
    Image(String),
    /// A composite action that needs special step-based execution.
    Composite,
}

// Add Action preparation functions
async fn prepare_action(
    action: &ActionInfo,
    runtime: &dyn ContainerRuntime,
) -> Result<PreparedAction, ExecutionError> {
    if action.is_docker {
        // Docker action: pull the image
        let image = action.repository.trim_start_matches("docker://");

        runtime
            .pull_image(image)
            .await
            .map_err(|e| ExecutionError::Runtime(format!("Failed to pull Docker image: {}", e)))?;

        return Ok(PreparedAction::Image(image.to_string()));
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

            return Ok(PreparedAction::Image(tag));
        } else {
            // It's a JavaScript or composite action
            // For simplicity, we'll use node to run it (this would need more work for full support)
            return Ok(PreparedAction::Image("node:20-slim".to_string()));
        }
    }

    // GitHub action: try to fetch action.yml from the remote repository
    if !action.repository.is_empty() && !action.version.is_empty() {
        match action_resolver::resolve_remote_action(
            &action.repository,
            &action.version,
            action.sub_path.as_deref(),
        )
        .await
        {
            Ok(resolved) => match &resolved.action_type {
                action_resolver::ActionType::Node { version } => {
                    let image = format!("node:{}-slim", version);
                    wrkflw_logging::info(&format!(
                        "Resolved action '{}' -> image '{}'",
                        action.repository, image
                    ));
                    return Ok(PreparedAction::Image(image));
                }
                action_resolver::ActionType::Docker { image } => {
                    wrkflw_logging::info(&format!(
                        "Resolved action '{}' -> image '{}'",
                        action.repository, image
                    ));
                    return Ok(PreparedAction::Image(image.clone()));
                }
                action_resolver::ActionType::Composite => {
                    wrkflw_logging::info(&format!(
                        "Resolved action '{}' as composite action",
                        action.repository
                    ));
                    return Ok(PreparedAction::Composite);
                }
                action_resolver::ActionType::DockerBuild => {
                    return Err(ExecutionError::Execution(format!(
                        "Action '{}' bundles its own Dockerfile (runs.image = Dockerfile). \
                             Building remote Dockerfiles is not yet supported.",
                        action.repository
                    )));
                }
            },
            Err(e) => {
                wrkflw_logging::warning(&format!(
                    "Could not fetch action.yml for {}@{}: {}. Falling back to built-in mapping.",
                    action.repository, action.version, e
                ));
            }
        }
    }

    // Fallback: determine appropriate image based on hardcoded action type mapping
    let image = determine_action_image(&action.repository);
    Ok(PreparedAction::Image(image))
}

/// Shallow-clone a GitHub repository at a specific ref (branch, tag, or SHA).
///
/// For branch/tag refs, uses `git clone --depth 1 --branch <ref>`.
/// For SHA refs (40 hex chars), uses `git init` + `git fetch --depth 1` + `git checkout`.
///
/// Uses `tokio::process::Command` to avoid blocking the async runtime.
async fn shallow_clone(
    repo_url: &str,
    git_ref: &str,
    target_dir: &Path,
) -> Result<(), ExecutionError> {
    let is_sha = is_git_sha(git_ref);

    if is_sha {
        // SHA refs can't use --branch; use init + fetch + checkout instead
        let init = tokio::process::Command::new("git")
            .arg("init")
            .arg(target_dir)
            .stdout(std::process::Stdio::null())
            .stderr(std::process::Stdio::null())
            .status()
            .await
            .map_err(|e| ExecutionError::Execution(format!("Failed to execute git init: {}", e)))?;
        if !init.success() {
            return Err(ExecutionError::Execution(format!(
                "git init failed for {}",
                target_dir.display()
            )));
        }

        let fetch = tokio::process::Command::new("git")
            .arg("-C")
            .arg(target_dir)
            .arg("fetch")
            .arg("--depth")
            .arg("1")
            .arg("--")
            .arg(repo_url)
            .arg(git_ref)
            .stdout(std::process::Stdio::null())
            .stderr(std::process::Stdio::piped())
            .output()
            .await
            .map_err(|e| {
                ExecutionError::Execution(format!("Failed to execute git fetch: {}", e))
            })?;
        if !fetch.status.success() {
            let stderr = String::from_utf8_lossy(&fetch.stderr);
            return Err(ExecutionError::Execution(format!(
                "Failed to fetch {}@{}: {}",
                repo_url,
                git_ref,
                stderr.trim()
            )));
        }

        let checkout = tokio::process::Command::new("git")
            .arg("-C")
            .arg(target_dir)
            .arg("checkout")
            .arg("FETCH_HEAD")
            .stdout(std::process::Stdio::null())
            .stderr(std::process::Stdio::piped())
            .output()
            .await
            .map_err(|e| {
                ExecutionError::Execution(format!("Failed to execute git checkout: {}", e))
            })?;
        if !checkout.status.success() {
            let stderr = String::from_utf8_lossy(&checkout.stderr);
            return Err(ExecutionError::Execution(format!(
                "Failed to checkout FETCH_HEAD for {}@{}: {}",
                repo_url,
                git_ref,
                stderr.trim()
            )));
        }
    } else {
        // Branch/tag refs: standard shallow clone
        let output = tokio::process::Command::new("git")
            .arg("clone")
            .arg("--depth")
            .arg("1")
            .arg("--single-branch")
            .arg("--branch")
            .arg(git_ref)
            .arg("--")
            .arg(repo_url)
            .arg(target_dir)
            .stdout(std::process::Stdio::null())
            .stderr(std::process::Stdio::piped())
            .output()
            .await
            .map_err(|e| ExecutionError::Execution(format!("Failed to execute git: {}", e)))?;
        if !output.status.success() {
            let stderr = String::from_utf8_lossy(&output.stderr);
            return Err(ExecutionError::Execution(format!(
                "Failed to clone {}@{}: {}",
                repo_url,
                git_ref,
                stderr.trim()
            )));
        }
    }

    Ok(())
}

/// Returns `true` if `git_ref` looks like a full SHA-1 hex hash (40 hex chars).
///
/// NOTE: This only detects SHA-1 (40 hex chars). Git's SHA-256 transition uses
/// 64-char hashes — update this check if/when GitHub adopts SHA-256 refs.
fn is_git_sha(git_ref: &str) -> bool {
    git_ref.len() == 40 && git_ref.chars().all(|c| c.is_ascii_hexdigit())
}

/// Determine the appropriate Docker image for a GitHub action
fn determine_action_image(repository: &str) -> String {
    // Handle specific well-known actions
    match repository {
        // PHP setup actions
        repo if repo.starts_with("shivammathur/setup-php") => {
            "composer:latest".to_string() // Use composer image which includes PHP and composer
        }

        // Python setup actions
        repo if repo.starts_with("actions/setup-python") => "python:3.11-slim".to_string(),

        // Node.js setup actions
        repo if repo.starts_with("actions/setup-node") => "node:20-slim".to_string(),

        // Java setup actions
        repo if repo.starts_with("actions/setup-java") => "eclipse-temurin:17-jdk".to_string(),

        // Go setup actions
        repo if repo.starts_with("actions/setup-go") => "golang:1.21-slim".to_string(),

        // .NET setup actions
        repo if repo.starts_with("actions/setup-dotnet") => {
            "mcr.microsoft.com/dotnet/sdk:7.0".to_string()
        }

        // Rust setup actions
        repo if repo.starts_with("actions-rs/toolchain")
            || repo.starts_with("dtolnay/rust-toolchain") =>
        {
            "rust:latest".to_string()
        }

        // Docker/container actions
        repo if repo.starts_with("docker/") => "docker:latest".to_string(),

        // AWS actions
        repo if repo.starts_with("aws-actions/") => "amazon/aws-cli:latest".to_string(),

        // Default to Node.js for most GitHub actions (checkout, upload-artifact, etc.)
        _ => {
            // Check if it's a common core GitHub action that should use a more complete environment
            if repository.starts_with("actions/checkout")
                || repository.starts_with("actions/upload-artifact")
                || repository.starts_with("actions/download-artifact")
                || repository.starts_with("actions/cache")
            {
                "catthehacker/ubuntu:act-latest".to_string() // Use act runner image for core actions
            } else {
                "node:20-slim".to_string() // Default for other actions
            }
        }
    }
}

async fn execute_job_batch(
    jobs: &[String],
    workflow: &WorkflowDefinition,
    runtime: &dyn ContainerRuntime,
    env_context: &HashMap<String, String>,
    verbose: bool,
    secret_manager: Option<&SecretManager>,
    secret_masker: Option<&SecretMasker>,
) -> Result<Vec<JobResult>, ExecutionError> {
    // Execute jobs in parallel
    let futures = jobs.iter().map(|job_name| {
        execute_job_with_matrix(
            job_name,
            workflow,
            runtime,
            env_context,
            verbose,
            secret_manager,
            secret_masker,
        )
    });

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
    secret_manager: Option<&'a SecretManager>,
    secret_masker: Option<&'a SecretMasker>,
}

/// Execute a job, expanding matrix if present
async fn execute_job_with_matrix(
    job_name: &str,
    workflow: &WorkflowDefinition,
    runtime: &dyn ContainerRuntime,
    env_context: &HashMap<String, String>,
    verbose: bool,
    secret_manager: Option<&SecretManager>,
    secret_masker: Option<&SecretMasker>,
) -> Result<Vec<JobResult>, ExecutionError> {
    // Get the job definition
    let job = workflow.jobs.get(job_name).ok_or_else(|| {
        ExecutionError::Execution(format!("Job '{}' not found in workflow", job_name))
    })?;

    // Evaluate job condition if present
    if let Some(if_condition) = &job.if_condition {
        let should_run = evaluate_job_condition(if_condition, env_context, workflow);
        if !should_run {
            wrkflw_logging::info(&format!(
                "⏭️ Skipping job '{}' due to condition: {}",
                job_name, if_condition
            ));
            // Return a skipped job result
            return Ok(vec![JobResult {
                name: job_name.to_string(),
                status: JobStatus::Skipped,
                steps: Vec::new(),
                logs: String::new(),
            }]);
        }
    }

    // Check if this is a matrix job
    if let Some(matrix_config) = job.matrix_config() {
        // Expand the matrix into combinations
        let combinations = wrkflw_matrix::expand_matrix(matrix_config)
            .map_err(|e| ExecutionError::Execution(format!("Failed to expand matrix: {}", e)))?;

        if combinations.is_empty() {
            wrkflw_logging::info(&format!(
                "Matrix job '{}' has no valid combinations",
                job_name
            ));
            // Return empty result for jobs with no valid combinations
            return Ok(Vec::new());
        }

        wrkflw_logging::info(&format!(
            "Matrix job '{}' expanded to {} combinations",
            job_name,
            combinations.len()
        ));

        // Set maximum parallel jobs
        let max_parallel = job.max_parallel().unwrap_or_else(|| {
            // If not specified, use a reasonable default based on CPU cores
            std::cmp::max(1, num_cpus::get())
        });

        // Execute matrix combinations
        execute_matrix_combinations(MatrixExecutionContext {
            job_name,
            job_template: job,
            combinations: &combinations,
            max_parallel,
            fail_fast: job.fail_fast(),
            workflow,
            runtime,
            env_context,
            verbose,
            secret_manager,
            secret_masker,
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
            secret_manager,
            secret_masker,
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

    // Handle reusable workflow jobs (job-level 'uses')
    if let Some(uses) = &job.uses {
        return execute_reusable_workflow_job(&ctx, uses, job.with.as_ref(), job.secrets.as_ref())
            .await;
    }

    // Clone context and add job-specific variables
    let mut job_env = ctx.env_context.clone();

    // Add container-level environment variables (lowest precedence)
    if let Some(ref container) = job.container {
        warn_unsupported_container_fields(container);
        for (key, value) in &container.env {
            job_env.entry(key.clone()).or_insert_with(|| value.clone());
        }
    }

    // Add job-level environment variables (overrides container env)
    for (key, value) in &job.env {
        job_env.insert(key.clone(), value.clone());
    }

    // Execute job steps
    let mut step_results = Vec::new();
    let mut job_logs = String::new();

    // Create a temporary directory for this job execution
    let job_dir = tempfile::tempdir()
        .map_err(|e| ExecutionError::Execution(format!("Failed to create job directory: {}", e)))?;

    // Get the current project directory
    let current_dir = std::env::current_dir().map_err(|e| {
        ExecutionError::Execution(format!("Failed to get current directory: {}", e))
    })?;

    wrkflw_logging::info(&format!("Executing job: {}", ctx.job_name));

    let mut job_success = true;

    // Execute job steps
    // Determine runner image: prefer job container image, fall back to runs-on mapping
    let runner_image_value = get_effective_runner_image(job);

    for (idx, step) in job.steps.iter().enumerate() {
        let outcome = run_step_with_guards(
            step,
            idx,
            &job_env,
            ctx.workflow,
            StepExecutionContext {
                step,
                step_idx: idx,
                job_env: &job_env,
                working_dir: job_dir.path(),
                runtime: ctx.runtime,
                workflow: ctx.workflow,
                runner_image: &runner_image_value,
                verbose: ctx.verbose,
                matrix_combination: &None,
                secret_manager: ctx.secret_manager,
                secret_masker: ctx.secret_masker,
                container_config: job.container.as_ref(),
            },
        )
        .await?;

        match outcome {
            StepOutcome::Skipped(result) => {
                step_results.push(result);
            }
            StepOutcome::Completed { result, abort_job } => {
                // Add step output to logs only in verbose mode or if there's an error
                if ctx.verbose || result.status == StepStatus::Failure {
                    job_logs.push_str(&format!(
                        "\n=== Output from step '{}' ===\n{}\n=== End output ===\n\n",
                        result.name, result.output
                    ));
                } else {
                    job_logs.push_str(&format!(
                        "Step '{}' completed with status: {:?}\n",
                        result.name, result.status
                    ));
                }

                step_results.push(result);

                if abort_job {
                    job_success = false;
                    break;
                }
            }
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
    #[allow(dead_code)] // Planned for future implementation
    secret_manager: Option<&'a SecretManager>,
    #[allow(dead_code)] // Planned for future implementation
    secret_masker: Option<&'a SecretMasker>,
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
                let combination_name =
                    wrkflw_matrix::format_combination_name(ctx.job_name, combination);
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
                    wrkflw_logging::error(&format!("Matrix job failed: {}", e));

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
    let matrix_job_name = wrkflw_matrix::format_combination_name(job_name, combination);

    wrkflw_logging::info(&format!("Executing matrix job: {}", matrix_job_name));

    // Clone the environment and add matrix-specific values
    let mut job_env = base_env_context.clone();
    environment::add_matrix_context(&mut job_env, combination);

    // Add container-level environment variables (lowest precedence)
    if let Some(ref container) = job_template.container {
        warn_unsupported_container_fields(container);
        for (key, value) in &container.env {
            job_env.entry(key.clone()).or_insert_with(|| value.clone());
        }
    }

    // Add job-level environment variables (overrides container env)
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

    // Get the current project directory
    let current_dir = std::env::current_dir().map_err(|e| {
        ExecutionError::Execution(format!("Failed to get current directory: {}", e))
    })?;

    let job_success = if job_template.steps.is_empty() {
        wrkflw_logging::warning(&format!("Job '{}' has no steps", matrix_job_name));
        true
    } else {
        // Execute each step
        // Determine runner image: prefer job container image, fall back to runs-on mapping
        let runner_image_value = get_effective_runner_image(job_template);

        let mut all_steps_ok = true;
        for (idx, step) in job_template.steps.iter().enumerate() {
            let outcome = run_step_with_guards(
                step,
                idx,
                &job_env,
                workflow,
                StepExecutionContext {
                    step,
                    step_idx: idx,
                    job_env: &job_env,
                    working_dir: job_dir.path(),
                    runtime,
                    workflow,
                    runner_image: &runner_image_value,
                    verbose,
                    matrix_combination: &Some(combination.values.clone()),
                    secret_manager: None,
                    secret_masker: None,
                    container_config: job_template.container.as_ref(),
                },
            )
            .await?;

            match outcome {
                StepOutcome::Skipped(result) => {
                    step_results.push(result);
                }
                StepOutcome::Completed { result, abort_job } => {
                    job_logs.push_str(&format!("Step: {}\n", result.name));
                    job_logs.push_str(&format!("Status: {:?}\n", result.status));

                    if verbose || result.status == StepStatus::Failure {
                        job_logs.push_str(&result.output);
                        job_logs.push_str("\n\n");
                    } else {
                        job_logs.push('\n');
                        job_logs.push('\n');
                    }

                    step_results.push(result);

                    if abort_job {
                        all_steps_ok = false;
                        break;
                    }
                }
            }
        }

        all_steps_ok
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

/// Outcome of a single step after guards (if-condition, continue-on-error) are applied.
enum StepOutcome {
    /// Step ran (or was skipped). Contains the result and whether the job should abort.
    Completed { result: StepResult, abort_job: bool },
    /// Step was skipped due to an if-condition.
    Skipped(StepResult),
}

/// Run a step with if-condition and continue-on-error guards.
/// Returns the step result and whether the job should be aborted.
async fn run_step_with_guards(
    step: &Step,
    step_idx: usize,
    job_env: &HashMap<String, String>,
    workflow: &WorkflowDefinition,
    step_exec_ctx: StepExecutionContext<'_>,
) -> Result<StepOutcome, ExecutionError> {
    let step_name = step
        .name
        .clone()
        .unwrap_or_else(|| format!("Step {}", step_idx + 1));

    // Check step-level if condition
    if let Some(if_cond) = &step.if_condition {
        let should_run = evaluate_job_condition(if_cond, job_env, workflow);
        if !should_run {
            wrkflw_logging::info(&format!(
                "  ⏭️ Skipping step '{}' due to condition: {}",
                step_name, if_cond
            ));
            return Ok(StepOutcome::Skipped(StepResult {
                name: step_name,
                status: StepStatus::Skipped,
                output: format!("Skipped due to condition: {}", if_cond),
            }));
        }
    }

    match execute_step(step_exec_ctx).await {
        Ok(result) => {
            let abort_job = if result.status == StepStatus::Failure {
                if step.continue_on_error == Some(true) {
                    wrkflw_logging::info(&format!(
                        "  Step '{}' failed but continue-on-error is set, continuing",
                        result.name
                    ));
                    false
                } else {
                    true
                }
            } else {
                false
            };
            Ok(StepOutcome::Completed { result, abort_job })
        }
        Err(e) => {
            if step.continue_on_error == Some(true) {
                wrkflw_logging::info(&format!(
                    "  Step '{}' errored but continue-on-error is set, continuing",
                    step_name
                ));
                Ok(StepOutcome::Completed {
                    result: StepResult {
                        name: step_name,
                        status: StepStatus::Failure,
                        output: format!("Error: {}", e),
                    },
                    abort_job: false,
                })
            } else {
                Ok(StepOutcome::Completed {
                    result: StepResult {
                        name: step_name,
                        status: StepStatus::Failure,
                        output: format!("Error: {}", e),
                    },
                    abort_job: true,
                })
            }
        }
    }
}

// Before the execute_step function, add this struct
struct StepExecutionContext<'a> {
    step: &'a workflow::Step,
    step_idx: usize,
    job_env: &'a HashMap<String, String>,
    working_dir: &'a Path,
    runtime: &'a dyn ContainerRuntime,
    workflow: &'a WorkflowDefinition,
    runner_image: &'a str,
    verbose: bool,
    #[allow(dead_code)]
    matrix_combination: &'a Option<HashMap<String, Value>>,
    secret_manager: Option<&'a SecretManager>,
    #[allow(dead_code)] // Planned for future implementation
    secret_masker: Option<&'a SecretMasker>,
    container_config: Option<&'a JobContainer>,
}

async fn execute_step(ctx: StepExecutionContext<'_>) -> Result<StepResult, ExecutionError> {
    let step_name = ctx
        .step
        .name
        .clone()
        .unwrap_or_else(|| format!("Step {}", ctx.step_idx + 1));

    if ctx.verbose {
        wrkflw_logging::info(&format!("  Executing step: {}", step_name));
    }

    // Prepare step environment
    let mut step_env = ctx.job_env.clone();

    // Add step-level environment variables (with secret substitution)
    for (key, value) in &ctx.step.env {
        let resolved_value = if let Some(secret_manager) = ctx.secret_manager {
            let mut substitution = SecretSubstitution::new(secret_manager);
            match substitution.substitute(value).await {
                Ok(resolved) => resolved,
                Err(e) => {
                    wrkflw_logging::error(&format!(
                        "Failed to resolve secrets in environment variable {}: {}",
                        key, e
                    ));
                    value.clone()
                }
            }
        } else {
            value.clone()
        };
        step_env.insert(key.clone(), resolved_value);
    }

    // Execute the step based on its type
    let step_result = if let Some(uses) = &ctx.step.uses {
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
            let output = if ctx.verbose {
                let mut detailed_output =
                    "Emulated checkout: Copied current directory to workspace\n\n".to_string();

                // Add checkout action details
                detailed_output.push_str("Checkout Details:\n");
                detailed_output.push_str("  - Source: Local directory\n");
                detailed_output
                    .push_str(&format!("  - Destination: {}\n", ctx.working_dir.display()));

                // Add a summary count instead of listing all files
                if let Ok(entries) = std::fs::read_dir(&current_dir) {
                    let entry_count = entries.count();
                    detailed_output.push_str(&format!(
                        "\nCopied {} top-level items to workspace\n",
                        entry_count
                    ));
                }

                detailed_output
            } else {
                "Emulated checkout: Copied current directory to workspace".to_string()
            };

            if ctx.verbose {
                println!("  Emulated actions/checkout: copied project files to workspace");
            }

            StepResult {
                name: step_name,
                status: StepStatus::Success,
                output,
            }
        } else {
            // Get action info
            let prepared = prepare_action(&action_info, ctx.runtime).await?;

            match prepared {
                PreparedAction::Composite => {
                    if action_info.is_local {
                        // Handle local composite action
                        let action_path = Path::new(&action_info.repository);
                        execute_composite_action(
                            ctx.step,
                            action_path,
                            &step_env,
                            ctx.working_dir,
                            ctx.runtime,
                            ctx.runner_image,
                            ctx.verbose,
                        )
                        .await?
                    } else {
                        // Handle remote composite action: clone the repo and execute
                        let tempdir = tempfile::tempdir().map_err(|e| {
                            ExecutionError::Execution(format!("Failed to create temp dir: {}", e))
                        })?;
                        let repo_url = format!("https://github.com/{}.git", action_info.repository);
                        let repo_dir = tempdir.path().join("action");
                        shallow_clone(&repo_url, &action_info.version, &repo_dir).await?;
                        // If the action has a sub-path, the action.yml is inside that directory
                        let action_dir = match &action_info.sub_path {
                            Some(p) => repo_dir.join(p),
                            None => repo_dir,
                        };
                        // tempdir must stay alive until execute_composite_action completes
                        execute_composite_action(
                            ctx.step,
                            &action_dir,
                            &step_env,
                            ctx.working_dir,
                            ctx.runtime,
                            ctx.runner_image,
                            ctx.verbose,
                        )
                        .await?
                    }
                }
                PreparedAction::Image(image) => {
                    // Build command for Docker action
                    let mut cmd = Vec::new();
                    let mut owned_strings: Vec<String> = Vec::new(); // Keep strings alive until after we use cmd

                    // Special handling for Rust actions
                    if uses.starts_with("actions-rs/") || uses.starts_with("dtolnay/rust-toolchain")
                    {
                        wrkflw_logging::info(
                            "🔄 Detected Rust action - using system Rust installation",
                        );

                        // For toolchain action, verify Rust is installed
                        if uses.starts_with("actions-rs/toolchain@")
                            || uses.starts_with("dtolnay/rust-toolchain@")
                        {
                            let rustc_version = Command::new("rustc")
                                .arg("--version")
                                .output()
                                .map(|output| String::from_utf8_lossy(&output.stdout).to_string())
                                .unwrap_or_else(|_| "not found".to_string());

                            wrkflw_logging::info(&format!(
                                "🔄 Using system Rust: {}",
                                rustc_version.trim()
                            ));

                            // Return success since we're using system Rust
                            return Ok(StepResult {
                                name: step_name,
                                status: StepStatus::Success,
                                output: format!("Using system Rust: {}", rustc_version.trim()),
                            });
                        }

                        // For cargo action, execute cargo commands directly
                        if uses.starts_with("actions-rs/cargo@") {
                            let cargo_version = Command::new("cargo")
                                .arg("--version")
                                .output()
                                .map(|output| String::from_utf8_lossy(&output.stdout).to_string())
                                .unwrap_or_else(|_| "not found".to_string());

                            wrkflw_logging::info(&format!(
                                "🔄 Using system Rust/Cargo: {}",
                                cargo_version.trim()
                            ));

                            // Get the command from the 'with' parameters
                            if let Some(with_params) = &ctx.step.with {
                                if let Some(command) = with_params.get("command") {
                                    wrkflw_logging::info(&format!(
                                        "🔄 Found command parameter: {}",
                                        command
                                    ));

                                    // Build the actual command
                                    let mut real_command = format!("cargo {}", command);

                                    // Add any arguments if specified
                                    if let Some(args) = with_params.get("args") {
                                        if !args.is_empty() {
                                            // Resolve GitHub-style matrix variables in args
                                            let resolved_args =
                                                crate::substitution::process_step_run(
                                                    args,
                                                    ctx.matrix_combination,
                                                )
                                                .trim()
                                                .to_string();

                                            // Only add if we have something left after resolving variables
                                            // and it's not just "--target" without a value
                                            if !resolved_args.is_empty()
                                                && resolved_args != "--target"
                                            {
                                                real_command
                                                    .push_str(&format!(" {}", resolved_args));
                                            }
                                        }
                                    }

                                    wrkflw_logging::info(&format!(
                                        "🔄 Running actual command: {}",
                                        real_command
                                    ));

                                    // Execute the command
                                    let mut cmd = Command::new("sh");
                                    cmd.arg("-c");
                                    cmd.arg(&real_command);
                                    cmd.current_dir(ctx.working_dir);

                                    // Add environment variables
                                    for (key, value) in step_env {
                                        cmd.env(key, value);
                                    }

                                    match cmd.output() {
                                        Ok(output) => {
                                            let exit_code = output.status.code().unwrap_or(-1);
                                            let stdout =
                                                String::from_utf8_lossy(&output.stdout).to_string();
                                            let stderr =
                                                String::from_utf8_lossy(&output.stderr).to_string();

                                            return Ok(StepResult {
                                                name: step_name,
                                                status: if exit_code == 0 {
                                                    StepStatus::Success
                                                } else {
                                                    StepStatus::Failure
                                                },
                                                output: format!("{}\n{}", stdout, stderr),
                                            });
                                        }
                                        Err(e) => {
                                            return Ok(StepResult {
                                                name: step_name,
                                                status: StepStatus::Failure,
                                                output: format!("Failed to execute command: {}", e),
                                            });
                                        }
                                    }
                                }
                            }
                        }
                    }

                    if action_info.is_docker {
                        // Docker actions just run the container
                        cmd.push("sh");
                        cmd.push("-c");
                        cmd.push("echo 'Executing Docker action'");
                    } else if action_info.is_local {
                        // Local actions: run a placeholder since full local action
                        // execution is handled by the Composite branch above
                        cmd.push("sh");
                        cmd.push("-c");
                        cmd.push("echo 'Local action executed'");
                    } else {
                        // For GitHub actions, check if we have special handling
                        if let Err(e) = emulation::handle_special_action(uses).await {
                            // Log error but continue
                            println!("   Warning: Special action handling failed: {}", e);
                        }

                        // Check if we should hide GitHub action messages
                        let hide_action_value = ctx
                            .job_env
                            .get("WRKFLW_HIDE_ACTION_MESSAGES")
                            .cloned()
                            .unwrap_or_else(|| "not set".to_string());

                        wrkflw_logging::debug(&format!(
                            "WRKFLW_HIDE_ACTION_MESSAGES value: {}",
                            hide_action_value
                        ));

                        let hide_messages = hide_action_value == "true";
                        wrkflw_logging::debug(&format!("Should hide messages: {}", hide_messages));

                        // Only log a message to the console if we're showing action messages
                        if !hide_messages {
                            // For Emulation mode, log a message about what action would be executed
                            println!("   ⚙️ Would execute GitHub action: {}", uses);
                        }

                        // Extract the actual command from the GitHub action if applicable
                        let mut should_run_real_command = false;
                        let mut real_command_parts = Vec::new();

                        // Check if this action has 'with' parameters that specify a command to run
                        if let Some(with_params) = &ctx.step.with {
                            // Common GitHub action pattern: has a 'command' parameter
                            if let Some(cmd) = with_params.get("command") {
                                if ctx.verbose {
                                    wrkflw_logging::info(&format!(
                                        "🔄 Found command parameter: {}",
                                        cmd
                                    ));
                                }

                                // Convert to real command based on action type patterns
                                if uses.contains("cargo") || uses.contains("rust") {
                                    // Cargo command pattern
                                    real_command_parts.push("cargo".to_string());
                                    real_command_parts.push(cmd.clone());
                                    should_run_real_command = true;
                                } else if uses.contains("node") || uses.contains("npm") {
                                    // Node.js command pattern
                                    if cmd == "npm" || cmd == "yarn" || cmd == "pnpm" {
                                        real_command_parts.push(cmd.clone());
                                    } else {
                                        real_command_parts.push("npm".to_string());
                                        real_command_parts.push("run".to_string());
                                        real_command_parts.push(cmd.clone());
                                    }
                                    should_run_real_command = true;
                                } else if uses.contains("python") || uses.contains("pip") {
                                    // Python command pattern
                                    if cmd == "pip" {
                                        real_command_parts.push("pip".to_string());
                                    } else {
                                        real_command_parts.push("python".to_string());
                                        real_command_parts.push("-m".to_string());
                                        real_command_parts.push(cmd.clone());
                                    }
                                    should_run_real_command = true;
                                } else {
                                    // Generic command - try to execute directly if available
                                    real_command_parts.push(cmd.clone());
                                    should_run_real_command = true;
                                }

                                // Add any arguments if specified
                                if let Some(args) = with_params.get("args") {
                                    if !args.is_empty() {
                                        // Resolve GitHub-style matrix variables in args
                                        let resolved_args = crate::substitution::process_step_run(
                                            args,
                                            ctx.matrix_combination,
                                        )
                                        .trim()
                                        .to_string();

                                        // Only add if we have something left after resolving variables
                                        if !resolved_args.is_empty() {
                                            real_command_parts.push(resolved_args);
                                        }
                                    }
                                }
                            }
                        }

                        if should_run_real_command && !real_command_parts.is_empty() {
                            // Build a final command string
                            let command_str = real_command_parts.join(" ");
                            wrkflw_logging::info(&format!(
                                "🔄 Running actual command: {}",
                                command_str
                            ));

                            // Replace the emulated command with a shell command to execute our command
                            cmd.clear();
                            cmd.push("sh");
                            cmd.push("-c");
                            owned_strings.push(command_str);
                            cmd.push(owned_strings.last().unwrap());
                        } else {
                            // Fall back to emulation for actions we don't know how to execute
                            cmd.clear();
                            cmd.push("sh");
                            cmd.push("-c");

                            let escaped_uses = uses.replace('\'', "'\\''");
                            let echo_msg =
                                format!("echo 'Would execute GitHub action: {}'", escaped_uses);
                            owned_strings.push(echo_msg);
                            cmd.push(owned_strings.last().unwrap());
                        }
                    }

                    // Convert 'with' parameters to environment variables
                    if let Some(with_params) = &ctx.step.with {
                        for (key, value) in with_params {
                            step_env.insert(format!("INPUT_{}", key.to_uppercase()), value.clone());
                        }
                    }

                    // Define the standard workspace path inside the container
                    let container_workspace = Path::new("/github/workspace");

                    // Set up volume mapping from host working dir to container workspace
                    let mut volumes: Vec<(&Path, &Path)> =
                        vec![(ctx.working_dir, container_workspace)];

                    // Prepare container mounts and remap GitHub env paths
                    let (owned_volume_paths, github_mount) =
                        prepare_container_mounts(&mut step_env, ctx.job_env, ctx.container_config);
                    if let Some((ref host, ref container)) = github_mount {
                        volumes.push((host.as_path(), container.as_path()));
                    }
                    for (host, container) in &owned_volume_paths {
                        volumes.push((host.as_path(), container.as_path()));
                    }

                    // Convert environment HashMap to Vec<(&str, &str)> for container runtime
                    let env_vars: Vec<(&str, &str)> = step_env
                        .iter()
                        .map(|(k, v)| (k.as_str(), v.as_str()))
                        .collect();

                    let output = ctx
                        .runtime
                        .run_container(
                            &image,
                            &cmd.to_vec(),
                            &env_vars,
                            container_workspace,
                            &volumes,
                        )
                        .await
                        .map_err(|e| ExecutionError::Runtime(format!("{}", e)))?;

                    // Build verbose output for GitHub actions when applicable
                    let output_text = if ctx.verbose
                        && output.exit_code == 0
                        && uses.contains('/')
                        && !uses.starts_with("./")
                    {
                        let mut detailed_output =
                            format!("Would execute GitHub action: {}\n", uses);

                        // Add information about the action inputs if available
                        if let Some(with_params) = &ctx.step.with {
                            detailed_output.push_str("\nAction inputs:\n");
                            for (key, value) in with_params {
                                detailed_output.push_str(&format!("  {}: {}\n", key, value));
                            }
                        }

                        // Add standard GitHub action environment variables
                        detailed_output.push_str("\nEnvironment variables:\n");
                        for (key, value) in step_env.iter() {
                            if key.starts_with("GITHUB_") || key.starts_with("INPUT_") {
                                detailed_output.push_str(&format!("  {}: {}\n", key, value));
                            }
                        }

                        // Include the original output
                        detailed_output
                            .push_str(&format!("\nOutput:\n{}\n{}", output.stdout, output.stderr));
                        detailed_output
                    } else {
                        format!("{}\n{}", output.stdout, output.stderr)
                    };

                    // Add detailed error information for failed cargo/rust commands
                    if output.exit_code != 0 && (uses.contains("cargo") || uses.contains("rust")) {
                        let mut error_details = format!(
                            "\n\n❌ Command failed with exit code: {}\n",
                            output.exit_code
                        );

                        error_details.push_str(&format!("Command: {}\n", cmd.join(" ")));

                        error_details.push_str("\nEnvironment:\n");
                        for (key, value) in step_env.iter() {
                            if key.starts_with("GITHUB_")
                                || key.starts_with("INPUT_")
                                || key.starts_with("RUST")
                            {
                                error_details.push_str(&format!("  {}: {}\n", key, value));
                            }
                        }

                        error_details.push_str("\nDetailed output:\n");
                        error_details.push_str(&output.stdout);
                        error_details.push_str(&output.stderr);

                        return Ok(StepResult {
                            name: step_name,
                            status: StepStatus::Failure,
                            output: format!("{}\n{}", output_text, error_details),
                        });
                    }

                    StepResult {
                        name: step_name,
                        status: if output.exit_code == 0 {
                            StepStatus::Success
                        } else {
                            StepStatus::Failure
                        },
                        output: format!(
                            "Exit code: {}\n{}\n{}",
                            output.exit_code, output.stdout, output.stderr
                        ),
                    }
                }
            }
        }
    } else if let Some(run) = &ctx.step.run {
        // Run step
        let mut output = String::new();
        let mut status = StepStatus::Success;
        let mut error_details = None;

        // Perform secret substitution if secret manager is available
        let resolved_run = if let Some(secret_manager) = ctx.secret_manager {
            let mut substitution = SecretSubstitution::new(secret_manager);
            match substitution.substitute(run).await {
                Ok(resolved) => resolved,
                Err(e) => {
                    return Ok(StepResult {
                        name: step_name,
                        status: StepStatus::Failure,
                        output: format!("Secret substitution failed: {}", e),
                    });
                }
            }
        } else {
            run.clone()
        };

        // Check if this is a cargo command
        let is_cargo_cmd = resolved_run.trim().starts_with("cargo");

        // For complex shell commands, use bash to execute them properly
        // This handles quotes, pipes, redirections, and command substitutions correctly
        let cmd_parts = vec!["bash", "-c", &resolved_run];

        // Define the standard workspace path inside the container
        let container_workspace = Path::new("/github/workspace");

        // Set up volume mapping from host working dir to container workspace
        let mut volumes: Vec<(&Path, &Path)> = vec![(ctx.working_dir, container_workspace)];

        // Prepare container mounts and remap GitHub env paths
        let (owned_volume_paths, github_mount) =
            prepare_container_mounts(&mut step_env, ctx.job_env, ctx.container_config);
        if let Some((ref host, ref container)) = github_mount {
            volumes.push((host.as_path(), container.as_path()));
        }
        for (host, container) in &owned_volume_paths {
            volumes.push((host.as_path(), container.as_path()));
        }

        // Convert environment variables to the required format (after path remapping)
        let env_vars: Vec<(&str, &str)> = step_env
            .iter()
            .map(|(k, v)| (k.as_str(), v.as_str()))
            .collect();

        // Execute the command
        match ctx
            .runtime
            .run_container(
                ctx.runner_image,
                &cmd_parts,
                &env_vars,
                container_workspace,
                &volumes,
            )
            .await
        {
            Ok(container_output) => {
                // Add command details to output
                output.push_str(&format!("Command: {}\n\n", run));

                if !container_output.stdout.is_empty() {
                    output.push_str("Standard Output:\n");
                    output.push_str(&container_output.stdout);
                    output.push('\n');
                }

                if !container_output.stderr.is_empty() {
                    output.push_str("Standard Error:\n");
                    output.push_str(&container_output.stderr);
                    output.push('\n');
                }

                if container_output.exit_code != 0 {
                    status = StepStatus::Failure;

                    // For cargo commands, add more detailed error information
                    if is_cargo_cmd {
                        let mut error_msg = String::new();
                        error_msg.push_str(&format!(
                            "\nCargo command failed with exit code {}\n",
                            container_output.exit_code
                        ));
                        error_msg.push_str("Common causes for cargo command failures:\n");

                        if run.contains("fmt") {
                            error_msg.push_str(
                                "- Code formatting issues. Run 'cargo fmt' locally to fix.\n",
                            );
                        } else if run.contains("clippy") {
                            error_msg.push_str("- Linter warnings treated as errors. Run 'cargo clippy' locally to see details.\n");
                        } else if run.contains("test") {
                            error_msg.push_str("- Test failures. Run 'cargo test' locally to see which tests failed.\n");
                        } else if run.contains("build") {
                            error_msg.push_str(
                                "- Compilation errors. Check the error messages above.\n",
                            );
                        }

                        error_details = Some(error_msg);
                    }
                }
            }
            Err(e) => {
                status = StepStatus::Failure;
                output.push_str(&format!("Error executing command: {}\n", e));
            }
        }

        // If there are error details, append them to the output
        if let Some(details) = error_details {
            output.push_str(&details);
        }

        StepResult {
            name: step_name,
            status,
            output,
        }
    } else {
        return Ok(StepResult {
            name: step_name,
            status: StepStatus::Skipped,
            output: "Step has neither 'uses' nor 'run'".to_string(),
        });
    };

    Ok(step_result)
}

/// Create a gitignore matcher for the given directory
fn create_gitignore_matcher(
    dir: &Path,
) -> Result<Option<ignore::gitignore::Gitignore>, ExecutionError> {
    let mut builder = GitignoreBuilder::new(dir);

    // Try to add .gitignore file if it exists
    let gitignore_path = dir.join(".gitignore");
    if gitignore_path.exists() {
        builder.add(&gitignore_path);
    }

    // Add some common ignore patterns as fallback
    builder.add_line(None, "target/").map_err(|e| {
        ExecutionError::Execution(format!("Failed to add default ignore pattern: {}", e))
    })?;
    builder.add_line(None, ".git/").map_err(|e| {
        ExecutionError::Execution(format!("Failed to add default ignore pattern: {}", e))
    })?;

    match builder.build() {
        Ok(gitignore) => Ok(Some(gitignore)),
        Err(e) => {
            wrkflw_logging::warning(&format!("Failed to build gitignore matcher: {}", e));
            Ok(None)
        }
    }
}

fn copy_directory_contents(from: &Path, to: &Path) -> Result<(), ExecutionError> {
    copy_directory_contents_with_gitignore(from, to, None)
}

fn copy_directory_contents_with_gitignore(
    from: &Path,
    to: &Path,
    gitignore: Option<&ignore::gitignore::Gitignore>,
) -> Result<(), ExecutionError> {
    // If no gitignore provided, try to create one for the root directory
    let root_gitignore;
    let gitignore = if gitignore.is_none() {
        root_gitignore = create_gitignore_matcher(from)?;
        root_gitignore.as_ref()
    } else {
        gitignore
    };

    // Log summary of the copy operation
    wrkflw_logging::debug(&format!(
        "Copying directory contents from {} to {}",
        from.display(),
        to.display()
    ));

    for entry in std::fs::read_dir(from)
        .map_err(|e| ExecutionError::Execution(format!("Failed to read directory: {}", e)))?
    {
        let entry =
            entry.map_err(|e| ExecutionError::Execution(format!("Failed to read entry: {}", e)))?;
        let path = entry.path();

        if path.is_symlink() {
            wrkflw_logging::debug(&format!("Skipping symlink: {:?}", path));
            continue;
        }

        // Check if the file should be ignored according to .gitignore
        if let Some(gitignore) = gitignore {
            let relative_path = path.strip_prefix(from).unwrap_or(&path);
            match gitignore.matched(relative_path, path.is_dir()) {
                Match::Ignore(_) => {
                    wrkflw_logging::debug(&format!("Skipping ignored file/directory: {path:?}"));
                    continue;
                }
                Match::Whitelist(_) | Match::None => {
                    // File is not ignored or explicitly whitelisted
                }
            }
        }

        // Log individual files only in trace mode (removed verbose per-file logging)

        // Additional basic filtering for hidden files (but allow .gitignore and .github)
        let file_name = match path.file_name() {
            Some(name) => name.to_string_lossy(),
            None => {
                return Err(ExecutionError::Execution(format!(
                    "Failed to get file name from path: {:?}",
                    path
                )));
            }
        };

        // Skip most hidden files but allow important ones
        if file_name.starts_with(".")
            && file_name != ".gitignore"
            && file_name != ".github"
            && !file_name.starts_with(".env")
        {
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

            // Recursively copy subdirectories with the same gitignore
            copy_directory_contents_with_gitignore(&path, &dest_path, gitignore)?;
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
        // ubuntu runners - using Ubuntu base images for better compatibility
        "ubuntu-latest" => "ubuntu:latest",
        "ubuntu-22.04" => "ubuntu:22.04",
        "ubuntu-20.04" => "ubuntu:20.04",
        "ubuntu-18.04" => "ubuntu:18.04",

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

        // macOS runners - use a standard Rust image for compatibility
        "macos-latest" => "rust:latest",
        "macos-12" => "rust:latest",    // Monterey equivalent
        "macos-11" => "rust:latest",    // Big Sur equivalent
        "macos-10.15" => "rust:latest", // Catalina equivalent

        // Windows runners - using servercore-based images
        "windows-latest" => "mcr.microsoft.com/windows/servercore:ltsc2022",
        "windows-2022" => "mcr.microsoft.com/windows/servercore:ltsc2022",
        "windows-2019" => "mcr.microsoft.com/windows/servercore:ltsc2019",

        // Language-specific runners
        "python-latest" => "python:3.11-slim",
        "python-3.11" => "python:3.11-slim",
        "python-3.10" => "python:3.10-slim",
        "python-3.9" => "python:3.9-slim",
        "python-3.8" => "python:3.8-slim",

        "node-latest" => "node:20-slim",
        "node-20" => "node:20-slim",
        "node-18" => "node:18-slim",
        "node-16" => "node:16-slim",

        "java-latest" => "eclipse-temurin:17-jdk",
        "java-17" => "eclipse-temurin:17-jdk",
        "java-11" => "eclipse-temurin:11-jdk",
        "java-8" => "eclipse-temurin:8-jdk",

        "go-latest" => "golang:1.21-slim",
        "go-1.21" => "golang:1.21-slim",
        "go-1.20" => "golang:1.20-slim",
        "go-1.19" => "golang:1.19-slim",

        "dotnet-latest" => "mcr.microsoft.com/dotnet/sdk:7.0",
        "dotnet-7.0" => "mcr.microsoft.com/dotnet/sdk:7.0",
        "dotnet-6.0" => "mcr.microsoft.com/dotnet/sdk:6.0",
        "dotnet-5.0" => "mcr.microsoft.com/dotnet/sdk:5.0",

        // Default case for other runners or custom strings
        _ => {
            // Check for platform prefixes and provide appropriate images
            let runs_on_lower = runs_on.trim().to_lowercase();
            if runs_on_lower.starts_with("macos") {
                "rust:latest" // Use Rust image for macOS runners
            } else if runs_on_lower.starts_with("windows") {
                "mcr.microsoft.com/windows/servercore:ltsc2022" // Default Windows image
            } else if runs_on_lower.starts_with("python") {
                "python:3.11-slim" // Default Python image
            } else if runs_on_lower.starts_with("node") {
                "node:20-slim" // Default Node.js image
            } else if runs_on_lower.starts_with("java") {
                "eclipse-temurin:17-jdk" // Default Java image
            } else if runs_on_lower.starts_with("go") {
                "golang:1.21-slim" // Default Go image
            } else if runs_on_lower.starts_with("dotnet") {
                "mcr.microsoft.com/dotnet/sdk:7.0" // Default .NET image
            } else {
                "ubuntu:latest" // Default to Ubuntu for everything else
            }
        }
    }
    .to_string()
}

fn get_runner_image_from_opt(runs_on: &Option<Vec<String>>) -> String {
    let default = "ubuntu-latest";
    let ro = runs_on
        .as_ref()
        .and_then(|vec| vec.first())
        .map(|s| s.as_str())
        .unwrap_or(default);
    get_runner_image(ro)
}

fn get_effective_runner_image(job: &Job) -> String {
    if let Some(ref container) = job.container {
        if container.image.is_empty() {
            wrkflw_logging::warning("container image is empty, falling back to runs-on");
            get_runner_image_from_opt(&job.runs_on)
        } else {
            container.image.clone()
        }
    } else {
        get_runner_image_from_opt(&job.runs_on)
    }
}

type VolumePathPair = (PathBuf, PathBuf);

/// Prepare container volume mounts and remap GitHub environment file paths for container runtimes.
///
/// Returns owned volume path pairs that should be appended to the volumes list,
/// and mutates `step_env` to remap GITHUB_ENV/GITHUB_OUTPUT/GITHUB_PATH/GITHUB_STEP_SUMMARY
/// to container-internal paths when running under Docker/Podman.
fn prepare_container_mounts(
    step_env: &mut HashMap<String, String>,
    job_env: &HashMap<String, String>,
    container_config: Option<&JobContainer>,
) -> (Vec<VolumePathPair>, Option<VolumePathPair>) {
    let container_github_dir = Path::new("/github/workflow");
    let is_container_runtime = step_env
        .get("WRKFLW_RUNTIME_MODE")
        .map(|m| m == "docker" || m == "podman")
        .unwrap_or(false);

    // Mount GitHub environment files directory and remap paths
    let github_mount = if let Some(github_env_path) = job_env.get("GITHUB_ENV") {
        if let Some(github_dir) = Path::new(github_env_path).parent() {
            if is_container_runtime {
                // Remap each GitHub env file path by deriving the filename from the actual
                // host path, so the mapping stays correct if environment.rs renames them.
                // Only remap keys that actually exist in job_env to avoid phantom paths.
                for env_key in &[
                    "GITHUB_ENV",
                    "GITHUB_OUTPUT",
                    "GITHUB_PATH",
                    "GITHUB_STEP_SUMMARY",
                ] {
                    if let Some(host_path) = job_env.get(*env_key) {
                        if let Some(filename) = Path::new(host_path).file_name() {
                            step_env.insert(
                                env_key.to_string(),
                                format!("/github/workflow/{}", filename.to_string_lossy()),
                            );
                        }
                    }
                }
                Some((github_dir.to_path_buf(), container_github_dir.to_path_buf()))
            } else {
                github_dir
                    .parent()
                    .map(|p| (p.to_path_buf(), p.to_path_buf()))
            }
        } else {
            None
        }
    } else {
        None
    };

    // Collect container-defined volumes
    // Docker volume syntax: host:container[:options] — splitn(3) handles the optional :ro/:rw
    let mut owned_volume_paths: Vec<VolumePathPair> = Vec::new();
    if let Some(container_volumes) = container_config.and_then(|c| c.volumes.as_ref()) {
        for vol_spec in container_volumes {
            if vol_spec.is_empty() {
                wrkflw_logging::warning("skipping empty volume spec");
                continue;
            }
            // NOTE: splitn(3, ':') won't correctly handle Windows-style host paths (e.g. C:\data:/container)
            let parts: Vec<&str> = vol_spec.splitn(3, ':').collect();
            // Check host path for path traversal (only the host component, not the full spec)
            let host_path = parts[0];
            if std::path::Path::new(host_path)
                .components()
                .any(|c| matches!(c, std::path::Component::ParentDir))
            {
                wrkflw_logging::warning(&format!(
                    "Skipping volume with path traversal in host path: {}",
                    vol_spec
                ));
                continue;
            }
            match parts.len() {
                3 => {
                    if parts[0].is_empty() || parts[1].is_empty() {
                        wrkflw_logging::warning(&format!(
                            "skipping volume spec with empty host or container path: '{}'",
                            vol_spec
                        ));
                        continue;
                    }
                    wrkflw_logging::warning(&format!(
                        "volume mount option '{}' in '{}' is not yet supported and will be ignored",
                        parts[2], vol_spec
                    ));
                    owned_volume_paths.push((PathBuf::from(parts[0]), PathBuf::from(parts[1])));
                }
                2 => {
                    if parts[0].is_empty() || parts[1].is_empty() {
                        wrkflw_logging::warning(&format!(
                            "skipping volume spec with empty host or container path: '{}'",
                            vol_spec
                        ));
                        continue;
                    }
                    owned_volume_paths.push((PathBuf::from(parts[0]), PathBuf::from(parts[1])));
                }
                _ => {
                    // Single path: mount at same location inside container
                    let p = PathBuf::from(parts[0]);
                    owned_volume_paths.push((p.clone(), p));
                }
            }
        }
    }

    (owned_volume_paths, github_mount)
}

/// Log warnings for container fields that are parsed but not yet supported.
fn warn_unsupported_container_fields(container: &JobContainer) {
    if container.options.is_some() {
        wrkflw_logging::warning(
            "container 'options' field is not yet supported and will be ignored",
        );
    }
    if container.credentials.is_some() {
        wrkflw_logging::warning(
            "container 'credentials' field is not yet supported and will be ignored",
        );
    }
    if container.ports.is_some() {
        wrkflw_logging::warning(
            "container 'ports' field is not yet supported (service containers are not implemented)",
        );
    }
}

async fn execute_reusable_workflow_job(
    ctx: &JobExecutionContext<'_>,
    uses: &str,
    with: Option<&HashMap<String, String>>,
    secrets: Option<&serde_yaml::Value>,
) -> Result<JobResult, ExecutionError> {
    wrkflw_logging::info(&format!(
        "Executing reusable workflow job '{}' -> {}",
        ctx.job_name, uses
    ));

    // Resolve the called workflow file path
    enum UsesRef<'a> {
        LocalPath(&'a str),
        Remote {
            owner: String,
            repo: String,
            path: String,
            r#ref: String,
        },
    }

    let uses_ref = if uses.starts_with("./") || uses.starts_with('/') {
        UsesRef::LocalPath(uses)
    } else {
        // Expect format owner/repo/path/to/workflow.yml@ref
        let parts: Vec<&str> = uses.split('@').collect();
        if parts.len() != 2 {
            return Err(ExecutionError::Execution(format!(
                "Invalid reusable workflow reference: {}",
                uses
            )));
        }
        let left = parts[0];
        let r#ref = parts[1].to_string();
        let mut segs = left.splitn(3, '/');
        let owner = segs.next().unwrap_or("").to_string();
        let repo = segs.next().unwrap_or("").to_string();
        let path = segs.next().unwrap_or("").to_string();
        if owner.is_empty() || repo.is_empty() || path.is_empty() {
            return Err(ExecutionError::Execution(format!(
                "Invalid reusable workflow reference: {}",
                uses
            )));
        }
        UsesRef::Remote {
            owner,
            repo,
            path,
            r#ref,
        }
    };

    // Load workflow file
    let workflow_path = match uses_ref {
        UsesRef::LocalPath(p) => {
            // Resolve relative to current directory
            let current_dir = std::env::current_dir().map_err(|e| {
                ExecutionError::Execution(format!("Failed to get current dir: {}", e))
            })?;
            let path = current_dir.join(p);
            if !path.exists() {
                return Err(ExecutionError::Execution(format!(
                    "Reusable workflow not found at path: {}",
                    path.display()
                )));
            }
            path
        }
        UsesRef::Remote {
            owner,
            repo,
            path,
            r#ref,
        } => {
            // Clone minimal repository and checkout ref
            let tempdir = tempfile::tempdir().map_err(|e| {
                ExecutionError::Execution(format!("Failed to create temp dir: {}", e))
            })?;
            let repo_url = format!("https://github.com/{}/{}.git", owner, repo);

            // Clone into a subdirectory within tempdir to get clean structure
            let repo_dir = tempdir.path().join("cloned_repo");

            shallow_clone(&repo_url, &r#ref, &repo_dir).await?;
            let joined = repo_dir.join(path);

            if !joined.exists() {
                return Err(ExecutionError::Execution(format!(
                    "Reusable workflow file not found in repo: {}",
                    joined.display()
                )));
            }

            // Parse called workflow while keeping tempdir alive
            let called = parse_workflow(&joined)?;

            // Create child env context
            let mut child_env = ctx.env_context.clone();
            if let Some(with_map) = with {
                for (k, v) in with_map {
                    child_env.insert(format!("INPUT_{}", k.to_uppercase()), v.clone());
                }
            }
            if let Some(secrets_val) = secrets {
                if let Some(map) = secrets_val.as_mapping() {
                    for (k, v) in map {
                        if let (Some(key), Some(value)) = (k.as_str(), v.as_str()) {
                            child_env.insert(
                                format!("SECRET_{}", key.to_uppercase()),
                                value.to_string(),
                            );
                        }
                    }
                }
            }

            // Execute called workflow
            let plan = dependency::resolve_dependencies(&called)?;
            let mut all_results = Vec::new();
            let mut any_failed = false;
            for batch in plan {
                let results = execute_job_batch(
                    &batch,
                    &called,
                    ctx.runtime,
                    &child_env,
                    ctx.verbose,
                    None,
                    None,
                )
                .await?;
                for r in &results {
                    if r.status == JobStatus::Failure {
                        any_failed = true;
                    }
                }
                all_results.extend(results);
            }

            // Summarize into a single JobResult
            let mut logs = String::new();
            logs.push_str(&format!("Called workflow: {}\n", joined.display()));
            for r in &all_results {
                logs.push_str(&format!("- {}: {:?}\n", r.name, r.status));
            }

            // Represent as one summary step for UI
            let summary_step = StepResult {
                name: format!("Run reusable workflow: {}", uses),
                status: if any_failed {
                    StepStatus::Failure
                } else {
                    StepStatus::Success
                },
                output: logs.clone(),
            };

            return Ok(JobResult {
                name: ctx.job_name.to_string(),
                status: if any_failed {
                    JobStatus::Failure
                } else {
                    JobStatus::Success
                },
                steps: vec![summary_step],
                logs,
            });
        }
    };

    // Parse called workflow (for local paths)
    let called = parse_workflow(&workflow_path)?;

    // Create child env context
    let mut child_env = ctx.env_context.clone();
    if let Some(with_map) = with {
        for (k, v) in with_map {
            child_env.insert(format!("INPUT_{}", k.to_uppercase()), v.clone());
        }
    }
    if let Some(secrets_val) = secrets {
        if let Some(map) = secrets_val.as_mapping() {
            for (k, v) in map {
                if let (Some(key), Some(value)) = (k.as_str(), v.as_str()) {
                    child_env.insert(format!("SECRET_{}", key.to_uppercase()), value.to_string());
                }
            }
        }
    }

    // Execute called workflow
    let plan = dependency::resolve_dependencies(&called)?;
    let mut all_results = Vec::new();
    let mut any_failed = false;
    for batch in plan {
        let results = execute_job_batch(
            &batch,
            &called,
            ctx.runtime,
            &child_env,
            ctx.verbose,
            None,
            None,
        )
        .await?;
        for r in &results {
            if r.status == JobStatus::Failure {
                any_failed = true;
            }
        }
        all_results.extend(results);
    }

    // Summarize into a single JobResult
    let mut logs = String::new();
    logs.push_str(&format!("Called workflow: {}\n", workflow_path.display()));
    for r in &all_results {
        logs.push_str(&format!("- {}: {:?}\n", r.name, r.status));
    }

    // Represent as one summary step for UI
    let summary_step = StepResult {
        name: format!("Run reusable workflow: {}", uses),
        status: if any_failed {
            StepStatus::Failure
        } else {
            StepStatus::Success
        },
        output: logs.clone(),
    };

    Ok(JobResult {
        name: ctx.job_name.to_string(),
        status: if any_failed {
            JobStatus::Failure
        } else {
            JobStatus::Success
        },
        steps: vec![summary_step],
        logs,
    })
}

#[allow(dead_code)]
async fn prepare_runner_image(
    image: &str,
    runtime: &dyn ContainerRuntime,
    verbose: bool,
) -> Result<(), ExecutionError> {
    // Try to pull the image first
    if let Err(e) = runtime.pull_image(image).await {
        wrkflw_logging::warning(&format!("Failed to pull image {}: {}", image, e));
    }

    // Check if this is a language-specific runner
    let language_info = extract_language_info(image);
    if let Some((language, version)) = language_info {
        // Try to prepare a language-specific environment
        if let Ok(custom_image) = runtime
            .prepare_language_environment(language, version, None)
            .await
            .map_err(|e| ExecutionError::Runtime(e.to_string()))
        {
            if verbose {
                wrkflw_logging::info(&format!("Using customized image: {}", custom_image));
            }
            return Ok(());
        }
    }

    Ok(())
}

#[allow(dead_code)]
fn extract_language_info(image: &str) -> Option<(&'static str, Option<&str>)> {
    let image_lower = image.to_lowercase();

    // Check for language-specific images
    if image_lower.starts_with("python:") {
        Some(("python", Some(&image[7..])))
    } else if image_lower.starts_with("node:") {
        Some(("node", Some(&image[5..])))
    } else if image_lower.starts_with("eclipse-temurin:") {
        Some(("java", Some(&image[15..])))
    } else if image_lower.starts_with("golang:") {
        Some(("go", Some(&image[6..])))
    } else if image_lower.starts_with("mcr.microsoft.com/dotnet/sdk:") {
        Some(("dotnet", Some(&image[29..])))
    } else if image_lower.starts_with("rust:") {
        Some(("rust", Some(&image[5..])))
    } else {
        None
    }
}

async fn execute_composite_action(
    step: &workflow::Step,
    action_path: &Path,
    job_env: &HashMap<String, String>,
    working_dir: &Path,
    runtime: &dyn ContainerRuntime,
    runner_image: &str,
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
                    workflow: &workflow::WorkflowDefinition {
                        name: "Composite Action".to_string(),
                        on: vec![],
                        on_raw: serde_yaml::Value::Null,
                        jobs: HashMap::new(),
                    },
                    runner_image,
                    verbose,
                    matrix_combination: &None,
                    secret_manager: None, // Composite actions don't have secrets yet
                    secret_masker: None,
                    container_config: None, // Composite actions don't use job containers
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
                        output: step_outputs.join("\n"),
                    });
                }
            }

            // All steps completed successfully
            let output = if verbose {
                let mut detailed_output = format!(
                    "Executed composite action from: {}\n\n",
                    action_path.display()
                );

                // Add information about the composite action if available
                if let Ok(action_content) =
                    serde_yaml::from_str::<serde_yaml::Value>(&action_content)
                {
                    if let Some(name) = action_content.get("name").and_then(|v| v.as_str()) {
                        detailed_output.push_str(&format!("Action name: {}\n", name));
                    }

                    if let Some(description) =
                        action_content.get("description").and_then(|v| v.as_str())
                    {
                        detailed_output.push_str(&format!("Description: {}\n", description));
                    }

                    detailed_output.push('\n');
                }

                // Add individual step outputs
                detailed_output.push_str("Step outputs:\n");
                for output in &step_outputs {
                    detailed_output.push_str(&format!("{}\n", output));
                }

                detailed_output
            } else {
                format!(
                    "Executed composite action with {} steps",
                    step_outputs.len()
                )
            };

            Ok(StepResult {
                name: step
                    .name
                    .clone()
                    .unwrap_or_else(|| "Composite Action".to_string()),
                status: StepStatus::Success,
                output,
            })
        }
        _ => Err(ExecutionError::Execution(
            "Action is not a composite action or has invalid format".to_string(),
        )),
    }
}

// Helper function to convert YAML step to our Step struct
fn convert_yaml_to_step(step_yaml: &serde_yaml::Value) -> Result<workflow::Step, String> {
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
    let continue_on_error = step_yaml.get("continue-on-error").and_then(|v| v.as_bool());

    let if_condition = step_yaml
        .get("if")
        .and_then(|v| v.as_str())
        .map(|s| s.to_string());

    let id = step_yaml
        .get("id")
        .and_then(|v| v.as_str())
        .map(|s| s.to_string());

    let working_directory = step_yaml
        .get("working-directory")
        .and_then(|v| v.as_str())
        .map(|s| s.to_string());

    let timeout_minutes = step_yaml.get("timeout-minutes").and_then(|v| v.as_f64());

    Ok(workflow::Step {
        name,
        uses,
        run: final_run,
        with,
        env,
        continue_on_error,
        if_condition,
        id,
        working_directory,
        shell,
        timeout_minutes,
    })
}

/// Evaluate a job condition expression
/// This is a simplified implementation that handles basic GitHub Actions expressions.
/// Note: step-level expressions like `steps.<id>.outcome`, `success()`, `failure()`,
/// `always()`, and `cancelled()` are not yet fully supported — a warning is emitted
/// and the condition defaults to its most likely state (`always()`/`success()` → true,
/// `failure()`/`cancelled()` → false).
fn evaluate_job_condition(
    condition: &str,
    env_context: &HashMap<String, String>,
    workflow: &WorkflowDefinition,
) -> bool {
    wrkflw_logging::debug(&format!("Evaluating condition: {}", condition));

    // Handle status functions and step references that we can't fully evaluate.
    // We default conservatively: only `always()` and `success()` resolve to true,
    // since those represent the common "run this step" intent. Bare `steps.*`
    // references (e.g. `steps.X.outcome == 'failure'`) default to false to avoid
    // running steps that depend on prior failure/output we can't evaluate.
    let has_always = condition.contains("always()");
    let has_success = condition.contains("success()");
    let has_failure = condition.contains("failure()");
    let has_cancelled = condition.contains("cancelled()");
    // Match "steps." only at word boundaries to avoid false positives on env var
    // names like "env.MY_STEPS_COUNT" or "env._STEPS_CHECK". We check for
    // start-of-string or a character that isn't alphanumeric/underscore before "steps.".
    let has_steps_ref = condition.match_indices("steps.").any(|(pos, _)| {
        pos == 0 || {
            let b = condition.as_bytes()[pos - 1];
            !b.is_ascii_alphanumeric() && b != b'_'
        }
    });
    let has_unsupported =
        has_always || has_success || has_failure || has_cancelled || has_steps_ref;

    if has_unsupported {
        wrkflw_logging::warning(&format!(
            "Condition '{}' uses status functions/step references not fully supported in local execution",
            condition
        ));

        // In GitHub Actions, `always()` means "run this step regardless of job
        // status" — it is a *scheduling* directive, not a boolean `true` literal.
        // Similarly, `success()` means "run when all previous steps succeeded".
        // Since we can't evaluate actual job/step status locally, we treat
        // `always()` and `success()` as "likely to run" → true, and `failure()`
        // / `cancelled()` as "unlikely" → false.
        //
        // Known limitation: compound expressions like `always() && failure()` will
        // return true (because `always()` is present) even though a real evaluator
        // would AND the two. This is acceptable because we lack step-status context
        // and would rather over-run than silently skip steps.
        if has_always || has_success {
            return true;
        }
        // Bare steps.* refs, failure(), cancelled() without positive counterpart → false
        return false;
    }

    // For now, implement basic pattern matching for common conditions
    // TODO: Implement a full GitHub Actions expression evaluator

    // Handle simple boolean conditions
    if condition == "true" {
        return true;
    }
    if condition == "false" {
        return false;
    }

    // Handle github.event.pull_request.draft == false
    if condition.contains("github.event.pull_request.draft == false") {
        // For local execution, assume this is always true (not a draft)
        return true;
    }

    // Handle needs.jobname.outputs.outputname == 'value' patterns
    if condition.contains("needs.") && condition.contains(".outputs.") {
        // For now, simulate that outputs are available but empty
        // This means conditions like needs.changes.outputs.source-code == 'true' will be false
        wrkflw_logging::debug(
            "Evaluating needs.outputs condition - defaulting to false for local execution",
        );
        return false;
    }

    // Default to true for unknown conditions to avoid breaking workflows
    wrkflw_logging::warning(&format!(
        "Unknown condition pattern: '{}' - defaulting to true",
        condition
    ));
    true
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn is_git_sha_recognizes_valid_sha1() {
        assert!(is_git_sha("a81bbbf8298c0fa03ea29cdc473d45769f953675"));
    }

    #[test]
    fn is_git_sha_recognizes_uppercase_hex() {
        assert!(is_git_sha("A81BBBF8298C0FA03EA29CDC473D45769F953675"));
    }

    #[test]
    fn is_git_sha_rejects_short_hash() {
        assert!(!is_git_sha("a81bbbf"));
    }

    #[test]
    fn is_git_sha_rejects_branch_name() {
        assert!(!is_git_sha("main"));
    }

    #[test]
    fn is_git_sha_rejects_tag() {
        assert!(!is_git_sha("v4"));
    }

    #[test]
    fn is_git_sha_rejects_empty() {
        assert!(!is_git_sha(""));
    }

    #[test]
    fn is_git_sha_rejects_non_hex_40_chars() {
        assert!(!is_git_sha("zzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzz"));
    }

    #[test]
    fn is_git_sha_rejects_41_chars() {
        assert!(!is_git_sha("a81bbbf8298c0fa03ea29cdc473d45769f9536750"));
    }

    // --- get_effective_runner_image tests ---

    fn make_job(container: Option<JobContainer>, runs_on: Option<Vec<String>>) -> Job {
        Job {
            runs_on,
            needs: None,
            container,
            steps: Vec::new(),
            env: HashMap::new(),
            strategy: None,
            services: HashMap::new(),
            if_condition: None,
            outputs: None,
            permissions: None,
            uses: None,
            with: None,
            secrets: None,
        }
    }

    #[test]
    fn effective_runner_image_prefers_container() {
        let job = make_job(
            Some(JobContainer {
                image: "alpine:3.22".into(),
                credentials: None,
                env: HashMap::new(),
                ports: None,
                volumes: None,
                options: None,
            }),
            Some(vec!["ubuntu-latest".into()]),
        );
        assert_eq!(get_effective_runner_image(&job), "alpine:3.22");
    }

    #[test]
    fn effective_runner_image_falls_back_to_runs_on() {
        let job = make_job(None, Some(vec!["ubuntu-latest".into()]));
        let image = get_effective_runner_image(&job);
        // Should delegate to get_runner_image_from_opt, not return empty
        assert!(!image.is_empty());
    }

    // --- prepare_container_mounts tests ---

    #[test]
    fn container_mounts_docker_runtime_remaps_env_paths() {
        let mut step_env = HashMap::new();
        step_env.insert("WRKFLW_RUNTIME_MODE".into(), "docker".into());

        let mut job_env = HashMap::new();
        job_env.insert("GITHUB_ENV".into(), "/tmp/abc/github/env".into());
        job_env.insert("GITHUB_OUTPUT".into(), "/tmp/abc/github/output".into());
        job_env.insert("GITHUB_PATH".into(), "/tmp/abc/github/path".into());
        job_env.insert(
            "GITHUB_STEP_SUMMARY".into(),
            "/tmp/abc/github/step_summary".into(),
        );

        let (volumes, github_mount) = prepare_container_mounts(&mut step_env, &job_env, None);

        // Env vars should be remapped to container paths
        assert_eq!(step_env.get("GITHUB_ENV").unwrap(), "/github/workflow/env");
        assert_eq!(
            step_env.get("GITHUB_OUTPUT").unwrap(),
            "/github/workflow/output"
        );
        assert_eq!(
            step_env.get("GITHUB_PATH").unwrap(),
            "/github/workflow/path"
        );
        assert_eq!(
            step_env.get("GITHUB_STEP_SUMMARY").unwrap(),
            "/github/workflow/step_summary"
        );

        // Should mount the github dir at /github/workflow
        let (host, container) = github_mount.unwrap();
        assert_eq!(host, PathBuf::from("/tmp/abc/github"));
        assert_eq!(container, PathBuf::from("/github/workflow"));

        // No container config volumes
        assert!(volumes.is_empty());
    }

    #[test]
    fn container_mounts_non_container_runtime_does_not_remap() {
        let mut step_env = HashMap::new();
        // No WRKFLW_RUNTIME_MODE set

        let mut job_env = HashMap::new();
        job_env.insert("GITHUB_ENV".into(), "/tmp/abc/github/env".into());

        let (_volumes, github_mount) = prepare_container_mounts(&mut step_env, &job_env, None);

        // Env vars should NOT be remapped
        assert!(!step_env.contains_key("GITHUB_ENV"));

        // Should mount the parent directory identity-mapped
        let (host, container) = github_mount.unwrap();
        assert_eq!(host, container); // identity mount
    }

    #[test]
    fn container_mounts_no_github_env() {
        let mut step_env = HashMap::new();
        let job_env = HashMap::new(); // no GITHUB_ENV

        let (volumes, github_mount) = prepare_container_mounts(&mut step_env, &job_env, None);

        assert!(github_mount.is_none());
        assert!(volumes.is_empty());
    }

    #[test]
    fn container_mounts_parses_host_container_volumes() {
        let mut step_env = HashMap::new();
        let job_env = HashMap::new();

        let container = JobContainer {
            image: "node:18".into(),
            credentials: None,
            env: HashMap::new(),
            ports: None,
            volumes: Some(vec!["/host/data:/container/data".into()]),
            options: None,
        };

        let (volumes, _) = prepare_container_mounts(&mut step_env, &job_env, Some(&container));

        assert_eq!(volumes.len(), 1);
        assert_eq!(volumes[0].0, PathBuf::from("/host/data"));
        assert_eq!(volumes[0].1, PathBuf::from("/container/data"));
    }

    #[test]
    fn container_mounts_parses_single_path_volumes() {
        let mut step_env = HashMap::new();
        let job_env = HashMap::new();

        let container = JobContainer {
            image: "node:18".into(),
            credentials: None,
            env: HashMap::new(),
            ports: None,
            volumes: Some(vec!["/data".into()]),
            options: None,
        };

        let (volumes, _) = prepare_container_mounts(&mut step_env, &job_env, Some(&container));

        assert_eq!(volumes.len(), 1);
        assert_eq!(volumes[0].0, PathBuf::from("/data"));
        assert_eq!(volumes[0].1, PathBuf::from("/data"));
    }

    #[test]
    fn container_mounts_strips_docker_options_from_volumes() {
        let mut step_env = HashMap::new();
        let job_env = HashMap::new();

        let container = JobContainer {
            image: "node:18".into(),
            credentials: None,
            env: HashMap::new(),
            ports: None,
            volumes: Some(vec!["/host:/container:ro".into(), "/src:/dest:rw".into()]),
            options: None,
        };

        let (volumes, _) = prepare_container_mounts(&mut step_env, &job_env, Some(&container));

        assert_eq!(volumes.len(), 2);
        // :ro should be stripped — container path should be clean
        assert_eq!(volumes[0].0, PathBuf::from("/host"));
        assert_eq!(volumes[0].1, PathBuf::from("/container"));
        // :rw should be stripped
        assert_eq!(volumes[1].0, PathBuf::from("/src"));
        assert_eq!(volumes[1].1, PathBuf::from("/dest"));
    }

    #[test]
    fn container_mounts_podman_runtime_remaps_env_paths() {
        let mut step_env = HashMap::new();
        step_env.insert("WRKFLW_RUNTIME_MODE".into(), "podman".into());

        let mut job_env = HashMap::new();
        job_env.insert("GITHUB_ENV".into(), "/tmp/xyz/github/env".into());
        job_env.insert("GITHUB_OUTPUT".into(), "/tmp/xyz/github/output".into());
        job_env.insert("GITHUB_PATH".into(), "/tmp/xyz/github/path".into());
        job_env.insert(
            "GITHUB_STEP_SUMMARY".into(),
            "/tmp/xyz/github/step_summary".into(),
        );

        let (_, github_mount) = prepare_container_mounts(&mut step_env, &job_env, None);

        // Podman should behave identically to Docker for remapping
        assert_eq!(step_env.get("GITHUB_ENV").unwrap(), "/github/workflow/env");
        assert!(github_mount.is_some());
    }

    #[test]
    fn container_mounts_only_remaps_existing_env_keys() {
        let mut step_env = HashMap::new();
        step_env.insert("WRKFLW_RUNTIME_MODE".into(), "docker".into());

        let mut job_env = HashMap::new();
        // Only set GITHUB_ENV — the others are absent
        job_env.insert("GITHUB_ENV".into(), "/tmp/abc/github/env".into());

        let (_, _) = prepare_container_mounts(&mut step_env, &job_env, None);

        // GITHUB_ENV should be remapped
        assert_eq!(step_env.get("GITHUB_ENV").unwrap(), "/github/workflow/env");
        // Others should NOT be inserted (no phantom paths)
        assert!(!step_env.contains_key("GITHUB_OUTPUT"));
        assert!(!step_env.contains_key("GITHUB_PATH"));
        assert!(!step_env.contains_key("GITHUB_STEP_SUMMARY"));
    }

    #[test]
    fn effective_runner_image_empty_image_falls_back() {
        let job = make_job(
            Some(JobContainer {
                image: "".into(),
                credentials: None,
                env: HashMap::new(),
                ports: None,
                volumes: None,
                options: None,
            }),
            Some(vec!["ubuntu-latest".into()]),
        );
        let image = get_effective_runner_image(&job);
        // Should fall back to runs-on, not return empty string
        assert!(!image.is_empty());
    }

    #[test]
    fn container_mounts_skips_empty_container_path() {
        let mut step_env = HashMap::new();
        let job_env = HashMap::new();

        let container = JobContainer {
            image: "node:18".into(),
            credentials: None,
            env: HashMap::new(),
            ports: None,
            volumes: Some(vec!["/host:".into(), ":/container".into()]),
            options: None,
        };

        let (volumes, _) = prepare_container_mounts(&mut step_env, &job_env, Some(&container));

        // Both specs have an empty path component and should be skipped
        assert!(volumes.is_empty());
    }

    // --- container env precedence tests ---

    #[test]
    fn container_env_has_lowest_precedence() {
        // Simulate the env merging logic from execute_job:
        // 1. Container env is inserted with or_insert (lowest precedence)
        // 2. Job env is inserted with insert (overrides container env)
        let mut job_env = HashMap::new();

        // Step 1: container env (lowest precedence)
        let container = JobContainer {
            image: "node:18".into(),
            credentials: None,
            env: HashMap::from([
                ("SHARED".into(), "from-container".into()),
                ("CONTAINER_ONLY".into(), "container-value".into()),
            ]),
            ports: None,
            volumes: None,
            options: None,
        };
        for (key, value) in &container.env {
            job_env.entry(key.clone()).or_insert_with(|| value.clone());
        }

        // Step 2: job env (overrides container env)
        let job_level_env: HashMap<String, String> = HashMap::from([
            ("SHARED".into(), "from-job".into()),
            ("JOB_ONLY".into(), "job-value".into()),
        ]);
        for (key, value) in &job_level_env {
            job_env.insert(key.clone(), value.clone());
        }

        // Job-level env wins for shared keys
        assert_eq!(job_env.get("SHARED").unwrap(), "from-job");
        // Container-only keys are preserved
        assert_eq!(job_env.get("CONTAINER_ONLY").unwrap(), "container-value");
        // Job-only keys are preserved
        assert_eq!(job_env.get("JOB_ONLY").unwrap(), "job-value");
    }

    // --- evaluate_job_condition tests for step-level expressions ---

    fn empty_workflow() -> workflow::WorkflowDefinition {
        workflow::WorkflowDefinition {
            name: "test".to_string(),
            on: Vec::new(),
            on_raw: serde_yaml::Value::Null,
            jobs: HashMap::new(),
        }
    }

    #[test]
    fn condition_true_false_literals() {
        let env = HashMap::new();
        let wf = empty_workflow();
        assert!(evaluate_job_condition("true", &env, &wf));
        assert!(!evaluate_job_condition("false", &env, &wf));
    }

    #[test]
    fn condition_steps_reference_defaults_false() {
        let env = HashMap::new();
        let wf = empty_workflow();
        // Bare step-level expressions default to false (conservative — we can't evaluate them)
        assert!(!evaluate_job_condition(
            "steps.build.outcome == 'success'",
            &env,
            &wf
        ));
    }

    #[test]
    fn condition_success_function_defaults_true() {
        let env = HashMap::new();
        let wf = empty_workflow();
        assert!(evaluate_job_condition("success()", &env, &wf));
    }

    #[test]
    fn condition_failure_function_defaults_false() {
        let env = HashMap::new();
        let wf = empty_workflow();
        assert!(!evaluate_job_condition("failure()", &env, &wf));
    }

    #[test]
    fn condition_always_function_defaults_true() {
        let env = HashMap::new();
        let wf = empty_workflow();
        assert!(evaluate_job_condition("always()", &env, &wf));
    }

    #[test]
    fn condition_cancelled_function_defaults_false() {
        let env = HashMap::new();
        let wf = empty_workflow();
        assert!(!evaluate_job_condition("cancelled()", &env, &wf));
    }

    #[test]
    fn condition_compound_failure_or_success_defaults_true() {
        let env = HashMap::new();
        let wf = empty_workflow();
        // success() is present, so compound expression should default to true
        assert!(evaluate_job_condition("failure() || success()", &env, &wf));
    }

    #[test]
    fn condition_compound_failure_and_cancelled_defaults_false() {
        let env = HashMap::new();
        let wf = empty_workflow();
        // Only negative functions, no positive counterpart → false
        assert!(!evaluate_job_condition(
            "failure() || cancelled()",
            &env,
            &wf
        ));
    }

    #[test]
    fn condition_always_with_failure_defaults_true() {
        let env = HashMap::new();
        let wf = empty_workflow();
        // always() present → true regardless of other functions
        assert!(evaluate_job_condition("always() && failure()", &env, &wf));
    }

    #[test]
    fn condition_env_var_containing_steps_not_treated_as_step_ref() {
        let env = HashMap::new();
        let wf = empty_workflow();
        // "env.MY_STEPS_COUNT" contains "steps." as a substring but should NOT
        // trigger the step-reference heuristic (which returns false). Instead it
        // falls through to the unknown-condition default (true).
        // A bare "steps.build.outcome" at the start SHOULD be caught.
        assert!(evaluate_job_condition(
            "env.MY_STEPS_COUNT == '5'",
            &env,
            &wf
        ));
        // Underscore-prefixed names should also NOT be treated as step refs
        assert!(evaluate_job_condition(
            "env._STEPS_CHECK == 'ok'",
            &env,
            &wf
        ));
        assert!(!evaluate_job_condition(
            "steps.build.outcome == 'success'",
            &env,
            &wf
        ));
    }

    // --- volume path traversal tests ---

    fn has_path_traversal(host_path: &str) -> bool {
        std::path::Path::new(host_path)
            .components()
            .any(|c| matches!(c, std::path::Component::ParentDir))
    }

    #[test]
    fn volume_traversal_check_rejects_host_traversal() {
        assert!(has_path_traversal("../../../etc/passwd"));
        assert!(has_path_traversal("/safe/../etc/passwd"));
    }

    #[test]
    fn volume_traversal_check_allows_dotdot_in_container_path() {
        // Container path with ".." in it should NOT trigger the host check
        let vol_spec = "/safe/host:/container/..weird";
        let parts: Vec<&str> = vol_spec.splitn(3, ':').collect();
        assert!(!has_path_traversal(parts[0]));
    }

    #[test]
    fn volume_traversal_allows_double_dot_prefix_dir() {
        // A directory literally named "..hidden" is not path traversal
        assert!(!has_path_traversal("/data/..hidden/files"));
    }
}
