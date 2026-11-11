// executor crate

#![allow(unused_variables, unused_assignments)]

pub mod acceleration;
pub mod action_fidelity;
pub mod action_resolver;
pub(crate) mod artifacts;
pub(crate) mod cache;
pub(crate) mod checkout_sync;
mod container_guard;
pub mod dependency;
pub mod docker;
pub mod engine;
pub mod environment;
pub mod events;
pub mod expression;
pub mod github_env_files;
pub(crate) mod job_memory;
pub(crate) mod job_session;
pub mod kubernetes;
pub(crate) mod live_step;
pub(crate) mod log_redaction;
pub(crate) mod matrix_fold;
pub mod podman;
pub mod run_options;
pub mod runner_fidelity;
pub(crate) mod setup_actions;
pub mod substitution;
pub mod vm_memory;
pub(crate) mod vm_workspace;
pub(crate) mod workflow_commands;
pub mod workspace;

/// How the errors that stop a job before its first step begin, because its
/// image, job container or network couldn't be set up. The failure digest
/// tells these from the workflow's own errors by them.
pub mod setup_errors {
    pub const COMBINED_IMAGE_BUILD: &str = "Failed to build combined runtime image";
    pub const EXISTING_IMAGE_CHECK: &str = "Failed to check for existing image";
    pub const JOB_CONTAINER: &str = "job container didn't start";
}

// Re-export public items
pub use acceleration::{validate_remote, Acceleration, Secret, Sidecar, SIDECAR_PATH};
pub use action_fidelity::UnexecutedAction;
pub use docker::cleanup_resources;
pub use engine::{
    detect_runtime, execute_workflow, parse_platform_override, ExecutionConfig, JobResult,
    JobStatus, RuntimeType, StepResult, StepStatus,
};
pub use events::{
    with_run_hooks, ExecutionEvent, ExecutionObserver, FromStep, RunHooks, EXIT_CODE_LABEL,
    STDERR_HEADER, STDOUT_HEADER,
};
pub use kubernetes::{parse_key_value as parse_k8s_key_value, KubernetesConfig};
pub use log_redaction::{begin_run as begin_log_scope, RunScope as LogScope};
pub use runner_fidelity::RunnerSubstitution;
