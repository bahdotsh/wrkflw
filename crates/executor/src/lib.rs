// executor crate

#![allow(unused_variables, unused_assignments)]

pub mod action_resolver;
pub mod artifacts; // Implemented but not yet wired into engine execution
pub mod cache; // Implemented but not yet wired into engine execution
pub mod dependency;
pub mod docker;
pub mod engine;
pub mod environment;
pub mod expression;
pub mod github_env_files;
pub mod podman;
pub mod substitution;
pub mod workflow_commands;

// Re-export public items
pub use docker::cleanup_resources;
pub use engine::{
    execute_workflow, ExecutionConfig, JobResult, JobStatus, RuntimeType, StepResult, StepStatus,
};
