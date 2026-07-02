// executor crate

#![allow(unused_variables, unused_assignments)]

pub mod action_resolver;
pub(crate) mod artifacts;
pub(crate) mod cache;
pub mod dependency;
pub mod docker;
pub mod engine;
pub mod environment;
pub mod expression;
pub mod github_env_files;
pub mod podman;
pub mod substitution;
pub(crate) mod workflow_commands;

// Re-export public items
pub use docker::cleanup_resources;
pub use engine::{
    detect_runtime, execute_workflow, ExecutionConfig, JobResult, JobStatus, RuntimeType,
    StepResult, StepStatus,
};
