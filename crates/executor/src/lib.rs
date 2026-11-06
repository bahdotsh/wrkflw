// executor crate

#![allow(unused_variables, unused_assignments)]

pub mod acceleration;
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
pub(crate) mod job_session;
pub mod kubernetes;
pub(crate) mod live_step;
pub(crate) mod log_redaction;
pub(crate) mod matrix_fold;
pub mod podman;
pub mod run_options;
pub mod substitution;
pub(crate) mod workflow_commands;
pub mod workspace;

// Re-export public items
pub use acceleration::{validate_remote, Acceleration, Secret, Sidecar, SIDECAR_PATH};
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
