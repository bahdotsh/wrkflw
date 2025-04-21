use async_trait::async_trait;
use std::path::Path;

#[async_trait]
pub trait ContainerRuntime {
    async fn run_container(
        &self,
        image: &str,
        cmd: &[&str],
        env_vars: &[(&str, &str)],
        working_dir: &Path,
        volumes: &[(&Path, &Path)],
    ) -> Result<ContainerOutput, ContainerError>;

    async fn pull_image(&self, image: &str) -> Result<(), ContainerError>;

    async fn build_image(&self, dockerfile: &Path, tag: &str) -> Result<(), ContainerError>;
}

pub struct ContainerOutput {
    pub stdout: String,
    pub stderr: String,
    pub exit_code: i32,
}

use std::fmt;

#[derive(Debug)]
pub enum ContainerError {
    ImagePullFailed(String),
    ImageBuildFailed(String),
    ContainerStartFailed(String),
    ContainerExecutionFailed(String),
    NetworkCreationFailed(String),
    NetworkOperationFailed(String),
}

impl fmt::Display for ContainerError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            ContainerError::ImagePullFailed(msg) => write!(f, "Failed to pull image: {}", msg),
            ContainerError::ImageBuildFailed(msg) => write!(f, "Failed to build image: {}", msg),
            ContainerError::ContainerStartFailed(msg) => {
                write!(f, "Failed to start container: {}", msg)
            }
            ContainerError::ContainerExecutionFailed(msg) => {
                write!(f, "Container execution failed: {}", msg)
            }
            ContainerError::NetworkCreationFailed(msg) => {
                write!(f, "Failed to create Docker network: {}", msg)
            }
            ContainerError::NetworkOperationFailed(msg) => {
                write!(f, "Network operation failed: {}", msg)
            }
        }
    }
}
